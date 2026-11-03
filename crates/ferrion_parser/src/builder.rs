//! Turns the parser's events into a [`SyntaxTree`], in one loop with an explicit stack.
//!
//! The parser sees only non-trivia tokens. The builder places trivia: a node starts at its first
//! non-trivia token and ends after its last, so the trivia around a node belong to its parent,
//! and the trivia between two tokens belong to the innermost node covering both. The root is the
//! exception: it covers every token, from the leading trivia to `Eof`.

use std::sync::Arc;

use ferrion_diagnostics::Span;
use ferrion_lexer::Token;

use ferrion_syntax::{Node, NodeId, SyntaxKind, SyntaxTree, TokenId, flags};

#[cfg(test)]
mod tests;

/// What the parser does, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// Opens a node. `forward_parent`, when it isn't 0, is the distance to a later `Start` event
    /// for a node that encloses this one: a marker that was moved earlier (`precede`).
    Start {
        kind: SyntaxKind,
        forward_parent: u32,
    },
    /// Closes the innermost open node.
    Finish,
    /// Consumes the next non-trivia token, recording `kind` as the kind it was parsed as.
    Token { kind: SyntaxKind },
    /// Marks the innermost open node as having an error of its own, and so its ancestors as
    /// containing one.
    Error,
    /// A `Start` that was abandoned or moved.
    Tombstone,
}

pub(crate) struct Builder {
    text: Arc<str>,
    tokens: Vec<Token>,
    token_kinds: Vec<SyntaxKind>,
    nodes: Vec<Node>,
    /// The open nodes, innermost last.
    stack: Vec<NodeId>,
    /// The next token to place.
    next_token: usize,
    /// One past the last non-trivia token consumed.
    last_token_end: usize,
    /// Starts of the lexical errors' spans, in order, and the next one to place.
    lex_errors: Vec<u32>,
    next_lex_error: usize,
    release: u32,
    /// Whether the root has been closed; later events are ignored.
    done: bool,
}

impl Builder {
    /// `tokens` must be the lexer's tokens for `text`, ending with `Eof`, and `lex_errors` the
    /// spans of its errors, in order.
    pub fn new(text: Arc<str>, tokens: Vec<Token>, lex_errors: &[Span], release: u32) -> Builder {
        let token_kinds = tokens
            .iter()
            .map(|t| SyntaxKind::from_token_kind(t.kind))
            .collect();
        Builder {
            text,
            tokens,
            token_kinds,
            nodes: Vec::new(),
            stack: Vec::new(),
            next_token: 0,
            last_token_end: 0,
            lex_errors: lex_errors.iter().map(|span| span.start).collect(),
            next_lex_error: 0,
            release,
            done: false,
        }
    }

    pub fn build(mut self, mut events: Vec<Event>) -> SyntaxTree {
        let mut kinds = Vec::new();
        for i in 0..events.len() {
            match std::mem::replace(&mut events[i], Event::Tombstone) {
                Event::Start {
                    kind,
                    forward_parent,
                } => {
                    // Open the outermost of the nodes this one is preceded by first.
                    kinds.push(kind);
                    let mut index = i;
                    let mut forward_parent = forward_parent;
                    while forward_parent != 0 {
                        index += forward_parent as usize;
                        let Some(event) = events.get_mut(index) else {
                            break;
                        };
                        match std::mem::replace(event, Event::Tombstone) {
                            Event::Start {
                                kind,
                                forward_parent: next,
                            } => {
                                kinds.push(kind);
                                forward_parent = next;
                            }
                            _ => break,
                        }
                    }
                    for kind in kinds.drain(..).rev() {
                        self.start(kind);
                    }
                }
                Event::Finish => self.finish(),
                Event::Token { kind } => self.token(kind),
                Event::Error => self.mark_error(),
                Event::Tombstone => {}
            }
        }
        // Close whatever the events left open, the root last. With no events at all, the root
        // is still made.
        if self.nodes.is_empty() {
            self.start(SyntaxKind::CompilationUnit);
        }
        while !self.done && !self.stack.is_empty() {
            self.finish();
        }
        SyntaxTree::new(
            self.text,
            self.tokens,
            self.token_kinds,
            self.nodes,
            self.release,
        )
    }

    fn start(&mut self, kind: SyntaxKind) {
        if self.done {
            return;
        }
        let parent = match self.stack.last() {
            Some(&parent) => {
                // Trivia before the node belong to its parent.
                self.skip_trivia();
                parent
            }
            None => NodeId::ROOT,
        };
        let id = NodeId::from_index(self.nodes.len());
        self.nodes.push(Node {
            kind,
            flags: 0,
            parent,
            first_token: TokenId::from_index(self.next_token),
            token_end: TokenId::from_index(self.next_token),
            subtree_end: NodeId::from_index(self.nodes.len() + 1),
        });
        self.stack.push(id);
    }

    fn finish(&mut self) {
        let is_root = self.stack.len() == 1;
        if is_root {
            // The root covers every token: trailing trivia, and `Eof` if the parser didn't
            // consume it.
            while self.next_token < self.tokens.len() {
                self.consume_raw();
            }
            self.last_token_end = self.tokens.len();
            self.done = true;
        }
        let Some(id) = self.stack.pop() else {
            return;
        };
        let subtree_end = NodeId::from_index(self.nodes.len());
        let node = &mut self.nodes[id.index()];
        node.token_end = TokenId::from_index(self.last_token_end);
        node.subtree_end = subtree_end;
        let node = *node;
        let empty = node.token_end <= node.first_token && !is_root;
        // An empty node is a parser bug. In release builds it is dropped, with its subtree,
        // which is empty too, keeping what it says about errors.
        debug_assert!(
            !empty,
            "the parser made an empty {:?} at token {}",
            node.kind,
            node.first_token.index()
        );
        if empty {
            self.nodes.truncate(id.index());
        }
        if node.flags & flags::CONTAINS_ERROR != 0
            && let Some(&parent) = self.stack.last()
        {
            // A dropped node's error becomes its parent's own.
            let own = if empty { flags::OWN_ERROR } else { 0 };
            self.nodes[parent.index()].flags |= flags::CONTAINS_ERROR | own;
        }
    }

    fn token(&mut self, kind: SyntaxKind) {
        if self.done {
            return;
        }
        self.skip_trivia();
        if self.next_token < self.tokens.len() {
            self.token_kinds[self.next_token] = kind;
            self.consume_raw();
            self.last_token_end = self.next_token;
        }
    }

    fn mark_error(&mut self) {
        if let Some(&id) = self.stack.last() {
            self.nodes[id.index()].flags |= flags::CONTAINS_ERROR | flags::OWN_ERROR;
        }
    }

    fn skip_trivia(&mut self) {
        while self
            .tokens
            .get(self.next_token)
            .is_some_and(|t| crate::parser::skipped(t.kind))
        {
            self.consume_raw();
        }
    }

    /// Places the next token in the innermost open node, marking it if a lexical error starts in
    /// the token.
    fn consume_raw(&mut self) {
        let token = self.tokens[self.next_token];
        let last = self.next_token + 1 == self.tokens.len();
        while let Some(&start) = self.lex_errors.get(self.next_lex_error) {
            if start < token.span.end || (last && start <= token.span.end) {
                self.mark_error();
                self.next_lex_error += 1;
            } else {
                break;
            }
        }
        self.next_token += 1;
    }
}
