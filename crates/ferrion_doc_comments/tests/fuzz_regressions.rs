//! Runs every reduced fuzz failure in `tests/fuzz-regressions/` through the fuzz targets' checks:
//! the files in `javadoc/` through the `javadoc` target's, and those in `markdown/` through the
//! `markdown` target's.

use std::fs;
use std::panic;
use std::path::Path;

use ferrion_doc_comments::invariants;

/// A fuzz target's checks.
type Check = fn(&[u8]) -> Result<(), String>;

#[test]
fn fuzz_regressions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz-regressions");
    let targets: [(&str, Check); 2] = [
        ("javadoc", invariants::check_javadoc_bytes),
        ("markdown", invariants::check_markdown_bytes),
    ];
    let mut checked = 0;
    let mut failures = Vec::new();
    for (target, check) in targets {
        let mut paths: Vec<_> = fs::read_dir(root.join(target))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| !path.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        paths.sort();
        for path in &paths {
            let bytes = fs::read(path).unwrap();
            let result = panic::catch_unwind(|| check(&bytes));
            let name = format!("{target}/{}", path.file_name().unwrap().to_string_lossy());
            match result {
                Ok(Ok(())) => {}
                Ok(Err(message)) => failures.push(format!("{name}: {message}")),
                Err(_) => failures.push(format!("{name}: panicked")),
            }
        }
        checked += paths.len();
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} inputs failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
