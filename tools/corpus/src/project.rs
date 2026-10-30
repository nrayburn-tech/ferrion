//! The project list in `projects.toml`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

const MAVEN_CENTRAL: &str = "https://repo.maven.apache.org/maven2";

#[derive(Deserialize)]
struct ProjectList {
    project: Vec<Project>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub repository: String,
    /// The tag the commit was taken from.
    pub tag: String,
    pub commit: String,
    pub build: Build,
    /// The JDK feature version needed to run the project's build.
    pub jdk: u32,
    pub notes: String,
    /// Maven repositories to download jars from, in order.
    #[serde(default = "default_repositories")]
    pub repositories: Vec<String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Build {
    Maven,
    Gradle,
}

impl std::fmt::Display for Build {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(match self {
            Build::Maven => "maven",
            Build::Gradle => "gradle",
        })
    }
}

fn default_repositories() -> Vec<String> {
    vec![MAVEN_CENTRAL.to_owned()]
}

impl Project {
    pub fn load_all(path: &Path) -> Result<Vec<Project>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let list: ProjectList =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(list.project)
    }
}
