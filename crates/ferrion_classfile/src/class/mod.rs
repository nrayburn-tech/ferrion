//! The class file model: a lazy view over a class file's bytes (docs/design/classfile-format.md).
//! Section numbers are from the JVM Specification, Java SE 27 edition, chapter 4.
//!
//! [`ClassFile::parse`] walks the structure once and records where each constant pool entry,
//! member, and attribute table starts. After that, iterating members and attributes can't fail;
//! resolving a constant, decoding text, and decoding an attribute's contents can, and return
//! `Result`.

mod annotation;
mod attribute;
mod module;

use std::sync::Arc;

pub use annotation::{
    Annotation, Annotations, ElementValue, ElementValuePairs, ElementValues, ParameterAnnotations,
    TypeAnnotation, TypeAnnotationTarget, TypeAnnotations, TypePathStep,
};
pub use attribute::{
    Attribute, ClassRefs, ConstantValue, EnclosingMethod, InnerClass, InnerClasses,
    MethodParameter, ModuleAttribute, ModuleExport, ModuleProvides, ModuleRequires,
    RecordComponent, RecordComponents,
};
pub use module::{Exports, ModuleDescriptor, Provides, Requires};

use crate::cursor::Cursor;
use crate::error::{Error, Result};
use crate::text::{JavaStr, Name};

/// The newest class file major version Ferrion reads: the latest release's.
pub const MAX_MAJOR_VERSION: u16 = 44 + ferrion_java_version::LATEST_RELEASE as u16;

/// The oldest major version, Java 1.0.2's.
pub const MIN_MAJOR_VERSION: u16 = 45;

/// The largest class file read, the containers' limit.
pub const MAX_CLASS_SIZE: u64 = 64 << 20;

/// A class file's version (4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

impl Version {
    /// Whether the class depends on preview features (minor 0xFFFF, major 56 and later).
    pub fn is_preview(&self) -> bool {
        self.major >= 56 && self.minor == 0xffff
    }
}

/// Access flags of a class, member, inner class, parameter, or module item, as stored. The
/// constants name the bits of JVMS tables 4.1-B, 4.5-A, 4.6-A, 4.7.6-A, and 4.7.25.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessFlags(pub u16);

impl AccessFlags {
    pub const PUBLIC: u16 = 0x0001;
    pub const PRIVATE: u16 = 0x0002;
    pub const PROTECTED: u16 = 0x0004;
    pub const STATIC: u16 = 0x0008;
    pub const FINAL: u16 = 0x0010;
    /// `ACC_SUPER` on classes, `ACC_SYNCHRONIZED` on methods, `ACC_OPEN` on modules,
    /// `ACC_TRANSITIVE` on requires.
    pub const SUPER: u16 = 0x0020;
    pub const SYNCHRONIZED: u16 = 0x0020;
    pub const OPEN: u16 = 0x0020;
    pub const TRANSITIVE: u16 = 0x0020;
    /// `ACC_VOLATILE` on fields, `ACC_BRIDGE` on methods, `ACC_STATIC_PHASE` on requires.
    pub const VOLATILE: u16 = 0x0040;
    pub const BRIDGE: u16 = 0x0040;
    pub const STATIC_PHASE: u16 = 0x0040;
    /// `ACC_TRANSIENT` on fields, `ACC_VARARGS` on methods.
    pub const TRANSIENT: u16 = 0x0080;
    pub const VARARGS: u16 = 0x0080;
    pub const NATIVE: u16 = 0x0100;
    pub const INTERFACE: u16 = 0x0200;
    pub const ABSTRACT: u16 = 0x0400;
    pub const STRICT: u16 = 0x0800;
    pub const SYNTHETIC: u16 = 0x1000;
    pub const ANNOTATION: u16 = 0x2000;
    pub const ENUM: u16 = 0x4000;
    /// `ACC_MODULE` on classes, `ACC_MANDATED` on parameters and module items.
    pub const MODULE: u16 = 0x8000;
    pub const MANDATED: u16 = 0x8000;

    pub fn contains(self, flag: u16) -> bool {
        self.0 & flag == flag
    }
}

