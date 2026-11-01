//! The project model's JSON format (docs/design/project-inputs.md, "The project model"), for
//! integrations that write it and for [`crate::Project::load`], which reads it.
//!
//! Paths are absolute, or relative to the model file's directory. Unknown fields are rejected.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The newest format version this Ferrion reads, and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectModel {
    pub format_version: u32,
    pub source_sets: Vec<ModelSourceSet>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSourceSet {
    /// Any unique, non-empty string. The Maven plugin uses `<module path>/<main|test>`.
    pub id: String,
    pub java_release: u32,
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
