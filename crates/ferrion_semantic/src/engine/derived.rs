//! Derived entries: values computed from inputs and other entries, which record what they read
//! and are verified on first use after a change (docs/design/semantic-incremental.md#project-symbols).
//!
//! A computation reads only through a [`Reads`], which records each dependency it reads with the
//! fingerprint it had. The entry stores its value, its value's fingerprint, those dependencies,
//! and the revision at which it was last known valid. At a later revision, the first use checks
//! each dependency in the order it was read, verifying (and if need be recomputing) derived ones
//! first. If every fingerprint matches, the entry is valid again without computing anything. If
//! one differs, the entry is recomputed, and if the new value has the old fingerprint, the old
//! value is kept, so that what read it sees the fingerprint it recorded and stays valid: early
//! cutoff.
//!
//! An input's fingerprint is its version, so a file's summary is valid exactly while its text is
//! unchanged. Every chain of dependencies ends at inputs, which are read under the revision
//! check, so a snapshot whose revision has moved can't mark anything valid.
//!
//! Locks are held only to look up or store an entry, never while one is computed or verified.
//! Two threads that need the same entry at once may both compute it; the values are equal, and
//! the second store replaces the first.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::{Cancelled, Snapshot, TableStats};
use crate::{FileId, Fingerprint, Name, SourceSetId};

/// Something a derived entry can read. [`Snapshot::current`] gives each one's fingerprint now.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Dep {
    /// A file's text, path, and source set's release, by the file's version.
    FileInput(FileId),
    /// The list of a source set's files, by its version.
    SetFiles(SourceSetId),
    /// A file's whole summary.
    Summary(FileId),
    /// The package a file declares and its top-level types' names: the part of its summary the
    /// package table reads.
    TopLevel(FileId),
    /// A source set's package index.
    PackageIndex(SourceSetId),
    /// The files of one package in a source set's package index.
    PackageFiles(SourceSetId, Name),
    /// A package entry.
    Package(SourceSetId, Name),
}

/// One derived entry.
pub(crate) struct Memo<V> {
    pub(crate) value: Arc<V>,
    pub(crate) fingerprint: Fingerprint,
    /// What the computation read, in order, with the fingerprints it saw.
    deps: Box<[(Dep, Fingerprint)]>,
    /// The latest revision at which the entry is known valid. Only grows.
    verified_at: AtomicU64,
}

/// What a computation reads, recorded as it reads it. The accessors that read engine data and
/// record it are in the modules that own the data (`engine`, `packages`).
pub(crate) struct Reads<'s> {
    pub(crate) snapshot: &'s Snapshot,
    deps: Vec<(Dep, Fingerprint)>,
}

impl<'s> Reads<'s> {
    pub(crate) fn record(&mut self, dep: Dep, fingerprint: Fingerprint) {
        self.deps.push((dep, fingerprint));
    }

    /// For loops in a computation: `Err(Cancelled)` once the revision has moved.
    pub(crate) fn check(&self) -> Result<(), Cancelled> {
        self.snapshot.check()
    }
}

/// How many shards each table has. More than the threads that use it, so two threads rarely
/// want the same shard's lock.
const SHARDS: usize = 64;

type Shard<K, V> = RwLock<HashMap<K, Arc<Memo<V>>>>;

/// A memo table of derived entries, sharded by key.
pub(crate) struct DerivedTable<K, V> {
    shards: Box<[Shard<K, V>]>,
    hasher: RandomState,
    computed: AtomicU64,
    unchanged: AtomicU64,
    verified: AtomicU64,
}

impl<K: Clone + Eq + Hash, V: Hash> DerivedTable<K, V> {
    pub(crate) fn new() -> Self {
        DerivedTable {
            shards: (0..SHARDS).map(|_| RwLock::default()).collect(),
            hasher: RandomState::new(),
            computed: AtomicU64::new(0),
            unchanged: AtomicU64::new(0),
            verified: AtomicU64::new(0),
        }
    }

    /// The entry for `key`, valid at `snapshot`'s revision: the stored one if it was verified at
    /// this revision or its dependencies still have the fingerprints it saw, else a new one from
    /// `compute`.
    pub(crate) fn get(
        &self,
        snapshot: &Snapshot,
        key: &K,
        compute: impl FnOnce(&mut Reads<'_>) -> Result<V, Cancelled>,
    ) -> Result<Arc<Memo<V>>, Cancelled> {
        // Verifying an entry verifies what it read first, so chains of entries recurse.
        ferrion_stack::ensure_stack(|| {
            let revision = snapshot.revision.0;
            let old = read(self.shard(key)).get(key).cloned();
            if let Some(memo) = &old {
                if memo.verified_at.load(Ordering::Acquire) == revision {
                    return Ok(memo.clone());
                }
                if snapshot.unchanged(&memo.deps)? {
                    memo.verified_at.fetch_max(revision, Ordering::AcqRel);
                    self.verified.fetch_add(1, Ordering::Relaxed);
                    return Ok(memo.clone());
                }
            }
            let mut reads = Reads {
                snapshot,
                deps: Vec::new(),
            };
            let value = compute(&mut reads)?;
            let fingerprint = Fingerprint::of(&value);
            self.computed.fetch_add(1, Ordering::Relaxed);
            let value = match &old {
                Some(old) if old.fingerprint == fingerprint => {
                    self.unchanged.fetch_add(1, Ordering::Relaxed);
                    old.value.clone()
                }
                _ => Arc::new(value),
            };
            let memo = Arc::new(Memo {
                value,
                fingerprint,
                deps: reads.deps.into_boxed_slice(),
                verified_at: AtomicU64::new(revision),
            });
            // Stored even if the revision has moved meanwhile: its dependencies say what it was
            // computed from, so the next revision verifies it like any other. Only an entry
            // already verified at a later revision is kept instead.
            let mut shard = write(self.shard(key));
            let newer = shard
                .get(key)
                .is_some_and(|stored| stored.verified_at.load(Ordering::Acquire) > revision);
            if !newer {
                shard.insert(key.clone(), memo.clone());
            }
            Ok(memo)
        })
    }

    /// Drops the entry for `key`, if any, to free its memory: for an input that no longer
    /// exists. Not needed for correctness.
    pub(crate) fn remove(&self, key: &K) {
        write(self.shard(key)).remove(key);
    }

    pub(crate) fn len(&self) -> usize {
        self.shards.iter().map(|s| read(s).len()).sum()
    }

    pub(crate) fn stats(&self) -> TableStats {
        TableStats {
            computed: self.computed.load(Ordering::Relaxed),
            unchanged: self.unchanged.load(Ordering::Relaxed),
            verified: self.verified.load(Ordering::Relaxed),
        }
    }

    fn shard(&self, key: &K) -> &Shard<K, V> {
        &self.shards[self.hasher.hash_one(key) as usize % SHARDS]
    }
}

impl Snapshot {
    /// Whether each of `deps` still has the fingerprint recorded, checked in order, stopping at
    /// the first that doesn't.
    fn unchanged(&self, deps: &[(Dep, Fingerprint)]) -> Result<bool, Cancelled> {
        for (dep, fingerprint) in deps {
            self.check()?;
            if self.current(dep)? != *fingerprint {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Locks for reading. A panic never leaves a table or the inputs half-changed (every change is
/// a single insert or assignment), so a poisoned lock's data is used as it is.
pub(crate) fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

/// Locks for writing; see [`read`].
pub(crate) fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}