/// A constant pool entry (4.4), with references left as indexes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Constant<'a> {
    Utf8(Name<'a>),
    Integer(i32),
    /// The bits, so NaN payloads survive.
    Float(u32),
    Long(i64),
    Double(u64),
    Class {
        name: u16,
    },
    String {
        string: u16,
    },
    Fieldref {
        class: u16,
        name_and_type: u16,
    },
    Methodref {
        class: u16,
        name_and_type: u16,
    },
    InterfaceMethodref {
        class: u16,
        name_and_type: u16,
    },
    NameAndType {
        name: u16,
        descriptor: u16,
    },
    MethodHandle {
        kind: u8,
        reference: u16,
    },
    MethodType {
        descriptor: u16,
    },
    Dynamic {
        bootstrap: u16,
        name_and_type: u16,
    },
    InvokeDynamic {
        bootstrap: u16,
        name_and_type: u16,
    },
    Module {
        name: u16,
    },
    Package {
        name: u16,
    },
}

mod tag {
    pub const UTF8: u8 = 1;
    pub const INTEGER: u8 = 3;
    pub const FLOAT: u8 = 4;
    pub const LONG: u8 = 5;
    pub const DOUBLE: u8 = 6;
    pub const CLASS: u8 = 7;
    pub const STRING: u8 = 8;
    pub const FIELDREF: u8 = 9;
    pub const METHODREF: u8 = 10;
    pub const INTERFACE_METHODREF: u8 = 11;
    pub const NAME_AND_TYPE: u8 = 12;
    pub const METHOD_HANDLE: u8 = 15;
    pub const METHOD_TYPE: u8 = 16;
    pub const DYNAMIC: u8 = 17;
    pub const INVOKE_DYNAMIC: u8 = 18;
    pub const MODULE: u8 = 19;
    pub const PACKAGE: u8 = 20;
}

/// The number of bytes after the tag of a constant pool entry with a fixed size.
fn fixed_size(tag: u8) -> Option<usize> {
    Some(match tag {
        tag::INTEGER | tag::FLOAT => 4,
        tag::LONG | tag::DOUBLE => 8,
        tag::CLASS | tag::STRING | tag::METHOD_TYPE | tag::MODULE | tag::PACKAGE => 2,
        tag::FIELDREF
        | tag::METHODREF
        | tag::INTERFACE_METHODREF
        | tag::NAME_AND_TYPE
        | tag::DYNAMIC
        | tag::INVOKE_DYNAMIC => 4,
        tag::METHOD_HANDLE => 3,
        _ => return None,
    })
}

fn tag_name(tag: u8) -> &'static str {
    match tag {
        tag::UTF8 => "Utf8",
        tag::INTEGER => "Integer",
        tag::FLOAT => "Float",
        tag::LONG => "Long",
        tag::DOUBLE => "Double",
        tag::CLASS => "Class",
        tag::STRING => "String",
        tag::FIELDREF => "Fieldref",
        tag::METHODREF => "Methodref",
        tag::INTERFACE_METHODREF => "InterfaceMethodref",
        tag::NAME_AND_TYPE => "NameAndType",
        tag::METHOD_HANDLE => "MethodHandle",
        tag::METHOD_TYPE => "MethodType",
        tag::DYNAMIC => "Dynamic",
        tag::INVOKE_DYNAMIC => "InvokeDynamic",
        tag::MODULE => "Module",
        tag::PACKAGE => "Package",
        _ => "unknown",
    }
}

/// A parsed class file. Holds its bytes; cheap to keep, and `Send + Sync`.
#[derive(Clone)]
pub struct ClassFile {
    bytes: Arc<[u8]>,
    version: Version,
    /// The offset of each constant pool entry's tag; 0 for index 0 and the slot after a `Long`
    /// or `Double`, which can't be used.
    pool: Box<[u32]>,
    /// The offset of `access_flags`.
    header: u32,
    interface_count: u16,
    fields: Box<[u32]>,
    methods: Box<[u32]>,
    /// The offset of the class's `attributes_count`.
    attributes: u32,
    /// Where the class file ends; bytes after it are reported by [`ClassFile::trailing_bytes`].
    end: u32,
}

