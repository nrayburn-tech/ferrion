//! The engine's inputs from a `ferrion_project::Project`: each source set's files read from its
//! roots, and with its opened `TypeInputs`, its classpath and platform API.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_project::{Project, TypeEntry, TypeInputs};

use super::{ClassPathInput, ClassesInput, FileInput, ProjectInput, SourceSetId, SourceSetInput};

/// A file the adapter couldn't read, with why.
#[derive(Clone, Debug)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: String,
}

impl ProjectInput {
    /// Every source set of `project`, with the `.java` files under its source roots and
    /// generated-source roots (those marked generated), in path order. With `types`, each source
    /// set's classpath is its opened entries in order, source sets and jars and directories
    /// interleaved as the model gives them, the module path's after them, and its platform is
    /// its JDK's platform API at its release. Without, the classpath has only the source sets,
    /// and there is no platform: what a run that needs no types can resolve.
    ///
    /// Files that can't be read or aren't UTF-8 are left out and listed, as the formatter skips
    /// them.
    pub fn from_project(
        project: &Project,
        types: Option<&TypeInputs>,
    ) -> (ProjectInput, Vec<SkippedFile>) {
        let mut skipped = Vec::new();
        let mut input = ProjectInput::default();
        for set in project.source_sets() {
            let mut files = Vec::new();
            for (roots, generated) in [(&set.roots, false), (&set.generated_roots, true)] {
                for root in roots {
                    for path in java_files(root) {
                        match read(&path) {
                            Ok(text) => files.push(FileInput {
                                path,
                                text,
                                generated,
                            }),
                            Err(reason) => skipped.push(SkippedFile { path, reason }),
                        }
                    }
                }
            }
            let mut source_set = SourceSetInput::new(&set.name, set.release.value);
            source_set.files = files;
            match types {
                Some(types) => {
                    let types = types.source_set(set.id);
                    source_set.platform = Some(types.platform.clone());
                    source_set.classpath = types
                        .entries
                        .iter()
                        .map(|entry| match entry {
                            TypeEntry::SourceSet(id) => {
                                ClassPathInput::SourceSet(SourceSetId::from_index(id.value.index()))
                            }
                            TypeEntry::Classes(classes) => ClassPathInput::Classes(ClassesInput {
                                entry: classes.entry.clone(),
                                path: classes.path.clone(),
                                module: classes.module.clone(),
                            }),
                        })
                        .collect();
                }
                None => {
                    source_set.classpath = set
                        .classpath
                        .iter()
                        .flatten()
                        .filter_map(|item| match item {
                            ferrion_project::ClassPathItem::SourceSet(id) => {
                                Some(ClassPathInput::SourceSet(SourceSetId::from_index(
                                    id.value.index(),
                                )))
                            }
                            ferrion_project::ClassPathItem::Path(_) => None,
                        })
                        .collect();
                }
            }
            input.source_sets.push(source_set);
        }
        (input, skipped)
    }
}

fn read(path: &Path) -> Result<Arc<str>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let source = SourceText::decode(bytes).map_err(|e| format!("{e:?}"))?;
    Ok(Arc::from(source.text()))
}

/// The `.java` files under `dir`, sorted; none if it doesn't exist. A stack of directories, so
/// a deep tree doesn't recurse; symbolic links to directories aren't followed, so a link cycle
/// can't loop.
fn java_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "java") && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
