//! The incremental engine (docs/design/semantic-incremental.md): versioned inputs, a revision
//! counter, memo tables of derived entries, the tree cache, and batch phases.
//!
//! The [`Engine`] holds the inputs and every table. Changing an input bumps the revision and
//! returns at once; it never waits for running work. Work reads through a [`Snapshot`], fixed at
//! the revision it was taken at: its queries return `Err(Cancelled)` once the revision moves, and
//! everything it reads is from its own revision's inputs, since every input is read under the
//! inputs lock with the revision checked.
//!
//! The layers, each a table of derived entries ([`derived`]):
//!
//! 1. inputs: each file's text, path, and whether it is generated, and each source set's
//!    release, dependencies, and files, with a version each;
//! 2. summaries, one per file, from its tree;
//! 3. project symbols: each source set's package index and package entries
//!    ([`crate::packages`]) and module ([`crate::view`]), and each type's supertypes, member
//!    types, and entry ([`crate::symbols`]);
//! 4. each file's names in declarations ([`crate::decl_names`]) and in bodies
//!    ([`crate::body`]).
//!
//! Class sources (platforms and classpath entries) are fixed when the engine is made, and what is
//! read from them is cached without dependencies ([`crate::classes`]).
//!
//! Trees aren't a layer: they are parsed from inputs and kept in a cache with a byte budget.

mod batch;
pub(crate) mod derived;
mod trees;

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::SyntaxTree;

use self::derived::{Dep, DerivedTable, Memo, Reads, read, write};
use self::trees::TreeCache;
use crate::body::BodyNames;
use crate::classes::{Cache, ClassSource, ClassSources};
use crate::decl_names::FileNames;
use crate::input::ClassPathInput;
use crate::packages::{Package, PackageIndex, TopLevel};
use crate::resolve::FileScope;
use crate::summary::{Summary, summarize};
use crate::symbols::{ClassSourceId, MemberTypes, Supertypes, TypeEntry, TypeKey};
use crate::view::{ModuleView, PlatformModules, SetConfig, SetEntry};
use crate::{FileId, FileInput, Fingerprint, Interner, Name, ProjectInput, SourceSetId};

/// The number of input changes so far. Every change makes a new revision, before which no
/// snapshot sees it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Revision(u64);

/// The error a query returns once the revision it was asked at has moved on. An ordinary value
/// passed up with `?`, never a panic: the work is stale and should stop, and be retried at the
/// new revision if it's still wanted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the inputs changed while the analysis ran")
    }
}

impl Error for Cancelled {}

#[derive(Clone, Copy, Debug)]
pub struct EngineOptions {
    /// The bytes of trees kept between phases and queries, counting their node and token arrays.
    pub tree_cache_bytes: usize,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            tree_cache_bytes: 256 << 20,
        }
    }
}

/// One per process: the inputs and every memo table. Shared by reference across threads; work
/// runs on [`Snapshot`]s.
pub struct Engine {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    revision: AtomicU64,
    inputs: RwLock<Inputs>,
    trees: TreeCache,
    parses: AtomicU64,
    pub(crate) summaries: DerivedTable<FileId, FileSummary>,
    pub(crate) package_indexes: DerivedTable<SourceSetId, PackageIndex>,
    pub(crate) packages: DerivedTable<(SourceSetId, Name), Package>,
    pub(crate) module_views: DerivedTable<SourceSetId, ModuleView>,
    pub(crate) file_scopes: DerivedTable<FileId, FileScope>,
    pub(crate) supertypes: DerivedTable<TypeKey, Supertypes>,
    pub(crate) member_types: DerivedTable<(SourceSetId, TypeKey), MemberTypes>,
    pub(crate) type_entries: DerivedTable<TypeKey, TypeEntry>,
    pub(crate) file_names: DerivedTable<FileId, FileNames>,
    pub(crate) body_names: DerivedTable<FileId, BodyNames>,
    pub(crate) names: Arc<Interner>,
    /// Each source set's classpath and platform, by its index.
    pub(crate) configs: Box<[SetConfig]>,
    pub(crate) classes: ClassSources,
    pub(crate) platform_modules: Cache<ClassSourceId, Arc<PlatformModules>>,
}

struct Inputs {
    sets: Vec<SetState>,
    /// By [`FileId`]; `None` once removed.
    files: Vec<Option<FileState>>,
}

