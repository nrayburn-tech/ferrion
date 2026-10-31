//! The parser's core, shared by every part of the grammar: lookahead over non-trivia tokens,
//! contextual keywords, events and markers, error reporting and recovery, the progress guard, and
//! the nesting limit (docs/design/parser-error-recovery.md, parser-deep-nesting.md).
//!
//! The grammar never builds nodes itself. It emits events: [`Parser::start`] opens a node with a
//! [`Marker`], tokens are consumed with [`Parser::bump`] and friends, and the marker is completed
//! with the node's kind, or abandoned. A completed node can be wrapped in a new one that starts
//! before it with [`CompletedMarker::precede`], which is how left-nested nodes (`a.b.c`, `a + b`)
//! are parsed in loops.
//!
//! Rules every grammar function follows:
//!
//! - **Every loop makes progress.** Each loop calls [`Progress::advanced`] once per iteration. A
//!   loop that consumed nothing is a bug: a debug assertion catches it, and in release builds the
//!   current token goes in an `Error` node so the parser can't hang.
//! - **Every recursive function nests through [`Parser::nested`]**, which grows the stack when it
//!   runs low and enforces [`NESTING_LIMIT`]. Chains at one level (operators, `else if`, member
//!   lists) are loops, not recursion.
//! - **No node is empty.** Complete a marker only after consuming a token; otherwise abandon it.
//! - **Decisions come from lookahead, never from backtracking.**

mod token_set;

#[cfg(test)]
mod tests;

use ferrion_diagnostics::Span;
use ferrion_lexer::{ContextualKeyword, Token, cooked_text};

use crate::builder::Event;
use crate::diagnostic::{Expected, ParseDiagnostic, ParseDiagnosticKind};
use crate::{NESTING_LIMIT, SyntaxKind};

pub(crate) use token_set::TokenSet;

/// A non-trivia token, as the parser sees it.
#[derive(Clone, Copy)]
struct Significant {
    /// Its index among all tokens.
    raw: u32,
    /// Its kind from the lexer.
    kind: SyntaxKind,
    /// For an identifier that spells a contextual keyword the parser records, that keyword's
    /// kind.
    contextual: Option<SyntaxKind>,
}

pub(crate) struct Parser<'t> {
    text: &'t str,
    tokens: &'t [Token],
    /// The non-trivia tokens, ending with `Eof`.
    significant: Vec<Significant>,
    /// The current token, an index into `significant`.
    pos: usize,
    events: Vec<Event>,
    diagnostics: Vec<ParseDiagnostic>,
    #[allow(dead_code, reason = "for the grammar areas still to come")]
    release: u32,
    /// How many [`Parser::nested`] calls are active.
    depth: u32,
    /// The depth at which `->` ends an expression instead of starting a lambda, set by
    /// [`Parser::without_lambdas`].
    no_lambda_depth: Option<u32>,
    /// Whether an error was reported since a token was last consumed normally. While it is, no
    /// more errors are reported, so each mistake gets one diagnostic.
    recovering: bool,
}

