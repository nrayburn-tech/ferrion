//! `corpus generate`: makes what a fetched project's lock names from the build's own output,
//! without building the project (decision 3 of the corpus inputs review in
//! docs/design/semantic.md). That is the roots the build's plugins other than annotation
//! processors generate sources into (Helidon's templates, ANTLR's and protobuf's output,
//! caffeine's code generator's), and the files in the checkout that a source set's classpath,
//! processor path, or patch paths list and that fetching doesn't give: files the build makes
//! for itself, such as the jar on caffeine's processor paths, which its build downloads into
//! `build/errorprone`. What to make comes from the lock, and how from the build:
//!
//! - **Maven:** the modules with such a path run the lifecycle to `process-sources`, with the
//!   lock's local repository, so the sources are as the compiler saw them (Helidon's build
//!   generates ANTLR lexers, then a replacer plugin makes them package-private); a module whose
//!   test source set has one runs to `process-test-sources`, which compiles its main sources
//!   first. Only those modules are built: the others they depend on resolve from the
//!   repositories, since the corpus pins releases.
//! - **Gradle:** an init script (`generate.gradle`) finds the tasks whose declared outputs are
//!   the paths or are inside them, and runs them, with the tasks they depend on.
//!
//! Paths that exist already are left as they are, so running it again does nothing. Each build
//! runs on the JDK the lock names ([`crate::jdks`]); its output goes to
//! `logs/<project>-generate.log` in the corpus directory.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::lock::{self, ClasspathEntry, Lock, run};
use crate::project::{Build, Project};
use crate::{Dirs, fetch, jdks, maven};

const INIT_SCRIPT: &str = include_str!("generate.gradle");

/// A path the lock names that the build makes, and the source set that names it.
#[derive(Debug, PartialEq, Eq)]
struct Wanted {
    module: String,
    source_set: String,
    /// Absolute, in the checkout.
    path: PathBuf,
}

pub fn generate(project: &Project, dirs: &Dirs) -> Result<()> {
    let lock =
        lock::read(dirs, project)?.with_context(|| format!("{} has no lock file", project.name))?;
    if lock.commit != project.commit || !fetch::is_checked_out(project, dirs) {
        bail!(
            "{} isn't fetched at its lock's commit; run `cargo corpus fetch {}`",
            project.name,
            project.name
        );
    }
    let checkout = dirs.checkout(project);
    let wanted: Vec<Wanted> = wanted(&lock, &checkout)
        .into_iter()
        .filter(|w| !w.path.exists())
        .collect();
    if wanted.is_empty() {
        println!("{}: nothing to generate", project.name);
        return Ok(());
    }
    let java_home = jdks::find(lock.jdk)
        .with_context(|| format!("{}'s lock names JDK {}", project.name, lock.jdk))?;
    let log = dirs
        .corpus
        .join("logs")
        .join(format!("{}-generate.log", project.name));
    std::fs::create_dir_all(log.parent().unwrap())?;
    println!(
        "{}: generating {} paths with JDK {} ({}); the build's log is {}",
        project.name,
        wanted.len(),
        lock.jdk,
        java_home.display(),
        log.display()
    );
    match project.build {
        Build::Maven => run_maven(dirs, &checkout, &java_home, &wanted, &log)?,
        Build::Gradle => run_gradle(dirs, &checkout, &java_home, &wanted, &log)?,
    }
    let missing: Vec<String> = wanted
        .iter()
        .filter(|w| !w.path.exists())
        .map(|w| format!("{} ({}/{})", w.path.display(), w.module, w.source_set))
        .collect();
    if !missing.is_empty() {
        bail!(
            "{}: the build didn't make {}; see {}",
            project.name,
            missing.join(", "),
            log.display()
        );
    }
    let files: usize = wanted.iter().map(|w| count_files(&w.path)).sum();
    println!(
        "{}: generated {} paths ({files} files)",
        project.name,
        wanted.len()
    );
    Ok(())
}

/// The paths in the checkout that the lock names and the build may make: every source set's
/// generated-source roots, and the files in the checkout on its classpath, processor path, or
/// patch paths (those committed exist after fetching). In lock order, each once.
fn wanted(lock: &Lock, checkout: &Path) -> Vec<Wanted> {
    let mut found: Vec<Wanted> = Vec::new();
    for module in &lock.modules {
        for set in &module.source_sets {
            let mut add = |path: PathBuf| {
                if !found.iter().any(|w| w.path == path) {
                    found.push(Wanted {
                        module: module.path.clone(),
                        source_set: set.name.clone(),
                        path,
                    });
                }
            };
            for root in &set.generated_source_roots {
                add(checkout.join(root));
            }
            let entries = set
                .classpath
                .iter()
                .chain(set.processor_path.iter().flatten())
                .chain(set.options.patch_modules.values().flatten());
            for entry in entries {
                if let ClasspathEntry::ProjectFile { file } = entry {
                    add(checkout.join(file));
                }
            }
        }
    }
    found
}

