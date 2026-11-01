//! The module image, `lib/modules` (docs/design/classfile-jimage.md). The format has no public
//! specification; what is read here is what the design's observation log established.
//!
//! Opening reads the index (header, tables, locations, strings) into memory and builds Ferrion's
//! own index of resources by module and package. The redirect table, directory entries, and
//! package entries aren't used. Resources are read with positional reads and decompressed
//! (`zip`, `compact-cp`) when needed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::cursor::Cursor;
use crate::error::{Error, ErrorKind, Result};
use crate::source::Source;
use crate::text::Name;
use crate::zip::{MAX_ENTRY_SIZE, inflate};

const HEADER_SIZE: u64 = 28;
const MAGIC: u32 = 0xcafe_dada;
const COMPRESSED_MAGIC: u32 = 0xcafe_fafa;
const COMPRESSED_HEADER: usize = 29;
/// How many compressed-resource headers may be stacked on one resource.
const MAX_LAYERS: usize = 4;

/// A resource's location attributes (kinds 1 to 7). Strings are offsets into the strings area.
#[derive(Clone, Copy, Debug, Default)]
struct Location {
    module: u32,
    parent: u32,
    base: u32,
    extension: u32,
    offset: u64,
    compressed: u64,
    size: u64,
}

/// One resource of an image, indexed.
#[derive(Clone, Copy, Debug)]
struct Resource {
    location: Location,
    module: u16,
}

/// A module image.
pub struct Image {
    source: Source,
    big_endian: bool,
    major: u16,
    minor: u16,
    index: Vec<u8>,
    index_size: u64,
    strings: std::ops::Range<usize>,
    resources: Vec<Resource>,
    modules: Vec<Box<str>>,
    /// For each package (internal form, "" for the module root), its resources.
    packages: HashMap<Box<str>, Vec<u32>>,
    problems: Vec<Error>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("origin", &self.source.origin())
            .field("version", &(self.major, self.minor))
            .field("resources", &self.resources.len())
            .finish_non_exhaustive()
    }
}

/// A resource of an [`Image`].
#[derive(Clone, Copy)]
pub struct ImageResource<'a> {
    image: &'a Image,
    index: u32,
}

impl<'a> ImageResource<'a> {
    fn resource(&self) -> &'a Resource {
        &self.image.resources[self.index as usize]
    }

    pub fn module(&self) -> &'a str {
        &self.image.modules[usize::from(self.resource().module)]
    }

    /// The path inside the module: `java/lang/Object.class`.
    pub fn path(&self) -> String {
        let location = &self.resource().location;
        let text = |offset| self.image.text(offset).unwrap_or_default();
        let mut path = String::new();
        let parent = text(location.parent);
        if !parent.is_empty() {
            path.push_str(&parent);
            path.push('/');
        }
        path.push_str(&text(location.base));
        let extension = text(location.extension);
        if !extension.is_empty() {
            path.push('.');
            path.push_str(&extension);
        }
        path
    }

    /// The name `jimage list` and the jrt file system use: `/java.base/java/lang/Object.class`.
    pub fn name(&self) -> String {
        format!("/{}/{}", self.module(), self.path())
    }

    /// The package (`java/lang`), from the parent attribute.
    pub fn package(&self) -> String {
        self.image
            .text(self.resource().location.parent)
            .unwrap_or_default()
    }

    /// The file name without its extension.
    pub fn base(&self) -> String {
        self.image
            .text(self.resource().location.base)
            .unwrap_or_default()
    }

    pub fn is_class(&self) -> bool {
        self.image.bytes_at(self.resource().location.extension) == Some(b"class")
    }

    /// The size of the content after decompression.
    pub fn size(&self) -> u64 {
        self.resource().location.size
    }

    pub fn is_compressed(&self) -> bool {
        self.resource().location.compressed != 0
    }

    pub fn read(&self) -> Result<Vec<u8>> {
        self.image.read(*self)
    }
}

impl std::fmt::Debug for ImageResource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImageResource({})", self.name())
    }
}

