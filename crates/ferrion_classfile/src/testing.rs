//! Writers for small class files, zip files, and jimage files, for tests, fuzz seeds, and the
//! fuzz targets that build structured input. Not part of the public API.
//!
//! The formats written here are the ones the readers in this crate accept, as documented in
//! docs/design/classfile-containers.md and docs/design/classfile-jimage.md. Nothing here is
//! copied from a JDK.

use std::collections::HashMap;

use crate::text;

/// CRC-32 (the zip polynomial, 0xEDB88320 reflected), table-driven. The readers don't check
/// CRCs; tests and the zip writer use this.
pub fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        table
    });
    let mut crc = !0u32;
    for &b in bytes {
        crc = table[((crc ^ u32::from(b)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// Builds a class file: a constant pool, members, and attributes given as bytes.
#[derive(Default)]
pub struct ClassBuilder {
    pool: Vec<u8>,
    slots: u16,
    known: HashMap<Vec<u8>, u16>,
    pub major: u16,
    pub minor: u16,
    pub access: u16,
    this: u16,
    superclass: u16,
    interfaces: Vec<u16>,
    fields: Vec<u8>,
    field_count: u16,
    methods: Vec<u8>,
    method_count: u16,
    attributes: Vec<u8>,
    attribute_count: u16,
}

impl ClassBuilder {
    /// A class named `name` extending `java/lang/Object`, version 71.0, public.
    pub fn new(name: &str) -> ClassBuilder {
        let mut builder = ClassBuilder {
            slots: 1,
            major: 71,
            access: 0x0021,
            ..Default::default()
        };
        builder.this = builder.class(name);
        builder.superclass = builder.class("java/lang/Object");
        builder
    }

    /// Adds a constant pool entry (tag and contents), sharing equal entries.
    pub fn entry(&mut self, bytes: &[u8]) -> u16 {
        if let Some(&index) = self.known.get(bytes) {
            return index;
        }
        let index = self.slots;
        self.pool.extend_from_slice(bytes);
        self.slots += if matches!(bytes[0], 5 | 6) { 2 } else { 1 };
        self.known.insert(bytes.to_vec(), index);
        index
    }

    pub fn utf8_bytes(&mut self, bytes: &[u8]) -> u16 {
        let mut entry = vec![1];
        entry.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        entry.extend_from_slice(bytes);
        self.entry(&entry)
    }

    pub fn utf8(&mut self, text: &str) -> u16 {
        self.utf8_bytes(&text::encode(text))
    }

    fn indirect(&mut self, tag: u8, text: &str) -> u16 {
        let name = self.utf8(text);
        let [a, b] = name.to_be_bytes();
        self.entry(&[tag, a, b])
    }

    pub fn class(&mut self, name: &str) -> u16 {
        self.indirect(7, name)
    }

    pub fn string(&mut self, text: &str) -> u16 {
        self.indirect(8, text)
    }

    pub fn module(&mut self, name: &str) -> u16 {
        self.indirect(19, name)
    }

    pub fn package(&mut self, name: &str) -> u16 {
        self.indirect(20, name)
    }

    pub fn integer(&mut self, value: i32) -> u16 {
        let [a, b, c, d] = value.to_be_bytes();
        self.entry(&[3, a, b, c, d])
    }

    pub fn float(&mut self, bits: u32) -> u16 {
        let [a, b, c, d] = bits.to_be_bytes();
        self.entry(&[4, a, b, c, d])
    }

    pub fn long(&mut self, value: i64) -> u16 {
        let mut entry = vec![5];
        entry.extend_from_slice(&value.to_be_bytes());
        self.entry(&entry)
    }

    pub fn double(&mut self, bits: u64) -> u16 {
        let mut entry = vec![6];
        entry.extend_from_slice(&bits.to_be_bytes());
        self.entry(&entry)
    }

    pub fn name_and_type(&mut self, name: &str, descriptor: &str) -> u16 {
        let name = self.utf8(name);
        let descriptor = self.utf8(descriptor);
        let mut entry = vec![12];
        entry.extend_from_slice(&name.to_be_bytes());
        entry.extend_from_slice(&descriptor.to_be_bytes());
        self.entry(&entry)
    }

    pub fn set_super(&mut self, name: Option<&str>) {
        self.superclass = name.map_or(0, |name| self.class(name));
    }

    pub fn interface(&mut self, name: &str) {
        let index = self.class(name);
        self.interfaces.push(index);
    }

    /// An attribute: its name's index, length, and `contents`.
    pub fn attribute(&mut self, name: &str, contents: &[u8]) -> Vec<u8> {
        let mut bytes = self.utf8(name).to_be_bytes().to_vec();
        bytes.extend_from_slice(&(contents.len() as u32).to_be_bytes());
        bytes.extend_from_slice(contents);
        bytes
    }

    fn member(
        &mut self,
        access: u16,
        name: &str,
        descriptor: &str,
        attributes: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut bytes = access.to_be_bytes().to_vec();
        bytes.extend_from_slice(&self.utf8(name).to_be_bytes());
        bytes.extend_from_slice(&self.utf8(descriptor).to_be_bytes());
        bytes.extend_from_slice(&(attributes.len() as u16).to_be_bytes());
        for attribute in attributes {
            bytes.extend_from_slice(attribute);
        }
        bytes
    }

    pub fn field(&mut self, access: u16, name: &str, descriptor: &str, attributes: &[Vec<u8>]) {
        let member = self.member(access, name, descriptor, attributes);
        self.fields.extend_from_slice(&member);
        self.field_count += 1;
    }

    pub fn method(&mut self, access: u16, name: &str, descriptor: &str, attributes: &[Vec<u8>]) {
        let member = self.member(access, name, descriptor, attributes);
        self.methods.extend_from_slice(&member);
        self.method_count += 1;
    }

    pub fn class_attribute(&mut self, attribute: Vec<u8>) {
        self.attributes.extend_from_slice(&attribute);
        self.attribute_count += 1;
    }

    pub fn finish(&self) -> Vec<u8> {
        let mut bytes = 0xcafe_babe_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(&self.minor.to_be_bytes());
        bytes.extend_from_slice(&self.major.to_be_bytes());
        bytes.extend_from_slice(&self.slots.to_be_bytes());
        bytes.extend_from_slice(&self.pool);
        bytes.extend_from_slice(&self.access.to_be_bytes());
        bytes.extend_from_slice(&self.this.to_be_bytes());
        bytes.extend_from_slice(&self.superclass.to_be_bytes());
        bytes.extend_from_slice(&(self.interfaces.len() as u16).to_be_bytes());
        for interface in &self.interfaces {
            bytes.extend_from_slice(&interface.to_be_bytes());
        }
        bytes.extend_from_slice(&self.field_count.to_be_bytes());
        bytes.extend_from_slice(&self.fields);
        bytes.extend_from_slice(&self.method_count.to_be_bytes());
        bytes.extend_from_slice(&self.methods);
        bytes.extend_from_slice(&self.attribute_count.to_be_bytes());
        bytes.extend_from_slice(&self.attributes);
        bytes
    }
}

/// One entry for [`ZipBuilder`].
pub struct ZipEntrySpec {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
    /// 0 (stored) or 8 (deflate); anything else is written as stored bytes with that method.
    pub method: u16,
    pub flags: u16,
}

/// Builds a zip file with the features the reader handles.
#[derive(Default)]
pub struct ZipBuilder {
    pub entries: Vec<ZipEntrySpec>,
    /// Bytes before the archive, such as a launcher script.
    pub prefix: Vec<u8>,
    pub comment: Vec<u8>,
    /// Writes zip64 records and extra fields for every entry.
    pub zip64: bool,
    /// Sets flag bit 3 and writes data descriptors, with zero sizes in local headers.
    pub data_descriptors: bool,
}

impl ZipBuilder {
    pub fn new() -> ZipBuilder {
        ZipBuilder::default()
    }

    pub fn add(&mut self, name: &str, data: &[u8], method: u16) -> &mut ZipBuilder {
        self.entries.push(ZipEntrySpec {
            name: name.as_bytes().to_vec(),
            data: data.to_vec(),
            method,
            flags: 0x0800,
        });
        self
    }

    pub fn stored(&mut self, name: &str, data: &[u8]) -> &mut ZipBuilder {
        self.add(name, data, 0)
    }

    pub fn deflated(&mut self, name: &str, data: &[u8]) -> &mut ZipBuilder {
        self.add(name, data, 8)
    }

    pub fn finish(&self) -> Vec<u8> {
        let mut out = self.prefix.clone();
        let start = out.len();
        let mut central = Vec::new();
        for entry in &self.entries {
            let offset = (out.len() - start) as u64;
            let compressed = match entry.method {
                8 => miniz_oxide::deflate::compress_to_vec(&entry.data, 6),
                _ => entry.data.clone(),
            };
            let crc = crc32(&entry.data);
            let flags = entry.flags | if self.data_descriptors { 0x0008 } else { 0 };
            let (size32, csize32, offset32) = if self.zip64 {
                (u32::MAX, u32::MAX, u32::MAX)
            } else {
                (
                    entry.data.len() as u32,
                    compressed.len() as u32,
                    offset as u32,
                )
            };
            let mut extra = Vec::new();
            if self.zip64 {
                extra.extend_from_slice(&1u16.to_le_bytes());
                extra.extend_from_slice(&24u16.to_le_bytes());
                extra.extend_from_slice(&(entry.data.len() as u64).to_le_bytes());
                extra.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
                extra.extend_from_slice(&offset.to_le_bytes());
            }
            // The local header.
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&45u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&entry.method.to_le_bytes());
            out.extend_from_slice(&[0; 4]); // time, date
            let (lcrc, lc, lu) = if self.data_descriptors {
                (0, 0, 0)
            } else {
                (crc, csize32, size32)
            };
            out.extend_from_slice(&lcrc.to_le_bytes());
            out.extend_from_slice(&lc.to_le_bytes());
            out.extend_from_slice(&lu.to_le_bytes());
            out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            out.extend_from_slice(&entry.name);
            out.extend_from_slice(&extra);
            out.extend_from_slice(&compressed);
            if self.data_descriptors {
                out.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
                out.extend_from_slice(&crc.to_le_bytes());
                if self.zip64 {
                    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
                    out.extend_from_slice(&(entry.data.len() as u64).to_le_bytes());
                } else {
                    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
                    out.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
                }
            }
            // The central directory header.
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&45u16.to_le_bytes());
            central.extend_from_slice(&45u16.to_le_bytes());
            central.extend_from_slice(&flags.to_le_bytes());
            central.extend_from_slice(&entry.method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&csize32.to_le_bytes());
            central.extend_from_slice(&size32.to_le_bytes());
            central.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            central.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 2]); // comment length
            central.extend_from_slice(&[0; 2]); // disk
            central.extend_from_slice(&[0; 2]); // internal attributes
            central.extend_from_slice(&[0; 4]); // external attributes
            central.extend_from_slice(&offset32.to_le_bytes());
            central.extend_from_slice(&entry.name);
            central.extend_from_slice(&extra);
        }
        let cd_offset = (out.len() - start) as u64;
        out.extend_from_slice(&central);
        let count = self.entries.len() as u64;
        if self.zip64 {
            let record = (out.len() - start) as u64;
            out.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
            out.extend_from_slice(&44u64.to_le_bytes());
            out.extend_from_slice(&45u16.to_le_bytes());
            out.extend_from_slice(&45u16.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // disks
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&(central.len() as u64).to_le_bytes());
            out.extend_from_slice(&cd_offset.to_le_bytes());
            out.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&record.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
        }
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // disks
        let (count16, size32, offset32) = if self.zip64 {
            (u16::MAX, u32::MAX, u32::MAX)
        } else {
            (count as u16, central.len() as u32, cd_offset as u32)
        };
        out.extend_from_slice(&count16.to_le_bytes());
        out.extend_from_slice(&count16.to_le_bytes());
        out.extend_from_slice(&size32.to_le_bytes());
        out.extend_from_slice(&offset32.to_le_bytes());
        out.extend_from_slice(&(self.comment.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.comment);
        out
    }
}

/// How [`ImageBuilder`] stores resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageCompression {
    None,
    /// A zlib stream behind one compressed-resource header, as the JetBrains Runtime does.
    Zip,
    /// Class files' `CONSTANT_Utf8` entries moved to the strings, as `jlink --compress=1` does.
    /// Other resources are stored as they are.
    CompactCp,
}

