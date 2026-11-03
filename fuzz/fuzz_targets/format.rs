//! Reads arbitrary bytes as a source file, parses it at a few releases, and wherever it parses
//! without errors, formats it and checks the four safety properties, and that the edits give the
//! same output.

#![no_main]

use ferrion_formatter::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_bytes(bytes) {
        panic!("{message}");
    }
});
