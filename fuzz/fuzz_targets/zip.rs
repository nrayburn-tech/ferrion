//! Reads arbitrary bytes as a zip file, and lists and reads every entry.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_zip(bytes) {
        panic!("{message}");
    }
});
