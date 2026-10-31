//! Lexes text built from fragments of Java, to reach lexer states that random bytes rarely do, and
//! checks the lexer's invariants.
//!
//! Each input byte picks a fragment: the low 7 bits index `FRAGMENTS`, and the high bit writes the
//! fragment as Unicode escapes. When `FERRION_FUZZ_SAVE_TEXT` is set to a path, the text is written
//! there, which turns an input into a source file for the regression tests.

#![no_main]

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::Write;
use std::fs;
use std::sync::OnceLock;

use ferrion_lexer::invariants;
use libfuzzer_sys::fuzz_target;

const FRAGMENTS: [&str; 128] = [
    // Keywords, contextual keywords, and identifiers.
    "class",
    "interface",
    "enum",
    "record",
    "var",
    "yield",
    "sealed",
    "non",
    "permits",
    "_",
    "int",
    "long",
    "char",
    "true",
    "null",
    "new",
    "return",
    "switch",
    "case",
    "x",
    "Ab9",
    "$",
    "é",
    "Ω",
    "𝔸",
    "\u{660}",
    // Characters that are easy to mishandle: format characters, a no-break space, a line
    // separator that isn't a Java line terminator, a byte-order mark, a supplementary character,
    // and controls (Ctrl-Z may end a file).
    "\u{200B}",
    "\u{A0}",
    "\u{2028}",
    "\u{FEFF}",
    "😀",
    "\0",
    "\x1A",
    "\x7F",
    // Whitespace and line terminators.
    " ",
    "\t",
    "\x0C",
    "\n",
    "\r",
    "\r\n",
    "\x0B",
    // Comments.
    "//",
    "///",
    "/*",
    "/**",
    "*/",
    "/**/",
    "*",
    "/",
    // Quotes and escapes.
    "\"",
    "\"\"\"",
    "'",
    "\\",
    "\\n",
    "\\s",
    "\\b",
    "\\'",
    "\\\"",
    "\\\\",
    "\\0",
    "\\377",
    "\\400",
    "\\\n",
    "\\x",
    // Unicode escapes, well-formed and not.
    "\\u",
    "\\uu",
    "\\uuuu005c",
    "\\u005c",
    "\\u000a",
    "\\u000d",
    "\\u0022",
    "\\u0027",
    "\\u002f",
    "\\u002a",
    "\\uD83D",
    "\\uDE00",
    "\\uDBFF\\uDFFF",
    "\\u00",
    "\\u001a",
    "\\u0063lass",
    // Numbers and their parts.
    "0",
    "7",
    "9",
    "00",
    "0x",
    "0B",
    "1_000",
    "0xCAFE",
    "0x1p-3",
    "0x1.8P1",
    "1e",
    "p",
    "L",
    "l",
    "f",
    "2147483648",
    "9223372036854775808L",
    ".5",
    // Separators and operators.
    ".",
    "...",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    ",",
    "@",
    "::",
    ">",
    ">>>=",
    "<",
    "<<=",
    "=",
    "==",
    "!=",
    "&&",
    "|",
    "^",
    "%",
    "~",
    "?",
    ":",
    "->",
    "+",
    "-",
    "#",
];

fuzz_target!(|bytes: &[u8]| {
    let mut text = String::new();
    for &byte in bytes {
        let fragment = FRAGMENTS[usize::from(byte & 0x7F)];
        if byte & 0x80 == 0 {
            text.push_str(fragment);
        } else {
            for unit in fragment.encode_utf16() {
                write!(text, "\\u{unit:04x}").unwrap();
            }
        }
    }
    if let Some(path) = save_path() {
        fs::write(path, &text).unwrap();
    }
    if let Err(message) = invariants::check_bytes(text.as_bytes()) {
        panic!("{message}");
    }
});

fn save_path() -> Option<&'static OsStr> {
    static PATH: OnceLock<Option<OsString>> = OnceLock::new();
    PATH.get_or_init(|| env::var_os("FERRION_FUZZ_SAVE_TEXT"))
        .as_deref()
}
