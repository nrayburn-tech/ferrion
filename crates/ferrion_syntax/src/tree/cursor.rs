//! Cursors into a [`SyntaxTree`], and iterators over it. Every iterator is a loop over the tree's
//! arrays; none recurses.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use ferrion_diagnostics::Span;
use ferrion_lexer::TokenKind;

use super::{NodeId, SyntaxTree, TokenId, flags};
use crate::SyntaxKind;

/// A node of a tree: the tree and the node's id. Free to copy.
#[derive(Clone, Copy)]
pub struct SyntaxNode<'t> {
    tree: &'t SyntaxTree,
    id: NodeId,
}

/// A token of a tree: the tree and the token's id. Free to copy.
#[derive(Clone, Copy)]
pub struct SyntaxToken<'t> {
    tree: &'t SyntaxTree,
    id: TokenId,
}

/// A child of a node: a node or one of its direct tokens.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NodeOrToken<'t> {
    Node(SyntaxNode<'t>),
    Token(SyntaxToken<'t>),
}

/// Entering or leaving a node in a [`Preorder`] walk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WalkEvent<'t> {
    Enter(SyntaxNode<'t>),
    Leave(SyntaxNode<'t>),
}

impl<'t> SyntaxNode<'t> {
    pub(crate) fn new(tree: &'t SyntaxTree, id: NodeId) -> SyntaxNode<'t> {
        SyntaxNode { tree, id }
    }

    pub fn tree(self) -> &'t SyntaxTree {
        self.tree
    }

    pub fn id(self) -> NodeId {
        self.id
    }

    pub fn kind(self) -> SyntaxKind {
        self.raw().kind
    }

    /// Whether the node's subtree holds an `Error` node, a missing child, or a token with a
    /// lexical error.
    pub fn contains_error(self) -> bool {
        self.raw().flags & flags::CONTAINS_ERROR != 0
    }

    /// Whether the node has an error of its own, not only in its children: it is an `Error`
    /// node, it lacks a token or a child, or one of its direct tokens has a lexical error.
    pub fn has_own_error(self) -> bool {
        self.raw().flags & flags::OWN_ERROR != 0
    }

    /// From the start of its first token to the end of its last. For the root, the whole text.
    pub fn span(self) -> Span {
        let node = self.raw();
        let start = self.tree.token_span(node.first_token).start;
        let end = self
            .tree
            .token_span(TokenId::from_index(node.token_end.index() - 1))
            .end;
        Span::new(start, end)
    }

    pub fn text(self) -> &'t str {
        &self.tree.text()[self.span().range()]
    }

    /// The ids of the node's tokens, trivia included.
    pub fn token_range(self) -> Range<TokenId> {
        let node = self.raw();
        node.first_token..node.token_end
    }

    /// The ids of the node and its descendants.
    pub fn subtree(self) -> Range<NodeId> {
        self.id..self.raw().subtree_end
    }

    /// Whether `other` is this node or one of its descendants.
    pub fn contains(self, other: SyntaxNode<'_>) -> bool {
        self.subtree().contains(&other.id)
    }

    /// `None` for the root.
    pub fn parent(self) -> Option<SyntaxNode<'t>> {
        let parent = self.raw().parent;
        (self.id != NodeId::ROOT).then(|| SyntaxNode::new(self.tree, parent))
    }

    /// The node and its ancestors, innermost first.
    pub fn ancestors(self) -> Ancestors<'t> {
        Ancestors { next: Some(self) }
    }

    pub fn first_child(self) -> Option<SyntaxNode<'t>> {
        let first = NodeId::from_index(self.id.index() + 1);
        (first < self.raw().subtree_end).then(|| SyntaxNode::new(self.tree, first))
    }

    pub fn next_sibling(self) -> Option<SyntaxNode<'t>> {
        let parent = self.parent()?;
        let next = self.raw().subtree_end;
        (next < parent.raw().subtree_end).then(|| SyntaxNode::new(self.tree, next))
    }

    /// The node's child nodes, in order.
    pub fn children(self) -> Children<'t> {
        Children {
            tree: self.tree,
            next: NodeId::from_index(self.id.index() + 1),
            end: self.raw().subtree_end,
        }
    }

    /// The node's child nodes and direct tokens (the tokens not in any child), trivia included,
    /// in order.
    pub fn children_with_tokens(self) -> ChildrenWithTokens<'t> {
        let node = self.raw();
        ChildrenWithTokens {
            children: self.children(),
            next_token: node.first_token,
            token_end: node.token_end,
        }
    }

    /// The node's direct tokens, trivia included.
    pub fn direct_tokens(self) -> impl Iterator<Item = SyntaxToken<'t>> + 't {
        self.children_with_tokens().filter_map(|child| match child {
            NodeOrToken::Token(token) => Some(token),
            NodeOrToken::Node(_) => None,
        })
    }

    /// The node and its descendants, in pre-order.
    pub fn descendants(self) -> Descendants<'t> {
        Descendants {
            tree: self.tree,
            next: self.id,
            end: self.raw().subtree_end,
        }
    }

    /// A walk over the node's subtree that reports entering and leaving each node.
    pub fn preorder(self) -> Preorder<'t> {
        Preorder {
            tree: self.tree,
            next: self.id,
            end: self.raw().subtree_end,
            open: Vec::new(),
        }
    }

    /// Every token in the node, trivia included, in order.
    pub fn tokens(self) -> Tokens<'t> {
        let range = self.token_range();
        Tokens {
            tree: self.tree,
            next: range.start,
            end: range.end,
        }
    }

    /// The node's first token. For nodes other than the root, it isn't trivia.
    pub fn first_token(self) -> SyntaxToken<'t> {
        SyntaxToken::new(self.tree, self.raw().first_token)
    }

    /// The node's last token. For nodes other than the root, it isn't trivia.
    pub fn last_token(self) -> SyntaxToken<'t> {
        SyntaxToken::new(
            self.tree,
            TokenId::from_index(self.raw().token_end.index() - 1),
        )
    }

    fn raw(self) -> &'t super::Node {
        self.tree.raw_node(self.id)
    }
}

