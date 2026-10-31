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
    /// An expression is missing: the current token, given here, can't start one. Nothing is
    /// consumed. Anchor: the token's start, as for javac's `illegal.start.of.expr`.
    ExpressionExpected(SyntaxKind),
    /// A type is missing: the current token, given here, can't start one. Nothing is consumed.
    /// Anchor: the token's start, as for javac's `illegal.start.of.type`.
    TypeExpected(SyntaxKind),
    /// A primitive or array type in an expression (`int`, `a[]`) isn't followed by `.class` or
    /// `::`. Anchor: the start of the token after the type, as for javac's `dot.class.expected`.
    ClassLiteralExpected,
    /// An array creation with neither a dimension expression nor an initializer: `new int[]`. Its
    /// span runs from `new` to the token after it. Anchor: the start of that token, as for
    /// javac's `array.dimension.missing`.
    ArrayDimensionMissing,
    /// An array creation with both dimension expressions and an initializer: `new int[1] {2}`.
    /// Anchor: the initializer's `{`, as for javac's
    /// `illegal.array.creation.both.dimension.and.initialization`.
    ArrayDimensionsAndInitializer,
    /// A lambda's parameters mix inferred types (`x`) with declared ones (`int y`, `var z`). Its
    /// span covers the parameters. Anchor: the lambda's `(`, as for javac's
    /// `invalid.lambda.parameter.declaration`.
    MixedLambdaParameters,
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
    /// One of several tokens, such as `')'` or `','` after an argument: javac's `expected2` and
    /// `expected3`.
    OneOf(&'static [SyntaxKind]),
}

impl ParseDiagnosticKind {
    pub fn javac_phase(self) -> JavacPhase {
        match self {
            ParseDiagnosticKind::Lexical(_)
            | ParseDiagnosticKind::Expected(_)
            | ParseDiagnosticKind::UnexpectedToken(_)
            | ParseDiagnosticKind::ExpressionExpected(_)
            | ParseDiagnosticKind::TypeExpected(_)
            | ParseDiagnosticKind::ClassLiteralExpected
            | ParseDiagnosticKind::ArrayDimensionMissing
            | ParseDiagnosticKind::ArrayDimensionsAndInitializer
            | ParseDiagnosticKind::MixedLambdaParameters => JavacPhase::Parse,
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
            ParseDiagnosticKind::Expected(Expected::OneOf(kinds)) => {
                format!("{} expected", one_of(kinds))
            }
            ParseDiagnosticKind::UnexpectedToken(kind) => format!("unexpected {}", describe(kind)),
            ParseDiagnosticKind::ExpressionExpected(kind) => {
                format!("expected an expression, found {}", describe(kind))
            }
            ParseDiagnosticKind::TypeExpected(kind) => {
                format!("expected a type, found {}", describe(kind))
            }
            ParseDiagnosticKind::ClassLiteralExpected => {
                "a type in an expression must be followed by '.class' or '::'".to_owned()
            }
            ParseDiagnosticKind::ArrayDimensionMissing => {
                "an array creation needs a dimension or an initializer".to_owned()
            }
            ParseDiagnosticKind::ArrayDimensionsAndInitializer => {
                "an array creation can't have both dimensions and an initializer".to_owned()
            }
            ParseDiagnosticKind::MixedLambdaParameters => {
                "a lambda's parameters must all have types or all be inferred".to_owned()
            }
            ParseDiagnosticKind::NestingTooDeep => format!(
                "too deeply nested: more than {} levels",
                crate::NESTING_LIMIT
            ),
        }
    }
}

/// `')' or ','`, or `',', ')', or '['`.
fn one_of(kinds: &[SyntaxKind]) -> String {
    let described: Vec<_> = kinds.iter().map(|&kind| describe(kind)).collect();
    match described.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
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
