//! Java parser, error recovery, and language version checks.
//!
//! [`parse`] lexes and parses a file into a [`SyntaxTree`] (from `ferrion_syntax`, re-exported as
//! [`syntax`]) and diagnostics. The design is in docs/design/parser.md and the documents it links.
//! Crates that only read trees depend on `ferrion_syntax` instead.

mod builder;
mod diagnostic;
mod grammar;
#[doc(hidden)]
pub mod invariants;
mod parser;
pub mod version;

use std::fmt::Write as _;
use std::sync::Arc;

use ferrion_java_version::{LATEST_RELEASE, OLDEST_RELEASE};
pub use ferrion_syntax::{self as syntax, SyntaxKind, SyntaxTree};

pub use diagnostic::{Expected, JavacPhase, ParseDiagnostic, ParseDiagnosticKind, Severity};

/// How many constructs can nest in each other before the parser stops with
/// [`ParseDiagnosticKind::NestingTooDeep`] (docs/design/parser-deep-nesting.md). A resource
/// limit, not a stack limit: chains at one level, such as `+` or `else if`, don't count.
pub const NESTING_LIMIT: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseOptions {
    /// The Java release to parse for, in `ferrion_java_version::SUPPORTED_RELEASES`. It decides
    /// the release-dependent readings (`yield`), and the tree records it. A release outside the
    /// range is treated as the nearest supported one.
    pub release: u32,
}

/// A parsed file.
#[derive(Clone, Debug)]
pub struct Parse {
    /// The tree, holding every token of the text. Immutable, `Send`, and `Sync`.
    pub tree: SyntaxTree,
    /// Lexical and syntax errors, and the version check's errors and warnings, in source order.
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// Lexes and parses `text`, a [`SourceText`](ferrion_diagnostics::SourceText)'s text. Never fails
/// and never panics: any text gives a tree that reproduces it, and diagnostics for what's wrong.
pub fn parse(text: Arc<str>, options: ParseOptions) -> Parse {
    let release = options.release.clamp(OLDEST_RELEASE, LATEST_RELEASE);
    let lexed = ferrion_lexer::lex(&text);
    let mut p = parser::Parser::new(&text, &lexed.tokens, release);
    grammar::compilation_unit(&mut p);
    let (events, mut diagnostics) = p.finish();
    diagnostics.extend(lexed.errors.iter().map(ParseDiagnostic::lexical));
    let lex_errors: Vec<_> = lexed.errors.iter().map(|e| e.span).collect();
    let tree =
        builder::Builder::new(text.clone(), lexed.tokens, &lex_errors, release).build(events);
    diagnostics.extend(version::check(&tree));
    diagnostics.sort_by_key(|d| d.span.start);
    Parse { tree, diagnostics }
}

impl Parse {
    /// The tree's dump ([`SyntaxTree::debug_dump`]), then each diagnostic on a line: severity,
    /// span, anchor, kind, and message. For snapshot tests.
    pub fn debug_dump(&self) -> String {
        let mut out = self.tree.debug_dump();
        for d in &self.diagnostics {
            let severity = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            writeln!(
                out,
                "{severity} {:?} @{} {:?}: {}",
                d.span,
                d.anchor,
                d.kind,
                d.message()
            )
            .unwrap();
        }
        out
    }
}
