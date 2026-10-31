//! Documentation comment parsers.
//!
//! The data model of traditional documentation comments (`/** ... */`) and Markdown documentation
//! comments (runs of `///` lines, JEP 467): block tags, inline tags, HTML, and references to
//! program elements. Every part of a comment has a [`Span`] in the source text, covering it as
//! written, Unicode escapes included.

mod model;

pub use ferrion_diagnostics::Span;
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
