//! Applies a sequence of edits, chosen by the fuzz input, to a small project, and after each one
//! compares the engine's summaries, package tables, type entries, names in declarations and
//! bodies, and each file's attribution with a fresh engine's. The input's bytes up to a first
//! zero byte, if any, are one more file of the project.

#![no_main]

use ferrion_semantic::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_bytes(bytes) {
        panic!("{message}");
    }
});
