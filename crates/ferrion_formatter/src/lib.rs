//! Java formatter (docs/design/formatter.md).
//!
//! [`format_tree`] formats a tree that's already parsed: it builds a flat Wadler–Leijen document
//! from the tree (`rules`, through the token and comment helpers in `format`), then prints it
//! (`printer`). [`format`] parses first, and [`verify`] checks the four safety properties for an
//! input and its output. Every piece of text in the output is a token of the input, copied from
//! the source; the formatter writes only whitespace of its own, and reorders tokens only to put
//! modifiers in order.

mod document;
mod format;
#[doc(hidden)]
pub mod invariants;
mod modifiers;
mod printer;
mod regions;
mod rules;
mod verify;

use std::sync::Arc;

use ferrion_diagnostics::{LineEnding, SourceText};
use ferrion_parser::{ParseDiagnostic, ParseOptions, Severity, parse};
use ferrion_syntax::SyntaxTree;

pub use verify::{Property, Violation, verify};

/// How to format a file. There are no style options.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FormatOptions {
    /// The Java release to parse for, in `ferrion_java_version::SUPPORTED_RELEASES`.
    pub release: u32,
}

/// What formatting a file gave.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Formatted {
    /// The file is already formatted.
    Unchanged,
    /// The new file, byte-order mark and line endings included.
    Changed(Vec<u8>),
    /// The file has syntax or version errors, so nothing was formatted. These are the errors,
    /// for the caller to report; empty when [`format_tree`] was given a tree with errors in it.
    Skipped(Vec<ParseDiagnostic>),
}

/// Formats a tree without errors, parsed from `source` at the tree's release. `source` gives the
/// file's line ending and byte-order mark, which the tree doesn't hold.
///
/// The caller checks the parse for errors: a tree can't tell that a version error was reported.
/// A tree that holds a syntax error is skipped.
pub fn format_tree(tree: &SyntaxTree, source: &SourceText) -> Formatted {
    debug_assert_eq!(tree.text(), source.text(), "the tree isn't the source's");
    if tree.root().contains_error() {
        return Formatted::Skipped(Vec::new());
    }
    let document = format::Formatter::new(tree).format_root();
    let output = printer::print(
        &document,
        &printer::Source {
            tree,
            line_ending: source.line_ending().unwrap_or(LineEnding::Lf),
            bom: source.has_bom(),
        },
    );
    let bom = if source.has_bom() { 3 } else { 0 };
    if output.len() == bom + source.text().len() && &output[bom..] == source.text().as_bytes() {
        Formatted::Unchanged
    } else {
        Formatted::Changed(output)
    }
}

/// Parses `source` at `options.release`, then formats it with [`format_tree`] unless the parse
/// has errors. Warnings don't stop it.
pub fn format(source: &SourceText, options: &FormatOptions) -> Formatted {
    let parsed = parse(
        Arc::from(source.text()),
        ParseOptions {
            release: options.release,
        },
    );
    let errors: Vec<_> = parsed
        .diagnostics
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        return Formatted::Skipped(errors);
    }
    format_tree(&parsed.tree, source)
}

#[cfg(test)]
mod tests;
