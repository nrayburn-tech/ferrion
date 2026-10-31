//! Parses arbitrary bytes, read as the content of a traditional documentation comment (between
//! `/**` and `*/`), and checks the doc comment parser's invariants.

#![no_main]

use ferrion_doc_comments::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_javadoc_bytes(bytes) {
        panic!("{message}");
    }
});
