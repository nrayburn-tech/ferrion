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
//!
//! **Cycles.** Entries of the kinds that can read themselves through others (a type's supertypes
//! read the member types of the types named in its `extends` clause, which read their
//! supertypes) are tracked while they are verified or computed, on a stack per thread. A request
//! for an entry already on the thread's stack is a cycle: [`DerivedTable::get_or_cycle`] returns
//! `None` for it, and every entry on the stack from that one up is marked as in the cycle. A
//! computation asks [`Reads::in_cycle`] when it is done, and gives its cyclic value then (for a
//! type, error supertypes), so that every entry in a cycle comes out the same whichever was asked
//! for first. A cyclic request is recorded with [`Fingerprint::CYCLE`], which no entry has, so an
//! entry that saw a cycle is computed again at every revision: cycles are rare and small, and this
//! is what notices one being broken. Tracking is per thread, so two threads in the same cycle each
//! find it on their own stack, and neither waits for the other.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::{Cancelled, Snapshot, TableStats};
use crate::symbols::TypeKey;
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
    /// The files declaring one top-level type of a package entry: what a lookup by name reads,
    /// so that a new type in a package reaches only the lookups of its name.
    PackageType(SourceSetId, Name, Name),
    /// The list of a source set's `module-info.java` files, in its package index.
    ModuleFiles(SourceSetId),
    /// A source set's module.
    ModuleView(SourceSetId),
    /// A file's scope: its summary and its types' keys.
    FileScope(FileId),
    /// A source type's erased supertypes.
    Supertypes(TypeKey),
    /// A type's member types, in a view (the type's own set for a source type).
    MemberTypes(SourceSetId, TypeKey),
    /// A source type's entry.
    TypeEntry(TypeKey),
}

thread_local! {
    /// The tracked entries this thread is verifying or computing, outermost first.
    static ACTIVE: RefCell<Active> = RefCell::default();
}

#[derive(Default)]
struct Active {
    stack: Vec<Frame>,
    /// Each tracked entry on the stack, by its position there.
    positions: HashMap<Dep, usize>,
}

struct Frame {
    dep: Dep,
    /// The cycles found through this entry; empty if none.
    cycles: Vec<Arc<Cycle>>,
}

/// One cycle found: its entries, shared by every entry in it, so that a cycle of n entries costs
/// O(n) to record, not O(n^2).
pub(crate) struct Cycle {
    deps: Box<[Dep]>,
    /// The source types whose supertypes are in it, sorted, made once.
    source_types: OnceLock<Arc<[TypeKey]>>,
}

impl Cycle {
    /// The source types whose supertypes entries are in the cycle, sorted.
    pub(crate) fn source_types(&self) -> Arc<[TypeKey]> {
        self.source_types
            .get_or_init(|| {
                let mut keys: Vec<TypeKey> = self
                    .deps
                    .iter()
                    .filter_map(|d| match d {
                        Dep::Supertypes(key) => Some(*key),
                        _ => None,
                    })
                    .collect();
                keys.sort();
                keys.dedup();
                Arc::from(keys)
            })
            .clone()
    }
}

/// Pushes `dep` on this thread's stack, or marks the cycle and returns `None` if it is on it
/// already: every entry from it to the top of the stack is in the cycle, and each is told the
/// cycle's entries.
fn enter(dep: &Dep) -> Option<Entered> {
    ACTIVE.with_borrow_mut(|active| {
        if let Some(&position) = active.positions.get(dep) {
            let cycle = Arc::new(Cycle {
                deps: active.stack[position..]
                    .iter()
                    .map(|f| f.dep.clone())
                    .collect(),
                source_types: OnceLock::new(),
            });
            for frame in &mut active.stack[position..] {
                frame.cycles.push(cycle.clone());
            }
            return None;
        }
        let position = active.stack.len();
        active.positions.insert(dep.clone(), position);
        active.stack.push(Frame {
            dep: dep.clone(),
            cycles: Vec::new(),
        });
        Some(Entered { position })
    })
}

/// An entry on this thread's stack, popped when dropped (also when the computation returns
/// `Cancelled` or panics).
struct Entered {
    position: usize,
}

impl Entered {
    /// The entries of the cycles that requests made since the entry was pushed (or since
    /// [`Entered::reset`]) found through it; empty if none.
    fn cycles(&self) -> Vec<Arc<Cycle>> {
        ACTIVE.with_borrow(|active| active.stack[self.position].cycles.clone())
    }

    fn cyclic(&self) -> bool {
        ACTIVE.with_borrow(|active| !active.stack[self.position].cycles.is_empty())
    }

    fn reset(&self) {
        ACTIVE.with_borrow_mut(|active| active.stack[self.position].cycles.clear());
    }
}

impl Drop for Entered {
    fn drop(&mut self) {
        ACTIVE.with_borrow_mut(|active| {
            // Inner entries are dropped first, so this one is on top.
            let frame = active.stack.pop().expect("an entered frame");
            debug_assert_eq!(active.stack.len(), self.position);
            active.positions.remove(&frame.dep);
        });
    }
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
    /// The computation's frame on the thread's stack, for a tracked entry.
    frame: Option<&'s Entered>,
}

