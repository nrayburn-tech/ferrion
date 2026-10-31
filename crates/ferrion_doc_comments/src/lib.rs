//! Documentation comment parsers.
//!
//! Parses traditional documentation comments (`/** ... */`) with [`parse_javadoc`], and Markdown
//! documentation comments (runs of `///` lines, JEP 467) with [`parse_markdown`], into structured
//! data: the main description, block tags, inline tags, HTML, and references to program elements.
//! The syntax is the Documentation Comment Specification for the Standard Doclet's; where it is
//! silent, javadoc 27's observed behavior is followed. `docs/design/doc-comments.md` describes the
//! data model and its differences from javadoc.
//!
//! - **Source positions:** every part of a comment has a [`Span`] in the source text, covering it
//!   as written, Unicode escapes included. Text values have the escapes translated, as javadoc
//!   reads them.
//! - **Comments, not files:** the parsers take a comment's span (or, for Markdown, the spans of
//!   its lines) in the source text, as the lexer produces them, and don't depend on the Java
//!   parser. Whether `///` comments are documentation comments at all depends on the Java version
//!   (23 and later), which is the caller's decision; [`markdown_comment_runs`] finds the runs of
//!   lines that would form them.
//! - **Never fails:** any input produces a comment, with [`Diagnostic`]s for what is malformed.
//!   The parsers never panic, and nesting deeper than [`MAX_NESTING`] is not followed, so deeply
//!   nested input can't overflow the stack.

mod content;
mod escape;
mod inline;
#[doc(hidden)]
pub mod invariants;
mod javadoc;
pub mod markdown;
mod model;
mod reference;

pub use ferrion_diagnostics::Span;
pub use javadoc::{Javadoc, parse_javadoc};
pub use markdown::{MarkdownComment, markdown_comment_runs, parse_markdown};
pub use model::{
    Attribute, AttributeValue, BlockTag, BlockTagKind, Diagnostic, DiagnosticKind, EndElement,
    Entity, Escape, HtmlComment, Inline, InlineTag, InlineTagContent, Member, ParamName, Parameter,
    Parameters, QualifiedName, RefType, Reference, Snippet, SnippetAttribute, StartElement,
    TagArgument, Text, TypeArgument, WildcardBound,
};

/// How deeply inline tags, HTML attribute values, type arguments in references, and Markdown
/// blocks and inlines may nest. Deeper nesting is read flat, with a
/// [`NestingTooDeep`](DiagnosticKind::NestingTooDeep) diagnostic: an inline tag as literal text,
/// a type argument as a malformed reference, and Markdown content as part of the deepest
/// container. Real comments come nowhere near it.
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
