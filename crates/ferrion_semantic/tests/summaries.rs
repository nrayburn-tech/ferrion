//! Snapshot tests: each `.java` file under `tests/summaries/` is parsed and summarized, and the
//! summary ([`Summary::debug_dump`]) is compared with the `.summary` file next to it. A first line
//! `// release: N` parses at release N; otherwise at the latest release. A file named
//! `package-info.java` is summarized as one.
//!
//! To write the snapshots after an intended change, run with `FERRION_BLESS=1`, then review the
//! diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_semantic --test summaries
//! ```

use std::fs;
use std::path::Path;
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, parse};
use ferrion_semantic::summary::{Summary, summarize};

const LATEST_RELEASE: u32 = 27;

#[test]
fn snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/summaries");
    let bless = std::env::var_os("FERRION_BLESS").is_some();
    let mut inputs: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "java"))
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no inputs in {}", dir.display());
    let mut failures = Vec::new();
    for input in &inputs {
        let source = SourceText::decode(fs::read(input).unwrap()).unwrap();
        let text: Arc<str> = Arc::from(source.text());
        let release = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// release: "))
            .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
        let tree = parse(text, ParseOptions { release }).tree;
        let is_package_info = input.file_name().is_some_and(|n| n == "package-info.java");
        let summary: Summary = summarize(&tree, is_package_info);
        let actual = summary.debug_dump();
        let expected_path = input.with_extension("summary");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{}: the summary differs from {}:\n{actual}",
                    input.file_name().unwrap().display(),
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