fn run_maven(
    dirs: &Dirs,
    checkout: &Path,
    java_home: &Path,
    wanted: &[Wanted],
    log: &Path,
) -> Result<()> {
    // By phase, the modules to run it in.
    let mut phases: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let tests: Vec<&str> = wanted
        .iter()
        .filter(|w| w.source_set == "test")
        .map(|w| w.module.as_str())
        .collect();
    for w in wanted {
        let phase = if tests.contains(&w.module.as_str()) {
            "process-test-sources"
        } else {
            "process-sources"
        };
        let modules = phases.entry(phase).or_default();
        if !modules.contains(&w.module.as_str()) {
            modules.push(&w.module);
        }
    }
    let mut out = File::create(log)?;
    for (phase, modules) in phases {
        use std::io::Write as _;
        writeln!(out, "== {phase} in {}", modules.join(","))?;
        run(Command::new("mvn")
            .current_dir(checkout)
            .env("JAVA_HOME", java_home)
            .args(["--batch-mode", "--no-transfer-progress"])
            .arg(format!(
                "-Dmaven.repo.local={}",
                dirs.corpus.join("maven-repository").display()
            ))
            .args(maven::BUILD_CACHE_OFF)
            .arg("--projects")
            .arg(modules.join(","))
            .arg(phase)
            .stdout(out.try_clone()?)
            .stderr(out.try_clone()?))
        .with_context(|| format!("the build's log is {}", log.display()))?;
    }
    Ok(())
}

fn run_gradle(
    dirs: &Dirs,
    checkout: &Path,
    java_home: &Path,
    wanted: &[Wanted],
    log: &Path,
) -> Result<()> {
    let init_script = dirs.corpus.join("ferrion-corpus-generate.gradle");
    std::fs::write(&init_script, INIT_SCRIPT)?;
    let list = dirs.corpus.join("ferrion-corpus-generate.txt");
    let mut text = String::new();
    for w in wanted {
        text.push_str(&w.path.display().to_string());
        text.push('\n');
    }
    std::fs::write(&list, text)?;
    let out = File::create(log)?;
    run(Command::new(checkout.join("gradlew"))
        .current_dir(checkout)
        .env("JAVA_HOME", java_home)
        .env("FERRION_CORPUS_WANTED", &list)
        .arg("--init-script")
        .arg(&init_script)
        .args([
            "--no-configuration-cache",
            // Every project is configured before the init script looks for the tasks; with
            // caffeine's configuration on demand, the task graph was made before it found them.
            "-Dorg.gradle.configureondemand=false",
            "--console=plain",
            "ferrionCorpusGenerate",
        ])
        .stdout(out.try_clone()?)
        .stderr(out))
    .with_context(|| format!("the build's log is {}", log.display()))
}

/// The files under `path`, or 1 for a file.
fn count_files(path: &Path) -> usize {
    if path.is_file() {
        return 1;
    }
    let mut count = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(_) => count += 1,
                Err(_) => {}
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::{CompileOptions, Module, PlatformKind, SourceSet};

    fn set(name: &str, generated: &[&str], processor_path: Vec<ClasspathEntry>) -> SourceSet {
        SourceSet {
            name: name.to_owned(),
            java_release: Some(21),
            platform: PlatformKind::default(),
            options: CompileOptions::default(),
            source_roots: Vec::new(),
            generated_source_roots: generated.iter().map(|g| (*g).to_owned()).collect(),
            classpath: Vec::new(),
            processor_path: Some(processor_path),
            module_path: Vec::new(),
            processors: None,
            processor_options: Vec::new(),
        }
    }

    #[test]
    fn what_the_lock_wants() {
        let jar = |file: &str| ClasspathEntry::ProjectFile {
            file: file.to_owned(),
        };
        let lock = Lock {
            format_version: 2,
            commit: String::new(),
            jdk: 21,
            modules: vec![Module {
                path: "a".to_owned(),
                source_sets: vec![
                    set(
                        "main",
                        &["a/build/gen"],
                        vec![jar("a/build/p.jar"), jar("a/build/p.jar")],
                    ),
                    set("test", &["a/build/gen"], vec![jar("a/build/p.jar")]),
                ],
            }],
            jars: BTreeMap::new(),
        };
        let checkout = PathBuf::from("/corpus/projects/p");
        let wanted = wanted(&lock, &checkout);
        let paths: Vec<&Path> = wanted.iter().map(|w| w.path.as_path()).collect();
        assert_eq!(
            paths,
            [checkout.join("a/build/gen"), checkout.join("a/build/p.jar")]
        );
        assert_eq!(wanted[0].source_set, "main");
    }
}
