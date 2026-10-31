//! Lock files: what `corpus lock` records about a project, and how it's generated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::project::{Build, Project};
use crate::{Dirs, fetch, gradle, maven};

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lock {
    pub format_version: u32,
    pub commit: String,
    /// The JDK feature version the build ran with.
    pub jdk: u32,
    pub modules: Vec<Module>,
    /// Every jar on any classpath: its path in a Maven repository, and its SHA-256.
    pub jars: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Module {
    /// The module's directory, relative to the project root (`.` for the root).
    pub path: String,
    pub source_sets: Vec<SourceSet>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSet {
    pub name: String,
    /// The `--release` the source set is compiled with, if the build sets one.
    pub java_release: Option<u32>,
    /// Source roots relative to the project root. Only roots that exist are recorded, so generated
    /// sources are not included.
    pub source_roots: Vec<String>,
    pub classpath: Vec<ClasspathEntry>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum ClasspathEntry {
    /// A jar, by its path in a Maven repository.
    Jar { jar: String },
    /// Another source set in the same project, read from source.
    SourceSet {
        module: String,
        #[serde(rename = "sourceSet")]
        source_set: String,
    },
    /// A file committed to the project, relative to the project root.
    ProjectFile { file: String },
    /// A file that isn't from a Maven repository or the project, so it can't be fetched.
    Unavailable { unavailable: String },
}

pub fn read(dirs: &Dirs, project: &Project) -> Result<Option<Lock>> {
    let path = dirs.lock_file(project);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let lock: Lock =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if lock.format_version != FORMAT_VERSION {
        bail!(
            "{}: unsupported format version {}",
            path.display(),
            lock.format_version
        );
    }
    Ok(Some(lock))
}

pub fn lock(project: &Project, dirs: &Dirs) -> Result<()> {
    let java_home = PathBuf::from(
        std::env::var_os("JAVA_HOME").context("JAVA_HOME must point to the JDK to build with")?,
    );
    let jdk = jdk_version(&java_home)?;
    if jdk != project.jdk {
        bail!(
            "{} builds with JDK {}, but JAVA_HOME is JDK {jdk}",
            project.name,
            project.jdk
        );
    }
    let checkout = fetch::checkout(project, dirs)?;
    println!("{}: running the {} build", project.name, project.build);
    let mut jars = Jars::default();
    let modules = match project.build {
        Build::Maven => maven::resolve(&checkout, dirs, &mut jars)?,
        Build::Gradle => gradle::resolve(&checkout, dirs, &mut jars)?,
    };
    let lock = Lock {
        format_version: FORMAT_VERSION,
        commit: project.commit.clone(),
        jdk,
        modules,
        jars: jars.0,
    };
    let path = dirs.lock_file(project);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(&lock)? + "\n")?;
    let unavailable = lock
        .modules
        .iter()
        .flat_map(|module| &module.source_sets)
        .flat_map(|source_set| &source_set.classpath)
        .filter(|entry| matches!(entry, ClasspathEntry::Unavailable { .. }))
        .count();
    println!(
        "{}: wrote {} ({} modules, {} jars, {unavailable} unavailable classpath entries)",
        project.name,
        path.display(),
        lock.modules.len(),
        lock.jars.len(),
    );
    Ok(())
}

/// The jars found while resolving a project.
#[derive(Default)]
pub struct Jars(BTreeMap<String, String>);

impl Jars {
    /// Records a jar at `file`, whose path in a Maven repository is `repository_path`.
    pub fn add(&mut self, repository_path: String, file: &Path) -> Result<ClasspathEntry> {
        if !self.0.contains_key(&repository_path) {
            self.0.insert(repository_path.clone(), sha256(file)?);
        }
        Ok(ClasspathEntry::Jar {
            jar: repository_path,
        })
    }
}

pub fn sha256(file: &Path) -> Result<String> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Reads the JDK feature version from `$JAVA_HOME/release`.
pub fn jdk_version(java_home: &Path) -> Result<u32> {
    let release = std::fs::read_to_string(java_home.join("release"))
        .with_context(|| format!("{} is not a JDK", java_home.display()))?;
    let version = release
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))
        .context("no JAVA_VERSION in the JDK's release file")?
        .trim_matches('"');
    parse_java_version(version).with_context(|| format!("unrecognized JDK version `{version}`"))
}

/// Parses a Java version as written in builds and JDKs: `17`, `1.8`, or `21.0.4`.
pub fn parse_java_version(version: &str) -> Option<u32> {
    let mut parts = version.trim().split(['.', '_', '+', '-']);
    match parts.next()?.parse().ok()? {
        1 => parts.next()?.parse().ok(),
        major => Some(major),
    }
}

/// `path` relative to `root`, with `/` separators, or `None` if it isn't under `root`.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    Some(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

/// Runs `command`, failing if it exits unsuccessfully.
pub fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {command:?}"))?;
    if !status.success() {
        bail!("{command:?} failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_versions() {
        assert_eq!(parse_java_version("1.8"), Some(8));
        assert_eq!(parse_java_version("1.8.0_452"), Some(8));
        assert_eq!(parse_java_version("17"), Some(17));
        assert_eq!(parse_java_version("21.0.4"), Some(21));
        assert_eq!(parse_java_version("27+10"), Some(27));
        assert_eq!(parse_java_version("VERSION_17"), None);
    }

    #[test]
    fn relative_paths() {
        let root = Path::new("/corpus/project");
        assert_eq!(
            relative(root, Path::new("/corpus/project")).as_deref(),
            Some(".")
        );
        assert_eq!(
            relative(root, Path::new("/corpus/project/a/b")).as_deref(),
            Some("a/b")
        );
        assert_eq!(relative(root, Path::new("/elsewhere")), None);
    }
}
