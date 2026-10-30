//! Resolving Gradle projects.
//!
//! An init script adds a task to every Gradle project that writes its source sets and resolved
//! compile classpaths as JSON. Nothing is compiled.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::Dirs;
use crate::lock::{ClasspathEntry, Jars, Module, SourceSet, parse_java_version, relative, run};

const INIT_SCRIPT: &str = include_str!("model.gradle");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GradleProject {
    path: String,
    dir: PathBuf,
    source_sets: Vec<GradleSourceSet>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GradleSourceSet {
    name: String,
    release: Option<String>,
    source_roots: Vec<PathBuf>,
    classpath: Vec<GradleEntry>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GradleEntry {
    #[serde(rename_all = "camelCase")]
    Project {
        project: String,
        source_set: String,
    },
    Module {
        group: String,
        module: String,
        version: String,
        file: PathBuf,
    },
    File {
        file: PathBuf,
    },
}

pub fn resolve(checkout: &Path, dirs: &Dirs, jars: &mut Jars) -> Result<Vec<Module>> {
    let out = dirs.corpus.join("gradle-model");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    std::fs::create_dir_all(&out)?;
    let init_script = dirs.corpus.join("ferrion-corpus-model.gradle");
    std::fs::write(&init_script, INIT_SCRIPT)?;
    run(Command::new(checkout.join("gradlew"))
        .current_dir(checkout)
        .env("FERRION_CORPUS_OUT", &out)
        .arg("--init-script")
        .arg(&init_script)
        .args([
            "--no-configuration-cache",
            "--console=plain",
            "--quiet",
            "ferrionCorpusModel",
        ]))?;

    let mut projects = Vec::new();
    for entry in std::fs::read_dir(&out)? {
        let path = entry?.path();
        let text = std::fs::read_to_string(&path)?;
        let project: GradleProject =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        projects.push(project);
    }
    let module_paths: BTreeMap<String, String> = projects
        .iter()
        .map(|project| Ok((project.path.clone(), within(checkout, &project.dir)?)))
        .collect::<Result<_>>()?;
    projects.sort_by(|a, b| module_paths[&a.path].cmp(&module_paths[&b.path]));

    let module_dirs: Vec<(PathBuf, String)> = module_paths
        .values()
        .map(|path| (checkout.join(path), path.clone()))
        .collect();

    let mut modules = Vec::new();
    for project in projects {
        let mut source_sets = Vec::new();
        for source_set in project.source_sets {
            let mut classpath = Vec::new();
            for entry in source_set.classpath {
                let entry = match entry {
                    GradleEntry::Project {
                        project,
                        source_set,
                    } => ClasspathEntry::SourceSet {
                        module: module_paths
                            .get(&project)
                            .with_context(|| format!("unknown Gradle project {project}"))?
                            .clone(),
                        source_set,
                    },
                    GradleEntry::Module {
                        group,
                        module,
                        version,
                        file,
                    } => {
                        let name = file.file_name().unwrap().to_string_lossy();
                        let path = format!("{}/{module}/{version}/{name}", group.replace('.', "/"));
                        jars.add(path, &file)?
                    }
                    GradleEntry::File { file } => match classify(checkout, &module_dirs, &file) {
                        Some(entry) => entry,
                        None => continue,
                    },
                };
                // A module's Java and Kotlin output directories both map to its source set.
                if !classpath.contains(&entry) {
                    classpath.push(entry);
                }
            }
            let source_roots = source_set
                .source_roots
                .iter()
                .map(|root| within(checkout, root))
                .collect::<Result<_>>()?;
            source_sets.push(SourceSet {
                name: source_set.name,
                java_release: source_set.release.as_deref().and_then(parse_java_version),
                source_roots,
                classpath,
            });
        }
        modules.push(Module {
            path: module_paths[&project.path].clone(),
            source_sets,
        });
    }
    Ok(modules)
}

/// Classifies a classpath file that isn't from a Maven repository. Returns `None` for files that
/// don't matter to analysis, such as resource directories.
fn classify(
    checkout: &Path,
    module_dirs: &[(PathBuf, String)],
    file: &Path,
) -> Option<ClasspathEntry> {
    let Some(path) = relative(checkout, file) else {
        return Some(ClasspathEntry::Unavailable {
            unavailable: file.display().to_string(),
        });
    };
    // Another module's build output is recorded as that module's source set.
    if let Some((dir, module)) = module_dirs
        .iter()
        .filter(|(dir, _)| file.starts_with(dir))
        .max_by_key(|(dir, _)| dir.components().count())
        && let Ok(output) = file.strip_prefix(dir.join("build"))
    {
        let parts: Vec<_> = output
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect();
        let source_set = |name: &str| {
            Some(ClasspathEntry::SourceSet {
                module: module.clone(),
                source_set: name.to_owned(),
            })
        };
        match parts.as_slice() {
            [classes, _language, name] if classes == "classes" => return source_set(name),
            [resources, _] if resources == "resources" => return None,
            [libs, jar] if libs == "libs" && jar.ends_with(".jar") => {
                return source_set(if jar.contains("-test-fixtures") {
                    "testFixtures"
                } else {
                    "main"
                });
            }
            _ => {}
        }
    }
    Some(ClasspathEntry::ProjectFile { file: path })
}

fn within(checkout: &Path, path: &Path) -> Result<String> {
    relative(checkout, path).with_context(|| {
        format!(
            "{} is outside the checkout {}",
            path.display(),
            checkout.display()
        )
    })
}
