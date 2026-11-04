//! Runs every reduced failure of the `format` fuzz target in `tests/fuzz-regressions/`, and every
//! source file the parser's tests use, through the fuzz target's checks: at each release where the
//! input parses without errors, the four safety properties hold, and at every release, errors or
//! not, a range picked from the input formats as `verify_range` checks. The `javadoc` and
//! `markdown` targets' reflow failures, in `tests/javadoc-regressions/` and
//! `tests/markdown-regressions/`, go through their reflow checks.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};

use ferrion_formatter::invariants;

#[test]
fn fuzz_regressions() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz-regressions");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name != "README.md" && !name.starts_with('.')
        })
        .collect();
    paths.sort();
    check_all(&paths);
}

#[test]
fn javadoc_regressions() {
    check_reflows(
        "tests/javadoc-regressions",
        invariants::check_javadoc_reflow,
    );
}

#[test]
fn markdown_regressions() {
    check_reflows(
        "tests/markdown-regressions",
        invariants::check_markdown_reflow,
    );
}

/// Runs each input in `dir` but its README through `check`, a doc comment target's reflow check.
fn check_reflows(dir: &str, check: fn(&[u8]) -> Result<(), String>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut failures = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() == "README.md" {
            continue;
        }
        count += 1;
        let bytes = fs::read(&path).unwrap();
        if let Err(message) = check(&bytes) {
            failures.push(format!("{}: {message}", path.display()));
        }
    }
    assert!(count > 0, "no inputs in {}", dir.display());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn parser_test_inputs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ferrion_parser/tests");
    let mut paths = Vec::new();
    collect(&dir, &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "no inputs in {}", dir.display());
    check_all(&paths);
}

fn check_all(paths: &[PathBuf]) {
    let mut failures = Vec::new();
    for path in paths {
        let bytes = fs::read(path).unwrap();
        let result = panic::catch_unwind(|| invariants::check_bytes(&bytes));
        let name = path.display();
        match result {
            Ok(Ok(())) => {}
            Ok(Err(message)) => failures.push(format!("{name}: {message}")),
            Err(_) => failures.push(format!("{name}: panicked")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} inputs failed:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "java") {
            out.push(path);
        }
    }
}