impl Image {
    pub fn open(path: &Path) -> Result<Image> {
        Image::new(Source::open(path)?)
    }

    /// Reads an image in memory, for tests and fuzzing.
    pub fn from_bytes(bytes: Arc<[u8]>) -> Result<Image> {
        Image::new(Source::Memory(bytes))
    }

    fn new(source: Source) -> Result<Image> {
        let origin = source.origin();
        read_index(source).map_err(|e| e.in_origin(origin))
    }

    pub fn path(&self) -> Option<&Path> {
        self.source.path().map(|p| &**p)
    }

    /// The image's version, (major, minor): (1, 0) in JDK 11 to 26, (1, 1) in JDK 27.
    pub fn version(&self) -> (u16, u16) {
        (self.major, self.minor)
    }

    pub fn is_big_endian(&self) -> bool {
        self.big_endian
    }

    /// The modules with resources, sorted.
    pub fn modules(&self) -> &[Box<str>] {
        &self.modules
    }

    /// Every resource, in the offsets table's order.
    pub fn resources(&self) -> impl ExactSizeIterator<Item = ImageResource<'_>> + '_ {
        (0..self.resources.len() as u32).map(|index| ImageResource { image: self, index })
    }

    /// The resources whose parent is `package`, in every module.
    pub fn package(&self, package: &str) -> impl Iterator<Item = ImageResource<'_>> + '_ {
        self.packages
            .get(package)
            .into_iter()
            .flatten()
            .map(|&index| ImageResource { image: self, index })
    }

    /// Every package with a resource, in no particular order.
    pub fn packages(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(|k| &**k)
    }

