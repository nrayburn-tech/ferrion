//! Parses arbitrary bytes, read as text, as class, method, and field signatures and as
//! descriptors; whatever parses must print back as the same text.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_signature(&String::from_utf8_lossy(bytes)) {
        panic!("{message}");
    }
});
