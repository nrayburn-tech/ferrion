//! Documentation comment parsers.
//!
//! Parses traditional documentation comments (`/** ... */`) with [`parse_javadoc`] into
//! structured data: the main description, block tags, inline tags, HTML, and references to
//! program elements. Markdown documentation comments (runs of `///` lines, JEP 467) share the
//! data model.
//! The syntax is the Documentation Comment Specification for the Standard Doclet's; where it is
//! silent, javadoc 27's observed behavior is followed. `docs/design/doc-comments.md` describes the
//! data model and its differences from javadoc.
//!
//! - **Source positions:** every part of a comment has a [`Span`] in the source text, covering it
//!   as written, Unicode escapes included. Text values have the escapes translated, as javadoc
//!   reads them.
//! - **Comments, not files:** the parser takes a comment's span in the source text, as the
//!   lexer produces it, and doesn't depend on the Java parser.
//! - **Never fails:** any input produces a comment, with [`Diagnostic`]s for what is malformed.
//!   The parsers never panic, and nesting deeper than [`MAX_NESTING`] is not followed, so deeply
//!   nested input can't overflow the stack.

mod content;
mod escape;
mod inline;
#[doc(hidden)]
pub mod invariants;
mod javadoc;
mod model;
mod reference;

pub use ferrion_diagnostics::Span;
pub use javadoc::{Javadoc, parse_javadoc};
pub use model::{
    Attribute, AttributeValue, BlockTag, BlockTagKind, Diagnostic, DiagnosticKind, EndElement,
    Entity, Escape, HtmlComment, Inline, InlineTag, InlineTagContent, Member, ParamName, Parameter,
    Parameters, QualifiedName, RefType, Reference, Snippet, SnippetAttribute, StartElement,
    TagArgument, Text, TypeArgument, WildcardBound,
};

/// How deeply inline tags, HTML attribute values, and type arguments in references may nest.
/// Deeper nesting is read flat, with a [`NestingTooDeep`](DiagnosticKind::NestingTooDeep)
/// diagnostic: an inline tag as literal text, and a type argument as a malformed reference. Real
/// comments come nowhere near it.
pub const MAX_NESTING: usize = 32;

/// `span` limited to `text` and to character boundaries, as byte offsets. Source texts have at
/// most `u32::MAX` bytes; anything after that is out of bounds.
fn clamp_span(text: &str, span: Span) -> (usize, usize) {
    let len = text.len().min(u32::MAX as usize);
    let mut start = (span.start as usize).min(len);
    let mut end = (span.end as usize).clamp(start, len);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (start, end.max(start))
}