impl<'s> Reads<'s> {
    /// A recorder for a query that isn't an entry, whose reads are dropped.
    pub(crate) fn untracked(snapshot: &'s Snapshot) -> Reads<'s> {
        Reads {
            snapshot,
            deps: Vec::new(),
            frame: None,
        }
    }

    pub(crate) fn record(&mut self, dep: Dep, fingerprint: Fingerprint) {
        self.deps.push((dep, fingerprint));
    }

    /// Whether the computation has read an entry that, through others, reads it: it is in a
    /// cycle, and should give its cyclic value. Always false for an entry that isn't tracked.
    pub(crate) fn in_cycle(&self) -> bool {
        self.frame.is_some_and(Entered::cyclic)
    }

    /// The cycles found through this computation: whose values every entry in the cycle gives,
    /// so that it is the same whichever was asked for first.
    pub(crate) fn cycles(&self) -> Vec<Arc<Cycle>> {
        self.frame.map(Entered::cycles).unwrap_or_default()
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
    /// For a table whose entries can be in cycles, the dependency each key is read as, by which
    /// it is tracked.
    tracked: Option<fn(&K) -> Dep>,
    computed: AtomicU64,
    unchanged: AtomicU64,
    verified: AtomicU64,
}

impl<K: Clone + Eq + Hash, V: Hash> DerivedTable<K, V> {
    pub(crate) fn new() -> Self {
        DerivedTable {
            shards: (0..SHARDS).map(|_| RwLock::default()).collect(),
            hasher: RandomState::new(),
            tracked: None,
            computed: AtomicU64::new(0),
            unchanged: AtomicU64::new(0),
            verified: AtomicU64::new(0),
        }
    }

    /// A table whose entries are tracked for cycles, each read as the dependency `dep_of` gives.
    pub(crate) fn tracked(dep_of: fn(&K) -> Dep) -> Self {
        DerivedTable {
            tracked: Some(dep_of),
            ..DerivedTable::new()
        }
    }

    /// The entry for `key`, valid at `snapshot`'s revision: the stored one if it was verified at
    /// this revision or its dependencies still have the fingerprints it saw, else a new one from
    /// `compute`. For a table that isn't tracked.
    pub(crate) fn get(
        &self,
        snapshot: &Snapshot,
        key: &K,
        compute: impl FnOnce(&mut Reads<'_>) -> Result<V, Cancelled>,
    ) -> Result<Arc<Memo<V>>, Cancelled> {
        debug_assert!(
            self.tracked.is_none(),
            "a tracked table's entries can be cyclic"
        );
        Ok(self
            .get_or_cycle(snapshot, key, compute)?
            .expect("only tracked entries are cyclic"))
    }

    /// As [`DerivedTable::get`], or `None` if the entry is being verified or computed on this
    /// thread already, so that asking for it is a cycle.
    pub(crate) fn get_or_cycle(
        &self,
        snapshot: &Snapshot,
        key: &K,
        compute: impl FnOnce(&mut Reads<'_>) -> Result<V, Cancelled>,
    ) -> Result<Option<Arc<Memo<V>>>, Cancelled> {
        let revision = snapshot.revision.0;
        let old = read(self.shard(key)).get(key).cloned();
        if let Some(memo) = &old
            && memo.verified_at.load(Ordering::Acquire) == revision
        {
            return Ok(Some(memo.clone()));
        }
        let entered = match self.tracked {
            Some(dep_of) => match enter(&dep_of(key)) {
                Some(entered) => Some(entered),
                None => return Ok(None),
            },
            None => None,
        };
        // Verifying an entry verifies what it read first, so chains of entries recurse.
        ferrion_stack::ensure_stack(|| {
            if let Some(memo) = &old
                && snapshot.unchanged(&memo.deps)?
            {
                memo.verified_at.fetch_max(revision, Ordering::AcqRel);
                self.verified.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(memo.clone()));
            }
            // A cycle found while verifying is found again, or not, by computing.
            if let Some(entered) = &entered {
                entered.reset();
            }
            let mut reads = Reads {
                snapshot,
                deps: Vec::new(),
                frame: entered.as_ref(),
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
                deps: distinct(reads.deps),
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
            Ok(Some(memo))
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

/// `deps` without repeats, in the order first read: a computation that resolves many names reads
/// the same package entries again and again, and one check of each is enough.
fn distinct(deps: Vec<(Dep, Fingerprint)>) -> Box<[(Dep, Fingerprint)]> {
    if deps.len() < 2 {
        return deps.into_boxed_slice();
    }
    let mut seen = std::collections::HashSet::with_capacity(deps.len());
    let mut kept = Vec::with_capacity(deps.len());
    for dep in deps {
        if !seen.contains(&dep) {
            seen.insert(dep.clone());
            kept.push(dep);
        }
    }
    kept.into_boxed_slice()
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
