//! Project models for fetched corpus projects (docs/design/project-inputs.md), written from their
//! lock files, for the javac oracle's name comparison and the semantic benchmarks.
//!
//! Each lock source set becomes a model source set with the id `<module path>/<name>` (just
//! `<name>` for the root module), at its release (the lock's JDK when the build sets none), with
//! absolute paths: source roots in the checkout, jars under the corpus's `jars` directory, and the
//! generated-source root `generated/<project>/<id>` in the corpus directory, which the oracle
//! fills by running the build's annotation processors. The JDK is the one passed in, for every
//! source set; the newest GA JDK serves every release the corpus uses.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ferrion_project::model::{FORMAT_VERSION, ModelEntry, ModelSourceSet, ProjectModel};

use crate::lock::{self, ClasspathEntry};
use crate::project::Project;
use crate::{Dirs, fetch};

/// The id of a lock source set in the model.
pub fn source_set_id(module: &str, source_set: &str) -> String {
    if module == "." {
        source_set.to_owned()
    } else {
        format!("{module}/{source_set}")
    }
}

/// Where the oracle writes the generated sources of a source set.
pub fn generated_root(dirs: &Dirs, project: &Project, id: &str) -> PathBuf {
    dirs.corpus.join("generated").join(&project.name).join(id)
}

/// Where `corpus project-model` writes a project's model.
pub fn model_file(dirs: &Dirs, project: &Project) -> PathBuf {
    dirs.corpus
        .join("models")
        .join(format!("{}.json", project.name))
}

/// The lock's classpath entries a model leaves out.
#[derive(Default)]
pub struct LeftOut {
    /// Entries the lock marks unavailable.
    pub unavailable: usize,
    /// References to source sets the lock doesn't have, as `(source set, missing source set)`.
    pub missing_source_sets: Vec<(String, String)>,
}

/// The model of a checked-out project, with `jdk` as every source set's JDK if given, and what it
/// leaves out of the lock.
pub fn project_model(
    dirs: &Dirs,
    project: &Project,
    jdk: Option<&Path>,
) -> Result<(ProjectModel, LeftOut)> {
    let lock =
        lock::read(dirs, project)?.with_context(|| format!("{} has no lock file", project.name))?;
    if lock.commit != project.commit {
        bail!(
            "{}: the lock file is for a different commit; run `cargo corpus lock`",
            project.name
        );
    }
    if !fetch::is_checked_out(project, dirs) {
        bail!(
            "{} isn't checked out at {}; run `cargo corpus fetch {}`",
            project.name,
            project.commit,
            project.name
        );
    }
    let checkout = dirs.checkout(project);
    let ids: HashSet<String> = lock
        .modules
        .iter()
        .flat_map(|module| {
            module
                .source_sets
                .iter()
                .map(|set| source_set_id(&module.path, &set.name))
        })
        .collect();
    let mut left_out = LeftOut::default();
    let mut source_sets = Vec::new();
    for module in &lock.modules {
        for set in &module.source_sets {
            let id = source_set_id(&module.path, &set.name);
            let mut classpath = Vec::with_capacity(set.classpath.len());
            for entry in &set.classpath {
                match entry {
                    ClasspathEntry::Jar { jar } => {
                        classpath.push(ModelEntry::path(dirs.jars().join(jar)));
                    }
                    ClasspathEntry::SourceSet { module, source_set } => {
                        let other = source_set_id(module, source_set);
                        if ids.contains(&other) {
                            classpath.push(ModelEntry::source_set(other));
                        } else {
                            left_out.missing_source_sets.push((id.clone(), other));
                        }
                    }
                    ClasspathEntry::ProjectFile { file } => {
                        classpath.push(ModelEntry::path(checkout.join(file)));
                    }
                    ClasspathEntry::Unavailable { .. } => left_out.unavailable += 1,
                }
            }
            source_sets.push(ModelSourceSet {
                generated_source_roots: vec![generated_root(dirs, project, &id)],
                id,
                // A build that doesn't set a release compiles for its JDK's.
                java_release: set.java_release.unwrap_or(lock.jdk),
                jdk: jdk.map(Path::to_path_buf),
                source_roots: set
                    .source_roots
                    .iter()
                    .map(|root| checkout.join(root))
                    .collect(),
                classpath,
                module_path: Vec::new(),
            });
        }
    }
    let model = ProjectModel {
        format_version: FORMAT_VERSION,
        source_sets,
    };
    Ok((model, left_out))
}