impl<'t> Parser<'t> {
    /// `tokens` are the lexer's tokens for `text`, ending with `Eof`.
    pub fn new(text: &'t str, tokens: &'t [Token], release: u32) -> Parser<'t> {
        let significant = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| !skipped(token.kind))
            .map(|(raw, &token)| {
                let kind = SyntaxKind::from_token_kind(token.kind);
                let contextual = if kind == SyntaxKind::Identifier {
                    ContextualKeyword::from_identifier(&cooked_text(text, token))
                        .and_then(SyntaxKind::from_contextual_keyword)
                } else {
                    None
                };
                Significant {
                    raw: raw as u32,
                    kind,
                    contextual,
                }
            })
            .collect();
        Parser {
            text,
            tokens,
            significant,
            pos: 0,
            events: Vec::new(),
            diagnostics: Vec::new(),
            release,
            depth: 0,
            no_lambda_depth: None,
            recovering: false,
        }
    }

    /// The events and the diagnostics, in the order they were reported.
    pub fn finish(self) -> (Vec<Event>, Vec<ParseDiagnostic>) {
        (self.events, self.diagnostics)
    }

    /// The release being parsed for, for the readings that depend on it
    /// (docs/design/parser-language-versions.md).
    #[allow(dead_code, reason = "for the grammar areas still to come")]
    pub fn release(&self) -> u32 {
        self.release
    }

    // ----- Lookahead -----

    fn significant(&self, n: usize) -> Significant {
        let last = self.significant.len() - 1;
        self.significant[(self.pos + n).min(last)]
    }

    /// The kind of the `n`th non-trivia token ahead (0 is the current one), as the lexer made it:
    /// a contextual keyword is still an `Identifier`. Past the end, `Eof`.
    pub fn nth(&self, n: usize) -> SyntaxKind {
        self.significant(n).kind
    }

    pub fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    /// Whether the `n`th token ahead is `kind`. For a contextual keyword's kind, whether it is
    /// an identifier spelling that keyword (for `NonSealedKw`, the tokens `non`, `-`, and `sealed`
    /// with nothing between them).
    pub fn nth_at(&self, n: usize, kind: SyntaxKind) -> bool {
        let token = self.significant(n);
        if token.kind == kind {
            return true;
        }
        if kind == SyntaxKind::NonSealedKw {
            return self.nth_at_non_sealed(n);
        }
        token.contextual == Some(kind)
    }

    pub fn at(&self, kind: SyntaxKind) -> bool {
        self.nth_at(0, kind)
    }

    /// Whether the `n`th token ahead is in `set`, counting contextual keywords as in
    /// [`Parser::nth_at`].
    pub fn nth_at_set(&self, n: usize, set: TokenSet) -> bool {
        let token = self.significant(n);
        set.contains(token.kind)
            || token.contextual.is_some_and(|kind| set.contains(kind))
            || set.contains(SyntaxKind::NonSealedKw) && self.nth_at_non_sealed(n)
    }

    pub fn at_set(&self, set: TokenSet) -> bool {
        self.nth_at_set(0, set)
    }

    pub fn at_eof(&self) -> bool {
        self.at(SyntaxKind::Eof)
    }

    /// Whether the `n`th token ahead directly follows the one before it, with no trivia between
    /// them, as the parts of `>>` and `non-sealed` must.
    pub fn nth_is_adjacent(&self, n: usize) -> bool {
        n > 0 && self.significant(n).raw == self.significant(n - 1).raw + 1
    }

    /// The span of the `n`th token ahead.
    pub fn nth_span(&self, n: usize) -> Span {
        self.tokens[self.significant(n).raw as usize].span
    }

    /// The text of the `n`th token ahead, with Unicode escapes translated.
    pub fn nth_text(&self, n: usize) -> std::borrow::Cow<'t, str> {
        cooked_text(self.text, self.tokens[self.significant(n).raw as usize])
    }

    fn nth_at_non_sealed(&self, n: usize) -> bool {
        self.nth(n) == SyntaxKind::Identifier
            && self.nth(n + 1) == SyntaxKind::Minus
            && self.nth(n + 2) == SyntaxKind::Identifier
            && self.nth_is_adjacent(n + 1)
            && self.nth_is_adjacent(n + 2)
            && self.nth_text(n) == "non"
            && self.nth_text(n + 2) == "sealed"
    }

    /// The end of the last token consumed, where "expected" errors are anchored.
    fn previous_end(&self) -> u32 {
        match self.pos.checked_sub(1) {
            Some(previous) => {
                self.tokens[self.significant[previous].raw as usize]
                    .span
                    .end
            }
            None => 0,
        }
    }

    // ----- Consuming tokens -----

    /// Consumes the current token with the lexer's kind. Does nothing at `Eof`, which only the
    /// compilation unit consumes.
    pub fn bump(&mut self) {
        let kind = self.current();
        self.bump_as(kind);
    }

    /// Consumes the current token, recording it as `kind`: a contextual keyword's kind for an
    /// identifier read as that keyword. For `NonSealedKw`, consumes `non`, `-`, and `sealed`.
    pub fn bump_as(&mut self, kind: SyntaxKind) {
        let count = if kind == SyntaxKind::NonSealedKw && self.nth_at_non_sealed(0) {
            3
        } else {
            1
        };
        for _ in 0..count {
            self.consume(kind);
        }
        self.recovering = false;
    }

    /// Consumes the current token if it is `kind` (see [`Parser::nth_at`]), recording that kind.
    pub fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump_as(kind);
            true
        } else {
            false
        }
    }

    /// Consumes the current token if it is `kind`, and otherwise reports it missing.
    pub fn expect(&mut self, kind: SyntaxKind) -> bool {
        if self.eat(kind) {
            true
        } else {
            self.error_expected(Expected::Token(kind));
            false
        }
    }

    /// Consumes `Eof`, as the last token of the compilation unit.
    pub fn bump_eof(&mut self) {
        if self.at_eof() && self.pos < self.significant.len() {
            self.events.push(Event::Token {
                kind: SyntaxKind::Eof,
            });
            self.pos += 1;
        }
    }

    /// Consumes the current token, without ending error recovery.
    fn consume(&mut self, kind: SyntaxKind) {
        if self.at_eof() {
            return;
        }
        self.events.push(Event::Token { kind });
        self.pos += 1;
    }

    // ----- Nodes -----

    /// Opens a node; its kind is given when the marker is completed.
    pub fn start(&mut self) -> Marker {
        let event = self.events.len() as u32;
        self.events.push(Event::Tombstone);
        Marker {
            event,
            start: self.pos,
            done: false,
        }
    }

    // ----- Errors and recovery -----

    /// Reports `kind`, unless an error was reported since a token was last consumed normally, and
    /// marks the innermost open node as containing an error either way.
    pub fn error(&mut self, kind: ParseDiagnosticKind, span: Span, anchor: u32) {
        self.events.push(Event::Error);
        if !self.recovering {
            self.diagnostics
                .push(ParseDiagnostic::error(kind, span, anchor));
            self.recovering = true;
        }
    }

    /// Reports that `what` is missing, after the previous token.
    pub fn error_expected(&mut self, what: Expected) {
        let end = self.previous_end();
        self.error(ParseDiagnosticKind::Expected(what), Span::empty(end), end);
    }

    /// Puts tokens in an `Error` node, reporting the first as unexpected, until a token in `stop`,
    /// an unmatched closing delimiter, or the end of the file. Delimited groups are skipped
    /// whole, so a stray `(` doesn't swallow the rest of the file. Returns the `Error` node, or
    /// `None`, consuming and reporting nothing, if the current token is already one of those.
    pub fn skip_until(&mut self, stop: TokenSet) -> Option<CompletedMarker> {
        if self.at_eof() || self.at_set(stop) || is_closing_delimiter(self.current()) {
            return None;
        }
        let m = self.start();
        self.report_unexpected();
        while !self.at_eof() && !self.at_set(stop) && !is_closing_delimiter(self.current()) {
            self.skip_token_or_group();
        }
        Some(m.complete(self, SyntaxKind::Error))
    }

    /// Puts the current token, or the delimited group it opens, in an `Error` node, reporting it
    /// as unexpected. A stray closing delimiter is consumed alone. Does nothing at `Eof`.
    pub fn error_and_bump(&mut self) {
        if self.at_eof() {
            return;
        }
        let m = self.start();
        self.report_unexpected();
        if is_closing_delimiter(self.current()) {
            self.consume(self.current());
        } else {
            self.skip_token_or_group();
        }
        m.complete(self, SyntaxKind::Error);
    }

    fn report_unexpected(&mut self) {
        let span = self.nth_span(0);
        self.error(
            ParseDiagnosticKind::UnexpectedToken(self.current()),
            span,
            span.start,
        );
    }

    /// Consumes the current token, or if it opens a delimited group, the whole group up to its
    /// closing delimiter. A group ends early at a closing delimiter that matches an enclosing
    /// group or none, which isn't consumed, or at the end of the file. Iterative, so a deep
    /// group costs no stack.
    fn skip_token_or_group(&mut self) {
        let Some(close) = closing_delimiter(self.current()) else {
            self.consume(self.current());
            return;
        };
        let mut open = vec![close];
        self.consume(self.current());
        while let Some(&expected) = open.last() {
            let kind = self.current();
            if kind == SyntaxKind::Eof {
                return;
            }
            if let Some(close) = closing_delimiter(kind) {
                open.push(close);
            } else if kind == expected {
                open.pop();
            } else if is_closing_delimiter(kind) {
                // Close the groups this one ends, or stop if it ends none of them.
                if !open.contains(&kind) {
                    return;
                }
                while open.last() != Some(&kind) {
                    open.pop();
                }
                open.pop();
            }
            self.consume(kind);
        }
    }

    /// Where `node` starts and ends, and where its direct tokens are (those not inside a child
    /// node), for diagnostics javac positions inside a node, such as at a binary operator.
    /// `node` must be the node completed last: nothing may have been consumed or completed since,
    /// though errors may have been reported. Walks the node's events backwards, so it costs time
    /// in proportion to the node's size.
    #[allow(dead_code, reason = "for the grammar areas still to come")]
    pub fn last_node_anchors(&self, node: CompletedMarker) -> NodeAnchors {
        let mut anchors = NodeAnchors {
            start: self.tokens[self.significant(0).raw as usize].span,
            end: self.previous_end(),
            first_direct_token: None,
            after_last_direct_token: None,
        };
        if let Some(start) = self.significant.get(node.start) {
            anchors.start = self.tokens[start.raw as usize].span;
        }
        if !matches!(
            self.events.get(node.event as usize),
            Some(Event::Start { .. })
        ) {
            return anchors;
        }
        // Inside the walk, `depth` is 1 between the node's `Start` and `Finish`, and more inside
        // its children. Each `Token` event is the token before `token`.
        let mut depth = 0u32;
        let mut token = self.pos;
        let mut last_direct = None;
        for event in self.events[node.event as usize + 1..].iter().rev() {
            match event {
                Event::Finish => depth += 1,
                Event::Start { .. } => depth = depth.saturating_sub(1),
                Event::Token { .. } => {
                    token = token.saturating_sub(1);
                    debug_assert!(depth > 0, "a token after the last node completed");
                    if depth == 1 {
                        anchors.first_direct_token = Some(self.span_of(token));
                        last_direct.get_or_insert(token);
                    }
                }
                Event::Error | Event::Tombstone => {}
            }
        }
        if let Some(last) = last_direct
            && last + 1 < self.significant.len()
        {
            anchors.after_last_direct_token = Some(self.span_of(last + 1));
        }
        anchors
    }

    /// The span of the token at index `index` among the non-trivia tokens.
    fn span_of(&self, index: usize) -> Span {
        let last = self.significant.len() - 1;
        self.tokens[self.significant[index.min(last)].raw as usize].span
    }

    // ----- Loops and nesting -----

    /// A progress guard for a loop: see [`Progress::advanced`].
    pub fn progress(&self) -> Progress {
        Progress { pos: self.pos }
    }

    /// Runs `f`, a recursive part of the grammar, one level deeper. Grows the stack if it runs
    /// low. At [`NESTING_LIMIT`] levels, reports `NestingTooDeep` instead, puts the current token
    /// (and the group it opens, or the rest of the construct up to a `;`, `,`, or closing
    /// delimiter) in an `Error` node, and returns `None`.
    pub fn nested<R>(&mut self, f: impl FnOnce(&mut Parser<'t>) -> Option<R>) -> Option<R> {
        if self.depth >= NESTING_LIMIT {
            self.skip_too_deep();
            return None;
        }
        self.depth += 1;
        let result = ferrion_stack::ensure_stack(|| f(self));
        self.depth -= 1;
        result
    }

    /// Runs `f` one level deeper, as [`Parser::nested`] does, at a level where `->` can't start
    /// a lambda: in a case label, where it ends the label. Expressions nested deeper, such as in
    /// parentheses or arguments, can have lambdas again.
    #[allow(dead_code, reason = "for case labels, in the expression grammar")]
    pub fn without_lambdas<R>(
        &mut self,
        f: impl FnOnce(&mut Parser<'t>) -> Option<R>,
    ) -> Option<R> {
        self.nested(|p| {
            let saved = p.no_lambda_depth.replace(p.depth);
            let result = f(p);
            p.no_lambda_depth = saved;
            result
        })
    }

    /// Whether a lambda can start here: false directly inside [`Parser::without_lambdas`].
    #[allow(dead_code, reason = "for case labels, in the expression grammar")]
    pub fn lambdas_allowed(&self) -> bool {
        self.no_lambda_depth != Some(self.depth)
    }

    fn skip_too_deep(&mut self) {
        let span = self.nth_span(0);
        self.error(ParseDiagnosticKind::NestingTooDeep, span, span.start);
        if self.at_eof() {
            return;
        }
        let m = self.start();
        if closing_delimiter(self.current()).is_some() {
            self.skip_token_or_group();
        } else {
            let stop = TokenSet::new(&[SyntaxKind::Semi, SyntaxKind::Comma]);
            while !self.at_eof() && !self.at_set(stop) && !is_closing_delimiter(self.current()) {
                self.skip_token_or_group();
            }
        }
        if self.pos > m.start {
            m.complete(self, SyntaxKind::Error);
        } else {
            m.abandon(self);
        }
    }
}

/// Whether the parser skips a token of this lexer kind, and the builder places it like trivia,
/// in whichever node encloses it: trivia, and characters that can't start a token, which the
/// lexer has reported. javac's scanner skips illegal characters the same way, so `import a.#b;`
/// gets one error, for the `#`.
pub(crate) fn skipped(kind: ferrion_lexer::TokenKind) -> bool {
    kind.is_trivia() || kind == ferrion_lexer::TokenKind::Error
}

/// For an opening delimiter, its closing one.
fn closing_delimiter(kind: SyntaxKind) -> Option<SyntaxKind> {
    match kind {
        SyntaxKind::LParen => Some(SyntaxKind::RParen),
        SyntaxKind::LBracket => Some(SyntaxKind::RBracket),
        SyntaxKind::LBrace => Some(SyntaxKind::RBrace),
        _ => None,
    }
}

fn is_closing_delimiter(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::RParen | SyntaxKind::RBracket | SyntaxKind::RBrace
    )
}