impl std::fmt::Debug for ClassFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassFile")
            .field("version", &self.version)
            .field("this_class", &self.this_class().ok())
            .finish_non_exhaustive()
    }
}

impl ClassFile {
    /// Checks the magic, the version, and the structure (every constant pool entry, member, and
    /// attribute fits in its parent), and indexes it. Never panics.
    pub fn parse(bytes: Arc<[u8]>) -> Result<ClassFile> {
        if bytes.len() as u64 > MAX_CLASS_SIZE {
            return Err(Error::too_large(format!(
                "a class file of {} bytes is above the limit of {MAX_CLASS_SIZE}",
                bytes.len()
            )));
        }
        let mut cursor = Cursor::new(&bytes, 0);
        let magic = cursor.u32()?;
        if magic != 0xcafe_babe {
            return Err(Error::malformed(0, format!("bad magic {magic:#010x}")));
        }
        let minor = cursor.u16()?;
        let major = cursor.u16()?;
        if !(MIN_MAJOR_VERSION..=MAX_MAJOR_VERSION).contains(&major) {
            return Err(Error::unsupported(format!(
                "class file version {major}.{minor} isn't supported; the newest is \
                 {MAX_MAJOR_VERSION}"
            ))
            .at(6));
        }
        let pool = parse_pool(&mut cursor)?;
        let header = cursor.pos() as u32;
        cursor.skip(6)?; // access_flags, this_class, super_class
        let interface_count = cursor.u16()?;
        cursor.skip(2 * usize::from(interface_count))?;
        let fields = parse_members(&mut cursor)?;
        let methods = parse_members(&mut cursor)?;
        let attributes = cursor.pos() as u32;
        skip_attributes(&mut cursor)?;
        Ok(ClassFile {
            version: Version { major, minor },
            pool,
            header,
            interface_count,
            fields,
            methods,
            attributes,
            end: cursor.pos() as u32,
            bytes,
        })
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    /// `Malformed` when bytes follow the class's last attribute (4.8 allows none). The rest of
    /// the class is still usable.
    pub fn trailing_bytes(&self) -> Result<()> {
        let extra = self.bytes.len() - self.end as usize;
        if extra > 0 {
            return Err(Error::malformed(
                u64::from(self.end),
                format!("{extra} bytes after the last attribute"),
            ));
        }
        Ok(())
    }

    pub fn version(&self) -> Version {
        self.version
    }

    /// The number of constant pool slots, including the unusable index 0.
    pub fn constant_pool_count(&self) -> u16 {
        self.pool.len() as u16
    }

    fn u16_at(&self, offset: usize) -> u16 {
        // Within the structure checked by `parse`.
        u16::from_be_bytes([self.bytes[offset], self.bytes[offset + 1]])
    }

    pub fn access(&self) -> AccessFlags {
        AccessFlags(self.u16_at(self.header as usize))
    }

    /// The `this_class` name, in internal form.
    pub fn this_class(&self) -> Result<Name<'_>> {
        self.class_name(self.u16_at(self.header as usize + 2))
    }

