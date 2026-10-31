//! Java lexer.
//!
//! Implements the lexical grammar of the Java Language Specification (chapter 3) for Java 8 through
//! the latest GA release.
//!
//! - **Lossless:** every byte of the text belongs to exactly one token, so concatenating the tokens'
//!   text reproduces the source. Whitespace, line terminators, and comments are tokens too.
//! - **Unicode escapes** (JLS §3.3) are translated before tokens are formed, so `\u0063lass` is the
//!   keyword `class`, and `\u000a` ends a line comment. Spans always cover the escapes as written.
//! - **Version independent:** the result doesn't depend on the Java version. Constructs that need a
//!   newer version (text blocks, `_` as a keyword, `\s` escapes, `///` comments) are lexed in every
//!   version, and the parser's version check reports them.
//! - **Never fails:** any text produces tokens. Malformed input produces [`LexError`]s alongside
//!   them, and the lexer never panics.

mod cursor;
mod escape;
mod ident;
mod kind;
mod lexer;
#[cfg(test)]
mod tests;
mod unicode_tables;

use std::borrow::Cow;

pub use ferrion_diagnostics::Span;
pub use kind::{ContextualKeyword, TokenKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// The token's text as written, including any Unicode escapes.
    pub span: Span,
}

/// The result of lexing a text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lexed {
    /// Tokens in source order, covering the text without gaps or overlaps, and ending with
    /// [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Errors in source order.
    pub errors: Vec<LexError>,
}

/// A problem found while lexing. The tokens around it are still produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub kind: LexErrorKind,
    /// The text at fault: the character, escape, or part of a number that's wrong (or the whole
    /// number, when something is missing), the literal for a malformed character or string
    /// literal, and the opening delimiter of an unterminated comment or text block. See
    /// [`LexErrorKind`] for the rest.
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexErrorKind {
    /// A character that can't start a token.
    IllegalCharacter,
    /// `\u` not followed by four hex digits.
    InvalidUnicodeEscape,
    /// An escape sequence in a character literal, string literal, or text block that isn't one of
    /// JLS §3.10.7's.
    InvalidEscape,
    UnterminatedComment,
    UnterminatedCharLiteral,
    UnterminatedStringLiteral,
    UnterminatedTextBlock,
    /// `''`
    EmptyCharLiteral,
    /// A character literal with more than one character, counting UTF-16 code units as javac does,
    /// so `'😀'` is one too.
    MultipleCharsInCharLiteral,
    /// A line terminator right after the opening `'` of a character literal, as in `'\u000a'`.
    LineTerminatorInCharLiteral,
    /// Text after `"""` on the line that opens a text block.
    TextBlockOpeningNotOnOwnLine,
    /// A numeric literal that doesn't match JLS §3.10.1 or §3.10.2, such as `0x`, `1e`, or `1_`.
    MalformedNumber,
}

/// Lexes `text`, a [`SourceText`](ferrion_diagnostics::SourceText)'s text.
pub fn lex(text: &str) -> Lexed {
    lexer::lex(text)
}

/// A token's text with Unicode escapes translated, which is what identifiers and keywords compare
/// by. Borrows when the token has no escapes.
///
/// For identifiers and keywords, identifier-ignorable characters (such as a soft hyphen, or most
/// control characters) are removed as well, since javac ignores them when comparing names:
/// `a\u00ADb` is the identifier `ab`. Invalid escapes are dropped, and an escape for a UTF-16
/// surrogate that isn't part of a pair becomes U+FFFD, so a literal's value needs its own decoding.
pub fn cooked_text(text: &str, token: Token) -> Cow<'_, str> {
    let is_name = token.kind == TokenKind::Identifier
        || token
            .kind
            .fixed_text()
            .is_some_and(|text| text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'));
    escape::cook(text, token.span, is_name)
}