/// An open node. Complete it with its kind, or abandon it; dropping it otherwise is a bug, which
/// a debug assertion catches.
#[must_use]
pub(crate) struct Marker {
    /// The index of its `Start` event.
    event: u32,
    /// The current token when it was started.
    start: usize,
    done: bool,
}

impl Marker {
    /// Whether a token was consumed since the marker was started.
    pub fn consumed(&self, p: &Parser<'_>) -> bool {
        p.pos > self.start
    }

    /// Closes the node as a `kind`. The node must have consumed a token: an empty node is a bug,
    /// which a debug assertion catches; in release builds the marker is abandoned.
    pub fn complete(mut self, p: &mut Parser<'_>, kind: SyntaxKind) -> CompletedMarker {
        self.done = true;
        debug_assert!(
            p.pos > self.start,
            "an empty {kind:?}: complete markers only after consuming a token"
        );
        if p.pos > self.start {
            p.events[self.event as usize] = Event::Start {
                kind,
                forward_parent: 0,
            };
            if kind == SyntaxKind::Error {
                p.events.push(Event::Error);
            }
            p.events.push(Event::Finish);
        }
        CompletedMarker {
            event: self.event,
            start: self.start,
            kind,
        }
    }

    /// Drops the node; whatever it consumed goes to its parent.
    pub fn abandon(mut self, p: &mut Parser<'_>) {
        self.done = true;
        if self.event as usize == p.events.len() - 1 {
            p.events.pop();
        }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        debug_assert!(
            self.done || std::thread::panicking(),
            "a marker was neither completed nor abandoned"
        );
    }
}

