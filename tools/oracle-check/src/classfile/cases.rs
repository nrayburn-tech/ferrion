//! Class files compiled from Ferrion's own sources, with javap's output for each.
//!
//! `classfile-cases/*.java` exercise every attribute Ferrion reads, every type annotation target
//! outside code, and javap's formats for constants and annotation values. `classes/` holds the
//! class files javac compiles from them and, next to each, `<class>.javap`: javap's normalized
//! output (see tools/javap-oracle). `cargo test` checks that Ferrion's model, rendered, matches
//! each snapshot without a JDK; `oracle classfile-cases` recompiles the sources and reruns javap,
//! and fails if anything changed, and `--bless` writes the new snapshots.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::harness::{Harness, java_of};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/classfile-cases");

fn classes_dir() -> PathBuf {
    Path::new(DIR).join("classes")
}

/// The committed class files, in name order.
pub fn committed() -> Result<Vec<PathBuf>> {
    let mut classes = Vec::new();
    for entry in std::fs::read_dir(classes_dir())? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "class") {
            classes.push(path);
        }
    }
    classes.sort();
    Ok(classes)
}

fn snapshot(class: &Path) -> PathBuf {
    class.with_extension("javap")
}

/// Recompiles the cases and reruns javap; returns whether every class file and snapshot is
/// unchanged. With `bless`, changed ones are written instead.
pub fn run(jar: PathBuf, bless: bool) -> Result<bool> {
    let harness = Harness::new(jar)?;
    let home = PathBuf::from(std::env::var_os("JAVA_HOME").context("JAVA_HOME must be set")?);
    let javac = java_of(&home).with_file_name(if cfg!(windows) { "javac.exe" } else { "javac" });
    let scratch =
        std::env::temp_dir().join(format!("ferrion-classfile-cases-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let mut sources: Vec<PathBuf> = std::fs::read_dir(DIR)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    sources.retain(|p| p.extension().is_some_and(|e| e == "java"));
    sources.sort();
    let status = Command::new(&javac)
        .args([
            "-parameters",
            "-nowarn",
            "--release",
            &ferrion_java_version::LATEST_RELEASE.to_string(),
            "-d",
        ])
        .arg(&scratch)
        .args(&sources)
        .status()
        .with_context(|| format!("running {}", javac.display()))?;
    if !status.success() {
        bail!("javac failed on the cases");
    }
    let mut compiled: Vec<PathBuf> = std::fs::read_dir(&scratch)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    compiled.retain(|p| p.extension().is_some_and(|e| e == "class"));
    compiled.sort();
    let outputs = harness.javap(&compiled, &scratch)?;
    let mut unchanged = true;
    if bless {
        let _ = std::fs::remove_dir_all(classes_dir());
        std::fs::create_dir_all(classes_dir())?;
    }
    for (path, output) in compiled.iter().zip(outputs) {
        let name = path.file_name().context("a class file name")?;
        let target = classes_dir().join(name);
        let bytes = std::fs::read(path)?;
        let lines = output
            .lines
            .with_context(|| format!("javap failed on {}: {:?}", path.display(), output.error))?;
        let text = lines.join("\n") + "\n";
        let same = std::fs::read(&target).ok().as_deref() == Some(&bytes[..])
            && std::fs::read_to_string(snapshot(&target)).ok().as_deref() == Some(&text[..]);
        if !same {
            unchanged = false;
            println!("{} changed", name.to_string_lossy());
        }
        if bless {
            std::fs::write(&target, &bytes)?;
            std::fs::write(snapshot(&target), &text)?;
        }
    }
    if !bless && committed()?.len() != compiled.len() {
        unchanged = false;
        println!(
            "the cases compile to {} classes; {} are committed",
            compiled.len(),
            committed()?.len()
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(unchanged || bless)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrion_classfile::ClassFile;

    use super::*;

    #[test]
    fn rendered_classes_match_javap() {
        let classes = committed().unwrap();
        assert!(classes.len() > 10, "{} cases", classes.len());
        for class in classes {
            let bytes = std::fs::read(&class).unwrap();
            let expected = std::fs::read_to_string(snapshot(&class)).unwrap();
            let parsed = ClassFile::parse(Arc::from(bytes)).unwrap();
            let rendered = super::super::render::render(&parsed).unwrap().join("\n") + "\n";
            if rendered != expected {
                let ours: Vec<String> = rendered.lines().map(str::to_owned).collect();
                let theirs: Vec<String> = expected.lines().map(str::to_owned).collect();
                panic!(
                    "{}: {}",
                    class.display(),
                    super::super::first_difference(&ours, &theirs).unwrap_or_default()
                );
            }
        }
    }
}