struct SetState {
    name: Arc<str>,
    release: u32,
    dependencies: Arc<[SourceSetId]>,
    files: Arc<[FileId]>,
    /// The revision at which `files` last changed.
    files_version: u64,
}

struct FileState {
    set: SourceSetId,
    path: Arc<Path>,
    text: Arc<str>,
    generated: bool,
    /// The revision at which the text, or the source set's release, last changed.
    version: u64,
}

/// A source set as the engine has it.
#[derive(Clone, Debug)]
pub struct SourceSetInfo {
    pub name: Arc<str>,
    pub release: u32,
    pub dependencies: Arc<[SourceSetId]>,
}

/// A file as the engine has it.
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub set: SourceSetId,
    pub path: Arc<Path>,
    pub generated: bool,
    /// The revision at which its text or its source set's release last changed.
    pub version: Revision,
}

/// How much work the engine has done, for tests and benchmarks: counts since it was created.
/// Subtract two to see what one step did.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Stats {
    pub parses: u64,
    pub summaries: TableStats,
    pub package_indexes: TableStats,
    pub packages: TableStats,
    pub module_views: TableStats,
    pub supertypes: TableStats,
    pub member_types: TableStats,
    pub type_entries: TableStats,
    pub file_names: TableStats,
    pub body_names: TableStats,
}

/// What one table's entries did.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct TableStats {
    /// Entries computed: the first time, or because something they read changed.
    pub computed: u64,
    /// Of those, the entries whose new value had the old one's fingerprint, so that whatever had
    /// read them stayed valid (early cutoff).
    pub unchanged: u64,
    /// Entries found valid at a new revision by checking what they read, without computing.
    pub verified: u64,
}

impl std::ops::Sub for Stats {
    type Output = Stats;

    fn sub(self, earlier: Stats) -> Stats {
        Stats {
            parses: self.parses - earlier.parses,
            summaries: self.summaries - earlier.summaries,
            package_indexes: self.package_indexes - earlier.package_indexes,
            packages: self.packages - earlier.packages,
            module_views: self.module_views - earlier.module_views,
            supertypes: self.supertypes - earlier.supertypes,
            member_types: self.member_types - earlier.member_types,
            type_entries: self.type_entries - earlier.type_entries,
            file_names: self.file_names - earlier.file_names,
            body_names: self.body_names - earlier.body_names,
        }
    }
}

impl std::ops::Sub for TableStats {
    type Output = TableStats;

    fn sub(self, earlier: TableStats) -> TableStats {
        TableStats {
            computed: self.computed - earlier.computed,
            unchanged: self.unchanged - earlier.unchanged,
            verified: self.verified - earlier.verified,
        }
    }
}

/// What the engine holds, for measuring memory.
#[derive(Clone, Copy, Default, Debug)]
pub struct Sizes {
    pub files: usize,
    pub trees: usize,
    pub tree_bytes: usize,
    pub summaries: usize,
    pub package_indexes: usize,
    pub packages: usize,
    pub supertypes: usize,
    pub member_types: usize,
    pub type_entries: usize,
    pub file_names: usize,
    pub body_names: usize,
    pub class_entries: usize,
    pub names: usize,
}

impl Engine {
    pub fn new(input: ProjectInput) -> Engine {
        Engine::with_options(input, EngineOptions::default())
    }

