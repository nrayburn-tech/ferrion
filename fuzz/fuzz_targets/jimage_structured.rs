//! Builds a valid module image from the fuzz input, in either byte order and with each
//! compression, then changes a few of its bytes, so the fuzzer reaches the location, string, and
//! decompression code that random bytes rarely do.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_jimage(&invariants::structured_image(bytes)) {
        panic!("{message}");
    }
});
