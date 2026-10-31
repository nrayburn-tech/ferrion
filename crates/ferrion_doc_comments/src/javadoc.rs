//! Traditional documentation comments, `/** ... */`.

use crate::content::Content;
use crate::escape::{self, Char};
use crate::inline::{Parser, is_blank};
use crate::model::{BlockTag, Diagnostic, Inline};
use crate::{Span, clamp_span};

/// A parsed traditional documentation comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Javadoc {
    /// The comment, `/**` to `*/`.
    pub span: Span,
    /// The main description: the content before the first block tag, without leading and
    /// trailing whitespace. Empty when the comment has none.
    pub description: Vec<Inline>,
    /// Block tags in source order.
    pub tags: Vec<BlockTag<Vec<Inline>>>,
    /// Problems found, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses the traditional documentation comment at `span` in `text`.
///
/// `span` is a [`DocComment`](ferrion_lexer::TokenKind::DocComment) token's span, so it starts
/// with `/**` and, unless the comment is unterminated, ends with `*/`, each possibly written with
/// Unicode escapes. Other spans are read as well as they can be: without a `/**`, the content starts
/// at the start of the span, and without a `*/`, it runs to the end. A span that is out of bounds
/// or splits a character is shrunk to fit.
pub fn parse_javadoc(text: &str, span: Span) -> Javadoc {
    let (start, end) = clamp_span(text, span);
    let content = content(text, start, end);
    let mut parser = Parser::new(text, &content, false);
    // A block tag may start the comment, as in `/** @return x */`.
    let description = if parser.block_tag_at(0) {
        Vec::new()
    } else {
        parser.section(true)
    };
    let mut tags = Vec::new();
    while parser.pos < parser.end {
        // Only a block tag ends a section early, so the parser is at the start of one.
        let head = parser.block_tag_head();
        let body = parser.section(true);
        let head_span = content.span(head.start..head.end);
        let end = body
            .last()
            .map_or(head_span.end, |node| node.span().end.max(head_span.end));
        tags.push(BlockTag {
            span: Span::new(head_span.start, end),
            name: head.name,
            kind: head.kind,
            argument: head.argument,
            body,
        });
    }
    let mut diagnostics = parser.diagnostics;
    diagnostics.sort_by_key(|d| (d.span.start, d.span.end));
    Javadoc {
        span: Span::new(start as u32, end as u32),
        description,
        tags,
        diagnostics,
    }
}

/// The content of the comment in `text[start..end]`: the text between `/**` and `*/`, with each
/// line's leading whitespace and asterisks taken out.
///
/// As javadoc 27 does: asterisks right after `/**` are taken out (`/*** x` reads as ` x`); on later
/// lines, whitespace followed by asterisks is taken out, but whitespace not followed by an
/// asterisk stays, as does whitespace after the asterisks.
fn content(text: &str, start: usize, end: usize) -> Content {
    let chars: Vec<Char> = escape::chars(text, start, end).collect();
    let is = |i: usize, c: char| chars.get(i).is_some_and(|ch| ch.ch == c);
    let mut from = 0;
    if is(0, '/') && is(1, '*') && is(2, '*') {
        from = 3;
    }
    let mut to = chars.len();
    if to >= from + 2 && is(to - 2, '*') && is(to - 1, '/') {
        to -= 2;
    } else if from == 3 && to == 4 && is(3, '/') {
        // `/**/`, which the lexer makes a block comment.
        to = 3;
    }
    while from < to && is(from, '*') {
        from += 1;
    }
    let raw_start = chars.get(from).map_or(end as u32, |c| c.start);
    let mut content = Content::new(raw_start);
    let mut i = from;
    while i < to {
        let c = chars[i];
        if c.ch != '\n' && c.ch != '\r' {
            content.push_char(c);
            i += 1;
            continue;
        }
        i += 1;
        if c.ch == '\r' && i < to && chars[i].ch == '\n' {
            i += 1;
        }
        let mut decoration = i;
        while decoration < to && is_blank(chars[decoration].ch) {
            decoration += 1;
        }
        if decoration < to && chars[decoration].ch == '*' {
            while decoration < to && chars[decoration].ch == '*' {
                decoration += 1;
            }
            i = decoration;
        }
        let raw_end = chars[i - 1].end;
        content.push_mapped('\n', c.start..raw_end);
    }
    content
}

#[cfg(test)]
mod tests;