    /// An engine over `input`. Panics if a source set's dependency isn't one of its source sets.
    pub fn with_options(input: ProjectInput, options: EngineOptions) -> Engine {
        let revision = 1;
        let count = input.source_sets.len();
        let mut inputs = Inputs {
            sets: Vec::with_capacity(count),
            files: Vec::new(),
        };
        let mut sources = Vec::new();
        let mut configs = Vec::with_capacity(count);
        for (index, set) in input.source_sets.into_iter().enumerate() {
            let dependencies: Vec<SourceSetId> = set.dependencies().collect();
            for dependency in &dependencies {
                assert!(
                    dependency.index() < count,
                    "source set {} depends on {dependency:?}, which isn't one",
                    set.name
                );
            }
            let classpath: Vec<SetEntry> = set
                .classpath
                .into_iter()
                .map(|entry| match entry {
                    ClassPathInput::SourceSet(id) => SetEntry::SourceSet(id),
                    ClassPathInput::Classes(classes) => SetEntry::Classes(ClassSources::add(
                        &mut sources,
                        ClassSource::Entry(classes),
                    )),
                })
                .collect();
            let platform = set
                .platform
                .map(|p| ClassSources::add(&mut sources, ClassSource::Platform(p)));
            configs.push(SetConfig {
                classpath: Arc::from(classpath),
                platform,
            });
            let id = SourceSetId::from_index(index);
            let mut files = Vec::with_capacity(set.files.len());
            for file in set.files {
                files.push(FileId::from_index(inputs.files.len()));
                inputs.files.push(Some(FileState::new(id, file, revision)));
            }
            inputs.sets.push(SetState {
                name: Arc::from(set.name),
                release: set.release,
                dependencies: Arc::from(dependencies),
                files: Arc::from(files),
                files_version: revision,
            });
        }
        Engine {
            shared: Arc::new(Shared {
                revision: AtomicU64::new(revision),
                inputs: RwLock::new(inputs),
                trees: TreeCache::new(options.tree_cache_bytes),
                parses: AtomicU64::new(0),
                summaries: DerivedTable::new(),
                package_indexes: DerivedTable::new(),
                packages: DerivedTable::new(),
                module_views: DerivedTable::new(),
                file_scopes: DerivedTable::new(),
                supertypes: DerivedTable::tracked(|key| Dep::Supertypes(*key)),
                member_types: DerivedTable::tracked(|(view, key)| Dep::MemberTypes(*view, *key)),
                type_entries: DerivedTable::new(),
                file_names: DerivedTable::new(),
                body_names: DerivedTable::new(),
                names: Arc::new(Interner::new()),
                configs: configs.into_boxed_slice(),
                classes: ClassSources::with_sources(sources),
                platform_modules: Cache::new(),
            }),
        }
    }

    pub fn revision(&self) -> Revision {
        Revision(self.shared.revision.load(Ordering::SeqCst))
    }

    /// The engine's names: every [`Name`] its results hold is one of these.
    pub fn names(&self) -> &Arc<Interner> {
        &self.shared.names
    }

    /// A read handle at the current revision.
    pub fn snapshot(&self) -> Snapshot {
        // Under the inputs lock, which every change holds while it bumps the revision and
        // changes the input, so the snapshot's revision is one whose inputs are complete.
        let _inputs = read(&self.shared.inputs);
        Snapshot {
            shared: self.shared.clone(),
            revision: self.revision(),
        }
    }

    /// Replaces `file`'s text. Running work sees the new revision at its next check and stops.
    /// Panics if `file` was removed.
    pub fn set_text(&self, file: FileId, text: Arc<str>) {
        let revision = self.change(|inputs, revision| {
            let state = inputs.file_mut(file);
            state.text = text;
            state.version = revision;
        });
        self.shared.trees.remove_older(file, revision);
    }

    /// Adds a file to `set`, after its others.
    pub fn add_file(&self, set: SourceSetId, file: FileInput) -> FileId {
        let mut id = None;
        self.change(|inputs, revision| {
            let new = FileId::from_index(inputs.files.len());
            inputs.files.push(Some(FileState::new(set, file, revision)));
            let state = &mut inputs.sets[set.index()];
            state.files = state.files.iter().copied().chain([new]).collect();
            state.files_version = revision;
            id = Some(new);
        });
        id.unwrap()
    }

    /// Removes `file` from its source set. Its id isn't reused. Panics if it was removed already.
    pub fn remove_file(&self, file: FileId) {
        let revision = self.change(|inputs, revision| {
            let set = inputs.file_mut(file).set;
            inputs.files[file.index()] = None;
            let state = &mut inputs.sets[set.index()];
            state.files = state.files.iter().copied().filter(|&f| f != file).collect();
            state.files_version = revision;
        });
        self.shared.trees.remove_older(file, revision + 1);
        self.shared.summaries.remove(&file);
    }

    /// Changes the release `set`'s files are parsed and analyzed at, which changes every one of
    /// them.
    pub fn set_release(&self, set: SourceSetId, release: u32) {
        let mut files = Arc::from([]);
        let revision = self.change(|inputs, revision| {
            inputs.sets[set.index()].release = release;
            files = inputs.sets[set.index()].files.clone();
            for &file in files.iter() {
                inputs.file_mut(file).version = revision;
            }
        });
        for &file in files.iter() {
            self.shared.trees.remove_older(file, revision);
        }
    }

