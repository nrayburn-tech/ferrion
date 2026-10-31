//! Snapshot tests: each `.java` file under `tests/snapshots/` is parsed, and its tree and
//! diagnostics ([`Parse::debug_dump`]) are compared with the `.tree` file next to it. A first line
//! `// release: N` parses at release N; otherwise at the latest release.
//!
//! To write the snapshots after an intended change, run with `FERRION_BLESS=1`, then review the
//! diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_parser --test snapshots
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_java_version::LATEST_RELEASE;
use ferrion_parser::{ParseOptions, invariants, parse};

#[test]
fn snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let bless = std::env::var_os("FERRION_BLESS").is_some();
    let mut inputs = Vec::new();
    collect(&dir, &mut inputs);
    inputs.sort();
    assert!(!inputs.is_empty(), "no inputs in {}", dir.display());
    let mut failures = Vec::new();
    for input in &inputs {
        let bytes = fs::read(input).unwrap();
        let source = SourceText::decode(bytes).unwrap();
        let text: Arc<str> = Arc::from(source.text());
        let release = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// release: "))
            .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
        let parsed = parse(text.clone(), ParseOptions { release });
        let name = input.strip_prefix(&dir).unwrap().display().to_string();
        if let Err(problem) = invariants::check(&text, release, &parsed) {
            failures.push(format!("{name}: {problem}"));
        }
        let actual = parsed.debug_dump();
        let expected_path = input.with_extension("tree");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{name}: the tree differs from {}:\n{actual}",
                    expected_path.display()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} snapshots failed (run with FERRION_BLESS=1 to update them):\n{}",
        failures.len(),
        inputs.len(),
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
