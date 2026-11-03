//! The doc comment part of the comments check (docs/design/formatter-completeness.md, "Doc
//! comments", "Checking it"): reflow changes a doc comment's text, so doc comments are compared
//! by what the reflow may not change.

/// A traditional doc comment's text as the comments check compares it. For now, as any block
/// comment: nothing reflows them yet.
pub(super) fn normalize(text: &str) -> String {
    super::normalize_comment(text, false)
}
