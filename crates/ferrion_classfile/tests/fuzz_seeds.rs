//! The class file fuzz targets' seeds in `fuzz/seeds/`, which test code writes rather than
//! copying any JDK's files: each must pass its target's checks, and the committed files must be
//! what `testing::fuzz_seeds` writes. Run with `FERRION_WRITE_FUZZ_SEEDS=1` to rewrite them.
//!
//! Inputs in `tests/fuzz-regressions/<target>/` (minimized fuzz failures) go through the same
//! checks.

use std::path::{Path, PathBuf};

use ferrion_classfile::{invariants, testing};

fn check(target: &str, bytes: &[u8]) -> Result<(), String> {
    match target {
        "classfile" => invariants::check_class(bytes),
        "signature" => invariants::check_signature(&String::from_utf8_lossy(bytes)),
        "zip" => invariants::check_zip(bytes),
        "jar" => invariants::check_jar(bytes),
        "jimage" => invariants::check_jimage(bytes),
        "jimage_structured" => invariants::check_jimage(&invariants::structured_image(bytes)),
        "ctsym" => invariants::check_ctsym(bytes),
        _ => panic!("unknown target {target}"),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn seeds_pass_and_are_committed() {
    let write = std::env::var_os("FERRION_WRITE_FUZZ_SEEDS").is_some();
    for (target, name, bytes) in testing::fuzz_seeds() {
        check(target, &bytes).unwrap_or_else(|e| panic!("seed {target}/{name}: {e}"));
        let path = root().join("fuzz/seeds").join(target).join(&name);
        if write {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        } else {
            let committed = std::fs::read(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; run with FERRION_WRITE_FUZZ_SEEDS=1",
                    path.display()
                )
            });
            assert!(
                committed == bytes,
                "{} differs from what testing::fuzz_seeds writes; run with FERRION_WRITE_FUZZ_SEEDS=1",
                path.display()
            );
        }
    }
}

#[test]
fn regressions_pass() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz-regressions");
    let Ok(targets) = std::fs::read_dir(&dir) else {
        return;
    };
    for target in targets {
        let target = target.unwrap().path();
        let name = target.file_name().unwrap().to_string_lossy().into_owned();
        for input in std::fs::read_dir(&target).unwrap() {
            let input = input.unwrap().path();
            let bytes = std::fs::read(&input).unwrap();
            check(&name, &bytes).unwrap_or_else(|e| panic!("{}: {e}", input.display()));
        }
    }
}
