//! Positional reads from an archive on disk or in memory.
//!
//! Reads use `read_exact_at` (Unix) or `seek_read` (Windows), which don't share a file cursor, so
//! threads read one archive without a lock. Open handles are capped process-wide: a handle
//! evicted from the pool is reopened on demand, and reopening checks that the file is still the
//! one that was indexed, reporting `Changed` otherwise.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::SystemTime;

use crate::error::{Error, ErrorKind, Origin, Result};

/// The most handles kept open at once, across every archive in the process.
const MAX_OPEN_HANDLES: usize = 128;

/// What identifies a file's contents: if any of it changes, the file is treated as a new one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Identity {
    len: u64,
    modified: Option<SystemTime>,
    /// The device and inode on Unix; zero elsewhere.
    device: u64,
    inode: u64,
}

impl Identity {
    fn of(metadata: &std::fs::Metadata) -> Identity {
        #[cfg(unix)]
        let (device, inode) = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        #[cfg(not(unix))]
        let (device, inode) = (0, 0);
        Identity {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            device,
            inode,
        }
    }
}

/// The key the [`crate::ArchiveCache`] shares archives by: the canonical path and the identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FileKey {
    path: PathBuf,
    identity: Identity,
}

impl FileKey {
    pub(crate) fn of(path: &Path) -> Result<FileKey> {
        let canonical = std::fs::canonicalize(path).map_err(|e| Error::io(&e, path))?;
        let metadata = std::fs::metadata(&canonical).map_err(|e| Error::io(&e, path))?;
        Ok(FileKey {
            path: canonical,
            identity: Identity::of(&metadata),
        })
    }
}

struct Pool {
    handles: HashMap<u64, (Arc<File>, u64)>,
    clock: u64,
}

fn pool() -> MutexGuard<'static, Pool> {
    static POOL: LazyLock<Mutex<Pool>> = LazyLock::new(|| {
        Mutex::new(Pool {
            handles: HashMap::new(),
            clock: 0,
        })
    });
    POOL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Pool {
    fn get(&mut self, id: u64) -> Option<Arc<File>> {
        self.clock += 1;
        let clock = self.clock;
        self.handles.get_mut(&id).map(|(file, used)| {
            *used = clock;
            file.clone()
        })
    }

    fn insert(&mut self, id: u64, file: Arc<File>) {
        self.clock += 1;
        self.handles.insert(id, (file, self.clock));
        while self.handles.len() > MAX_OPEN_HANDLES {
            let Some((&oldest, _)) = self.handles.iter().min_by_key(|(_, (_, used))| *used) else {
                break;
            };
            self.handles.remove(&oldest);
        }
    }
}

/// An archive's bytes.
pub(crate) enum Source {
    File(FileSource),
    Memory(Arc<[u8]>),
}

pub(crate) struct FileSource {
    path: Arc<Path>,
    id: u64,
    identity: Identity,
}

impl Drop for FileSource {
    fn drop(&mut self) {
        pool().handles.remove(&self.id);
    }
}

impl Source {
    pub(crate) fn open(path: &Path) -> Result<Source> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let file = File::open(path).map_err(|e| Error::io(&e, path))?;
        let metadata = file.metadata().map_err(|e| Error::io(&e, path))?;
        if !metadata.is_file() {
            return Err(
                Error::new(ErrorKind::Io, "not a file").in_origin(Origin::File(path.into()))
            );
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        pool().insert(id, Arc::new(file));
        Ok(Source::File(FileSource {
            path: path.into(),
            id,
            identity: Identity::of(&metadata),
        }))
    }

    pub(crate) fn len(&self) -> u64 {
        match self {
            Source::File(file) => file.identity.len,
            Source::Memory(bytes) => bytes.len() as u64,
        }
    }

    /// The archive's path, or `None` in memory.
    pub(crate) fn path(&self) -> Option<&Arc<Path>> {
        match self {
            Source::File(file) => Some(&file.path),
            Source::Memory(_) => None,
        }
    }

    /// The origin of errors in the archive itself.
    pub(crate) fn origin(&self) -> Origin {
        match self {
            Source::File(file) => Origin::File(file.path.clone()),
            Source::Memory(_) => Origin::Memory,
        }
    }

    /// The origin of errors in one entry.
    pub(crate) fn entry_origin(&self, entry: &str) -> Origin {
        match self {
            Source::File(file) => Origin::Entry {
                archive: file.path.clone(),
                entry: entry.into(),
            },
            Source::Memory(_) => Origin::Memory,
        }
    }

    /// Reads `len` bytes at `offset`. A range outside the archive is `Malformed`.
    pub(crate) fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let end = offset
            .checked_add(len)
            .filter(|&end| end <= self.len())
            .ok_or_else(|| {
                Error::malformed(
                    offset,
                    format!(
                        "{len} bytes at offset {offset} run past the end of the file ({} bytes)",
                        self.len()
                    ),
                )
            })?;
        match self {
            Source::Memory(bytes) => Ok(bytes[offset as usize..end as usize].to_vec()),
            Source::File(file) => {
                let mut buffer = vec![0; len as usize];
                file.read_at(offset, &mut buffer)?;
                Ok(buffer)
            }
        }
    }
}

impl FileSource {
    fn handle(&self) -> Result<Arc<File>> {
        if let Some(file) = pool().get(self.id) {
            return Ok(file);
        }
        let file = File::open(&self.path).map_err(|e| Error::io(&e, &self.path))?;
        let metadata = file.metadata().map_err(|e| Error::io(&e, &self.path))?;
        if Identity::of(&metadata) != self.identity {
            return Err(self.changed());
        }
        let file = Arc::new(file);
        pool().insert(self.id, file.clone());
        Ok(file)
    }

    fn changed(&self) -> Error {
        Error::new(ErrorKind::Changed, "the file changed since it was opened")
            .in_origin(Origin::File(self.path.clone()))
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let file = self.handle()?;
        match read_exact_at(&file, buffer, offset) {
            Ok(()) => Ok(()),
            Err(error) => {
                // A file truncated or replaced in place reads short.
                let now = std::fs::metadata(&self.path).map(|m| Identity::of(&m));
                if now.as_ref().ok() != Some(&self.identity) {
                    return Err(self.changed());
                }
                Err(Error::io(&error, &self.path))
            }
        }
    }
}

#[cfg(unix)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buffer: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buffer.is_empty() {
        match file.seek_read(buffer, offset) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                buffer = &mut buffer[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A scratch directory under the target directory, removed when dropped.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new(name: &str) -> Scratch {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ferrion-classfile-{}-{name}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_and_reopens() {
        let scratch = Scratch::new("source");
        let path = scratch.write("a.bin", b"0123456789");
        let sources: Vec<Source> = (0..MAX_OPEN_HANDLES + 5)
            .map(|_| Source::open(&path).unwrap())
            .collect();
        // The first sources' handles were evicted; reading reopens them.
        for source in &sources {
            assert_eq!(source.read(2, 3).unwrap(), b"234");
        }
        assert_eq!(
            sources[0].read(8, 3).unwrap_err().kind(),
            ErrorKind::Malformed
        );
        drop(sources);
        // A file replaced since it was opened is reported, once its handle has to be reopened.
        let source = Source::open(&path).unwrap();
        pool().handles.clear();
        std::fs::write(&path, b"changed contents").unwrap();
        assert_eq!(source.read(0, 2).unwrap_err().kind(), ErrorKind::Changed);
    }
}
