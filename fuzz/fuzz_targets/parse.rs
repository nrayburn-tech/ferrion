//! Parses arbitrary bytes, read as a source file, at a few releases, and checks the parser's and
//! the syntax tree's invariants.

#![no_main]

use ferrion_parser::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_bytes(bytes) {
        panic!("{message}");
    }
});