/// Writes a project's model to [`model_file`], and returns its path.
pub fn write(dirs: &Dirs, project: &Project, jdk: Option<&Path>) -> Result<PathBuf> {
    let (model, left_out) = project_model(dirs, project, jdk)?;
    let path = model_file(dirs, project);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, model.to_json()?)?;
    println!(
        "{}: wrote {} ({} source sets)",
        project.name,
        path.display(),
        model.source_sets.len()
    );
    if left_out.unavailable > 0 {
        println!(
            "{}: left out {} unavailable classpath entries",
            project.name, left_out.unavailable
        );
    }
    for (set, missing) in &left_out.missing_source_sets {
        println!(
            "{}: left out {set}'s dependency on {missing}, which the lock doesn't have",
            project.name
        );
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ferrion_project::{Environment, InputFlags, SourceSetId, TypeInputs};

    use super::*;
    use crate::jdk_tests::java_files;

    /// The projects that are checked out, each with its model written to a scratch directory and
    /// loaded. Projects that aren't checked out are skipped, so this runs where the corpus is
    /// fetched (`target/corpus`, or `FERRION_CORPUS_DIR`).
    fn checked_out(jdk: Option<&Path>) -> Vec<(Project, ferrion_project::Project)> {
        let dirs = Dirs::new().unwrap();
        // Per test as well as per process, since tests run in parallel.
        let scratch = std::env::temp_dir().join(format!(
            "ferrion-corpus-model-{}-{}",
            std::process::id(),
            if jdk.is_some() { "types" } else { "sources" }
        ));
        std::fs::create_dir_all(&scratch).unwrap();
        let mut loaded = Vec::new();
        for project in Project::load_all(&dirs.projects_file()).unwrap() {
            let locked = lock::read(&dirs, &project).unwrap().is_some();
            if !locked || !fetch::is_checked_out(&project, &dirs) {
                eprintln!("{}: not checked out, skipped", project.name);
                continue;
            }
            let (model, _) = project_model(&dirs, &project, jdk).unwrap();
            let path = scratch.join(format!("{}.json", project.name));
            std::fs::write(&path, model.to_json().unwrap()).unwrap();
            let flags = InputFlags {
                project_model: Some(path),
                ..InputFlags::default()
            };
            let env = Environment {
                working_dir: scratch.clone(),
                java_home: None,
            };
            let loaded_project =
                ferrion_project::Project::load(&flags, &env).unwrap_or_else(|errors| {
                    let errors: Vec<String> = errors
                        .iter()
                        .filter(|e| e.is_error())
                        .map(ToString::to_string)
                        .collect();
                    panic!("{}:\n{}", project.name, errors.join("\n"))
                });
            // Only the generated-source roots, which the oracle writes, may be missing.
            for warning in loaded_project.warnings() {
                assert!(
                    warning.message.starts_with("generated-source root"),
                    "{}: {warning}",
                    project.name
                );
            }
            loaded.push((project, loaded_project));
        }
        let _ = std::fs::remove_dir_all(&scratch);
        loaded
    }

    #[test]
    fn every_source_file_is_in_exactly_its_lock_source_sets() {
        let dirs = Dirs::new().unwrap();
        for (project, model) in checked_out(None) {
            let lock = lock::read(&dirs, &project).unwrap().unwrap();
            let checkout = dirs.checkout(&project);
            let ids: BTreeMap<&str, SourceSetId> = model
                .source_sets()
                .iter()
                .map(|set| (set.name.as_str(), set.id))
                .collect();
            let mut expected: BTreeMap<PathBuf, Vec<SourceSetId>> = BTreeMap::new();
            for module in &lock.modules {
                for set in &module.source_sets {
                    let id = ids[source_set_id(&module.path, &set.name).as_str()];
                    for root in &set.source_roots {
                        for file in java_files(&checkout.join(root)).unwrap() {
                            let sets = expected.entry(file).or_default();
                            if !sets.contains(&id) {
                                sets.push(id);
                            }
                        }
                    }
                }
            }
            assert!(!expected.is_empty(), "{}", project.name);
            for (file, sets) in &expected {
                assert_eq!(&model.source_sets_of(file), sets, "{}", file.display());
                let first = model.source_set(sets[0]);
                assert_eq!(model.release_of(file).unwrap().0, first.release.value);
            }
            eprintln!(
                "{}: {} files in {} source sets",
                project.name,
                expected.len(),
                model.source_sets().len()
            );
        }
    }

    /// Opens every checked-out project's type inputs with the JDK at `JAVA_HOME`, which must
    /// serve every release the corpus uses. Needs the jars fetched (`cargo corpus fetch`).
    #[test]
    #[ignore = "needs the corpus's jars fetched, and JAVA_HOME set to the newest JDK"]
    fn type_inputs_open() {
        let jdk = PathBuf::from(std::env::var_os("JAVA_HOME").expect("JAVA_HOME is set"));
        let cache = ferrion_classfile::ArchiveCache::new();
        for (project, model) in checked_out(Some(&jdk)) {
            let (types, warnings) = TypeInputs::open(&model, &cache).unwrap_or_else(|errors| {
                let errors: Vec<String> = errors
                    .iter()
                    .filter(|e| e.is_error())
                    .map(ToString::to_string)
                    .collect();
                panic!("{}:\n{}", project.name, errors.join("\n"))
            });
            let entries: usize = types.source_sets().iter().map(|s| s.entries.len()).sum();
            eprintln!(
                "{}: {entries} classpath entries, {} warnings",
                project.name,
                warnings.len()
            );
            for warning in &warnings {
                eprintln!("  {warning}");
            }
        }
    }
}