    /// The class with internal name `name` in `module`.
    pub fn find_class(&self, module: &str, name: &str) -> Option<ImageResource<'_>> {
        self.classes_named(name)
            .find(|resource| resource.module() == module)
    }

    /// The classes with internal name `name`, in every module, without decoding other names.
    pub fn classes_named<'a, 'b>(
        &'a self,
        name: &'b str,
    ) -> impl Iterator<Item = ImageResource<'a>> + use<'a, 'b> {
        let (package, simple) = name.rsplit_once('/').unwrap_or(("", name));
        self.package(package).filter(move |resource| {
            resource.is_class()
                && self.bytes_at(resource.resource().location.base) == Some(simple.as_bytes())
        })
    }

    /// Resources left out of the index, with why.
    pub fn problems(&self) -> &[Error] {
        &self.problems
    }

    /// The bytes of the string at `offset`, up to its NUL.
    fn bytes_at(&self, offset: u32) -> Option<&[u8]> {
        let strings = &self.index[self.strings.clone()];
        let rest = strings.get(offset as usize..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        Some(&rest[..end])
    }

    fn string(&self, offset: u32) -> Result<&[u8]> {
        self.bytes_at(offset).ok_or_else(|| {
            Error::new(
                ErrorKind::Malformed,
                format!("string offset {offset} is outside the strings or unterminated"),
            )
        })
    }

    fn text(&self, offset: u32) -> Result<String> {
        Ok(Name::new(self.string(offset)?).to_str()?.into_owned())
    }

    fn read(&self, resource: ImageResource<'_>) -> Result<Vec<u8>> {
        let origin = || self.source.entry_origin(&resource.name());
        self.read_location(&resource.resource().location)
            .map_err(|e| e.in_origin(origin()))
    }

    fn read_location(&self, location: &Location) -> Result<Vec<u8>> {
        if location.size > MAX_ENTRY_SIZE {
            return Err(Error::too_large(format!(
                "the resource's size, {} bytes, is above the limit of {MAX_ENTRY_SIZE}",
                location.size
            )));
        }
        let start = self
            .index_size
            .checked_add(location.offset)
            .ok_or_else(|| Error::new(ErrorKind::Malformed, "the resource's offset overflows"))?;
        if location.compressed == 0 {
            return self.source.read(start, location.size);
        }
        let mut data = self.source.read(start, location.compressed)?;
        let mut layers = 0;
        while data.len() >= COMPRESSED_HEADER && self.u32(&data[..4]) == COMPRESSED_MAGIC {
            layers += 1;
            if layers > MAX_LAYERS {
                return Err(Error::malformed(
                    start,
                    format!("more than {MAX_LAYERS} compressed-resource headers"),
                ));
            }
            data = self.decompress(&data, start)?;
        }
        if layers == 0 {
            return Err(Error::malformed(
                start,
                "a compressed resource without a compressed-resource header",
            ));
        }
        if data.len() as u64 != location.size {
            return Err(Error::malformed(
                start,
                format!(
                    "the resource decompresses to {} bytes, not the {} declared",
                    data.len(),
                    location.size
                ),
            ));
        }
        Ok(data)
    }

    fn u32(&self, bytes: &[u8]) -> u32 {
        let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        }
    }

    fn u64(&self, bytes: &[u8]) -> u64 {
        let mut array = [0; 8];
        array.copy_from_slice(&bytes[..8]);
        if self.big_endian {
            u64::from_be_bytes(array)
        } else {
            u64::from_le_bytes(array)
        }
    }

    /// Removes one compressed-resource header from `data` and decompresses its payload.
    fn decompress(&self, data: &[u8], at: u64) -> Result<Vec<u8>> {
        let compressed = self.u64(&data[4..12]);
        let size = self.u64(&data[12..20]);
        let decompressor = self.u32(&data[20..24]);
        let payload = &data[COMPRESSED_HEADER..];
        if compressed != payload.len() as u64 {
            return Err(Error::malformed(
                at,
                format!(
                    "a compressed payload of {} bytes declares {compressed}",
                    payload.len()
                ),
            ));
        }
        if size > MAX_ENTRY_SIZE {
            return Err(Error::too_large(format!(
                "a decompressed size of {size} bytes is above the limit"
            )));
        }
        match self.string(decompressor).map_err(|e| e.at(at))? {
            b"zip" => inflate(payload, size as usize, true).map_err(|e| e.at(at)),
            b"compact-cp" => self
                .compact_cp(payload, size as usize)
                .map_err(|e| e.at(at)),
            other => Err(Error::unsupported(format!(
                "decompressor `{}` isn't supported",
                String::from_utf8_lossy(other)
            ))),
        }
    }

    /// Rebuilds a class file whose `CONSTANT_Utf8` entries were moved to the strings area
    /// (`compact-cp`, observation O10).
    fn compact_cp(&self, payload: &[u8], size: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size);
        let mut c = Cursor::new(payload, 0);
        out.extend_from_slice(c.bytes(8)?);
        let count = c.u16()?;
        out.extend_from_slice(&count.to_be_bytes());
        let mut index = 1;
        while index < count {
            let tag = c.u8()?;
            let text: Vec<u8>;
            let utf8: &[u8] = match tag {
                1 => {
                    let len = c.u16()?;
                    c.bytes(usize::from(len))?
                }
                23 => self.string(number(&mut c)?)?,
                25 => {
                    let skeleton = self.string(number(&mut c)?)?;
                    let len = number(&mut c)?;
                    let mut parts = Cursor::new(c.bytes(len as usize)?, 0);
                    let mut names = Vec::new();
                    while !parts.is_empty() {
                        names.push(number(&mut parts)?);
                    }
                    text = descriptor(skeleton, &names, |offset| self.string(offset))?;
                    &text
                }
                _ => {
                    let size = match tag {
                        3 | 4 | 9..=12 | 17 | 18 => 4,
                        5 | 6 => 8,
                        7 | 8 | 16 | 19 | 20 => 2,
                        15 => 3,
                        _ => {
                            return Err(c.error(format!(
                                "unknown constant pool tag {tag} in a compact-cp class"
                            )));
                        }
                    };
                    out.push(tag);
                    out.extend_from_slice(c.bytes(size)?);
                    index += if tag == 5 || tag == 6 { 2 } else { 1 };
                    continue;
                }
            };
            let len = u16::try_from(utf8.len())
                .map_err(|_| c.error("a constant longer than 65,535 bytes"))?;
            out.push(1);
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(utf8);
            if out.len() > size {
                return Err(c.error(format!(
                    "the class rebuilds to more than the {size} bytes declared"
                )));
            }
            index += 1;
        }
        out.extend_from_slice(c.rest());
        if out.len() != size {
            return Err(Error::new(
                ErrorKind::Malformed,
                format!(
                    "the class rebuilds to {} bytes, not the {size} declared",
                    out.len()
                ),
            ));
        }
        Ok(out)
    }
}

