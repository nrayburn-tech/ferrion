//! A zip reader for jars, `ct.sym`, and JDK 8's jars (docs/design/classfile-containers.md).
//! Section numbers are from PKWARE's APPNOTE.TXT, version 6.3.10.
//!
//! The central directory is read once, with one positional read, and indexed by name. Entries
//! are read with positional reads, so threads share an archive without a lock. Sizes and the
//! compression method always come from the central directory, so data descriptors need no
//! handling. Only stored (0) and deflated (8) entries are read; CRCs aren't checked.

use std::path::Path;
use std::sync::Arc;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use crate::cursor::Cursor;
use crate::error::{Error, ErrorKind, Result};
use crate::source::Source;

/// The largest entry read: a class or a manifest declaring more is `TooLarge`.
pub const MAX_ENTRY_SIZE: u64 = 64 << 20;

const EOCD: u32 = 0x0605_4b50;
const EOCD64: u32 = 0x0606_4b50;
const EOCD64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

struct RawEntry {
    name: std::ops::Range<usize>,
    method: u16,
    flags: u16,
    compressed: u64,
    size: u64,
    /// The local header's offset in the file, the prefix included.
    local: u64,
}

/// A zip archive's central directory, indexed, and the means to read its entries.
pub struct ZipArchive {
    source: Source,
    names: String,
    /// Files (not directories) with valid names, in central directory order.
    entries: Vec<RawEntry>,
    /// Indexes into `entries`, sorted by name; for a name listed twice, only the last.
    sorted: Vec<u32>,
    /// Entries left out of the index, with why.
    problems: Vec<Error>,
    prefix: u64,
}

impl std::fmt::Debug for ZipArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipArchive")
            .field("origin", &self.source.origin())
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// An entry of a [`ZipArchive`].
#[derive(Clone, Copy)]
pub struct ZipEntry<'a> {
    archive: &'a ZipArchive,
    index: u32,
}

impl<'a> ZipEntry<'a> {
    fn raw(&self) -> &'a RawEntry {
        &self.archive.entries[self.index as usize]
    }

    pub fn name(&self) -> &'a str {
        &self.archive.names[self.raw().name.clone()]
    }

    /// The compression method: 0 (stored) or 8 (deflate) are read.
    pub fn method(&self) -> u16 {
        self.raw().method
    }

    /// The uncompressed size.
    pub fn size(&self) -> u64 {
        self.raw().size
    }

    pub fn compressed_size(&self) -> u64 {
        self.raw().compressed
    }

    /// The entry's position in the central directory, among files with valid names.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Reads the entry, up to [`MAX_ENTRY_SIZE`].
    pub fn read(&self) -> Result<Vec<u8>> {
        self.archive.read(*self, MAX_ENTRY_SIZE)
    }
}

impl std::fmt::Debug for ZipEntry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ZipEntry({:?})", self.name())
    }
}

impl ZipArchive {
    /// Opens a zip file on disk and reads its central directory.
    pub fn open(path: &Path) -> Result<ZipArchive> {
        ZipArchive::new(Source::open(path)?)
    }

    /// Reads a zip file in memory, for tests and fuzzing.
    pub fn from_bytes(bytes: Arc<[u8]>) -> Result<ZipArchive> {
        ZipArchive::new(Source::Memory(bytes))
    }

    fn new(source: Source) -> Result<ZipArchive> {
        let origin = source.origin();
        read_directory(source).map_err(|e| e.in_origin(origin))
    }

    /// The archive's path, or `None` in memory.
    pub fn path(&self) -> Option<&Path> {
        self.source.path().map(|p| &**p)
    }

    pub(crate) fn source(&self) -> &Source {
        &self.source
    }

    /// Bytes before the archive (a launcher script, a JMOD header), in bytes.
    pub fn prefix(&self) -> u64 {
        self.prefix
    }

