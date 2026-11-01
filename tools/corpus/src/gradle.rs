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
                        let path = repository_path(&group, &module, &version, &file);
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

/// A jar's path in a Maven repository. Gradle caches a jar under the name its module's Gradle
/// metadata gives it, which can differ from the file in the repository: Kotlin Multiplatform
/// modules call `clikt-core-jvm-5.0.3.jar` `clikt-jvm.jar`, and Guava's `-android` versions point
/// to the `-jre` version's jar. The metadata's URL for the file, relative to the module version's
/// directory in the repository, gives the path. Gradle caches the metadata in a sibling of the
/// jar's directory.
fn repository_path(group: &str, module: &str, version: &str, file: &Path) -> String {
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    let url = file
        .parent()
        .and_then(Path::parent)
        .and_then(|dir| cached_metadata(dir, &format!("{module}-{version}.module")))
        .and_then(|metadata| metadata_url(&metadata, &name))
        .unwrap_or(name);
    resolve_url(group, module, version, &url)
}

fn cached_metadata(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| std::fs::read_to_string(entry.path().join(name)).ok())
}

/// The URL that a Gradle module metadata file gives for the file named `name`.
fn metadata_url(metadata: &str, name: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Metadata {
        #[serde(default)]
        variants: Vec<Variant>,
    }
    #[derive(Deserialize)]
    struct Variant {
        #[serde(default)]
        files: Vec<VariantFile>,
    }
    #[derive(Deserialize)]
    struct VariantFile {
        name: String,
        url: String,
    }
    let metadata: Metadata = serde_json::from_str(metadata).ok()?;
    metadata
        .variants
        .into_iter()
        .flat_map(|variant| variant.files)
        .find(|file| file.name == name)
        .map(|file| file.url)
}

/// The repository path of `url`, relative to the directory of `version` of `group:module`.
fn resolve_url(group: &str, module: &str, version: &str, url: &str) -> String {
    let mut parts: Vec<&str> = group.split('.').chain([module, version]).collect();
    for part in url.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "" | "." => {}
            part => parts.push(part),
        }
    }
    parts.join("/")
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
                return source_set(if is_test_fixtures_jar(jar, &module_name(dir)) {
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

fn module_name(dir: &Path) -> String {
    dir.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// Whether a jar in a module's `build/libs` holds its test fixtures, whose classifier is
/// `test-fixtures`. The module's own jar can end that way too, when the module is named
/// `*-test-fixtures` (lucene's `spatial-test-fixtures`) and the jar has no version.
fn is_test_fixtures_jar(jar: &str, module: &str) -> bool {
    let stem = jar.strip_suffix(".jar").unwrap_or(jar);
    stem.ends_with("-test-fixtures")
        && (!stem.ends_with(module) || stem.ends_with(&format!("{module}-test-fixtures")))
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

#[cfg(test)]
mod tests {
    use super::*;

    const METADATA: &str = r#"{
      "formatVersion": "1.1",
      "variants": [
        {"name": "androidApiElements",
         "files": [{"name": "guava-33.4.3-android.jar", "url": "guava-33.4.3-android.jar"}]},
        {"name": "jreApiElements",
         "files": [{"name": "guava-33.4.3-jre.jar", "url": "../33.4.3-jre/guava-33.4.3-jre.jar"}]},
        {"name": "platform", "dependencies": []}
      ]
    }"#;

    #[test]
    fn metadata_urls() {
        assert_eq!(
            metadata_url(METADATA, "guava-33.4.3-jre.jar").as_deref(),
            Some("../33.4.3-jre/guava-33.4.3-jre.jar")
        );
        assert_eq!(metadata_url(METADATA, "other.jar"), None);
        assert_eq!(metadata_url("not json", "guava-33.4.3-jre.jar"), None);
    }

    #[test]
    fn test_fixtures_jars() {
        assert!(is_test_fixtures_jar(
            "lucene-core-10.5.2-test-fixtures.jar",
            "core"
        ));
        assert!(is_test_fixtures_jar("core-test-fixtures.jar", "core"));
        assert!(!is_test_fixtures_jar("lucene-core-10.5.2.jar", "core"));
        // A module named `*-test-fixtures`: its own jar, with and without a version, and its
        // fixtures.
        let module = "spatial-test-fixtures";
        assert!(!is_test_fixtures_jar(
            "lucene-spatial-test-fixtures-10.5.2.jar",
            module
        ));
        assert!(!is_test_fixtures_jar(
            "lucene-spatial-test-fixtures.jar",
            module
        ));
        assert!(is_test_fixtures_jar(
            "lucene-spatial-test-fixtures-test-fixtures.jar",
            module
        ));
    }

    #[test]
    fn resolved_urls() {
        assert_eq!(
            resolve_url(
                "com.github.ajalt.clikt",
                "clikt-core-jvm",
                "5.0.3",
                "clikt-core-jvm-5.0.3.jar"
            ),
            "com/github/ajalt/clikt/clikt-core-jvm/5.0.3/clikt-core-jvm-5.0.3.jar"
        );
        assert_eq!(
            resolve_url(
                "com.google.guava",
                "guava",
                "33.4.3-android",
                "../33.4.3-jre/guava-33.4.3-jre.jar"
            ),
            "com/google/guava/guava/33.4.3-jre/guava-33.4.3-jre.jar"
        );
    }
}
