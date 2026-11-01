//! Reads arbitrary bytes as a module image (`lib/modules`), indexes it, and reads every resource.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_jimage(bytes) {
        panic!("{message}");
    }
});