impl<'t> SyntaxToken<'t> {
    pub(crate) fn new(tree: &'t SyntaxTree, id: TokenId) -> SyntaxToken<'t> {
        SyntaxToken { tree, id }
    }

    pub fn tree(self) -> &'t SyntaxTree {
        self.tree
    }

    pub fn id(self) -> TokenId {
        self.id
    }

    /// The kind the parser read the token as: the lexer's kind, or a contextual keyword's.
    pub fn kind(self) -> SyntaxKind {
        self.tree.token_kind(self.id)
    }

    /// The kind the lexer gave the token.
    pub fn lexer_kind(self) -> TokenKind {
        self.tree.lexer_tokens()[self.id.index()].kind
    }

    pub fn is_trivia(self) -> bool {
        self.kind().is_trivia()
    }

    pub fn span(self) -> Span {
        self.tree.token_span(self.id)
    }

    /// The token's text as written, including any Unicode escapes.
    pub fn text(self) -> &'t str {
        &self.tree.text()[self.span().range()]
    }

    /// The innermost node containing the token.
    pub fn parent(self) -> SyntaxNode<'t> {
        SyntaxNode::new(self.tree, self.tree.token_parent(self.id))
    }

    pub fn prev_token(self) -> Option<SyntaxToken<'t>> {
        let index = self.id.index().checked_sub(1)?;
        Some(SyntaxToken::new(self.tree, TokenId::from_index(index)))
    }

    pub fn next_token(self) -> Option<SyntaxToken<'t>> {
        let index = self.id.index() + 1;
        (index < self.tree.token_count())
            .then(|| SyntaxToken::new(self.tree, TokenId::from_index(index)))
    }

    /// The previous token that isn't trivia.
    pub fn prev_non_trivia(self) -> Option<SyntaxToken<'t>> {
        let mut token = self.prev_token()?;
        while token.is_trivia() {
            token = token.prev_token()?;
        }
        Some(token)
    }

    /// The next token that isn't trivia.
    pub fn next_non_trivia(self) -> Option<SyntaxToken<'t>> {
        let mut token = self.next_token()?;
        while token.is_trivia() {
            token = token.next_token()?;
        }
        Some(token)
    }
}

impl PartialEq for SyntaxNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.tree, other.tree) && self.id == other.id
    }
}

impl Eq for SyntaxNode<'_> {}

impl Hash for SyntaxNode<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.tree, state);
        self.id.hash(state);
    }
}

impl PartialEq for SyntaxToken<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.tree, other.tree) && self.id == other.id
    }
}

impl Eq for SyntaxToken<'_> {}

impl Hash for SyntaxToken<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.tree, state);
        self.id.hash(state);
    }
}

impl fmt::Debug for SyntaxNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}@{:?}", self.kind(), self.span())
    }
}

impl fmt::Debug for SyntaxToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}@{:?} {:?}", self.kind(), self.span(), self.text())
    }
}

impl<'t> NodeOrToken<'t> {
    pub fn kind(self) -> SyntaxKind {
        match self {
            NodeOrToken::Node(node) => node.kind(),
            NodeOrToken::Token(token) => token.kind(),
        }
    }

    pub fn span(self) -> Span {
        match self {
            NodeOrToken::Node(node) => node.span(),
            NodeOrToken::Token(token) => token.span(),
        }
    }

    pub fn as_node(self) -> Option<SyntaxNode<'t>> {
        match self {
            NodeOrToken::Node(node) => Some(node),
            NodeOrToken::Token(_) => None,
        }
    }

    pub fn as_token(self) -> Option<SyntaxToken<'t>> {
        match self {
            NodeOrToken::Node(_) => None,
            NodeOrToken::Token(token) => Some(token),
        }
    }
}

