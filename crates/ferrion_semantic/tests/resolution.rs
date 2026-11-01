//! Resolution snapshot tests: each `.case` file under `tests/resolution/` is a small project
//! (source sets, jars, and files, as `ferrion_semantic::testing::case_input` reads them),
//! resolved against the stand-in platform, and what every declaration name resolves to and each
//! type's entry ([`testing::dump`]) is compared with the `.names` file next to it. Each case was
//! checked against javac when it was written: its files compile, or javac reports the errors the
//! snapshot's error symbols stand for (the case's first lines say which).
//!
//! To write the snapshots after an intended change, run with `FERRION_BLESS=1`, then review the
//! diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_semantic --test resolution
//! ```

use std::fs;
use std::path::Path;

use ferrion_semantic::Engine;
use ferrion_semantic::testing::{case_input, dump};

#[test]
fn snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/resolution");
    let bless = std::env::var_os("FERRION_BLESS").is_some();
    let mut inputs: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "case"))
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no cases in {}", dir.display());
    let mut failures = Vec::new();
    for input in &inputs {
        let text = fs::read_to_string(input).unwrap();
        let engine = Engine::new(case_input(&text));
        let actual = dump(&engine);
        // The same answers from a batch on several threads.
        engine.snapshot().run_batch(3, |_| Ok(())).unwrap();
        assert_eq!(dump(&engine), actual, "{}", input.display());
        let expected_path = input.with_extension("names");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{}: the names differ from {}:\n{actual}",
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
