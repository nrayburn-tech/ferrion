//! Parses arbitrary bytes, read as the lines of Markdown documentation comments (each line gets a
//! `///`), and checks the doc comment parser's invariants.

#![no_main]

use ferrion_doc_comments::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_markdown_bytes(bytes) {
        panic!("{message}");
    }
});
