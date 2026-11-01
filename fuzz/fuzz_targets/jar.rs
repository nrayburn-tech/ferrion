//! Reads arbitrary bytes as a jar manifest, and as a zip resolved as a multi-release jar at
//! several releases.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_jar(bytes) {
        panic!("{message}");
    }
});
