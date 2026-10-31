//! The flat, lossless syntax tree (docs/design/parser-syntax-tree.md): the lexer's tokens, the
//! kind each token was parsed as, and a pre-order array of nodes over token ranges.

mod cursor;
mod dump;
mod node_map;

use std::fmt;
use std::sync::Arc;

use ferrion_diagnostics::Span;
use ferrion_lexer::Token;

use crate::SyntaxKind;

pub use cursor::{
    Ancestors, Children, ChildrenWithTokens, Descendants, NodeOrToken, Preorder, SyntaxNode,
    SyntaxToken, Tokens, WalkEvent,
};
pub use node_map::{NodeMap, SyntaxPtr};

/// A node's index in its tree's node array. Ids are dense, in pre-order, and valid only for the
/// tree they come from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

/// A token's index in its tree's token array, which is also its index in the lexer's output.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenId(u32);

impl NodeId {
    /// The root's id.
    pub const ROOT: NodeId = NodeId(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The id of the node at `index` in the node array, for building trees.
    pub fn from_index(index: usize) -> NodeId {
        NodeId(index as u32)
    }
}

impl TokenId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The id of the token at `index` in the token array.
    pub fn from_index(index: usize) -> TokenId {
        TokenId(index as u32)
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({})", self.0)
    }
}

impl fmt::Debug for TokenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TokenId({})", self.0)
    }
}

/// Bits of [`Node::flags`].
pub mod flags {
    /// The node's subtree holds an `Error` node, a missing child, or a token with a lexical
    /// error.
    pub const CONTAINS_ERROR: u16 = 1;
    /// Every flag bit in use.
    pub const ALL: u16 = CONTAINS_ERROR;
}

/// One entry of a tree's node array: 20 bytes. The parser's builder makes these; everything else
/// reads trees through [`SyntaxNode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: SyntaxKind,
    pub flags: u16,
    /// The root's parent is itself.
    pub parent: NodeId,
    pub first_token: TokenId,
    /// Exclusive.
    pub token_end: TokenId,
    /// One past the last node in this node's subtree.
    pub subtree_end: NodeId,
}

const _: () = assert!(size_of::<Node>() == 20);

/// A parsed file: its text, its tokens, and its nodes. Immutable, and shared across threads
/// behind an `Arc`.
///
/// Every byte of the text is in exactly one token, and every token is in the root, so the tree
/// reproduces the text. Nodes are in pre-order: node `n`'s descendants are `n + 1..subtree_end`.
/// Each covers a contiguous, non-empty range of tokens that nests in its parent's, and nodes
/// other than the root start and end on non-trivia tokens. Use the cursors, [`SyntaxNode`] and
/// [`SyntaxToken`], to walk it.
#[derive(Clone)]
pub struct SyntaxTree {
    text: Arc<str>,
    tokens: Vec<Token>,
    token_kinds: Vec<SyntaxKind>,
    nodes: Vec<Node>,
    release: u32,
}

const _: () = {
    const fn send_and_sync<T: Send + Sync>() {}
    send_and_sync::<SyntaxTree>();
};

impl SyntaxTree {
    /// A tree from its finished arrays: `tokens` are the lexer's tokens for `text`, ending with
    /// `Eof`; `token_kinds` has the kind each was parsed as; `nodes` are in pre-order, the root
    /// first. Debug builds check every promise the tree makes ([`crate::invariants::check_tree`])
    /// and panic if one is broken; release builds check nothing.
    pub fn new(
        text: Arc<str>,
        tokens: Vec<Token>,
        token_kinds: Vec<SyntaxKind>,
        nodes: Vec<Node>,
        release: u32,
    ) -> SyntaxTree {
        let tree = SyntaxTree {
            text,
            tokens,
            token_kinds,
            nodes,
            release,
        };
        if cfg!(debug_assertions)
            && let Err(problem) = crate::invariants::check_structure(&tree)
        {
            panic!("an invalid syntax tree: {problem}");
        }
        tree
    }

    /// The text the tree was parsed from, after any byte-order mark.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text, shared with the caller of `parse`.
    pub fn shared_text(&self) -> &Arc<str> {
        &self.text
    }

    /// The release the text was parsed for.
    pub fn release(&self) -> u32 {
        self.release
    }

    pub fn root(&self) -> SyntaxNode<'_> {
        SyntaxNode::new(self, NodeId::ROOT)
    }

    /// The node with id `id`. Panics if `id` isn't from this tree.
    pub fn node(&self, id: NodeId) -> SyntaxNode<'_> {
        assert!(id.index() < self.nodes.len(), "{id:?} isn't in this tree");
        SyntaxNode::new(self, id)
    }

    /// The token with id `id`. Panics if `id` isn't from this tree.
    pub fn token(&self, id: TokenId) -> SyntaxToken<'_> {
        assert!(id.index() < self.tokens.len(), "{id:?} isn't in this tree");
        SyntaxToken::new(self, id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

    /// The lexer's tokens, which [`TokenId`]s index.
    pub fn lexer_tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Every node, in pre-order.
    pub fn nodes(&self) -> Descendants<'_> {
        self.root().descendants()
    }

    /// Every token, in source order.
    pub fn tokens(&self) -> Tokens<'_> {
        self.root().tokens()
    }

    /// The token containing `offset`, or the last token (`Eof`) for the text's length. `None`
    /// past the end of the text.
    pub fn token_at_offset(&self, offset: u32) -> Option<SyntaxToken<'_>> {
        if offset as usize > self.text.len() {
            return None;
        }
        let index = self
            .tokens
            .partition_point(|t| t.span.end <= offset)
            .min(self.tokens.len() - 1);
        Some(SyntaxToken::new(self, TokenId::from_index(index)))
    }

    /// The innermost node containing the token at `offset`.
    pub fn node_at_offset(&self, offset: u32) -> Option<SyntaxNode<'_>> {
        self.token_at_offset(offset).map(|token| token.parent())
    }

    pub(crate) fn raw_node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    pub(crate) fn raw_nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub(crate) fn token_kinds_len(&self) -> usize {
        self.token_kinds.len()
    }

    pub(crate) fn token_kind(&self, id: TokenId) -> SyntaxKind {
        self.token_kinds[id.index()]
    }

    pub(crate) fn token_span(&self, id: TokenId) -> Span {
        self.tokens[id.index()].span
    }

    /// The innermost node whose token range contains `token`.
    pub(crate) fn token_parent(&self, token: TokenId) -> NodeId {
        // Nodes are sorted by their first token, and the nodes containing a token are a chain of
        // ancestors. The last node starting at or before the token is in the innermost one's
        // subtree, so the first of its ancestors (or itself) that contains the token is it.
        let index = self
            .nodes
            .partition_point(|n| n.first_token <= token)
            .saturating_sub(1);
        let mut id = NodeId::from_index(index);
        loop {
            let node = &self.nodes[id.index()];
            if node.token_end > token || id == NodeId::ROOT {
                return id;
            }
            id = node.parent;
        }
    }
}

impl fmt::Debug for SyntaxTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntaxTree")
            .field("release", &self.release)
            .field("tokens", &self.tokens.len())
            .field("nodes", &self.nodes.len())
            .finish()
    }
}

#[cfg(test)]
mod tests;
