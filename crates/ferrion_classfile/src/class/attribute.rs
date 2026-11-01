//! Decoding attributes (4.7). Each decoder checks that the attribute's contents have exactly the
//! length the structure needs, so a decoded attribute's iterators can't run past it.

use super::annotation::{self, Annotations, ElementValue, ParameterAnnotations, TypeAnnotations};
use super::{AccessFlags, AttributeInfo, ClassFile, Constant};
use crate::cursor::Cursor;
use crate::error::Result;
use crate::text::{JavaStr, Name};

/// A decoded attribute.
#[derive(Clone, Debug)]
pub enum Attribute<'a> {
    /// 4.7.2, on fields.
    ConstantValue(ConstantValue<'a>),
    /// 4.7.5, on methods: the thrown classes.
    Exceptions(ClassRefs<'a>),
    /// 4.7.6, on classes.
    InnerClasses(InnerClasses<'a>),
    /// 4.7.7, on local and anonymous classes.
    EnclosingMethod(EnclosingMethod<'a>),
    /// 4.7.8.
    Synthetic,
    /// 4.7.9: parse the text with [`crate::signature`].
    Signature(Name<'a>),
    /// 4.7.10.
    SourceFile(Name<'a>),
    /// 4.7.15.
    Deprecated,
    /// 4.7.16 and 4.7.17.
    Annotations {
        visible: bool,
        annotations: Annotations<'a>,
    },
    /// 4.7.18 and 4.7.19.
    ParameterAnnotations {
        visible: bool,
        parameters: ParameterAnnotations<'a>,
    },
    /// 4.7.20 and 4.7.21.
    TypeAnnotations {
        visible: bool,
        annotations: TypeAnnotations<'a>,
    },
    /// 4.7.22.
    AnnotationDefault(ElementValue<'a>),
    /// 4.7.24.
    MethodParameters(Vec<MethodParameter<'a>>),
    /// 4.7.25.
    Module(ModuleAttribute<'a>),
    /// 4.7.26: the module's packages.
    ModulePackages(ClassRefs<'a>),
    /// 4.7.27.
    ModuleMainClass(Name<'a>),
    /// 4.7.28.
    NestHost(Name<'a>),
    /// 4.7.29.
    NestMembers(ClassRefs<'a>),
    /// 4.7.30.
    Record(RecordComponents<'a>),
    /// 4.7.31.
    PermittedSubclasses(ClassRefs<'a>),
    /// Not in the JVMS; seen in the JDK's module descriptors. The raw u2 value: its low bit means
    /// "not resolved by default" (docs/design/classfile.md, "Module visibility").
    ModuleResolution(u16),
    /// Any attribute not decoded, kept with its name and bytes: `Code` (skipped with everything
    /// inside it), `BootstrapMethods`, `SourceDebugExtension`, `ModuleTarget`, `ModuleHashes`,
    /// and attributes the JVMS doesn't define.
    Other { name: Name<'a>, bytes: &'a [u8] },
}

/// A field's constant value (4.7.2). `int`, `short`, `char`, `byte`, and `boolean` fields all
/// hold an `Integer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConstantValue<'a> {
    Int(i32),
    Long(i64),
    /// The bits, so NaN payloads survive.
    Float(u32),
    Double(u64),
    String(JavaStr<'a>),
}

impl<'a> ConstantValue<'a> {
    pub(crate) fn from_pool(class: &'a ClassFile, index: u16) -> Result<ConstantValue<'a>> {
        Ok(match class.constant(index)? {
            Constant::Integer(value) => ConstantValue::Int(value),
            Constant::Long(value) => ConstantValue::Long(value),
            Constant::Float(bits) => ConstantValue::Float(bits),
            Constant::Double(bits) => ConstantValue::Double(bits),
            Constant::String { .. } => ConstantValue::String(class.string(index)?),
            _ => return Err(class.wrong(index, "a constant value")),
        })
    }
}

/// What a list of u2 constant pool indexes refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefKind {
    Class,
    Package,
}

/// A list of class (or, for `ModulePackages`, package) references, resolved one at a time.
#[derive(Clone)]
pub struct ClassRefs<'a> {
    class: &'a ClassFile,
    data: &'a [u8],
    kind: RefKind,
}

impl<'a> ClassRefs<'a> {
    pub(crate) fn new(class: &'a ClassFile, data: &'a [u8]) -> ClassRefs<'a> {
        ClassRefs {
            class,
            data,
            kind: RefKind::Class,
        }
    }

    pub fn len(&self) -> usize {
        self.data.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl<'a> Iterator for ClassRefs<'a> {
    type Item = Result<Name<'a>>;

    fn next(&mut self) -> Option<Result<Name<'a>>> {
        let (index, rest) = self.data.split_first_chunk::<2>()?;
        self.data = rest;
        let index = u16::from_be_bytes(*index);
        Some(match self.kind {
            RefKind::Class => self.class.class_name(index),
            RefKind::Package => self.class.package_name(index),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

impl std::fmt::Debug for ClassRefs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// One entry of `InnerClasses` (4.7.6).
#[derive(Clone, Copy, Debug)]
pub struct InnerClass<'a> {
    pub inner: Name<'a>,
    /// `None` for top-level, local, and anonymous classes.
    pub outer: Option<Name<'a>>,
    /// The simple name; `None` for anonymous classes.
    pub name: Option<Name<'a>>,
    /// The flags of table 4.7.6-A, which the nested class's own flags don't carry.
    pub access: AccessFlags,
}

/// The entries of `InnerClasses`, resolved one at a time.
#[derive(Clone)]
pub struct InnerClasses<'a> {
    class: &'a ClassFile,
    data: &'a [u8],
}

impl InnerClasses<'_> {
    pub fn len(&self) -> usize {
        self.data.len() / 8
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl<'a> Iterator for InnerClasses<'a> {
    type Item = Result<InnerClass<'a>>;

    fn next(&mut self) -> Option<Result<InnerClass<'a>>> {
        let (entry, rest) = self.data.split_first_chunk::<8>()?;
        self.data = rest;
        let u2 = |i: usize| u16::from_be_bytes([entry[i], entry[i + 1]]);
        let class = self.class;
        Some((|| {
            Ok(InnerClass {
                inner: class.class_name(u2(0))?,
                outer: match u2(2) {
                    0 => None,
                    index => Some(class.class_name(index)?),
                },
                name: class.optional_utf8(u2(4))?,
                access: AccessFlags(u2(6)),
            })
        })())
    }
}

impl std::fmt::Debug for InnerClasses<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// `EnclosingMethod` (4.7.7).
#[derive(Clone, Copy, Debug)]
pub struct EnclosingMethod<'a> {
    pub class: Name<'a>,
    /// The method's name and descriptor; `None` when the class isn't enclosed by a method (an
    /// initializer, for example).
    pub method: Option<(Name<'a>, Name<'a>)>,
}

/// One parameter of `MethodParameters` (4.7.24).
#[derive(Clone, Copy, Debug)]
pub struct MethodParameter<'a> {
    /// `None` when the class file gives no name.
    pub name: Option<Name<'a>>,
    /// `final`, synthetic, and mandated.
    pub access: AccessFlags,
}

/// The `Module` attribute (4.7.25), decoded.
#[derive(Clone, Debug)]
pub struct ModuleAttribute<'a> {
    pub name: Name<'a>,
    pub flags: AccessFlags,
    pub version: Option<Name<'a>>,
    pub requires: Vec<ModuleRequires<'a>>,
    pub exports: Vec<ModuleExport<'a>>,
    pub opens: Vec<ModuleExport<'a>>,
    /// Service interfaces, in internal form.
    pub uses: Vec<Name<'a>>,
    pub provides: Vec<ModuleProvides<'a>>,
}

#[derive(Clone, Debug)]
pub struct ModuleRequires<'a> {
    pub module: Name<'a>,
    pub flags: AccessFlags,
    pub version: Option<Name<'a>>,
}

/// An `exports` or `opens` entry. `to` is empty when unqualified.
#[derive(Clone, Debug)]
pub struct ModuleExport<'a> {
    /// In internal form (`java/lang`).
    pub package: Name<'a>,
    pub flags: AccessFlags,
    pub to: Vec<Name<'a>>,
}

#[derive(Clone, Debug)]
pub struct ModuleProvides<'a> {
    pub service: Name<'a>,
    pub with: Vec<Name<'a>>,
}

/// A record component (4.7.30).
#[derive(Clone)]
pub struct RecordComponent<'a> {
    class: &'a ClassFile,
    name: u16,
    descriptor: u16,
    attributes: usize,
}

impl<'a> RecordComponent<'a> {
    pub fn name(&self) -> Result<Name<'a>> {
        self.class.utf8(self.name)
    }

    pub fn descriptor(&self) -> Result<Name<'a>> {
        self.class.utf8(self.descriptor)
    }

    pub fn attributes(&self) -> super::Attributes<'a> {
        super::Attributes::at(self.class, self.attributes)
    }

    pub fn signature(&self) -> Result<Option<Name<'a>>> {
        self.attributes().signature()
    }
}

impl std::fmt::Debug for RecordComponent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordComponent")
            .field("name", &self.name().ok())
            .field("descriptor", &self.descriptor().ok())
            .finish_non_exhaustive()
    }
}

/// The components of a `Record` attribute.
#[derive(Clone)]
pub struct RecordComponents<'a> {
    class: &'a ClassFile,
    offsets: std::vec::IntoIter<usize>,
}

impl<'a> Iterator for RecordComponents<'a> {
    type Item = RecordComponent<'a>;

    fn next(&mut self) -> Option<RecordComponent<'a>> {
        let offset = self.offsets.next()?;
        let b = &self.class.bytes;
        Some(RecordComponent {
            class: self.class,
            name: u16::from_be_bytes([b[offset], b[offset + 1]]),
            descriptor: u16::from_be_bytes([b[offset + 2], b[offset + 3]]),
            attributes: offset + 4,
        })
    }
}