    /// The number of files indexed, duplicates included.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every file, in central directory order, duplicates included. Directories and entries
    /// listed in [`ZipArchive::problems`] are left out.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = ZipEntry<'_>> + '_ {
        (0..self.entries.len() as u32).map(|index| ZipEntry {
            archive: self,
            index,
        })
    }

    /// The file at `index` in [`ZipArchive::entries`].
    pub fn entry(&self, index: u32) -> Option<ZipEntry<'_>> {
        ((index as usize) < self.entries.len()).then_some(ZipEntry {
            archive: self,
            index,
        })
    }

    /// Every file by name, sorted, with only the last of duplicated names.
    pub fn unique_entries(&self) -> impl ExactSizeIterator<Item = ZipEntry<'_>> + '_ {
        self.sorted.iter().map(|&index| ZipEntry {
            archive: self,
            index,
        })
    }

    /// The file named `name`; for a name listed twice, the last in central directory order, as
    /// javac and `java.util.zip.ZipFile` choose (observed: docs/design/classfile-containers.md,
    /// "Duplicate names").
    pub fn find(&self, name: &str) -> Option<ZipEntry<'_>> {
        let position = self
            .sorted
            .binary_search_by(|&index| {
                self.names[self.entries[index as usize].name.clone()].cmp(name)
            })
            .ok()?;
        Some(ZipEntry {
            archive: self,
            index: self.sorted[position],
        })
    }

    /// The sorted entries whose names start with `prefix`.
    pub fn with_prefix(&self, prefix: &str) -> impl ExactSizeIterator<Item = ZipEntry<'_>> + '_ {
        let name = |index: u32| &self.names[self.entries[index as usize].name.clone()];
        let start = self.sorted.partition_point(|&index| name(index) < prefix);
        // Names with the prefix sort together, right after `start`.
        let len = self.sorted[start..].partition_point(|&index| name(index).starts_with(prefix));
        self.sorted[start..start + len]
            .iter()
            .map(|&index| ZipEntry {
                archive: self,
                index,
            })
    }

    /// Entries that couldn't be indexed (a name that isn't UTF-8, a size past the end of the
    /// file), each an error naming the entry.
    pub fn problems(&self) -> &[Error] {
        &self.problems
    }

    /// Reads an entry, failing with `TooLarge` when its declared size is above `limit`.
    pub fn read(&self, entry: ZipEntry<'_>, limit: u64) -> Result<Vec<u8>> {
        let origin = || self.source.entry_origin(entry.name());
        self.read_raw(entry.raw(), limit)
            .map_err(|e| e.in_origin(origin()))
    }

    fn read_raw(&self, raw: &RawEntry, limit: u64) -> Result<Vec<u8>> {
        if raw.flags & 1 != 0 {
            return Err(Error::unsupported("the entry is encrypted"));
        }
        if raw.method != 0 && raw.method != 8 {
            return Err(Error::unsupported(format!(
                "compression method {} isn't supported",
                raw.method
            )));
        }
        if raw.size > limit {
            return Err(Error::too_large(format!(
                "the entry's size, {} bytes, is above the limit of {limit}",
                raw.size
            )));
        }
        let header = self.source.read(raw.local, 30)?;
        let mut c = Cursor::new(&header, raw.local);
        if c.u32_le()? != LOCAL {
            return Err(Error::malformed(
                raw.local,
                "no local file header where the central directory says",
            ));
        }
        c.skip(22)?;
        let name_len = u64::from(c.u16_le()?);
        let extra_len = u64::from(c.u16_le()?);
        let data = raw.local + 30 + name_len + extra_len;
        let stored = self.source.read(data, raw.compressed)?;
        match raw.method {
            0 if raw.compressed != raw.size => Err(Error::malformed(
                data,
                format!(
                    "a stored entry's sizes differ: {} and {}",
                    raw.compressed, raw.size
                ),
            )),
            0 => Ok(stored),
            _ => inflate(&stored, raw.size as usize, false).map_err(|e| e.at(data)),
        }
    }
}

/// Inflates `data` (raw deflate, or zlib when `zlib` is true) into exactly `size` bytes. More
/// output, less, or a stream that doesn't end is `Malformed`.
pub(crate) fn inflate(data: &[u8], size: usize, zlib: bool) -> Result<Vec<u8>> {
    let mut out = vec![0; size];
    let mut state = Box::<DecompressorOxide>::default();
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | if zlib {
            inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        } else {
            0
        };
    let (status, _, written) = decompress(&mut state, data, &mut out, 0, flags);
    match status {
        TINFLStatus::Done if written == size => Ok(out),
        TINFLStatus::Done => Err(Error::new(
            ErrorKind::Malformed,
            format!("the data inflates to {written} bytes, not the {size} declared"),
        )),
        TINFLStatus::HasMoreOutput => Err(Error::new(
            ErrorKind::Malformed,
            format!("the data inflates to more than the {size} bytes declared"),
        )),
        status => Err(Error::new(
            ErrorKind::Malformed,
            format!("the data doesn't inflate: {status:?}"),
        )),
    }
}

/// The most end-of-central-directory candidates tried.
const MAX_CANDIDATES: usize = 8;