    /// Bumps the revision and applies `change` under the inputs lock, so that no snapshot sees a
    /// revision without its change. The lock is held only by other changes and by reads of an
    /// input, never while work is done, so this doesn't wait for running work.
    fn change(&self, change: impl FnOnce(&mut Inputs, u64)) -> u64 {
        let mut inputs = write(&self.shared.inputs);
        let revision = self.shared.revision.fetch_add(1, Ordering::SeqCst) + 1;
        change(&mut inputs, revision);
        revision
    }

    pub fn stats(&self) -> Stats {
        Stats {
            parses: self.shared.parses.load(Ordering::Relaxed),
            summaries: self.shared.summaries.stats(),
            package_indexes: self.shared.package_indexes.stats(),
            packages: self.shared.packages.stats(),
            module_views: self.shared.module_views.stats(),
            supertypes: self.shared.supertypes.stats(),
            member_types: self.shared.member_types.stats(),
            type_entries: self.shared.type_entries.stats(),
            file_names: self.shared.file_names.stats(),
            body_names: self.shared.body_names.stats(),
        }
    }

    pub fn sizes(&self) -> Sizes {
        Sizes {
            files: read(&self.shared.inputs)
                .files
                .iter()
                .filter(|f| f.is_some())
                .count(),
            trees: self.shared.trees.len(),
            tree_bytes: self.shared.trees.bytes(),
            summaries: self.shared.summaries.len(),
            package_indexes: self.shared.package_indexes.len(),
            packages: self.shared.packages.len(),
            supertypes: self.shared.supertypes.len(),
            member_types: self.shared.member_types.len(),
            type_entries: self.shared.type_entries.len(),
            file_names: self.shared.file_names.len(),
            body_names: self.shared.body_names.len(),
            class_entries: self.shared.classes.entries_read(),
            names: self.shared.names.len(),
        }
    }
}

impl FileState {
    fn new(set: SourceSetId, file: FileInput, version: u64) -> FileState {
        FileState {
            set,
            path: Arc::from(file.path),
            text: file.text,
            generated: file.generated,
            version,
        }
    }
}

impl Inputs {
    fn file(&self, file: FileId) -> Option<&FileState> {
        self.files.get(file.index())?.as_ref()
    }

    fn file_mut(&mut self, file: FileId) -> &mut FileState {
        self.files
            .get_mut(file.index())
            .and_then(Option::as_mut)
            .unwrap_or_else(|| panic!("{file:?} isn't one of the engine's files"))
    }
}

/// A file's summary entry: the summary, and the part of it the package table reads, with its own
/// fingerprint so that a change elsewhere in the summary doesn't reach the package table.
pub(crate) struct FileSummary {
    pub(crate) summary: Arc<Summary>,
    pub(crate) top_level: Arc<TopLevel>,
    pub(crate) top_level_fingerprint: Fingerprint,
}

/// The entry's fingerprint is the summary's: the top-level part follows from it.
impl std::hash::Hash for FileSummary {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.summary.hash(state);
    }
}

/// A read handle at one revision. Cheap to clone and to send to another thread; every query
/// checks the revision and returns `Err(Cancelled)` once it has moved, so work started on it
/// stops soon after an input changes.
#[derive(Clone)]
pub struct Snapshot {
    shared: Arc<Shared>,
    pub(crate) revision: Revision,
}

impl Snapshot {
    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub(crate) fn shared(&self) -> &Shared {
        &self.shared
    }

    /// The engine's names: every [`Name`] its results hold is one of these.
    pub fn names(&self) -> &Arc<Interner> {
        &self.shared.names
    }

