//! Checks that a syntax tree keeps the promises its API documents, for tests and the fuzz targets
//! in `fuzz/`. Not part of the public API.
//!
//! Only documented promises are checked, so any failure is a bug in the tree's builder, the
//! parser, or the docs: the tree reproduces the text byte for byte with the lexer's tokens; nodes
//! are in a consistent pre-order with correct parent and `subtree_end` links; token ranges nest,
//! aren't empty, and (except the root's) start and end on non-trivia tokens; the root covers every
//! token; token kinds differ from the lexer's only for contextual keywords; and `CONTAINS_ERROR`
//! is set on every `Error` node and propagated to its ancestors.

use ferrion_lexer::TokenKind;

use crate::tree::{NodeId, TokenId, flags};
use crate::{NodeOrToken, SyntaxKind, SyntaxPtr, SyntaxTree};

/// Checks `tree` on its own: everything above, except that its tokens are what the lexer makes of
/// its text, which `ferrion_parser::invariants` checks.
pub fn check_tree(tree: &SyntaxTree) -> Result<(), String> {
    check_structure(tree)?;
    check_cursors(tree)
}

/// The arrays' part of [`check_tree`], which [`SyntaxTree::new`] runs in debug builds: linear in
/// the tree's size.
pub(crate) fn check_structure(tree: &SyntaxTree) -> Result<(), String> {
    check_tokens(tree)?;
    check_nodes(tree)
}

fn check_tokens(tree: &SyntaxTree) -> Result<(), String> {
    let tokens = tree.lexer_tokens();
    if tokens.is_empty() {
        return Err("no tokens, not even Eof".to_owned());
    }
    if tree.token_kinds_len() != tokens.len() {
        return Err("a token kind for each token".to_owned());
    }
    let joined: String = tokens
        .iter()
        .map(|t| &tree.text()[t.span.range()])
        .collect();
    if joined != tree.text() {
        return Err("concatenating the tokens doesn't reproduce the text".to_owned());
    }
    if tokens.last().map(|t| t.kind) != Some(TokenKind::Eof) {
        return Err("the last token isn't Eof".to_owned());
    }
    for token in tree.tokens() {
        let lexer_kind = SyntaxKind::from_token_kind(token.lexer_kind());
        let kind = token.kind();
        let fits = kind == lexer_kind
            || kind.is_contextual_keyword()
                && (lexer_kind == SyntaxKind::Identifier
                    || kind == SyntaxKind::NonSealedKw && lexer_kind == SyntaxKind::Minus);
        if !fits {
            return Err(format!(
                "{token:?}: parsed as {kind:?}, but the lexer made it {lexer_kind:?}"
            ));
        }
    }
    Ok(())
}

