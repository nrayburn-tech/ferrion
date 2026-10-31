//! Checks that [`parse`] keeps the promises its API documents, for the fuzz target in `fuzz/` and
//! the tests. Not part of the public API.
//!
//! Beyond the tree's own invariants (`ferrion_syntax::invariants`): the tree holds the text and
//! the lexer's tokens for it, and its release; diagnostics are in source order, inside the text,
//! and carry their kind's javac phase; the version diagnostics, if any, are the version check's
//! for the tree; and the root is marked as containing an error exactly when a lexical or syntax
//! error is reported.

use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_java_version::PRIMARY_RELEASES;

use crate::{Parse, ParseOptions, Severity, parse};

/// The releases [`check_bytes`] parses at: the primary releases, the LTS releases and the latest.
/// They include both sides of the `yield` reading (a name at 11, a statement from 17).
pub const RELEASES: &[u32] = PRIMARY_RELEASES;

/// Reads `bytes` as a source file and checks [`check_text`] at each of [`RELEASES`]. Bytes that
/// aren't a supported source file are checked as their lossy UTF-8 text, so that every input
/// reaches the parser.
pub fn check_bytes(bytes: &[u8]) -> Result<(), String> {
    let text: Arc<str> = match SourceText::decode(bytes) {
        Ok(source) => Arc::from(source.text()),
        Err(_) => Arc::from(String::from_utf8_lossy(bytes).as_ref()),
    };
    for &release in RELEASES {
        check_text(&text, release).map_err(|e| format!("at release {release}: {e}"))?;
    }
    Ok(())
}

/// Parses `text` at `release` and checks the result with [`check`].
pub fn check_text(text: &Arc<str>, release: u32) -> Result<(), String> {
    let parsed = parse(text.clone(), ParseOptions { release });
    check(text, release, &parsed)
}

/// Checks that `parsed` is a valid result of parsing `text` at `release`.
pub fn check(text: &str, release: u32, parsed: &Parse) -> Result<(), String> {
    let tree = &parsed.tree;
    if tree.text() != text {
        return Err("the tree's text isn't the text parsed".to_owned());
    }
    if tree.release() != release {
        return Err(format!("the tree's release is {}", tree.release()));
    }
    if tree.lexer_tokens() != ferrion_lexer::lex(text).tokens {
        return Err("the tree's tokens aren't the lexer's".to_owned());
    }
    ferrion_syntax::invariants::check_tree(tree)?;

    let len = text.len() as u32;
    let mut previous = 0;
    for d in &parsed.diagnostics {
        if d.span.start < previous {
            return Err(format!("{d:?}: diagnostics aren't in source order"));
        }
        previous = d.span.start;
        if d.span.end < d.span.start || d.span.end > len || d.anchor > len {
            return Err(format!("{d:?}: outside the text"));
        }
        if d.javac_phase != d.kind.javac_phase() {
            return Err(format!("{d:?}: the wrong javac phase"));
        }
    }
    let version: Vec<_> = parsed
        .diagnostics
        .iter()
        .filter(|d| d.kind.is_version())
        .cloned()
        .collect();
    // Tests that drive the grammar alone make a `Parse` without them.
    if !version.is_empty() && version != crate::version::check(tree) {
        return Err("the version diagnostics aren't the version check's".to_owned());
    }
    let reported = parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && !d.kind.is_version());
    if reported != tree.root().contains_error() {
        return Err(format!(
            "an error is {}reported, but the root is {}marked as containing one",
            if reported { "" } else { "not " },
            if tree.root().contains_error() {
                ""
            } else {
                "not "
            },
        ));
    }
    Ok(())
}