    /// The superclass, or `None` when `super_class` is 0 (`java/lang/Object` and `module-info`).
    pub fn super_class(&self) -> Result<Option<Name<'_>>> {
        match self.u16_at(self.header as usize + 4) {
            0 => Ok(None),
            index => self.class_name(index).map(Some),
        }
    }

    pub fn interfaces(&self) -> ClassRefs<'_> {
        let start = self.header as usize + 8;
        ClassRefs::new(
            self,
            &self.bytes[start..start + 2 * usize::from(self.interface_count)],
        )
    }

    pub fn fields(&self) -> Members<'_> {
        Members {
            class: self,
            offsets: self.fields.iter(),
        }
    }

    pub fn methods(&self) -> Members<'_> {
        Members {
            class: self,
            offsets: self.methods.iter(),
        }
    }

    pub fn attributes(&self) -> Attributes<'_> {
        Attributes::at(self, self.attributes as usize)
    }

    // Constant pool access.

    fn entry(&self, index: u16) -> Result<(u8, usize)> {
        match self.pool.get(usize::from(index)) {
            Some(&offset) if offset != 0 => {
                let offset = offset as usize;
                Ok((self.bytes[offset], offset + 1))
            }
            _ => Err(Error::malformed(
                0,
                format!(
                    "constant pool index {index} isn't usable (the pool has {} slots)",
                    self.pool.len()
                ),
            )),
        }
    }

    fn expect(&self, index: u16, expected: u8) -> Result<usize> {
        let (tag, offset) = self.entry(index)?;
        if tag != expected {
            return Err(Error::malformed(
                offset as u64 - 1,
                format!(
                    "constant pool index {index} is {}, but {} was expected",
                    tag_name(tag),
                    tag_name(expected)
                ),
            ));
        }
        Ok(offset)
    }

    /// The constant pool entry at `index`.
    pub fn constant(&self, index: u16) -> Result<Constant<'_>> {
        let (tag, offset) = self.entry(index)?;
        let b = &self.bytes[offset..];
        let u2 = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        let u4 = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let u8_ = |i: usize| u64::from(u4(i)) << 32 | u64::from(u4(i + 4));
        Ok(match tag {
            tag::UTF8 => Constant::Utf8(Name(&b[2..2 + usize::from(u2(0))])),
            tag::INTEGER => Constant::Integer(u4(0) as i32),
            tag::FLOAT => Constant::Float(u4(0)),
            tag::LONG => Constant::Long(u8_(0) as i64),
            tag::DOUBLE => Constant::Double(u8_(0)),
            tag::CLASS => Constant::Class { name: u2(0) },
            tag::STRING => Constant::String { string: u2(0) },
            tag::FIELDREF => Constant::Fieldref {
                class: u2(0),
                name_and_type: u2(2),
            },
            tag::METHODREF => Constant::Methodref {
                class: u2(0),
                name_and_type: u2(2),
            },
            tag::INTERFACE_METHODREF => Constant::InterfaceMethodref {
                class: u2(0),
                name_and_type: u2(2),
            },
            tag::NAME_AND_TYPE => Constant::NameAndType {
                name: u2(0),
                descriptor: u2(2),
            },
            tag::METHOD_HANDLE => Constant::MethodHandle {
                kind: b[0],
                reference: u2(1),
            },
            tag::METHOD_TYPE => Constant::MethodType { descriptor: u2(0) },
            tag::DYNAMIC => Constant::Dynamic {
                bootstrap: u2(0),
                name_and_type: u2(2),
            },
            tag::INVOKE_DYNAMIC => Constant::InvokeDynamic {
                bootstrap: u2(0),
                name_and_type: u2(2),
            },
            tag::MODULE => Constant::Module { name: u2(0) },
            tag::PACKAGE => Constant::Package { name: u2(0) },
            _ => unreachable!("parse checked every tag"),
        })
    }

    /// The text of the `CONSTANT_Utf8` entry at `index`.
    pub fn utf8(&self, index: u16) -> Result<Name<'_>> {
        let offset = self.expect(index, tag::UTF8)?;
        let len = usize::from(self.u16_at(offset));
        Ok(Name(&self.bytes[offset + 2..offset + 2 + len]))
    }

    /// The `CONSTANT_Utf8` entry at `index`, or `None` for index 0.
    pub fn optional_utf8(&self, index: u16) -> Result<Option<Name<'_>>> {
        match index {
            0 => Ok(None),
            index => self.utf8(index).map(Some),
        }
    }

    fn indirect(&self, index: u16, expected: u8) -> Result<Name<'_>> {
        let offset = self.expect(index, expected)?;
        self.utf8(self.u16_at(offset))
    }

    /// The name of the `CONSTANT_Class` entry at `index`, in internal form (or an array
    /// descriptor).
    pub fn class_name(&self, index: u16) -> Result<Name<'_>> {
        self.indirect(index, tag::CLASS)
    }

    /// The name of the `CONSTANT_Module` entry at `index`.
    pub fn module_name(&self, index: u16) -> Result<Name<'_>> {
        self.indirect(index, tag::MODULE)
    }

    /// The name of the `CONSTANT_Package` entry at `index`, in internal form.
    pub fn package_name(&self, index: u16) -> Result<Name<'_>> {
        self.indirect(index, tag::PACKAGE)
    }

    /// The text of the `CONSTANT_String` entry at `index`.
    pub fn string(&self, index: u16) -> Result<JavaStr<'_>> {
        Ok(JavaStr(self.indirect(index, tag::STRING)?.0))
    }

    pub(crate) fn integer(&self, index: u16) -> Result<i32> {
        match self.constant(index)? {
            Constant::Integer(value) => Ok(value),
            _ => Err(self.wrong(index, "Integer")),
        }
    }

    fn wrong(&self, index: u16, expected: &str) -> Error {
        let tag = self.entry(index).map_or(0, |(tag, _)| tag);
        Error::malformed(
            0,
            format!(
                "constant pool index {index} is {}, but {expected} was expected",
                tag_name(tag)
            ),
        )
    }

    /// The `Signature` attribute's text, if the class has one.
    pub fn signature(&self) -> Result<Option<Name<'_>>> {
        self.attributes().signature()
    }

    /// Every `module-info` class's module descriptor, decoded from its attributes.
    pub fn module_descriptor(&self) -> Result<ModuleDescriptor> {
        ModuleDescriptor::from_class(self)
    }
}

