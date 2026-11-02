//! Batch runs: phases over worker threads (docs/design/semantic-incremental.md#threads).
//!
//! Each phase hands its items out one at a time from an atomic counter, so a thread that
//! finishes early takes the next item and a large file holds up only its own thread. A phase
//! ends when every item is done, and the next phase starts from there: the package tables need
//! every summary, and bodies need the package tables.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{Cancelled, Snapshot};
use crate::{FileId, Name, SourceSetId};

impl Snapshot {
    /// Analyzes every file of every source set over `threads` workers, in phases:
    ///
    /// 1. every file's summary, parsing it;
    /// 2. each source set's package index, then every package entry;
    /// 3. the type entry of every type each file declares, with the supertypes and member types
    ///    they read;
    /// 4. for every file, last first, so that the trees the first phase left in the cache are
    ///    used before they are evicted: its names in declarations and in bodies, then `each`.
    ///
    /// Stops with `Err(Cancelled)` once the revision moves, or as soon as `each` returns it.
    /// With one thread, everything runs on the calling thread.
    pub fn run_batch(
        &self,
        threads: usize,
        each: impl Fn(FileId) -> Result<(), Cancelled> + Sync,
    ) -> Result<(), Cancelled> {
        let files = self.run_symbols(threads)?;
        parallel(self, threads, files.len(), |i| {
            let file = files[files.len() - 1 - i];
            self.decl_names(file)?;
            self.body_names(file)?;
            each(file)?;
            // A file's names are for its rules, and are dropped once they have run
            // (semantic-incremental.md#body-results); asking again computes them.
            self.shared().file_names.remove(&file);
            self.shared().body_names.remove(&file);
            Ok(())
        })
    }

    /// The batch's first three phases: everything project-wide, without any file's names. What
    /// a language server brings up to date after an edit before it answers for open files.
    /// Gives every file, in the order the phases took them.
    pub fn run_symbols(&self, threads: usize) -> Result<Vec<FileId>, Cancelled> {
        let mut files = Vec::new();
        let sets = self.source_sets()?;
        for &set in &sets {
            files.extend(self.files(set)?.iter().copied());
        }
        parallel(self, threads, files.len(), |i| {
            self.summary(files[i]).map(drop)
        })?;

        parallel(self, threads, sets.len(), |i| {
            self.package_index(sets[i]).map(drop)
        })?;
        let mut packages: Vec<(SourceSetId, Name)> = Vec::new();
        for &set in &sets {
            let index = self.package_index(set)?;
            packages.extend(index.names().map(|name| (set, name)));
        }
        parallel(self, threads, packages.len(), |i| {
            let (set, name) = packages[i];
            self.package_memo(set, name).map(drop)
        })?;

        parallel(self, threads, files.len(), |i| {
            let file = files[i];
            let scope = self.file_scope_memo(file)?;
            for &key in scope.value.keys.iter() {
                self.type_entry_memo(key)?;
            }
            Ok(())
        })?;
        Ok(files)
    }
}

/// Runs `work` for each index below `count` over `threads` threads, stopping at the first
/// `Err(Cancelled)` or once the snapshot's revision moves. A panic in any thread is passed on.
pub(crate) fn parallel(
    snapshot: &Snapshot,
    threads: usize,
    count: usize,
    work: impl Fn(usize) -> Result<(), Cancelled> + Sync,
) -> Result<(), Cancelled> {
    let next = AtomicUsize::new(0);
    // Set when a worker stops with `Cancelled`, so that the others stop too even if the
    // revision hasn't moved (`work` may return it for its own reasons).
    let stopped = AtomicBool::new(false);
    let worker = || -> Result<(), Cancelled> {
        loop {
            snapshot.check()?;
            if stopped.load(Ordering::Relaxed) {
                return Err(Cancelled);
            }
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= count {
                return Ok(());
            }
            if let Err(cancelled) = work(i) {
                stopped.store(true, Ordering::Relaxed);
                return Err(cancelled);
            }
        }
    };
    let threads = threads.clamp(1, count.max(1));
    if threads == 1 {
        return worker();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|_| scope.spawn(worker)).collect();
        let mut result = Ok(());
        for handle in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(Cancelled)) => result = Err(Cancelled),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        result
    })
}
