//! Typed views over the syntax tree, generated from `java.grammar`.
//!
//! A view is a [`SyntaxNode`] whose kind has been checked, so it is free to create and to convert
//! back. Its accessors find children by role among the node's direct children and tokens, and
//! allocate nothing. Every accessor returns an `Option` or an iterator, since error recovery can
//! leave any child missing.

mod doc_comment;
mod generated;
pub(crate) mod support;

pub use generated::*;
pub use support::{AstChildren, AstTokens};

use crate::{SyntaxKind, SyntaxNode};

/// A typed view of a node of one kind, or (for an enum) of several.
pub trait AstNode<'t>: Copy {
    /// Whether a node of `kind` can be viewed as `Self`.
    fn can_cast(kind: SyntaxKind) -> bool;

    fn cast(node: SyntaxNode<'t>) -> Option<Self>;

    fn syntax(self) -> SyntaxNode<'t>;
}
