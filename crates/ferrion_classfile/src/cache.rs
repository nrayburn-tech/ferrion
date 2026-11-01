//! The process-wide archive cache (docs/design/classfile.md, "Threads and sharing").
//!
//! Archives are keyed by canonical path and file identity (size, modification time, device and
//! inode) and held weakly: two source sets or projects that use the same jar or JDK share one
//! archive and one index, an archive nobody uses is dropped, and a jar rebuilt on disk gets a new
//! key, so new lookups see the new file.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use crate::ctsym::CtSym;
use crate::error::Result;
use crate::jar::Jar;
use crate::jdk::Jdk;
use crate::jimage::Image;
use crate::source::FileKey;
use crate::zip::ZipArchive;

#[derive(Default)]
struct Inner {
    zips: HashMap<FileKey, Weak<ZipArchive>>,
    jars: HashMap<(FileKey, u32), Weak<Jar>>,
    images: HashMap<FileKey, Weak<Image>>,
    ctsyms: HashMap<FileKey, Weak<CtSym>>,
    jdks: HashMap<FileKey, Weak<Jdk>>,
}

/// Archives shared by everything in the process. `Send + Sync`; usually one per process.
#[derive(Default)]
pub struct ArchiveCache {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for ArchiveCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveCache").finish_non_exhaustive()
    }
}

/// Looks `key` up in the map `select` picks, or opens the value without holding the lock and
/// inserts it, keeping a value another thread inserted meanwhile.
fn get_or_open<K: Hash + Eq + Clone, T>(
    cache: &ArchiveCache,
    key: K,
    select: impl Fn(&mut Inner) -> &mut HashMap<K, Weak<T>>,
    open: impl FnOnce() -> Result<T>,
) -> Result<Arc<T>> {
    if let Some(found) = select(&mut cache.lock()).get(&key).and_then(Weak::upgrade) {
        return Ok(found);
    }
    let opened = Arc::new(open()?);
    let mut inner = cache.lock();
    let map = select(&mut inner);
    if let Some(found) = map.get(&key).and_then(Weak::upgrade) {
        return Ok(found);
    }
    map.retain(|_, weak| weak.strong_count() > 0);
    map.insert(key, Arc::downgrade(&opened));
    Ok(opened)
}

impl ArchiveCache {
    pub fn new() -> ArchiveCache {
        ArchiveCache::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The zip archive at `path`.
    pub fn zip(&self, path: &Path) -> Result<Arc<ZipArchive>> {
        get_or_open(
            self,
            FileKey::of(path)?,
            |i| &mut i.zips,
            || ZipArchive::open(path),
        )
    }

    /// The jar at `path`, resolved for `release`. Jars at two releases share the archive.
    pub fn jar(&self, path: &Path, release: u32) -> Result<Arc<Jar>> {
        let key = (FileKey::of(path)?, release);
        get_or_open(
            self,
            key,
            |i| &mut i.jars,
            || Ok(Jar::new(self.zip(path)?, release)),
        )
    }

    /// The module image at `path`.
    pub fn image(&self, path: &Path) -> Result<Arc<Image>> {
        get_or_open(
            self,
            FileKey::of(path)?,
            |i| &mut i.images,
            || Image::open(path),
        )
    }

    /// The `ct.sym` at `path`.
    pub fn ctsym(&self, path: &Path) -> Result<Arc<CtSym>> {
        get_or_open(
            self,
            FileKey::of(path)?,
            |i| &mut i.ctsyms,
            || Ok(CtSym::new(self.zip(path)?)),
        )
    }

    /// The JDK at `home`, keyed by its `release` file (or the directory when there is none).
    pub fn jdk(&self, home: &Path) -> Result<Arc<Jdk>> {
        let release = home.join("release");
        let key = FileKey::of(if release.exists() { &release } else { home })?;
        get_or_open(
            self,
            key,
            |i| &mut i.jdks,
            || Jdk::open_uncached(home, self),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::tests::Scratch;
    use crate::testing::ZipBuilder;

    #[test]
    fn shares_and_notices_changes() {
        let scratch = Scratch::new("cache");
        let mut b = ZipBuilder::new();
        b.stored("p/A.class", b"one");
        let path = scratch.write("a.jar", &b.finish());
        let cache = ArchiveCache::new();
        let first = cache.jar(&path, 11).unwrap();
        let second = cache.jar(&path, 11).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let other_release = cache.jar(&path, 17).unwrap();
        assert!(!Arc::ptr_eq(&first, &other_release));
        assert!(Arc::ptr_eq(first.archive(), other_release.archive()));
        // A rebuilt jar is a new archive.
        b.stored("p/B.class", b"two");
        std::fs::write(&path, b.finish()).unwrap();
        let rebuilt = cache.jar(&path, 11).unwrap();
        assert!(rebuilt.find("p/B.class").is_some());
        assert!(!Arc::ptr_eq(&first, &rebuilt));
        // Nobody holds them: they're dropped.
        drop((first, second, other_release, rebuilt));
        let again = cache.zip(&path).unwrap();
        assert_eq!(Arc::strong_count(&again), 1);
    }
}
