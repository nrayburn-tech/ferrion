//! What the engine is given: source sets, each with its release, its classpath in order (the
//! source sets it reads from source, jars, and directories), its platform API, and its files.
//!
//! This is the part of `ferrion_project`'s project model the engine needs, kept in the engine's
//! own types so the engine doesn't depend on how a project is loaded: [`ProjectInput::from_project`]
//! builds it from a `ferrion_project::Project` and its opened `TypeInputs`, and tests and
//! benchmarks build it directly.

mod project;

pub use project::SkippedFile;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use ferrion_classfile::class::ModuleDescriptor;
use ferrion_classfile::{
    ClassBytes, ClassName, ClassPathEntry, PackageListing, PackageName, PlatformApi,
};

/// A project's source sets.
#[derive(Clone, Debug, Default)]
pub struct ProjectInput {
    /// Each source set's [`SourceSetId`] is its index here.
    pub source_sets: Vec<SourceSetInput>,
}

#[derive(Clone, Debug)]
pub struct SourceSetInput {
    /// The project model's id, such as `core/main`, for messages.
    pub name: String,
    /// The release its files are parsed and analyzed at.
    pub release: u32,
    /// Its classpath, in the order types are found in it: other source sets of the project, read
    /// from source (the project model is acyclic; `ferrion_project` validates it), and jars and
    /// directories of classes, the module path's included.
    pub classpath: Vec<ClassPathInput>,
    /// The platform API its names see first, for the packages it owns. `None` leaves every
    /// platform name unresolved, which only tests want.
    pub platform: Option<Arc<dyn Platform>>,
    /// Its files: those under its source roots and generated-source roots.
    pub files: Vec<FileInput>,
}

/// One entry of a source set's classpath.
#[derive(Clone, Debug)]
pub enum ClassPathInput {
    /// Another source set of the project, read from source in its own view.
    SourceSet(SourceSetId),
    /// A jar or a directory of class files.
    Classes(ClassesInput),
}

#[derive(Clone, Debug)]
pub struct ClassesInput {
    pub entry: ClassPathEntry,
    /// Where it is, for symbol keys and messages.
    pub path: PathBuf,
    /// For a module path entry, its module's name (from its `module-info.class`, its manifest's
    /// `Automatic-Module-Name`, or its file name), so `requires` and `import module` find it.
    pub module: Option<String>,
}

/// The platform API a source set sees: B1's [`PlatformApi`], or a stand-in for tests.
pub trait Platform: Send + Sync + fmt::Debug {
    /// Any class the platform has, exported or not.
    fn find(&self, name: &ClassName) -> ferrion_classfile::Result<Option<ClassBytes>>;
    /// The class names in `name`, or `None` when the platform has no such package.
    fn package(&self, name: &PackageName) -> ferrion_classfile::Result<Option<PackageListing>>;
    /// The module descriptors; empty for release 8, which has no modules.
    fn modules(&self) -> &[ModuleDescriptor];
    /// Whether code in the unnamed module can name classes in `package`.
    fn is_visible(&self, package: &PackageName) -> bool;
    /// The module of the class `name`, when the platform has it and knows its module.
    fn module_of(&self, name: &ClassName) -> Option<String>;
    /// The jar or directory the class `name` comes from when a `--patch-module` adds it to the
    /// platform, rather than the platform's own; `None` for the platform's classes.
    fn patched_from(&self, _name: &ClassName) -> Option<PathBuf> {
        None
    }
}

impl Platform for PlatformApi {
    fn find(&self, name: &ClassName) -> ferrion_classfile::Result<Option<ClassBytes>> {
        PlatformApi::find(self, name)
    }

    fn package(&self, name: &PackageName) -> ferrion_classfile::Result<Option<PackageListing>> {
        PlatformApi::package(self, name)
    }

    fn modules(&self) -> &[ModuleDescriptor] {
        PlatformApi::modules(self)
    }

    fn is_visible(&self, package: &PackageName) -> bool {
        PlatformApi::is_visible(self, package)
    }

    fn module_of(&self, name: &ClassName) -> Option<String> {
        PlatformApi::module_of(self, name)
    }
}

impl SourceSetInput {
    /// A source set with no classpath, no platform, and no files.
    pub fn new(name: impl Into<String>, release: u32) -> SourceSetInput {
        SourceSetInput {
            name: name.into(),
            release,
            classpath: Vec::new(),
            platform: None,
            files: Vec::new(),
        }
    }

    /// The source sets it reads from source, in classpath order.
    pub fn dependencies(&self) -> impl Iterator<Item = SourceSetId> + '_ {
        self.classpath.iter().filter_map(|entry| match entry {
            ClassPathInput::SourceSet(id) => Some(*id),
            ClassPathInput::Classes(_) => None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FileInput {
    pub path: PathBuf,
    /// The decoded text, as `ferrion_diagnostics::SourceText` gives it.
    pub text: Arc<str>,
    /// Under a generated-source root, so the linter and formatter skip it.
    pub generated: bool,
}

/// A source set, by its index in [`ProjectInput::source_sets`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceSetId(u32);

impl SourceSetId {
    pub const fn from_index(index: usize) -> SourceSetId {
        SourceSetId(index as u32)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A file of one source set. A file under the roots of two source sets is two files, analyzed in
/// each (docs/design/semantic-names.md#duplicates). Ids are never reused, so an id of a removed
/// file never names another.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    pub(crate) fn from_index(index: usize) -> FileId {
        FileId(index as u32)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for SourceSetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SourceSetId({})", self.0)
    }
}

impl fmt::Debug for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileId({})", self.0)
    }
}
