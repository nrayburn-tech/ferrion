//! The recovery snapshots (docs/design/parser-error-recovery.md, "Testing recovered trees") are
//! the inputs in any `recovery` directory under `tests/snapshots/`, such as
//! `tests/snapshots/recovery/mid-edit/` for the language server's mid-edit states, or an area's
//! own `recovery` directory. `snapshots.rs` compares each with its reviewed tree, and
//! `FERRION_BLESS=1` rewrites the trees as for every snapshot.
//!
//! An input there is broken on purpose, so this checks that each one reports an error: a change
//! that makes one parse cleanly is a regression, or the input no longer tests recovery.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_java_version::LATEST_RELEASE;
use ferrion_parser::{ParseOptions, Severity, parse};

#[test]
fn recovery_inputs_report_errors() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let mut inputs = Vec::new();
    collect(&dir, false, &mut inputs);
    inputs.sort();
    assert!(
        inputs.len() >= 20,
        "only {} recovery inputs in {}",
        inputs.len(),
        dir.display()
    );
    let clean: Vec<String> = inputs
        .iter()
        .filter(|input| {
            let source = SourceText::decode(fs::read(input).unwrap()).unwrap();
            let text: Arc<str> = Arc::from(source.text());
            let release = text
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("// release: "))
                .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
            let parsed = parse(text, ParseOptions { release });
            !parsed
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error)
        })
        .map(|input| input.strip_prefix(&dir).unwrap().display().to_string())
        .collect();
    assert!(
        clean.is_empty(),
        "recovery inputs that parse without errors: {clean:?}"
    );
}

/// The `.java` files under `dir` that are in a `recovery` directory, or under one.
fn collect(dir: &Path, in_recovery: bool, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let recovery = in_recovery || path.file_name().is_some_and(|n| n == "recovery");
            collect(&path, recovery, out);
        } else if in_recovery && path.extension().is_some_and(|e| e == "java") {
            out.push(path);
        }
    }
}