impl ExactSizeIterator for RecordComponents<'_> {}

impl std::fmt::Debug for RecordComponents<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// Decodes `info` by its name.
pub(super) fn decode<'a>(info: &AttributeInfo<'a>) -> Result<Attribute<'a>> {
    let name = info.name()?;
    let class = info.class;
    let bytes = info.bytes();
    let base = info.offset as u64;
    let mut cursor = Cursor::new(bytes, base);
    let c = &mut cursor;
    let attribute = match name.as_bytes() {
        b"ConstantValue" => Attribute::ConstantValue(ConstantValue::from_pool(class, c.u16()?)?),
        b"Exceptions" => Attribute::Exceptions(refs(class, c, RefKind::Class)?),
        b"InnerClasses" => {
            let count = c.u16()?;
            let data = c.bytes(8 * usize::from(count))?;
            Attribute::InnerClasses(InnerClasses { class, data })
        }
        b"EnclosingMethod" => {
            let enclosing = class.class_name(c.u16()?)?;
            let method = match c.u16()? {
                0 => None,
                index => match class.constant(index)? {
                    Constant::NameAndType { name, descriptor } => {
                        Some((class.utf8(name)?, class.utf8(descriptor)?))
                    }
                    _ => return Err(class.wrong(index, "NameAndType")),
                },
            };
            Attribute::EnclosingMethod(EnclosingMethod {
                class: enclosing,
                method,
            })
        }
        b"Synthetic" => Attribute::Synthetic,
        b"Deprecated" => Attribute::Deprecated,
        b"Signature" => Attribute::Signature(class.utf8(c.u16()?)?),
        b"SourceFile" => Attribute::SourceFile(class.utf8(c.u16()?)?),
        b"RuntimeVisibleAnnotations" | b"RuntimeInvisibleAnnotations" => Attribute::Annotations {
            visible: name.as_bytes() == b"RuntimeVisibleAnnotations",
            annotations: annotation::annotations(class, c)?,
        },
        b"RuntimeVisibleParameterAnnotations" | b"RuntimeInvisibleParameterAnnotations" => {
            Attribute::ParameterAnnotations {
                visible: name.as_bytes() == b"RuntimeVisibleParameterAnnotations",
                parameters: annotation::parameter_annotations(class, c)?,
            }
        }
        b"RuntimeVisibleTypeAnnotations" | b"RuntimeInvisibleTypeAnnotations" => {
            Attribute::TypeAnnotations {
                visible: name.as_bytes() == b"RuntimeVisibleTypeAnnotations",
                annotations: annotation::type_annotations(class, c)?,
            }
        }
        b"AnnotationDefault" => Attribute::AnnotationDefault(annotation::element_value(class, c)?),
        b"MethodParameters" => {
            let count = c.u8()?;
            let mut parameters = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                parameters.push(MethodParameter {
                    name: class.optional_utf8(c.u16()?)?,
                    access: AccessFlags(c.u16()?),
                });
            }
            Attribute::MethodParameters(parameters)
        }
        b"Module" => Attribute::Module(module(class, c)?),
        b"ModulePackages" => Attribute::ModulePackages(refs(class, c, RefKind::Package)?),
        b"ModuleMainClass" => Attribute::ModuleMainClass(class.class_name(c.u16()?)?),
        b"NestHost" => Attribute::NestHost(class.class_name(c.u16()?)?),
        b"NestMembers" => Attribute::NestMembers(refs(class, c, RefKind::Class)?),
        b"PermittedSubclasses" => Attribute::PermittedSubclasses(refs(class, c, RefKind::Class)?),
        b"Record" => {
            let count = c.u16()?;
            let mut offsets = Vec::with_capacity(usize::from(count).min(c.remaining() / 6));
            for _ in 0..count {
                offsets.push(info.offset + c.pos());
                c.skip(4)?;
                super::skip_attributes(c)?;
            }
            Attribute::Record(RecordComponents {
                class,
                offsets: offsets.into_iter(),
            })
        }
        b"ModuleResolution" => Attribute::ModuleResolution(c.u16()?),
        _ => return Ok(Attribute::Other { name, bytes }),
    };
    if !cursor.is_empty() {
        return Err(cursor.error(format!(
            "{} attribute has {} bytes after its contents",
            String::from_utf8_lossy(name.as_bytes()),
            cursor.remaining()
        )));
    }
    Ok(attribute)
}

