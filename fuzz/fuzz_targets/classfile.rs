//! Reads arbitrary bytes as a class file, calls every accessor, and decodes every
//! attribute, annotation, signature, and descriptor, checking the class file reader's invariants.

#![no_main]

use ferrion_classfile::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_class(bytes) {
        panic!("{message}");
    }
});