/// Builds a jimage file (`lib/modules`) in the layout of docs/design/classfile-jimage.md.
pub struct ImageBuilder {
    pub big_endian: bool,
    pub compression: ImageCompression,
    pub minor_version: u16,
    /// (module, path in the module, content): `("java.base", "java/lang/Object.class", ...)`.
    pub resources: Vec<(String, String, Vec<u8>)>,
}

impl ImageBuilder {
    pub fn new() -> ImageBuilder {
        ImageBuilder {
            big_endian: false,
            compression: ImageCompression::None,
            minor_version: 0,
            resources: Vec::new(),
        }
    }

    pub fn add(&mut self, module: &str, path: &str, content: &[u8]) -> &mut ImageBuilder {
        self.resources
            .push((module.to_owned(), path.to_owned(), content.to_vec()));
        self
    }

    pub fn finish(&self) -> Vec<u8> {
        let mut strings = Strings::default();
        strings.add(b"");
        let u32_bytes = |value: u32| {
            if self.big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let u64_bytes = |value: u64| {
            if self.big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let mut locations = vec![0u8]; // offset 0 is an empty stream
        let mut offsets = Vec::new();
        let mut content = Vec::new();
        // A package entry and a root, which the reader skips.
        let mut entries: Vec<(String, String, String, String, Vec<u8>, bool)> = Vec::new();
        for (module, path, data) in &self.resources {
            let (parent, file) = path.rsplit_once('/').unwrap_or(("", path));
            let (base, extension) = file.rsplit_once('.').unwrap_or((file, ""));
            entries.push((
                module.clone(),
                parent.to_owned(),
                base.to_owned(),
                extension.to_owned(),
                data.clone(),
                true,
            ));
        }
        entries.push((
            "packages".into(),
            String::new(),
            "java.lang".into(),
            String::new(),
            vec![0; 8],
            false,
        ));
        entries.push((
            String::new(),
            String::new(),
            "packages".into(),
            String::new(),
            Vec::new(),
            false,
        ));
        for (module, parent, base, extension, data, resource) in &entries {
            let stored = if *resource {
                self.compress(
                    data,
                    extension == "class",
                    &mut strings,
                    u32_bytes,
                    u64_bytes,
                )
            } else {
                None
            };
            let offset = content.len() as u64;
            let (bytes, compressed) = match &stored {
                Some(stored) => (stored.clone(), stored.len() as u64),
                None => (data.clone(), 0),
            };
            content.extend_from_slice(&bytes);
            offsets.push(locations.len() as u32);
            let mut attribute = |kind: u8, value: u64| {
                if value == 0 {
                    return;
                }
                let len = (8 - value.leading_zeros() as usize / 8).max(1);
                locations.push(kind << 3 | (len as u8 - 1));
                locations.extend_from_slice(&value.to_be_bytes()[8 - len..]);
            };
            attribute(1, u64::from(strings.add(module.as_bytes())));
            attribute(2, u64::from(strings.add(parent.as_bytes())));
            attribute(3, u64::from(strings.add(base.as_bytes())));
            attribute(4, u64::from(strings.add(extension.as_bytes())));
            attribute(5, offset);
            attribute(6, compressed);
            attribute(7, data.len() as u64);
            locations.push(0);
        }
        let count = offsets.len() as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&u32_bytes(0xcafe_dada));
        out.extend_from_slice(&u32_bytes(0x0001_0000 | u32::from(self.minor_version)));
        out.extend_from_slice(&u32_bytes(0));
        out.extend_from_slice(&u32_bytes(count));
        out.extend_from_slice(&u32_bytes(count));
        out.extend_from_slice(&u32_bytes(locations.len() as u32));
        out.extend_from_slice(&u32_bytes(strings.bytes.len() as u32));
        for _ in 0..count {
            out.extend_from_slice(&u32_bytes(0)); // the redirect table, unused by the reader
        }
        for offset in offsets {
            out.extend_from_slice(&u32_bytes(offset));
        }
        out.extend_from_slice(&locations);
        out.extend_from_slice(&strings.bytes);
        out.extend_from_slice(&content);
        out
    }

    fn compress(
        &self,
        data: &[u8],
        is_class: bool,
        strings: &mut Strings,
        u32_bytes: impl Fn(u32) -> [u8; 4],
        u64_bytes: impl Fn(u64) -> [u8; 8],
    ) -> Option<Vec<u8>> {
        let (payload, decompressor) = match self.compression {
            ImageCompression::None => return None,
            ImageCompression::Zip => (miniz_oxide::deflate::compress_to_vec_zlib(data, 6), "zip"),
            ImageCompression::CompactCp if is_class => (compact_cp(data, strings)?, "compact-cp"),
            ImageCompression::CompactCp => return None,
        };
        let name = strings.add(decompressor.as_bytes());
        let mut stored = u32_bytes(0xcafe_fafa).to_vec();
        stored.extend_from_slice(&u64_bytes(payload.len() as u64));
        stored.extend_from_slice(&u64_bytes(data.len() as u64));
        stored.extend_from_slice(&u32_bytes(name));
        stored.extend_from_slice(&u32_bytes(u32::MAX));
        stored.push(1);
        stored.extend_from_slice(&payload);
        Some(stored)
    }
}

impl Default for ImageBuilder {
    fn default() -> ImageBuilder {
        ImageBuilder::new()
    }
}

/// The strings area: NUL-terminated, shared by equal strings.
#[derive(Default)]
struct Strings {
    bytes: Vec<u8>,
    known: HashMap<Vec<u8>, u32>,
}

impl Strings {
    fn add(&mut self, text: &[u8]) -> u32 {
        if let Some(&offset) = self.known.get(text) {
            return offset;
        }
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(text);
        self.bytes.push(0);
        self.known.insert(text.to_vec(), offset);
        offset
    }
}

/// Writes a `compact-cp` number: one to three bytes when it fits in 5 + 8 × (n - 1) bits,
/// otherwise four plain bytes.
pub fn compact_number(value: u32, out: &mut Vec<u8>) {
    if value < 1 << 5 {
        out.push(0xa0 | value as u8);
    } else if value < 1 << 13 {
        out.extend_from_slice(&[0xc0 | (value >> 8) as u8, value as u8]);
    } else if value < 1 << 21 {
        out.extend_from_slice(&[0xe0 | (value >> 16) as u8, (value >> 8) as u8, value as u8]);
    } else {
        assert!(value < 1 << 31, "a plain number has its high bit clear");
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// Rewrites a class file's `CONSTANT_Utf8` entries as `compact-cp` does: descriptors with class
/// names (`(Ljava/lang/String;)V`) as tag 25, everything else as tag 23. `None` for a class file
/// it can't walk.
fn compact_cp(class: &[u8], strings: &mut Strings) -> Option<Vec<u8>> {
    let mut out = class.get(..10)?.to_vec();
    let count = u16::from_be_bytes([class[8], class[9]]);
    let mut pos = 10;
    let mut index = 1;
    while index < count {
        let tag = *class.get(pos)?;
        if tag == 1 {
            let len = usize::from(u16::from_be_bytes([
                *class.get(pos + 1)?,
                *class.get(pos + 2)?,
            ]));
            let text = class.get(pos + 3..pos + 3 + len)?;
            pos += 3 + len;
            match split_descriptor(text) {
                Some((skeleton, names)) => {
                    out.push(25);
                    compact_number(strings.add(&skeleton), &mut out);
                    let mut pairs = Vec::new();
                    for (package, name) in names {
                        compact_number(strings.add(package), &mut pairs);
                        compact_number(strings.add(name), &mut pairs);
                    }
                    compact_number(pairs.len() as u32, &mut out);
                    out.extend_from_slice(&pairs);
                }
                None => {
                    out.push(23);
                    compact_number(strings.add(text), &mut out);
                }
            }
        } else {
            let size = match tag {
                3 | 4 | 9..=12 | 17 | 18 => 4,
                5 | 6 => 8,
                7 | 8 | 16 | 19 | 20 => 2,
                15 => 3,
                _ => return None,
            };
            out.extend_from_slice(class.get(pos..pos + 1 + size)?);
            pos += 1 + size;
            if tag == 5 || tag == 6 {
                index += 1;
            }
        }
        index += 1;
    }
    out.extend_from_slice(class.get(pos..)?);
    Some(out)
}

/// Splits a descriptor into a skeleton with every class name after an `L` removed, and the
/// (package, simple name) of each, or `None` when the text has no `L...;` names or has an `L`
/// that doesn't start one.
type Split<'a> = (Vec<u8>, Vec<(&'a [u8], &'a [u8])>);

fn split_descriptor(text: &[u8]) -> Option<Split<'_>> {
    if !matches!(text.first(), Some(b'(' | b'[' | b'L')) {
        return None;
    }
    let mut skeleton = Vec::new();
    let mut names = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        skeleton.push(c);
        i += 1;
        if c == b'L' {
            let end = i + text[i..].iter().position(|&b| matches!(b, b';' | b'<'))?;
            let name = &text[i..end];
            let (package, simple) = match name.iter().rposition(|&b| b == b'/') {
                Some(slash) => (&name[..slash], &name[slash + 1..]),
                None => (&name[..0], name),
            };
            if simple.is_empty() {
                return None;
            }
            names.push((package, simple));
            i = end;
        }
    }
    if names.is_empty() {
        None
    } else {
        Some((skeleton, names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn compact_numbers() {
        let mut out = Vec::new();
        compact_number(3, &mut out);
        compact_number(41, &mut out);
        compact_number(641, &mut out);
        // The bytes observed in O10 of docs/design/classfile-jimage.md.
        assert_eq!(out, [0xa3, 0xc0, 0x29, 0xc2, 0x81]);
    }
}