/// Reads a `compact-cp` number: with the high bit of the first byte set, bits 6 and 5 give the
/// length in bytes (1 to 3) and the value is the low five bits followed by the other bytes;
/// otherwise four plain bytes.
fn number(c: &mut Cursor<'_>) -> Result<u32> {
    let offset = c.offset();
    let first = c.u8()?;
    if first & 0x80 == 0 {
        let rest = c.bytes(3)?;
        return Ok(u32::from_be_bytes([first, rest[0], rest[1], rest[2]]));
    }
    let len = (first >> 5) & 3;
    if len == 0 {
        return Err(Error::malformed(
            offset,
            format!("a compact-cp number starts with {first:#04x}"),
        ));
    }
    let mut value = u32::from(first & 0x1f);
    for _ in 1..len {
        value = value << 8 | u32::from(c.u8()?);
    }
    Ok(value)
}

/// Rebuilds a descriptor from its skeleton and (package, name) string offsets: after each `L`
/// of the skeleton, `package/name` (or just `name` for an empty package).
fn descriptor<'s>(
    skeleton: &[u8],
    names: &[u32],
    string: impl Fn(u32) -> Result<&'s [u8]>,
) -> Result<Vec<u8>> {
    let slots = skeleton.iter().filter(|&&b| b == b'L').count();
    if names.len() != 2 * slots {
        return Err(Error::new(
            ErrorKind::Malformed,
            format!(
                "a compact-cp descriptor has {} names for {slots} class types",
                names.len() / 2
            ),
        ));
    }
    let mut text = Vec::with_capacity(skeleton.len() + 16 * slots);
    let mut pairs = names.as_chunks::<2>().0.iter();
    for &b in skeleton {
        text.push(b);
        if b == b'L' {
            let pair = pairs.next().expect("as many pairs as `L`s");
            let package = string(pair[0])?;
            if !package.is_empty() {
                text.extend_from_slice(package);
                text.push(b'/');
            }
            text.extend_from_slice(string(pair[1])?);
        }
    }
    Ok(text)
}

