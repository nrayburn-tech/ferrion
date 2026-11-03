//! Range formatting tests (docs/design/formatter-completeness.md, "Testing"): each `.java` file
//! under `tests/ranges/` holds a range between `«` and `»`, which are removed to give the input.
//! The input is parsed, syntax errors and all, and formatted in the range with `format_range`.
//! The result is compared with the `.formatted.java` file next to it, or, when a syntax error
//! affects the range, the error unit's text with the `.affected` file. Every result is also
//! checked with `verify_range`. A first line `// release: N` parses at release N; otherwise at
//! the latest release.
//!
//! To write the expected files after an intended change, run with `FERRION_BLESS=1`, then review
//! the diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_formatter --test ranges
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_formatter::{FormatOptions, RangeError, apply_edits, format_range, verify_range};
use ferrion_java_version::LATEST_RELEASE;
use ferrion_parser::{ParseOptions, parse};

const START: char = '«';
const END: char = '»';

#[test]
fn ranges() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ranges");
    let bless = std::env::var_os("FERRION_BLESS").is_some();
    let mut inputs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.ends_with(".java") && !name.ends_with(".formatted.java")
        })
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no inputs in {}", dir.display());
    let mut failures = Vec::new();
    for input in &inputs {
        let name = input.file_name().unwrap().to_string_lossy().to_string();
        let marked = fs::read_to_string(input).unwrap();
        let (Some(start), Some(end)) = (marked.find(START), marked.find(END)) else {
            failures.push(format!("{name}: no range marked with « and »"));
            continue;
        };
        let text = marked.replacen(START, "", 1).replacen(END, "", 1);
        let range = start as u32..(end - START.len_utf8()) as u32;
        let source = SourceText::decode(text.into_bytes()).unwrap();
        let release = source
            .text()
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// release: "))
            .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
        let parsed = parse(Arc::from(source.text()), ParseOptions { release });
        let stem = name.trim_end_matches(".java");
        let (actual, expected_path) = match format_range(&parsed.tree, &source, range.clone()) {
            Ok(edits) => {
                if let Err(violation) =
                    verify_range(&source, range, &edits, &FormatOptions { release })
                {
                    failures.push(format!("{name}: {violation}"));
                }
                (
                    apply_edits(source.text(), &edits),
                    input.with_file_name(format!("{stem}.formatted.java")),
                )
            }
            Err(RangeError::Affected { error }) => (
                format!(
                    "{}\n",
                    &source.text()[error.start as usize..error.end as usize]
                ),
                input.with_file_name(format!("{stem}.affected")),
            ),
        };
        // Only one of the two expected files may exist.
        let other = ["formatted.java", "affected"]
            .map(|ext| input.with_file_name(format!("{stem}.{ext}")))
            .into_iter()
            .find(|path| *path != expected_path && path.exists());
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected || other.is_some() {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
                if let Some(other) = other {
                    fs::remove_file(other).unwrap();
                }
            } else {
                failures.push(format!(
                    "{name}: the result differs from {}:\n{actual}",
                    expected_path.display()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} range tests failed (run with FERRION_BLESS=1 to update them):\n{}",
        failures.len(),
        inputs.len(),
        failures.join("\n")
    );
}
