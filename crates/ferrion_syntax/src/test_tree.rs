//! Trees for unit tests, built without the parser, which this crate can't use.

use std::sync::Arc;

use crate::tree::{Node, NodeId, TokenId, flags};
use crate::{SyntaxKind, SyntaxTree, invariants};

/// A tree's nodes, described as the parser builds them: open a node, consume the next non-trivia
/// token with a kind, close a node. Trivia go to the innermost open node, so a node starts and
/// ends on non-trivia tokens; the root takes every token. A small stand-in for the parser's
/// builder.
pub(crate) struct Events(Vec<Step>);

#[derive(Clone, Copy)]
enum Step {
    Start(SyntaxKind),
    /// A token with this kind, or with the lexer's kind for `None`.
    Token(Option<SyntaxKind>),
    Finish,
    /// Marks the innermost open node as containing an error.
    Error,
}

impl Events {
    pub(crate) fn new() -> Events {
        Events(Vec::new())
    }

    pub(crate) fn start(mut self, kind: SyntaxKind) -> Events {
        self.0.push(Step::Start(kind));
        self
    }

    pub(crate) fn token(mut self, kind: SyntaxKind) -> Events {
        self.0.push(Step::Token(Some(kind)));
        self
    }

    pub(crate) fn tokens(mut self, kinds: &[SyntaxKind]) -> Events {
        for &kind in kinds {
            self = self.token(kind);
        }
        self
    }

    /// The next `n` non-trivia tokens, with the lexer's kinds.
    pub(crate) fn any(mut self, n: usize) -> Events {
        for _ in 0..n {
            self.0.push(Step::Token(None));
        }
        self
    }

    pub(crate) fn finish(mut self) -> Events {
        self.0.push(Step::Finish);
        self
    }

    pub(crate) fn error(mut self) -> Events {
        self.0.push(Step::Error);
        self
    }

    /// Events from a compact description: `Kind(...)` is a node of that kind around what's
    /// inside the parentheses, `*` is the next token with the lexer's kind (`*3` is the next
    /// three), and a token kind's name alone is the next token as that kind. For example,
    /// `CompilationUnit(PackageDecl(* Name(*) *) *)` for `package a;`.
    pub(crate) fn parse(spec: &str) -> Events {
        let kinds: std::collections::HashMap<String, SyntaxKind> = SyntaxKind::ALL
            .iter()
            .map(|&k| (format!("{k:?}"), k))
            .collect();
        let mut events = Events::new();
        let mut rest = spec;
        loop {
            rest = rest.trim_start();
            let Some(c) = rest.chars().next() else {
                return events;
            };
            if c == ')' {
                events = events.finish();
                rest = &rest[1..];
            } else if c == '*' {
                let digits = rest[1..].len()
                    - rest[1..]
                        .trim_start_matches(|c: char| c.is_ascii_digit())
                        .len();
                let n = if digits == 0 {
                    1
                } else {
                    rest[1..=digits].parse().unwrap()
                };
                events = events.any(n);
                rest = &rest[1 + digits..];
            } else {
                let len = rest.len()
                    - rest
                        .trim_start_matches(|c: char| c.is_ascii_alphanumeric())
                        .len();
                assert!(len > 0, "unexpected {c:?} in {spec:?}");
                let name = &rest[..len];
                let kind = *kinds
                    .get(name)
                    .unwrap_or_else(|| panic!("no kind {name:?}"));
                rest = &rest[len..];
                if let Some(after) = rest.strip_prefix('(') {
                    events = events.start(kind);
                    rest = after;
                } else {
                    events = events.token(kind);
                }
            }
        }
    }
}

/// The tree `events` describe over `text`'s tokens, parsed for release 21. Panics if it breaks
/// one of the tree's invariants.
pub(crate) fn build(text: &str, events: Events) -> SyntaxTree {
    build_for(text, events, 21)
}

/// [`build`] for `release`.
pub(crate) fn build_for(text: &str, events: Events, release: u32) -> SyntaxTree {
    let tokens = ferrion_lexer::lex(text).tokens;
    let mut token_kinds: Vec<_> = tokens
        .iter()
        .map(|t| SyntaxKind::from_token_kind(t.kind))
        .collect();
    let mut nodes: Vec<Node> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut next = 0;
    let mut last_end = 0;
    let skip_trivia = |next: &mut usize| {
        while tokens[*next].kind.is_trivia() {
            *next += 1;
        }
    };
    for step in events.0 {
        match step {
            Step::Start(kind) => {
                if !open.is_empty() {
                    skip_trivia(&mut next);
                }
                nodes.push(Node {
                    kind,
                    flags: 0,
                    parent: NodeId::from_index(open.last().copied().unwrap_or(0)),
                    first_token: TokenId::from_index(next),
                    token_end: TokenId::from_index(next),
                    subtree_end: NodeId::from_index(0),
                });
                open.push(nodes.len() - 1);
            }
            Step::Token(kind) => {
                skip_trivia(&mut next);
                if let Some(kind) = kind {
                    token_kinds[next] = kind;
                }
                next += 1;
                last_end = next;
            }
            Step::Finish => {
                let node = open.pop().unwrap();
                if open.is_empty() {
                    last_end = tokens.len();
                }
                nodes[node].token_end = TokenId::from_index(last_end);
                nodes[node].subtree_end = NodeId::from_index(nodes.len());
                if let Some(&parent) = open.last() {
                    nodes[parent].flags |= nodes[node].flags;
                }
            }
            Step::Error => nodes[*open.last().unwrap()].flags |= flags::CONTAINS_ERROR,
        }
    }
    let tree = SyntaxTree::new(Arc::from(text), tokens, token_kinds, nodes, release);
    invariants::check_tree(&tree).unwrap();
    tree
}
