//! Snapshot tests: each `.java` file under `tests/snapshots/<area>/` is formatted, and the output
//! compared with the `.formatted.java` file next to it. Every input is also checked with
//! `verify`. A first line `// release: N` formats at release N; otherwise at the latest release.
//!
//! To write the snapshots after an intended change, run with `FERRION_BLESS=1`, then review the
//! diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_formatter --test snapshots
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use ferrion_diagnostics::SourceText;
use ferrion_formatter::{FormatOptions, Formatted, format, verify};
use ferrion_java_version::LATEST_RELEASE;

const EXPECTED: &str = ".formatted.java";

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
        let name = input.strip_prefix(&dir).unwrap().display().to_string();
        let source = SourceText::decode(fs::read(input).unwrap()).unwrap();
        let release = source
            .text()
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// release: "))
            .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
        let options = FormatOptions { release };
        let actual = match format(&source, &options) {
            Formatted::Unchanged => source.to_bytes(),
            Formatted::Changed(output) => output,
            Formatted::Skipped(errors) => {
                failures.push(format!("{name}: skipped: {errors:?}"));
                continue;
            }
        };
        if let Err(violation) = verify(&source, &actual, &options) {
            failures.push(format!("{name}: {violation}"));
        }
        let file_name = input.file_name().unwrap().to_string_lossy();
        let expected_path =
            input.with_file_name(format!("{}{EXPECTED}", file_name.trim_end_matches(".java")));
        let expected = fs::read(&expected_path).unwrap_or_default();
        if actual != expected {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{name}: the output differs from {}:\n{}",
                    expected_path.display(),
                    String::from_utf8_lossy(&actual)
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
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            collect(&path, out);
        } else if name.ends_with(".java") && !name.ends_with(EXPECTED) {
            out.push(path);
        }
    }
}
