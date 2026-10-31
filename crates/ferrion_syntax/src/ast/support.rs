//! What the generated accessors are made of: finding a node's children and direct tokens by
//! kind, optionally only between two of its tokens.

use std::marker::PhantomData;
use std::ops::Range;

use super::AstNode;
use crate::tree::{ChildrenWithTokens, NodeOrToken, TokenId};
use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// A direct token of a node, by kind and occurrence: the `nth` (from 0) of its direct tokens whose
/// kind is in `kinds`.
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub kinds: &'static [SyntaxKind],
    pub nth: usize,
}

/// The token ids of `node` after `after` and before `before`, or `None` if `after` is missing.
fn range(
    node: SyntaxNode<'_>,
    after: Option<Anchor>,
    before: Option<Anchor>,
) -> Option<Range<TokenId>> {
    let mut range = node.token_range();
    let find = |anchor: Anchor| {
        node.direct_tokens()
            .filter(|t| anchor.kinds.contains(&t.kind()))
            .nth(anchor.nth)
            .map(|t| t.id())
    };
    if let Some(after) = after {
        range.start = TokenId::from_index(find(after)?.index() + 1);
    }
    if let Some(before) = before
        && let Some(token) = find(before)
    {
        range.end = token;
    }
    Some(range)
}

pub fn child<'t, N: AstNode<'t>>(
    node: SyntaxNode<'t>,
    after: Option<Anchor>,
    before: Option<Anchor>,
) -> Option<N> {
    children(node, after, before).next()
}

pub fn children<'t, N: AstNode<'t>>(
    node: SyntaxNode<'t>,
    after: Option<Anchor>,
    before: Option<Anchor>,
) -> AstChildren<'t, N> {
    AstChildren {
        inner: node.children_with_tokens(),
        range: range(node, after, before),
        _marker: PhantomData,
    }
}

pub fn token<'t>(
    node: SyntaxNode<'t>,
    kinds: &'static [SyntaxKind],
    after: Option<Anchor>,
    before: Option<Anchor>,
) -> Option<SyntaxToken<'t>> {
    tokens(node, kinds, after, before).next()
}

pub fn tokens<'t>(
    node: SyntaxNode<'t>,
    kinds: &'static [SyntaxKind],
    after: Option<Anchor>,
    before: Option<Anchor>,
) -> AstTokens<'t> {
    AstTokens {
        inner: node.children_with_tokens(),
        kinds,
        range: range(node, after, before),
    }
}

/// The children of a node that a typed view accepts, in order.
#[derive(Clone)]
pub struct AstChildren<'t, N> {
    inner: ChildrenWithTokens<'t>,
    /// `None` when there can be no such children.
    range: Option<Range<TokenId>>,
    _marker: PhantomData<N>,
}

impl<'t, N: AstNode<'t>> Iterator for AstChildren<'t, N> {
    type Item = N;

    fn next(&mut self) -> Option<N> {
        let range = self.range.as_ref()?;
        for child in self.inner.by_ref() {
            let NodeOrToken::Node(node) = child else {
                continue;
            };
            let tokens = node.token_range();
            if tokens.end > range.end {
                return None;
            }
            if tokens.start >= range.start
                && let Some(view) = N::cast(node)
            {
                return Some(view);
            }
        }
        None
    }
}

/// The direct tokens of a node of some kinds, in order.
#[derive(Clone)]
pub struct AstTokens<'t> {
    inner: ChildrenWithTokens<'t>,
    kinds: &'static [SyntaxKind],
    range: Option<Range<TokenId>>,
}

impl<'t> Iterator for AstTokens<'t> {
    type Item = SyntaxToken<'t>;

    fn next(&mut self) -> Option<SyntaxToken<'t>> {
        let range = self.range.as_ref()?;
        for child in self.inner.by_ref() {
            let NodeOrToken::Token(token) = child else {
                continue;
            };
            if token.id() >= range.end {
                return None;
            }
            if token.id() >= range.start && self.kinds.contains(&token.kind()) {
                return Some(token);
            }
        }
        None
    }
}
