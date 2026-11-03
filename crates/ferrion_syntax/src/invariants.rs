//! Checks that a syntax tree keeps the promises its API documents, for tests and the fuzz targets
//! in `fuzz/`. Not part of the public API.
//!
//! Only documented promises are checked, so any failure is a bug in the tree's builder, the
//! parser, or the docs: the tree reproduces the text byte for byte with the lexer's tokens; nodes
//! are in a consistent pre-order with correct parent and `subtree_end` links; token ranges nest,
//! aren't empty, and (except the root's) start and end on non-trivia tokens; the root covers every
//! token; token kinds differ from the lexer's only for contextual keywords; `CONTAINS_ERROR` is
//! set on every `Error` node and propagated to its ancestors; and the comment map and doc comment
//! lookup keep the promises `comments` documents.

use ferrion_lexer::TokenKind;

use crate::comments::{self, CommentMap, Element, Placement};
use crate::tree::{NodeId, TokenId, flags};
use crate::{NodeOrToken, SyntaxKind, SyntaxPtr, SyntaxTree};

/// Checks `tree` on its own: everything above, except that its tokens are what the lexer makes of
/// its text, which `ferrion_parser::invariants` checks.
pub fn check_tree(tree: &SyntaxTree) -> Result<(), String> {
    check_structure(tree)?;
    check_cursors(tree)?;
    check_comments(tree)
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
        if node.kind == SyntaxKind::Error && node.flags & flags::OWN_ERROR == 0 {
            return fail("an Error node without OWN_ERROR".to_owned());
        }
        if node.flags & flags::OWN_ERROR != 0 && node.flags & flags::CONTAINS_ERROR == 0 {
            return fail("OWN_ERROR without CONTAINS_ERROR".to_owned());
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
    // A node contains an error only if the error is its own or a child's.
    // Children come after their parent, so in reverse every child is seen first.
    let mut child_has_error = vec![false; nodes.len()];
    for (i, node) in nodes.iter().enumerate().rev() {
        if node.flags & flags::CONTAINS_ERROR != 0 {
            if node.flags & flags::OWN_ERROR == 0 && !child_has_error[i] {
                return Err(format!(
                    "node {i}, {:?}: CONTAINS_ERROR without an error of its own or a child's",
                    node.kind
                ));
            }
            if i > 0 {
                child_has_error[node.parent.index()] = true;
            }
        }
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

/// Checks the comment map and doc comment lookup against the tree: every comment is placed once,
/// in its enclosing node, on an element of that node next to it (the next or previous code token
/// starts or ends it), and is listed under that element; its line break counts are right; and
/// each documentation comment found is before its node with only trivia between, and is leading
/// a node that starts where its node does.
fn check_comments(tree: &SyntaxTree) -> Result<(), String> {
    let lookups = tree.token_count() <= LOOKUP_LIMIT;
    let map = CommentMap::new(tree);
    let count = tree.token_count();
    let is_code = |id: usize| !tree.token(TokenId::from_index(id)).is_trivia();
    let mut prev_code = vec![None; count];
    let mut last = None;
    for (id, prev) in prev_code.iter_mut().enumerate() {
        *prev = last;
        if is_code(id) {
            last = Some(TokenId::from_index(id));
        }
    }
    let mut next_code = vec![None; count];
    let mut next = None;
    for (id, following) in next_code.iter_mut().enumerate().rev() {
        *following = next;
        if is_code(id) {
            next = Some(TokenId::from_index(id));
        }
    }
    let mut expected = tree.tokens().filter(|t| comments::is_comment(t.kind()));
    for comment in map.comments() {
        let fail = |problem: &str| Err(format!("comment {comment:?}: {problem}"));
        let token = tree.token(comment.token);
        if expected.next() != Some(token) {
            return fail("not the next comment token in source order");
        }
        let enclosing = tree.node(comment.enclosing);
        if !enclosing.token_range().contains(&comment.token)
            || lookups && token.parent() != enclosing
        {
            return fail("not in its enclosing node");
        }
        let element = comment.element.get(tree);
        let in_enclosing = match element {
            NodeOrToken::Node(node) => node.parent() == Some(enclosing),
            NodeOrToken::Token(token) => {
                !token.is_trivia()
                    && enclosing.token_range().contains(&token.id())
                    && (!lookups || token.parent() == enclosing)
            }
        };
        let (first, last) = match element {
            NodeOrToken::Node(node) => (node.first_token().id(), node.last_token().id()),
            NodeOrToken::Token(token) => (token.id(), token.id()),
        };
        let fits = match comment.placement {
            Placement::Dangling => comment.element == Element::Node(comment.enclosing),
            Placement::Leading => in_enclosing && next_code[comment.token.index()] == Some(first),
            Placement::Trailing => in_enclosing && prev_code[comment.token.index()] == Some(last),
        };
        if !fits {
            return fail("its element isn't next to it in its enclosing node");
        }
        let listed = match comment.placement {
            Placement::Leading => map.leading(comment.element),
            Placement::Trailing => map.trailing(comment.element),
            Placement::Dangling => map.dangling(comment.element),
        };
        if listed
            .binary_search_by_key(&comment.token, |c| c.token)
            .is_err()
        {
            return fail("not listed under its element");
        }
        let breaks = |ids: &mut dyn Iterator<Item = usize>| {
            ids.map(|id| tree.token(TokenId::from_index(id)).kind())
                .take_while(|k| matches!(k, SyntaxKind::Whitespace | SyntaxKind::LineTerminator))
                .filter(|&k| k == SyntaxKind::LineTerminator)
                .count() as u32
        };
        let before = breaks(&mut (0..comment.token.index()).rev());
        let after = breaks(&mut (comment.token.index() + 1..count));
        if (before, after) != (comment.line_breaks_before, comment.line_breaks_after) {
            return fail(&format!("{before} line breaks before it and {after} after"));
        }
    }
    if expected.next().is_some() {
        return Err("a comment token isn't in the comment map".to_owned());
    }
    for node in tree.nodes() {
        let Some(doc) = comments::doc_comment(node) else {
            continue;
        };
        let fail = |problem: &str| Err(format!("{node:?}'s doc comment {doc:?}: {problem}"));
        if !comments::can_have_doc_comment(node.kind()) {
            return fail("its kind has no doc comment");
        }
        let start = node.first_token().id();
        if doc.last_token().id() >= start
            || (doc.last_token().id().index() + 1..start.index()).any(is_code)
        {
            return fail("isn't before the node with only trivia between");
        }
        for token in doc.tokens() {
            if !comments::is_doc_comment(token.kind(), tree.release()) {
                return fail("isn't a documentation comment");
            }
            let placed = map.get(token.id());
            let leads = placed.is_some_and(|c| match c.element {
                Element::Node(id) => {
                    c.placement == Placement::Leading && tree.node(id).first_token().id() == start
                }
                Element::Token(_) => false,
            });
            if !leads {
                return fail(&format!("placed as {placed:?}, not leading the node"));
            }
        }
    }
    Ok(())
}
