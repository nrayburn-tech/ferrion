//! Lexes arbitrary bytes, read as a source file, and checks the lexer's invariants.

#![no_main]

use ferrion_lexer::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_bytes(bytes) {
        panic!("{message}");
    }
});
