//! The tree cache: parsed trees kept within a byte budget, least recently used evicted first
//! (docs/design/semantic-incremental.md#trees).
//!
//! Trees are derived from inputs alone, so they aren't entries with dependencies: a tree is
//! found by its file and version, and one of an older version is never returned. A tree evicted
//! while in use lives on through its `Arc` until its user is done.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use ferrion_syntax::{Node, SyntaxKind, SyntaxTree};

use crate::FileId;

pub(crate) struct TreeCache {
    budget: usize,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<FileId, Entry>,
    /// Each entry's file by when it was last used, oldest first.
    by_use: BTreeMap<u64, FileId>,
    clock: u64,
    bytes: usize,
}

struct Entry {
    version: u64,
    tree: Arc<SyntaxTree>,
    bytes: usize,
    used: u64,
}

impl TreeCache {
    pub(crate) fn new(budget: usize) -> TreeCache {
        TreeCache {
            budget,
            inner: Mutex::default(),
        }
    }

    /// The tree of `file` at `version`, if it's kept.
    pub(crate) fn get(&self, file: FileId, version: u64) -> Option<Arc<SyntaxTree>> {
        let mut inner = self.lock();
        let inner = &mut *inner;
        let entry = inner.entries.get_mut(&file)?;
        if entry.version != version {
            return None;
        }
        inner.by_use.remove(&entry.used);
        inner.clock += 1;
        entry.used = inner.clock;
        inner.by_use.insert(entry.used, file);
        Some(entry.tree.clone())
    }

    /// Keeps `tree` as `file`'s at `version`, unless a newer version's is kept, then evicts the
    /// least recently used trees until the cache is within its budget. A tree larger than the
    /// whole budget isn't kept.
    pub(crate) fn insert(&self, file: FileId, version: u64, tree: Arc<SyntaxTree>) {
        let bytes = tree_bytes(&tree);
        let mut inner = self.lock();
        let inner = &mut *inner;
        if bytes > self.budget {
            return;
        }
        if let Some(old) = inner.entries.get(&file) {
            if old.version > version {
                return;
            }
            let old = inner.entries.remove(&file).unwrap();
            inner.by_use.remove(&old.used);
            inner.bytes -= old.bytes;
        }
        inner.clock += 1;
        let used = inner.clock;
        inner.entries.insert(
            file,
            Entry {
                version,
                tree,
                bytes,
                used,
            },
        );
        inner.by_use.insert(used, file);
        inner.bytes += bytes;
        while inner.bytes > self.budget {
            let (_, oldest) = inner.by_use.pop_first().unwrap();
            let evicted = inner.entries.remove(&oldest).unwrap();
            inner.bytes -= evicted.bytes;
        }
    }

    /// Drops `file`'s tree if it is older than `version`: after an edit, so the old tree's memory
    /// is freed at once.
    pub(crate) fn remove_older(&self, file: FileId, version: u64) {
        let mut inner = self.lock();
        let inner = &mut *inner;
        if inner
            .entries
            .get(&file)
            .is_some_and(|e| e.version < version)
        {
            let old = inner.entries.remove(&file).unwrap();
            inner.by_use.remove(&old.used);
            inner.bytes -= old.bytes;
        }
    }

    /// The bytes of the trees kept, as [`tree_bytes`] counts them.
    pub(crate) fn bytes(&self) -> usize {
        self.lock().bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.lock().entries.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A tree's size for the budget: its node and token arrays. The text isn't counted, since the
/// file's input holds it anyway.
pub(crate) fn tree_bytes(tree: &SyntaxTree) -> usize {
    let token = size_of::<ferrion_lexer::Token>() + size_of::<SyntaxKind>();
    tree.node_count() * size_of::<Node>() + tree.token_count() * token
}