/// A completed node.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompletedMarker {
    event: u32,
    start: usize,
    kind: SyntaxKind,
}

impl CompletedMarker {
    #[allow(dead_code, reason = "for the grammar areas still to come")]
    pub fn kind(self) -> SyntaxKind {
        self.kind
    }

    /// Opens a node that starts where this one does and will contain it, for left-nested
    /// constructs: in `a.b`, `a` is parsed first, then preceded by the node for `a.b`.
    pub fn precede(self, p: &mut Parser<'_>) -> Marker {
        let event = p.events.len() as u32;
        p.events.push(Event::Tombstone);
        if let Event::Start { forward_parent, .. } = &mut p.events[self.event as usize] {
            *forward_parent = event - self.event;
        }
        Marker {
            event,
            start: self.start,
            done: false,
        }
    }

    /// Changes the node's kind, for a construct recognized only after it was parsed.
    #[allow(dead_code, reason = "for the grammar areas still to come")]
    pub fn change_kind(&mut self, p: &mut Parser<'_>, kind: SyntaxKind) {
        if let Event::Start { kind: old, .. } = &mut p.events[self.event as usize] {
            *old = kind;
        }
        self.kind = kind;
    }
}

/// See [`Parser::last_node_anchors`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeAnchors {
    /// The node's first token.
    pub start: Span,
    /// The end of the node's last token.
    pub end: u32,
    /// The node's first direct token: in `a + b`, the `+`.
    pub first_direct_token: Option<Span>,
    /// The token after the node's last direct token: in `a.m(1)`, the `(` after `m`.
    pub after_last_direct_token: Option<Span>,
}

/// See [`Parser::progress`].
pub(crate) struct Progress {
    pos: usize,
}

impl Progress {
    /// Call once per loop iteration, after the iteration's work. Returns `false` if the loop
    /// must stop. If the iteration consumed nothing, that is a bug in the grammar: a debug
    /// assertion catches it, and in release builds the current token goes in an `Error` node (or
    /// at the end of the file, the loop stops) so the parser can't hang.
    pub fn advanced(&mut self, p: &mut Parser<'_>) -> bool {
        let advanced = p.pos > self.pos;
        debug_assert!(
            advanced,
            "a parser loop made no progress at {:?}",
            p.current()
        );
        if !advanced {
            if p.at_eof() {
                return false;
            }
            p.error_and_bump();
        }
        self.pos = p.pos;
        true
    }
}