fn check_nodes(tree: &SyntaxTree) -> Result<(), String> {
    let nodes = tree.raw_nodes();
    let Some(root) = nodes.first() else {
        return Err("no root".to_owned());
    };
    if root.kind != SyntaxKind::CompilationUnit {
        return Err(format!("the root is a {:?}", root.kind));
    }
    if root.parent != NodeId::ROOT {
        return Err("the root's parent isn't itself".to_owned());
    }
    if root.first_token.index() != 0 || root.token_end.index() != tree.token_count() {
        return Err("the root doesn't cover every token".to_owned());
    }
    if root.subtree_end.index() != nodes.len() {
        return Err("the root's subtree isn't every node".to_owned());
    }
    let eof = TokenId::from_index(tree.token_count() - 1);
    // The open ancestors of the current node, innermost last, each with the end of its last
    // child so far.
    let mut open: Vec<(usize, TokenId)> = vec![(0, root.first_token)];
    for (i, node) in nodes.iter().enumerate() {
        let fail = |problem: String| Err(format!("node {i}, {:?}: {problem}", node.kind));
        if !node.kind.is_node() {
            return fail("has a token kind".to_owned());
        }
        if node.flags & !flags::ALL != 0 {
            return fail(format!("unknown flags {:#x}", node.flags));
        }
        if node.subtree_end.index() <= i || node.subtree_end.index() > nodes.len() {
            return fail(format!("subtree_end {}", node.subtree_end.index()));
        }
        if node.token_end <= node.first_token {
            return fail("is empty".to_owned());
        }
        if node.kind == SyntaxKind::Error && node.flags & flags::CONTAINS_ERROR == 0 {
            return fail("an Error node without CONTAINS_ERROR".to_owned());
        }
        if i == 0 {
            continue;
        }
        while open
            .last()
            .is_some_and(|&(parent, _)| nodes[parent].subtree_end.index() <= i)
        {
            open.pop();
        }
        let Some(&mut (parent, ref mut previous_end)) = open.last_mut() else {
            return fail("is outside the root".to_owned());
        };
        if node.parent.index() != parent {
            return fail(format!(
                "its parent is {}, but it's in {parent}'s subtree",
                node.parent.index()
            ));
        }
        let parent_node = &nodes[parent];
        if node.subtree_end > parent_node.subtree_end {
            return fail("its subtree ends after its parent's".to_owned());
        }
        if node.first_token < *previous_end {
            return fail("overlaps its previous sibling or starts before its parent".to_owned());
        }
        if node.token_end > parent_node.token_end {
            return fail("ends after its parent".to_owned());
        }
        if node.token_end > eof {
            return fail("contains Eof".to_owned());
        }
        let first = tree.token(node.first_token);
        let last = tree.token(TokenId::from_index(node.token_end.index() - 1));
        if [first, last]
            .iter()
            .any(|t| t.is_trivia() || t.kind() == SyntaxKind::ErrorToken)
        {
            return fail("starts or ends with trivia".to_owned());
        }
        if node.flags & flags::CONTAINS_ERROR != 0 && parent_node.flags & flags::CONTAINS_ERROR == 0
        {
            return fail("CONTAINS_ERROR isn't set on its parent".to_owned());
        }
        *previous_end = node.token_end;
        open.push((i, node.first_token));
    }
    Ok(())
}

/// Above this many tokens, the per-token and per-node lookups in [`check_cursors`] are skipped:
/// on deeply nested input they take time quadratic in the depth.
const LOOKUP_LIMIT: usize = 20_000;

/// Checks the cursors against the arrays: children with tokens cover each node's tokens in order,
/// a direct token's parent is its node, and a node's [`SyntaxPtr`] finds it.
fn check_cursors(tree: &SyntaxTree) -> Result<(), String> {
    let lookups = tree.token_count() <= LOOKUP_LIMIT;
    for node in tree.nodes() {
        let mut next = node.token_range().start;
        for child in node.children_with_tokens() {
            let (start, end) = match child {
                NodeOrToken::Node(child) => {
                    if child.parent() != Some(node) {
                        return Err(format!("{child:?}: its parent isn't {node:?}"));
                    }
                    let range = child.token_range();
                    (range.start, range.end)
                }
                NodeOrToken::Token(token) => {
                    if lookups && token.parent() != node {
                        return Err(format!(
                            "{token:?}: its parent is {:?}, not {node:?}",
                            token.parent()
                        ));
                    }
                    (token.id(), TokenId::from_index(token.id().index() + 1))
                }
            };
            if start != next {
                return Err(format!("{node:?}: its children with tokens skip a token"));
            }
            next = end;
        }
        if next != node.token_range().end {
            return Err(format!("{node:?}: its children with tokens stop early"));
        }
        if lookups {
            let found = SyntaxPtr::new(node).resolve(tree);
            if found.map(|n| (n.kind(), n.span())) != Some((node.kind(), node.span())) {
                return Err(format!("{node:?}: its SyntaxPtr finds {found:?}"));
            }
        }
    }
    Ok(())
}