/// A u2 count followed by that many u2 indexes.
fn refs<'a>(class: &'a ClassFile, c: &mut Cursor<'a>, kind: RefKind) -> Result<ClassRefs<'a>> {
    let count = c.u16()?;
    let data = c.bytes(2 * usize::from(count))?;
    Ok(ClassRefs { class, data, kind })
}

fn names<'a>(
    c: &mut Cursor<'a>,
    mut resolve: impl FnMut(u16) -> Result<Name<'a>>,
) -> Result<Vec<Name<'a>>> {
    let count = c.u16()?;
    let mut names = Vec::with_capacity(usize::from(count).min(c.remaining() / 2));
    for _ in 0..count {
        names.push(resolve(c.u16()?)?);
    }
    Ok(names)
}

fn module<'a>(class: &'a ClassFile, c: &mut Cursor<'a>) -> Result<ModuleAttribute<'a>> {
    let name = class.module_name(c.u16()?)?;
    let flags = AccessFlags(c.u16()?);
    let version = class.optional_utf8(c.u16()?)?;
    let count = c.u16()?;
    let mut requires = Vec::with_capacity(usize::from(count).min(c.remaining() / 6));
    for _ in 0..count {
        requires.push(ModuleRequires {
            module: class.module_name(c.u16()?)?,
            flags: AccessFlags(c.u16()?),
            version: class.optional_utf8(c.u16()?)?,
        });
    }
    let exports_or_opens = |c: &mut Cursor<'a>| -> Result<Vec<ModuleExport<'a>>> {
        let count = c.u16()?;
        let mut list = Vec::with_capacity(usize::from(count).min(c.remaining() / 6));
        for _ in 0..count {
            list.push(ModuleExport {
                package: class.package_name(c.u16()?)?,
                flags: AccessFlags(c.u16()?),
                to: names(c, |index| class.module_name(index))?,
            });
        }
        Ok(list)
    };
    let exports = exports_or_opens(c)?;
    let opens = exports_or_opens(c)?;
    let uses = names(c, |index| class.class_name(index))?;
    let count = c.u16()?;
    let mut provides = Vec::with_capacity(usize::from(count).min(c.remaining() / 4));
    for _ in 0..count {
        provides.push(ModuleProvides {
            service: class.class_name(c.u16()?)?,
            with: names(c, |index| class.class_name(index))?,
        });
    }
    Ok(ModuleAttribute {
        name,
        flags,
        version,
        requires,
        exports,
        opens,
        uses,
        provides,
    })
}
