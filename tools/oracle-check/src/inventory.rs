//! The corpus files to compare, and the releases to compare each one at.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_diagnostics::SourceText;

use crate::cache::{SourceKey, source_key};
use ferrion_java_version::LATEST_RELEASE;

/// Which releases files are compared at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Releases {
    /// Each file at its source set's release.
    Own,
    /// Every file at each of these.
    These(Vec<u32>),
}

pub struct SourceFile {
    pub project: String,
    /// The path to give javac.
    pub path: PathBuf,
    /// The path to report: the project's name and the path within it.
    pub display: String,
    pub releases: Vec<u32>,
    pub bytes: Vec<u8>,
    /// The decoded text, or why the file is skipped.
    pub source: Result<SourceText, String>,
    pub key: SourceKey,
}

/// Finds the Java files in the projects' source roots, as their lock files record them.
pub fn collect(dirs: &Dirs, projects: &[&Project], releases: &Releases) -> Result<Vec<SourceFile>> {
    let mut files = Vec::new();
    for project in projects {
        let lock = corpus::lock::read(dirs, project)?
            .with_context(|| format!("{} has no lock file", project.name))?;
        if lock.commit != project.commit {
            bail!(
                "{}: the lock file is for a different commit; run `cargo corpus lock`",
                project.name
            );
        }
        if !corpus::fetch::is_checked_out(project, dirs) {
            bail!(
                "{} isn't checked out at {}; run `cargo corpus checkout {}`",
                project.name,
                project.commit,
                project.name
            );
        }
        let checkout = dirs.checkout(project);
        // A file in more than one source set is compared once, at the first one's release.
        let mut own = BTreeMap::new();
        for module in &lock.modules {
            for source_set in &module.source_sets {
                // A build that doesn't set a release compiles for its JDK's.
                let release = source_set.java_release.unwrap_or(lock.jdk);
                if !ferrion_java_version::is_supported(release) {
                    bail!(
                        "{} {} {} is for Java {release}, which isn't supported",
                        project.name,
                        module.path,
                        source_set.name
                    );
                }
                for root in &source_set.source_roots {
                    for path in java_files(&checkout.join(root))? {
                        own.entry(path).or_insert(release);
                    }
                }
            }
        }
        for (path, release) in own {
            let relative = corpus::lock::relative(&checkout, &path).unwrap();
            let display = format!("{}/{relative}", project.name);
            files.push(read(&project.name, path, display, releases, release)?);
        }
    }
    Ok(files)
}

/// Files named on the command line, as one project. Their own release is the latest.
pub fn collect_paths(paths: &[PathBuf], releases: &Releases) -> Result<Vec<SourceFile>> {
    paths
        .iter()
        .map(|path| {
            let display = path.display().to_string();
            let path = path
                .canonicalize()
                .with_context(|| format!("reading {}", path.display()))?;
            read("files", path, display, releases, LATEST_RELEASE)
        })
        .collect()
}

fn read(
    project: &str,
    path: PathBuf,
    display: String,
    releases: &Releases,
    own: u32,
) -> Result<SourceFile> {
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let key = source_key(&name, &bytes);
    let source = SourceText::decode(bytes.clone()).map_err(|reason| reason.to_string());
    Ok(SourceFile {
        project: project.to_owned(),
        display,
        path,
        releases: match releases {
            Releases::Own => vec![own],
            Releases::These(releases) => releases.clone(),
        },
        bytes,
        source,
        key,
    })
}

/// The `.java` files under `dir`, sorted. Symbolic links aren't followed.
fn java_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "java") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_java_files_in_order() {
        let dir = std::env::temp_dir().join(format!("oracle-check-files-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("b/c")).unwrap();
        for file in ["b/c/C.java", "A.java", "b/B.java", "b/notes.txt", "b/Bjava"] {
            std::fs::write(dir.join(file), "").unwrap();
        }
        let files: Vec<_> = java_files(&dir)
            .unwrap()
            .into_iter()
            .map(|path| corpus::lock::relative(&dir, &path).unwrap())
            .collect();
        assert_eq!(files, ["A.java", "b/B.java", "b/c/C.java"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
