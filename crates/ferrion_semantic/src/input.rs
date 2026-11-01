//! What the engine is given: source sets, each with its release, the source sets it reads from
//! source, and its files.
//!
//! This is the part of `ferrion_project`'s project model the engine needs, kept here so the
//! engine doesn't depend on how a project is loaded. The CLI and language server build it from a
//! `ferrion_project::Project`; tests and benchmarks build it directly.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

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
    /// The source sets it reads from source, in classpath order, which is the order their types
    /// are found in. Each is the index of an earlier or later source set of the same project; the
    /// project model is acyclic (`ferrion_project` validates it).
    pub dependencies: Vec<SourceSetId>,
    /// Its files: those under its source roots and generated-source roots.
    pub files: Vec<FileInput>,
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