/// Finds end of central directory records, in the order to try them: from the end of the file
/// backwards, first those whose comment reaches exactly to the end of the file, then those whose
/// comment fits.
fn find_ends(source: &Source) -> Result<Vec<(u64, Vec<u8>)>> {
    let len = source.len();
    if len < 22 {
        return Err(Error::malformed(0, "too short to be a zip file"));
    }
    let start = len.saturating_sub(22 + 65_535);
    let tail = source.read(start, len - start)?;
    let (mut exact, mut fitting) = (Vec::new(), Vec::new());
    for pos in (0..=tail.len() - 22).rev() {
        if tail[pos..pos + 4] != EOCD.to_le_bytes() {
            continue;
        }
        let comment = u64::from(u16::from_le_bytes([tail[pos + 20], tail[pos + 21]]));
        let end = pos as u64 + 22 + comment;
        let candidate = (start + pos as u64, tail[pos..pos + 22].to_vec());
        if end == tail.len() as u64 {
            exact.push(candidate);
        } else if end < tail.len() as u64 {
            fitting.push(candidate);
        }
        if exact.len() + fitting.len() == MAX_CANDIDATES {
            break;
        }
    }
    exact.extend(fitting);
    if exact.is_empty() {
        return Err(Error::malformed(len, "no end of central directory record"));
    }
    Ok(exact)
}

/// The central directory read from one end record.
struct Directory {
    names: String,
    entries: Vec<RawEntry>,
    problems: Vec<Error>,
    prefix: u64,
}

