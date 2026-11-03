//! Java's syntax tree: what `ferrion_parser` produces, and what the formatter, the linter, and
//! semantic analysis read.
//!
//! The tree ([`SyntaxTree`]) is a flat, lossless concrete syntax tree: the lexer's tokens, trivia
//! included, and a pre-order array of nodes over token ranges (docs/design/parser-syntax-tree.md).
//! Its kinds ([`SyntaxKind`]) and the typed views in [`ast`] are generated from `java.grammar`,
//! which documents how each construct is represented.
//!
//! This crate holds the tree and no parsing logic, so crates that only read trees depend on it
//! alone and don't rebuild when the grammar's implementation changes. [`comments`] places
//! comments relative to nodes and finds declarations' documentation comments
//! (docs/design/parser-comments.md).

pub mod ast;
pub mod comments;
mod error_units;
#[doc(hidden)]
pub mod invariants;
mod syntax_kind;
#[cfg(test)]
mod test_tree;
mod tree;

pub use syntax_kind::{SyntaxKind, token_sets};
pub use tree::{
    Ancestors, Children, ChildrenWithTokens, Descendants, Node, NodeId, NodeMap, NodeOrToken,
    Preorder, SyntaxNode, SyntaxPtr, SyntaxToken, SyntaxTree, TokenId, Tokens, WalkEvent, flags,
};
