//! Analysis inputs: source sets, the project model, and classpath and JDK validation
//! (docs/design/project-inputs.md).
//!
//! Two stages. [`Project::load`] reads flags, the environment, and the project model, and gives
//! the source sets with their roots and releases, which is all the formatter needs.
//! [`TypeInputs::open`] opens the JDKs, jars, and directories, only when an enabled rule needs
//! types.
//!
//! The CLI parses the command line into [`InputFlags`] and reads the process into an
//! [`Environment`], so loading is a pure function of them, the model file, and the classpath
//! file. Every value carries its [`Origin`], and every [`InputError`] names it.

mod error;
pub mod model;
mod paths;
mod project;
#[cfg(test)]
mod testing;
mod types;

use std::path::PathBuf;

pub use error::{InputError, Origin, Severity};
pub use model::PlatformKind;
pub use paths::PATH_SEPARATOR;
pub use project::{
    AddExports, AddReads, ClassPathItem, CompileOptions, Given, PatchModule, Project, SourceSet,
    SourceSetId,
};
pub use types::{ClassesEntry, SourceSetTypes, TypeEntry, TypeInputs};

/// Flag values as the CLI parsed them. `None` is "not given".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputFlags {
    pub java_version: Option<u32>,
    /// Entries separated by [`PATH_SEPARATOR`]. `Some("")` is an empty classpath, which differs
    /// from `None`.
    pub classpath: Option<String>,
    pub classpath_file: Option<PathBuf>,
    pub module_path: Option<String>,
    pub jdk: Option<PathBuf>,
    pub project_model: Option<PathBuf>,
}

/// What loading reads from the process, passed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    /// Absolute. Relative flag values are against it.
    pub working_dir: PathBuf,
    pub java_home: Option<PathBuf>,
}
