//! Reads arbitrary bytes as a zip, then as `ct.sym`, resolving every release.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_ctsym(bytes) {
        panic!("{message}");
    }
});