    /// `Err(Cancelled)` once the engine's revision has moved past this snapshot's.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.shared.revision.load(Ordering::Relaxed) == self.revision.0 {
            Ok(())
        } else {
            Err(Cancelled)
        }
    }

    /// Runs `read` on the inputs, at this snapshot's revision.
    fn inputs<R>(&self, read: impl FnOnce(&Inputs) -> R) -> Result<R, Cancelled> {
        let inputs = derived::read(&self.shared.inputs);
        // Changes bump the revision under the write lock, so a match here means the inputs are
        // this revision's.
        self.check()?;
        Ok(read(&inputs))
    }

    /// The source sets, in the order they were given.
    pub fn source_sets(&self) -> Result<Vec<SourceSetId>, Cancelled> {
        self.inputs(|i| (0..i.sets.len()).map(SourceSetId::from_index).collect())
    }

    pub fn source_set(&self, set: SourceSetId) -> Result<SourceSetInfo, Cancelled> {
        self.inputs(|i| {
            let state = &i.sets[set.index()];
            SourceSetInfo {
                name: state.name.clone(),
                release: state.release,
                dependencies: state.dependencies.clone(),
            }
        })
    }

    /// `set`'s files, in the order they were given and added.
    pub fn files(&self, set: SourceSetId) -> Result<Arc<[FileId]>, Cancelled> {
        self.inputs(|i| i.sets[set.index()].files.clone())
    }

    /// Panics if `file` isn't one of this snapshot's files.
    pub fn file(&self, file: FileId) -> Result<FileInfo, Cancelled> {
        self.inputs(|i| {
            let state = i.file(file).unwrap_or_else(|| missing(file));
            FileInfo {
                set: state.set,
                path: state.path.clone(),
                generated: state.generated,
                version: Revision(state.version),
            }
        })
    }

    /// Whether `file` is one of this snapshot's files.
    pub fn file_exists(&self, file: FileId) -> Result<bool, Cancelled> {
        self.inputs(|i| i.file(file).is_some())
    }

    /// Panics if `file` isn't one of this snapshot's files.
    pub fn text(&self, file: FileId) -> Result<Arc<str>, Cancelled> {
        self.inputs(|i| i.file(file).unwrap_or_else(|| missing(file)).text.clone())
    }

    /// `file`'s tree, from the cache or parsed now. Panics if `file` isn't one of this
    /// snapshot's files.
    pub fn tree(&self, file: FileId) -> Result<Arc<SyntaxTree>, Cancelled> {
        Ok(self.parse(file)?.0)
    }

    /// The tree, the path, and the version it is of.
    fn parse(&self, file: FileId) -> Result<(Arc<SyntaxTree>, Arc<Path>, u64), Cancelled> {
        let (text, path, version, release) = self.inputs(|i| {
            let state = i.file(file).unwrap_or_else(|| missing(file));
            let release = i.sets[state.set.index()].release;
            (
                state.text.clone(),
                state.path.clone(),
                state.version,
                release,
            )
        })?;
        if let Some(tree) = self.shared.trees.get(file, version) {
            return Ok((tree, path, version));
        }
        // Parsing doesn't poll for cancellation; a parse in flight finishes and its tree is kept
        // for its version (semantic-incremental.md#cancellation).
        let tree = Arc::new(parse(text, ParseOptions { release }).tree);
        self.shared.parses.fetch_add(1, Ordering::Relaxed);
        self.shared.trees.insert(file, version, tree.clone());
        Ok((tree, path, version))
    }

    /// `file`'s summary. Panics if `file` isn't one of this snapshot's files.
    pub fn summary(&self, file: FileId) -> Result<Arc<Summary>, Cancelled> {
        self.check()?;
        let memo = self.summary_memo(file)?.unwrap_or_else(|| missing(file));
        Ok(memo.value.summary.clone())
    }

    /// The summary entry, or `None` for a file that isn't in this revision.
    pub(crate) fn summary_memo(
        &self,
        file: FileId,
    ) -> Result<Option<Arc<Memo<FileSummary>>>, Cancelled> {
        if !self.inputs(|i| i.file(file).is_some())? {
            return Ok(None);
        }
        let memo = self.shared.summaries.get(self, &file, |reads| {
            let (tree, path) = reads.tree(file)?;
            let is_package_info = path.file_name().is_some_and(|n| n == "package-info.java");
            let summary = summarize(&tree, is_package_info, &reads.snapshot.shared.names);
            let top_level = TopLevel::of(&summary);
            Ok(FileSummary {
                top_level_fingerprint: Fingerprint::of(&top_level),
                top_level: Arc::new(top_level),
                summary: Arc::new(summary),
            })
        })?;
        Ok(Some(memo))
    }

    /// The current fingerprint of `dep`, verifying or computing the entry it names. The one
    /// place that knows how to read each kind of dependency.
    pub(crate) fn current(&self, dep: &Dep) -> Result<Fingerprint, Cancelled> {
        Ok(match dep {
            Dep::FileInput(file) => self.inputs(|i| {
                i.file(*file).map_or(Fingerprint::ABSENT, |f| {
                    Fingerprint::from_version(f.version)
                })
            })?,
            Dep::SetFiles(set) => {
                self.inputs(|i| Fingerprint::from_version(i.sets[set.index()].files_version))?
            }
            Dep::Summary(file) => self
                .summary_memo(*file)?
                .map_or(Fingerprint::ABSENT, |m| m.fingerprint),
            Dep::TopLevel(file) => self
                .summary_memo(*file)?
                .map_or(Fingerprint::ABSENT, |m| m.value.top_level_fingerprint),
            Dep::PackageIndex(set) => self.package_index_memo(*set)?.fingerprint,
            Dep::PackageFiles(set, name) => self
                .package_index_memo(*set)?
                .value
                .package_files(*name)
                .map_or(Fingerprint::ABSENT, |p| p.fingerprint),
            Dep::Package(set, name) => self.package_memo(*set, *name)?.fingerprint,
            Dep::PackageType(set, package, name) => self
                .package_memo(*set, *package)?
                .value
                .get(*name)
                .map_or(Fingerprint::ABSENT, |t| Fingerprint::of(&t.declarations)),
            Dep::ModuleFiles(set) => {
                Fingerprint::of(self.package_index_memo(*set)?.value.module_files())
            }
            Dep::ModuleView(set) => self.module_view_memo(*set)?.fingerprint,
            Dep::FileScope(file) => self.file_scope_memo(*file)?.fingerprint,
            Dep::Supertypes(key) => self
                .supertypes_memo(*key)?
                .map_or(Fingerprint::CYCLE, |m| m.fingerprint),
            Dep::MemberTypes(view, key) => self
                .member_types_memo(*view, *key)?
                .map_or(Fingerprint::CYCLE, |m| m.fingerprint),
            Dep::TypeEntry(key) => self.type_entry_memo(*key)?.fingerprint,
        })
    }
}