fn parse_pool(cursor: &mut Cursor<'_>) -> Result<Box<[u32]>> {
    let count = cursor.u16()?;
    if count == 0 {
        return Err(cursor.error("constant_pool_count is 0"));
    }
    let mut pool = vec![0u32; usize::from(count)];
    let mut index = 1;
    while index < usize::from(count) {
        let offset = cursor.pos();
        let tag = cursor.u8()?;
        pool[index] = offset as u32;
        if tag == tag::UTF8 {
            let len = cursor.u16()?;
            cursor.skip(usize::from(len))?;
        } else if let Some(size) = fixed_size(tag) {
            cursor.skip(size)?;
        } else {
            return Err(Error::malformed(
                offset as u64,
                format!("constant pool entry {index} has unknown tag {tag}"),
            ));
        }
        index += 1;
        if tag == tag::LONG || tag == tag::DOUBLE {
            if index >= usize::from(count) {
                return Err(Error::malformed(
                    offset as u64,
                    format!(
                        "constant pool entry {} is a {} in the last slot",
                        index - 1,
                        tag_name(tag)
                    ),
                ));
            }
            // The next slot is unusable (4.4.5).
            index += 1;
        }
    }
    Ok(pool.into_boxed_slice())
}

fn parse_members(cursor: &mut Cursor<'_>) -> Result<Box<[u32]>> {
    let count = cursor.u16()?;
    let mut offsets = Vec::with_capacity(usize::from(count).min(cursor.remaining() / 8));
    for _ in 0..count {
        offsets.push(cursor.pos() as u32);
        cursor.skip(6)?; // access_flags, name_index, descriptor_index
        skip_attributes(cursor)?;
    }
    Ok(offsets.into_boxed_slice())
}

pub(crate) fn skip_attributes(cursor: &mut Cursor<'_>) -> Result<()> {
    let count = cursor.u16()?;
    for _ in 0..count {
        cursor.skip(2)?;
        let len = cursor.u32()?;
        cursor.skip(len as usize)?;
    }
    Ok(())
}

/// A field or method (4.5, 4.6).
#[derive(Clone, Copy)]
pub struct Member<'a> {
    class: &'a ClassFile,
    offset: usize,
}

impl<'a> Member<'a> {
    pub fn access(&self) -> AccessFlags {
        AccessFlags(self.class.u16_at(self.offset))
    }

    pub fn name(&self) -> Result<Name<'a>> {
        self.class.utf8(self.class.u16_at(self.offset + 2))
    }

    /// The field or method descriptor (4.3.2, 4.3.3), parsed with [`crate::descriptor`].
    pub fn descriptor(&self) -> Result<Name<'a>> {
        self.class.utf8(self.class.u16_at(self.offset + 4))
    }

    pub fn attributes(&self) -> Attributes<'a> {
        Attributes::at(self.class, self.offset + 6)
    }

    pub fn signature(&self) -> Result<Option<Name<'a>>> {
        self.attributes().signature()
    }
}

