//! What `parse` reports: lexical errors, syntax errors, and (later) version errors, each with the
//! offset javac reports the same mistake at (docs/design/parser-error-recovery.md).

use ferrion_diagnostics::Span;
use ferrion_lexer::{LexError, LexErrorKind};

use crate::SyntaxKind;

/// A problem `parse` found. The tree still holds every token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub kind: ParseDiagnosticKind,
    pub severity: Severity,
    /// What an editor underlines. It can cover a whole run of unexpected tokens.
    pub span: Span,
    /// The offset javac reports the same mistake at (its diagnostic's `position`), by the rule
    /// documented on each kind.
    pub anchor: u32,
    /// When javac reports the same mistake.
    pub javac_phase: JavacPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    /// javac's parsing-phase warnings, which Ferrion produces only to compare them with javac's
    /// and never shows to users.
    Warning,
}

/// When javac reports a mistake: while parsing, in a later phase, or never. The comparison with
/// javac's parsing phase leaves out the last two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JavacPhase {
    Parse,
    /// While entering or attributing the file: syntax-shaped errors Ferrion reports earlier.
    Later,
    /// A deliberate departure from javac, such as the nesting limit.
    Never,
}

/// What went wrong. Kinds are Ferrion's own, named for its messages; the oracle comparison maps
/// each to javac's codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseDiagnosticKind {
    /// An error the lexer reported. Anchor: the start of its span.
    Lexical(LexErrorKind),
    /// Something the grammar needs here is missing. Anchor: the end of the previous non-trivia
    /// token, as for javac's `expected` errors.
    Expected(Expected),
    /// A token that nothing here can start or continue. Its span covers the run of tokens put in
    /// an `Error` node. Anchor: the start of the first, as for javac's `illegal.start.of.*`.
    UnexpectedToken(SyntaxKind),
    /// More than [`NESTING_LIMIT`](crate::NESTING_LIMIT) constructs nested in each other. javac
    /// has no such limit (docs/design/parser-deep-nesting.md). Anchor: the token that would have
    /// gone deeper.
    NestingTooDeep,
}

/// What an [`Expected`](ParseDiagnosticKind::Expected) error expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expected {
    /// A specific token, such as `;`.
    Token(SyntaxKind),
    /// A name.
    Identifier,
}

impl ParseDiagnosticKind {
    pub fn javac_phase(self) -> JavacPhase {
        match self {
            ParseDiagnosticKind::Lexical(_)
            | ParseDiagnosticKind::Expected(_)
            | ParseDiagnosticKind::UnexpectedToken(_) => JavacPhase::Parse,
            ParseDiagnosticKind::NestingTooDeep => JavacPhase::Never,
        }
    }
}

impl ParseDiagnostic {
    /// An error of `kind`, with javac's phase for it.
    pub fn error(kind: ParseDiagnosticKind, span: Span, anchor: u32) -> ParseDiagnostic {
        ParseDiagnostic {
            kind,
            severity: Severity::Error,
            span,
            anchor,
            javac_phase: kind.javac_phase(),
        }
    }

    pub(crate) fn lexical(error: &LexError) -> ParseDiagnostic {
        ParseDiagnostic::error(
            ParseDiagnosticKind::Lexical(error.kind),
            error.span,
            error.span.start,
        )
    }

    /// The message shown to users.
    pub fn message(&self) -> String {
        match self.kind {
            ParseDiagnosticKind::Lexical(kind) => lexical_message(kind).to_owned(),
            ParseDiagnosticKind::Expected(Expected::Token(kind)) => {
                format!("{} expected", describe(kind))
            }
            ParseDiagnosticKind::Expected(Expected::Identifier) => "name expected".to_owned(),
            ParseDiagnosticKind::UnexpectedToken(kind) => format!("unexpected {}", describe(kind)),
            ParseDiagnosticKind::NestingTooDeep => format!(
                "too deeply nested: more than {} levels",
                crate::NESTING_LIMIT
            ),
        }
    }
}

/// `';'` for tokens with a fixed spelling, and a description for the others.
fn describe(kind: SyntaxKind) -> String {
    if let Some(text) = kind.fixed_text() {
        return format!("'{text}'");
    }
    match kind {
        SyntaxKind::Identifier => "name",
        SyntaxKind::Eof => "end of file",
        SyntaxKind::ErrorToken => "character",
        SyntaxKind::TextBlock => "text block",
        SyntaxKind::StringLiteral => "string literal",
        SyntaxKind::CharLiteral => "character literal",
        _ => "literal",
    }
    .to_owned()
}

fn lexical_message(kind: LexErrorKind) -> &'static str {
    match kind {
        LexErrorKind::IllegalCharacter => "illegal character",
        LexErrorKind::InvalidUnicodeEscape => "invalid Unicode escape",
        LexErrorKind::InvalidEscape => "invalid escape sequence",
        LexErrorKind::UnterminatedComment => "unterminated comment",
        LexErrorKind::UnterminatedCharLiteral => "unterminated character literal",
        LexErrorKind::UnterminatedStringLiteral => "unterminated string literal",
        LexErrorKind::UnterminatedTextBlock => "unterminated text block",
        LexErrorKind::EmptyCharLiteral => "empty character literal",
        LexErrorKind::MultipleCharsInCharLiteral => {
            "more than one character in a character literal"
        }
        LexErrorKind::LineTerminatorInCharLiteral => "line break in a character literal",
        LexErrorKind::TextBlockOpeningNotOnOwnLine => {
            "a text block's opening delimiter must end its line"
        }
        LexErrorKind::MalformedNumber => "malformed number",
    }
}