/// See [`SyntaxNode::ancestors`].
#[derive(Clone)]
pub struct Ancestors<'t> {
    next: Option<SyntaxNode<'t>>,
}

impl<'t> Iterator for Ancestors<'t> {
    type Item = SyntaxNode<'t>;

    fn next(&mut self) -> Option<SyntaxNode<'t>> {
        let node = self.next?;
        self.next = node.parent();
        Some(node)
    }
}

/// See [`SyntaxNode::children`].
#[derive(Clone)]
pub struct Children<'t> {
    tree: &'t SyntaxTree,
    next: NodeId,
    end: NodeId,
}

impl<'t> Iterator for Children<'t> {
    type Item = SyntaxNode<'t>;

    fn next(&mut self) -> Option<SyntaxNode<'t>> {
        if self.next >= self.end {
            return None;
        }
        let node = SyntaxNode::new(self.tree, self.next);
        self.next = self.tree.raw_node(self.next).subtree_end;
        Some(node)
    }
}

/// See [`SyntaxNode::children_with_tokens`].
#[derive(Clone)]
pub struct ChildrenWithTokens<'t> {
    children: Children<'t>,
    next_token: TokenId,
    token_end: TokenId,
}

impl<'t> Iterator for ChildrenWithTokens<'t> {
    type Item = NodeOrToken<'t>;

    fn next(&mut self) -> Option<NodeOrToken<'t>> {
        if self.next_token >= self.token_end {
            return None;
        }
        let tree = self.children.tree;
        if self.children.next < self.children.end {
            let child = tree.raw_node(self.children.next);
            if child.first_token == self.next_token {
                self.next_token = child.token_end;
                return self.children.next().map(NodeOrToken::Node);
            }
        }
        let token = SyntaxToken::new(tree, self.next_token);
        self.next_token = TokenId::from_index(self.next_token.index() + 1);
        Some(NodeOrToken::Token(token))
    }
}

/// See [`SyntaxNode::descendants`].
#[derive(Clone)]
pub struct Descendants<'t> {
    tree: &'t SyntaxTree,
    next: NodeId,
    end: NodeId,
}

impl<'t> Iterator for Descendants<'t> {
    type Item = SyntaxNode<'t>;

    fn next(&mut self) -> Option<SyntaxNode<'t>> {
        if self.next >= self.end {
            return None;
        }
        let node = SyntaxNode::new(self.tree, self.next);
        self.next = NodeId::from_index(self.next.index() + 1);
        Some(node)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.end.index().saturating_sub(self.next.index());
        (len, Some(len))
    }
}

impl ExactSizeIterator for Descendants<'_> {}

/// See [`SyntaxNode::preorder`].
#[derive(Clone)]
pub struct Preorder<'t> {
    tree: &'t SyntaxTree,
    next: NodeId,
    end: NodeId,
    /// The entered nodes not yet left, innermost last.
    open: Vec<NodeId>,
}

impl<'t> Preorder<'t> {
    /// Skips the subtree of the node just entered: its `Leave` comes next.
    pub fn skip_subtree(&mut self) {
        if let Some(&open) = self.open.last() {
            self.next = self.next.max(self.tree.raw_node(open).subtree_end);
        }
    }
}

impl<'t> Iterator for Preorder<'t> {
    type Item = WalkEvent<'t>;

    fn next(&mut self) -> Option<WalkEvent<'t>> {
        if let Some(&open) = self.open.last()
            && (self.next >= self.tree.raw_node(open).subtree_end || self.next >= self.end)
        {
            self.open.pop();
            return Some(WalkEvent::Leave(SyntaxNode::new(self.tree, open)));
        }
        if self.next >= self.end {
            return None;
        }
        let node = self.next;
        self.open.push(node);
        self.next = NodeId::from_index(node.index() + 1);
        Some(WalkEvent::Enter(SyntaxNode::new(self.tree, node)))
    }
}

/// See [`SyntaxNode::tokens`].
#[derive(Clone)]
pub struct Tokens<'t> {
    tree: &'t SyntaxTree,
    next: TokenId,
    end: TokenId,
}

impl<'t> Iterator for Tokens<'t> {
    type Item = SyntaxToken<'t>;

    fn next(&mut self) -> Option<SyntaxToken<'t>> {
        if self.next >= self.end {
            return None;
        }
        let token = SyntaxToken::new(self.tree, self.next);
        self.next = TokenId::from_index(self.next.index() + 1);
        Some(token)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.end.index().saturating_sub(self.next.index());
        (len, Some(len))
    }
}

impl ExactSizeIterator for Tokens<'_> {}

impl DoubleEndedIterator for Tokens<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.next >= self.end {
            return None;
        }
        self.end = TokenId::from_index(self.end.index() - 1);
        Some(SyntaxToken::new(self.tree, self.end))
    }
}
