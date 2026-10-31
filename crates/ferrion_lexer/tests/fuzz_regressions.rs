//! Runs every reduced fuzz failure in `tests/fuzz-regressions/` through the fuzz targets' checks.

use std::fs;
use std::panic;
use std::path::Path;

use ferrion_lexer::invariants;

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
    let mut failures = Vec::new();
    for path in &paths {
        let bytes = fs::read(path).unwrap();
        let result = panic::catch_unwind(|| invariants::check_bytes(&bytes));
        let name = path.file_name().unwrap().to_string_lossy();
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