impl std::fmt::Debug for Member<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Member")
            .field("name", &self.name().ok())
            .field("descriptor", &self.descriptor().ok())
            .finish_non_exhaustive()
    }
}

/// The fields or the methods of a class.
#[derive(Clone)]
pub struct Members<'a> {
    class: &'a ClassFile,
    offsets: std::slice::Iter<'a, u32>,
}

impl<'a> Iterator for Members<'a> {
    type Item = Member<'a>;

    fn next(&mut self) -> Option<Member<'a>> {
        let &offset = self.offsets.next()?;
        Some(Member {
            class: self.class,
            offset: offset as usize,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.offsets.size_hint()
    }
}

impl ExactSizeIterator for Members<'_> {}

/// An attribute's header and contents, not yet decoded.
#[derive(Clone, Copy)]
pub struct AttributeInfo<'a> {
    class: &'a ClassFile,
    name_index: u16,
    /// The offset of the contents in the class file.
    offset: usize,
    len: usize,
}

impl<'a> AttributeInfo<'a> {
    pub fn name(&self) -> Result<Name<'a>> {
        self.class.utf8(self.name_index)
    }

    /// The contents, after the name and length.
    pub fn bytes(&self) -> &'a [u8] {
        &self.class.bytes[self.offset..self.offset + self.len]
    }

    /// The offset of the contents in the class file.
    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn class(&self) -> &'a ClassFile {
        self.class
    }

    /// Decodes the contents by the attribute's name. Attributes Ferrion doesn't decode are
    /// returned as [`Attribute::Other`] with their name and bytes.
    pub fn decode(&self) -> Result<Attribute<'a>> {
        attribute::decode(self)
    }
}

impl std::fmt::Debug for AttributeInfo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttributeInfo")
            .field("name", &self.name().ok())
            .field("len", &self.len)
            .finish()
    }
}

/// A table of attributes: of the class, a member, or a record component.
#[derive(Clone)]
pub struct Attributes<'a> {
    class: &'a ClassFile,
    /// The offset of the next attribute header.
    offset: usize,
    remaining: u16,
}

impl<'a> Attributes<'a> {
    /// The table whose `attributes_count` is at `offset`, already checked by `parse`.
    fn at(class: &'a ClassFile, offset: usize) -> Attributes<'a> {
        Attributes {
            class,
            offset: offset + 2,
            remaining: class.u16_at(offset),
        }
    }

    /// The first attribute named `name`.
    pub fn named(&self, name: &str) -> Option<AttributeInfo<'a>> {
        self.clone()
            .find(|info: &AttributeInfo<'a>| matches!(info.name(), Ok(n) if n.eq_str(name)))
    }

    /// The text of the `Signature` attribute, if there is one.
    pub fn signature(&self) -> Result<Option<Name<'a>>> {
        match self.named("Signature") {
            None => Ok(None),
            Some(info) => match info.decode()? {
                Attribute::Signature(name) => Ok(Some(name)),
                _ => Err(Error::malformed(
                    info.offset as u64,
                    "a Signature attribute that isn't one",
                )),
            },
        }
    }
}

impl<'a> Iterator for Attributes<'a> {
    type Item = AttributeInfo<'a>;

    fn next(&mut self) -> Option<AttributeInfo<'a>> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let b = &self.class.bytes;
        let o = self.offset;
        let name_index = u16::from_be_bytes([b[o], b[o + 1]]);
        let len = u32::from_be_bytes([b[o + 2], b[o + 3], b[o + 4], b[o + 5]]) as usize;
        self.offset = o + 6 + len;
        Some(AttributeInfo {
            class: self.class,
            name_index,
            offset: o + 6,
            len,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            usize::from(self.remaining),
            Some(usize::from(self.remaining)),
        )
    }
}

impl ExactSizeIterator for Attributes<'_> {}

#[cfg(test)]
pub(crate) mod tests;