/// Reads the header and index, and builds the resource index.
fn read_index(source: Source) -> Result<Image> {
    let header = source.read(0, HEADER_SIZE)?;
    let big_endian = match header[..4] {
        [0xda, 0xda, 0xfe, 0xca] => false,
        [0xca, 0xfe, 0xda, 0xda] => true,
        _ => return Err(Error::malformed(0, "not a jimage file: bad magic")),
    };
    let field = |i: usize| {
        let bytes = [header[i], header[i + 1], header[i + 2], header[i + 3]];
        if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        }
    };
    debug_assert_eq!(field(0), MAGIC);
    let version = field(4);
    let (major, minor) = ((version >> 16) as u16, version as u16);
    if major != 1 {
        return Err(
            Error::unsupported(format!("jimage version {major}.{minor} isn't supported")).at(4),
        );
    }
    let table_length = u64::from(field(16));
    let locations_size = u64::from(field(20));
    let strings_size = u64::from(field(24));
    let index_size = HEADER_SIZE + 8 * table_length + locations_size + strings_size;
    if index_size > source.len() {
        return Err(Error::malformed(
            16,
            format!(
                "the index ({index_size} bytes) is larger than the file ({} bytes)",
                source.len()
            ),
        ));
    }
    let index = source.read(0, index_size)?;
    let offsets_start = (HEADER_SIZE + 4 * table_length) as usize;
    let locations_start = offsets_start + 4 * table_length as usize;
    let strings_start = locations_start + locations_size as usize;
    let mut image = Image {
        source,
        big_endian,
        major,
        minor,
        strings: strings_start..strings_start + strings_size as usize,
        index_size,
        resources: Vec::with_capacity(table_length as usize),
        modules: Vec::new(),
        packages: HashMap::new(),
        problems: Vec::new(),
        index,
    };
    let mut module_ids: HashMap<String, u16> = HashMap::new();
    let mut packages: HashMap<Box<str>, Vec<u32>> = HashMap::new();
    for i in 0..table_length as usize {
        let at = offsets_start + 4 * i;
        let offset = image.u32(&image.index[at..at + 4]) as usize;
        let location = match decode_location(&image.index[locations_start..strings_start], offset) {
            Ok(location) => location,
            Err(error) => {
                image
                    .problems
                    .push(error.at((locations_start + offset) as u64));
                continue;
            }
        };
        let names = (|| -> Result<Option<(String, String)>> {
            let module = image.text(location.module)?;
            if module.is_empty() || module == "modules" || module == "packages" {
                return Ok(None);
            }
            Ok(Some((module, image.text(location.parent)?)))
        })();
        let (module, package) = match names {
            Ok(Some(names)) => names,
            Ok(None) => continue,
            Err(error) => {
                image
                    .problems
                    .push(error.at((locations_start + offset) as u64));
                continue;
            }
        };
        let next = module_ids.len();
        let Ok(next) = u16::try_from(next) else {
            image
                .problems
                .push(Error::new(ErrorKind::Malformed, "more than 65,535 modules"));
            continue;
        };
        let module = *module_ids.entry(module).or_insert(next);
        let index = image.resources.len() as u32;
        image.resources.push(Resource { location, module });
        packages
            .entry(package.into_boxed_str())
            .or_default()
            .push(index);
    }
    // Module ids in sorted order.
    let mut names: Vec<(String, u16)> = module_ids.into_iter().collect();
    names.sort();
    let mut remap = vec![0u16; names.len()];
    for (new, (_, old)) in names.iter().enumerate() {
        remap[usize::from(*old)] = new as u16;
    }
    for resource in &mut image.resources {
        resource.module = remap[usize::from(resource.module)];
    }
    image.modules = names
        .into_iter()
        .map(|(name, _)| name.into_boxed_str())
        .collect();
    image.packages = packages;
    Ok(image)
}

/// Decodes the attribute stream at `offset`: bytes `kind << 3 | (length - 1)`, then `length`
/// bytes of big-endian value, until kind 0.
fn decode_location(locations: &[u8], offset: usize) -> Result<Location> {
    let mut location = Location::default();
    let mut c = Cursor::new(locations.get(offset..).unwrap_or_default(), 0);
    if offset >= locations.len() {
        return Err(Error::new(
            ErrorKind::Malformed,
            format!("location offset {offset} is outside the location table"),
        ));
    }
    loop {
        let byte = c.u8()?;
        let kind = byte >> 3;
        if kind == 0 {
            return Ok(location);
        }
        let len = usize::from(byte & 7) + 1;
        let value = c
            .bytes(len)?
            .iter()
            .fold(0u64, |value, &b| value << 8 | u64::from(b));
        let string = || {
            u32::try_from(value).map_err(|_| {
                Error::new(
                    ErrorKind::Malformed,
                    format!("string offset {value} is too large"),
                )
            })
        };
        match kind {
            1 => location.module = string()?,
            2 => location.parent = string()?,
            3 => location.base = string()?,
            4 => location.extension = string()?,
            5 => location.offset = value,
            6 => location.compressed = value,
            7 => location.size = value,
            _ => {
                return Err(Error::new(
                    ErrorKind::Malformed,
                    format!("unknown location attribute kind {kind}"),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
