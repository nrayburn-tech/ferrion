//! Parses arbitrary bytes, read as the lines of Markdown documentation comments (each line gets a
//! `///`), and checks the doc comment parser's invariants, then the formatter's reflow of the lines
//! as one comment: reflowing it twice at a column gives the same text and keeps its content.

#![no_main]

use ferrion_doc_comments::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_markdown_bytes(bytes) {
        panic!("{message}");
    }
    if let Err(message) = ferrion_formatter::invariants::check_markdown_reflow(bytes) {
        panic!("{message}");
    }
});
