//! The project model's JSON format (docs/design/project-inputs.md, "The project model"), for
//! integrations that write it and for [`crate::Project::load`], which reads it.
//!
//! Paths are absolute, or relative to the model file's directory. Unknown fields are rejected.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The newest format version this Ferrion reads, and the one it writes. Version 2 added how a
/// source set is compiled (`platform`, `enablePreview`, `addModules`, `addExports`, `addReads`,
/// `patchModules`) and `processorPath`; a version 1 model is read as one without them.
pub const FORMAT_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectModel {
    pub format_version: u32,
    pub source_sets: Vec<ModelSourceSet>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSourceSet {
    /// Any unique, non-empty string. The Maven plugin uses `<module path>/<main|test>`.
    pub id: String,
    pub java_release: u32,
    /// Which platform API the code sees. Left out: `release`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub platform: PlatformKind,
    /// javac's `--enable-preview`: the preview features and APIs of `javaRelease`, which must be
    /// the JDK's own release.
    #[serde(default, skip_serializing_if = "is_default")]
    pub enable_preview: bool,
    /// javac's `--add-modules`: more root modules for the unnamed module's module graph, or
    /// `ALL-DEFAULT`, `ALL-SYSTEM`, or `ALL-MODULE-PATH`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_modules: Vec<String>,
    /// javac's `--add-exports`, each `<module>/<package>=<target>(,<target>)*`, the targets
    /// module names or `ALL-UNNAMED`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_exports: Vec<String>,
    /// javac's `--add-reads`, each `<module>=<target>(,<target>)*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_reads: Vec<String>,
    /// javac's `--patch-module`: for a module, the jars and directories whose classes are added
    /// to it or replace its own.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub patch_modules: BTreeMap<String, Vec<PathBuf>>,
    /// Left out: `--jdk`, then `JAVA_HOME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jdk: Option<PathBuf>,
    pub source_roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_source_roots: Vec<PathBuf>,
    /// In order: javac takes the first entry that has a class. Left out: empty.
    #[serde(default)]
    pub classpath: Vec<ModelEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub module_path: Vec<PathBuf>,
    /// The annotation processor path the build gives javac, in order. Left out: processors are
    /// found on the classpath, as javac finds them without `-processorpath`. Ferrion runs no
    /// processors; their output is in `generatedSourceRoots`. The javac oracle uses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processor_path: Option<Vec<ModelEntry>>,
}

/// The platform API a source set's code sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlatformKind {
    /// The API of `javaRelease` alone, as javac's `--release` gives it.
    #[default]
    Release,
    /// The JDK's whole API, as `-source` and `-target` without `--release` give it: every
    /// module of the JDK, internal packages included, which code in the unnamed module sees
    /// as the JDK's module graph allows from release 9, and all of it before.
    Jdk,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// A classpath entry: exactly one of a path (a jar or a directory of classes) and another source
/// set, which is read from source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_set: Option<String>,
}

impl ModelEntry {
    pub fn path(path: impl Into<PathBuf>) -> ModelEntry {
        ModelEntry {
            path: Some(path.into()),
            source_set: None,
        }
    }

    pub fn source_set(id: impl Into<String>) -> ModelEntry {
        ModelEntry {
            path: None,
            source_set: Some(id.into()),
        }
    }
}

impl ProjectModel {
    /// The model as JSON, pretty-printed, ending in a newline. Fails only for a path that isn't
    /// UTF-8.
    pub fn to_json(&self) -> serde_json::Result<String> {
        Ok(serde_json::to_string_pretty(self)? + "\n")
    }
}