/// Reads the central directory through the first end record that leads to one, preferring one
/// with entries: a comment can hold bytes that look like an end record.
fn read_directory(source: Source) -> Result<ZipArchive> {
    let mut first_error = None;
    let mut empty = None;
    let mut found = None;
    for (eocd, record) in find_ends(&source)? {
        match read_central(&source, eocd, &record) {
            Ok(directory) if directory.entries.is_empty() && directory.problems.is_empty() => {
                empty.get_or_insert(directory);
            }
            Ok(directory) => {
                found = Some(directory);
                break;
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    let directory = match (found.or(empty), first_error) {
        (Some(directory), _) => directory,
        (None, Some(error)) => return Err(error),
        (None, None) => return Err(Error::malformed(0, "no end of central directory record")),
    };
    let Directory {
        names,
        entries,
        problems,
        prefix,
    } = directory;
    let mut archive = ZipArchive {
        source,
        names,
        entries,
        sorted: Vec::new(),
        problems,
        prefix,
    };

    let mut sorted: Vec<u32> = (0..archive.entries.len() as u32).collect();
    let name = |index: &u32| &archive.names[archive.entries[*index as usize].name.clone()];
    // Duplicated names sort last first, so deduplicating keeps the last.
    sorted.sort_by(|a, b| name(a).cmp(name(b)).then(b.cmp(a)));
    sorted.dedup_by(|b, a| name(a) == name(b));
    archive.sorted = sorted;
    Ok(archive)
}

fn read_central(source: &Source, eocd: u64, record: &[u8]) -> Result<Directory> {
    let mut c = Cursor::new(record, eocd);
    c.skip(10)?; // signature, disk numbers, entries on this disk
    let mut count = u64::from(c.u16_le()?);
    let mut cd_size = u64::from(c.u32_le()?);
    let mut cd_offset = u64::from(c.u32_le()?);
    let mut cd_end = eocd;
    let zip64 = count == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff;
    if zip64 && let Some((record, values)) = zip64_end(source, eocd)? {
        (count, cd_size, cd_offset) = values;
        cd_end = record;
    }
    let cd_start = cd_end.checked_sub(cd_size).ok_or_else(|| {
        Error::malformed(
            eocd,
            format!("the central directory's size, {cd_size}, is larger than the file"),
        )
    })?;
    let prefix = cd_start.checked_sub(cd_offset).ok_or_else(|| {
        Error::malformed(
            eocd,
            format!("the central directory's offset, {cd_offset}, is past where it is"),
        )
    })?;
    if count > cd_size / 46 {
        return Err(Error::malformed(
            eocd,
            format!("{count} entries can't fit in a central directory of {cd_size} bytes"),
        ));
    }
    let directory = source.read(cd_start, cd_size)?;
    let mut c = Cursor::new(&directory, cd_start);
    let mut archive = Directory {
        names: String::new(),
        entries: Vec::with_capacity(count as usize),
        problems: Vec::new(),
        prefix,
    };
    for _ in 0..count {
        let offset = c.offset();
        if c.u32_le()? != CENTRAL {
            return Err(Error::malformed(
                offset,
                "a central directory header has a bad signature",
            ));
        }
        c.skip(4)?; // versions
        let flags = c.u16_le()?;
        let method = c.u16_le()?;
        c.skip(8)?; // time, date, CRC
        let mut compressed = u64::from(c.u32_le()?);
        let mut size = u64::from(c.u32_le()?);
        let name_len = usize::from(c.u16_le()?);
        let extra_len = usize::from(c.u16_le()?);
        let comment_len = usize::from(c.u16_le()?);
        c.skip(8)?; // disk, attributes
        let mut local = u64::from(c.u32_le()?);
        let name = c.bytes(name_len)?;
        let extra = c.bytes(extra_len)?;
        c.skip(comment_len)?;
        let problem = |message: String| {
            Error::malformed(offset, message)
                .in_origin(source.entry_origin(&String::from_utf8_lossy(name)))
        };
        if let Err(message) = apply_zip64(extra, &mut size, &mut compressed, &mut local) {
            archive.problems.push(problem(message));
            continue;
        }
        let Ok(name) = std::str::from_utf8(name) else {
            archive
                .problems
                .push(problem("the entry's name isn't UTF-8".to_owned()));
            continue;
        };
        if name.ends_with('/') {
            continue;
        }
        let local = prefix.saturating_add(local);
        if local.saturating_add(30).saturating_add(compressed) > source.len() {
            archive.problems.push(problem(format!(
                "the entry's {compressed} bytes at offset {local} run past the end of the file"
            )));
            continue;
        }
        let start = archive.names.len();
        archive.names.push_str(name);
        archive.entries.push(RawEntry {
            name: start..archive.names.len(),
            method,
            flags,
            compressed,
            size,
            local,
        });
    }
    Ok(archive)
}

/// Reads the zip64 end of central directory record through its locator, just before the end
/// of central directory record. Returns the record's position and (count, size, offset), or
/// `None` when there is no locator.
/// The zip64 record's position, and its (entry count, central directory size, offset).
type Zip64End = (u64, (u64, u64, u64));

fn zip64_end(source: &Source, eocd: u64) -> Result<Option<Zip64End>> {
    let Some(locator) = eocd.checked_sub(20) else {
        return Ok(None);
    };
    let bytes = source.read(locator, 20)?;
    let mut c = Cursor::new(&bytes, locator);
    if c.u32_le()? != EOCD64_LOCATOR {
        return Ok(None);
    }
    c.skip(4)?;
    let recorded = c.u64_le()?;
    // The recorded offset is relative to the archive's start; with a prefix, the record is
    // just before the locator instead.
    for candidate in [Some(recorded), locator.checked_sub(56)]
        .into_iter()
        .flatten()
    {
        if candidate.checked_add(56).is_none_or(|end| end > locator) {
            continue;
        }
        let record = source.read(candidate, 56)?;
        let mut c = Cursor::new(&record, candidate);
        if c.u32_le()? != EOCD64 {
            continue;
        }
        c.skip(28)?; // size, versions, disks, entries on this disk
        let count = c.u64_le()?;
        let size = c.u64_le()?;
        let offset = c.u64_le()?;
        return Ok(Some((candidate, (count, size, offset))));
    }
    Err(Error::malformed(
        locator,
        "no zip64 end of central directory record where its locator says",
    ))
}

/// Replaces 0xFFFFFFFF sizes and offset with the zip64 extended information extra field's
/// values, in APPNOTE's order (4.5.3).
fn apply_zip64(
    extra: &[u8],
    size: &mut u64,
    compressed: &mut u64,
    local: &mut u64,
) -> std::result::Result<(), String> {
    let needed = [
        *size == 0xffff_ffff,
        *compressed == 0xffff_ffff,
        *local == 0xffff_ffff,
    ];
    if !needed.contains(&true) {
        return Ok(());
    }
    let mut c = Cursor::new(extra, 0);
    while c.remaining() >= 4 {
        let (Ok(id), Ok(len)) = (c.u16_le(), c.u16_le()) else {
            break;
        };
        let Ok(data) = c.bytes(usize::from(len)) else {
            return Err("an extra field runs past its area".to_owned());
        };
        if id != 1 {
            continue;
        }
        let mut field = Cursor::new(data, 0);
        for (needed, value) in needed.into_iter().zip([size, compressed, local]) {
            if needed {
                *value = field
                    .u64_le()
                    .map_err(|_| "the zip64 extra field is too short".to_owned())?;
            }
        }
        return Ok(());
    }
    Err("a 0xFFFFFFFF size or offset without a zip64 extra field".to_owned())
}

#[cfg(test)]
mod tests;