/// The accessors computations read inputs and summaries through, recording each read.
impl Reads<'_> {
    /// A file's tree and path. Records the file's input.
    pub(crate) fn tree(&mut self, file: FileId) -> Result<(Arc<SyntaxTree>, Arc<Path>), Cancelled> {
        let (tree, path, version) = self.snapshot.parse(file)?;
        self.record(Dep::FileInput(file), Fingerprint::from_version(version));
        Ok((tree, path))
    }

    /// A source set's files. Records the list.
    pub(crate) fn set_files(&mut self, set: SourceSetId) -> Result<Arc<[FileId]>, Cancelled> {
        let (files, version) = self.snapshot.inputs(|i| {
            let state = &i.sets[set.index()];
            (state.files.clone(), state.files_version)
        })?;
        self.record(Dep::SetFiles(set), Fingerprint::from_version(version));
        Ok(files)
    }

    /// A file's summary entry. Records the whole summary.
    pub(crate) fn summary_memo_recorded(
        &mut self,
        file: FileId,
    ) -> Result<Arc<Memo<FileSummary>>, Cancelled> {
        let memo = self
            .snapshot
            .summary_memo(file)?
            .unwrap_or_else(|| missing(file));
        self.record(Dep::Summary(file), memo.fingerprint);
        Ok(memo)
    }

    /// A file's summary. Records the whole summary.
    pub(crate) fn summary(&mut self, file: FileId) -> Result<Arc<Summary>, Cancelled> {
        let memo = self
            .snapshot
            .summary_memo(file)?
            .unwrap_or_else(|| missing(file));
        self.record(Dep::Summary(file), memo.fingerprint);
        Ok(memo.value.summary.clone())
    }

    /// A file's package and top-level types. Records only that part of its summary.
    pub(crate) fn top_level(&mut self, file: FileId) -> Result<Arc<TopLevel>, Cancelled> {
        let memo = self
            .snapshot
            .summary_memo(file)?
            .unwrap_or_else(|| missing(file));
        self.record(Dep::TopLevel(file), memo.value.top_level_fingerprint);
        Ok(memo.value.top_level.clone())
    }
}

fn missing(file: FileId) -> ! {
    panic!("{file:?} isn't one of the snapshot's files")
}

/// Accepts a path as a [`FileInput`]'s, for tests and tools.
impl FileInput {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<Arc<str>>) -> FileInput {
        FileInput {
            path: path.into(),
            text: text.into(),
            generated: false,
        }
    }
}
