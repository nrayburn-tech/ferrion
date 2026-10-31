//! A comment's content: the text the parsers read, and where each part of it is written.
//!
//! The content of a comment is its text with the comment syntax taken out and Unicode escapes
//! translated. For `/** */` comments, that is the text between the delimiters, with each line's
//! leading whitespace and asterisks taken out; for `///` comments, the text after each line's
//! `///` and common indentation. Each line break in between is a single `\n`, however it is
//! written, and stands for the line terminator together with the next line's leading syntax.
//!
//! The parsers work on offsets into the content and map them back to [`Span`]s in the source text
//! with [`Content::span`], so every span covers what is written, escapes and all.

use std::ops::Range;

use crate::Span;
use crate::escape::Char;

#[derive(Debug, Default)]
pub(crate) struct Content {
    pub text: String,
    /// Pieces in content order, covering the content without gaps.
    pieces: Vec<Piece>,
    /// Where empty content is in the source.
    raw_start: u32,
}

/// A run of content and where it is written. An identity piece is written as it reads, byte for
/// byte; any other piece is a single character written some other way (a Unicode escape, or a
/// line break with the next line's leading syntax), whose inside has no position of its own.
#[derive(Clone, Copy, Debug)]
struct Piece {
    start: u32,
    raw_start: u32,
    raw_end: u32,
    identity: bool,
}

impl Content {
    pub fn new(raw_start: u32) -> Content {
        Content {
            raw_start,
            ..Content::default()
        }
    }

    /// Appends a character of the comment.
    pub fn push_char(&mut self, c: Char) {
        if c.escaped {
            self.push_mapped(c.ch, c.start..c.end);
            return;
        }
        let start = self.text.len() as u32;
        self.text.push(c.ch);
        if let Some(last) = self.pieces.last_mut()
            && last.identity
            && last.raw_end == c.start
        {
            last.raw_end = c.end;
            return;
        }
        self.pieces.push(Piece {
            start,
            raw_start: c.start,
            raw_end: c.end,
            identity: true,
        });
    }

    /// Appends one character written as `raw`, such as a line break written as a line terminator
    /// followed by the next line's leading `*`.
    pub fn push_mapped(&mut self, ch: char, raw: Range<u32>) {
        self.pieces.push(Piece {
            start: self.text.len() as u32,
            raw_start: raw.start,
            raw_end: raw.end,
            identity: false,
        });
        self.text.push(ch);
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Where the content in `range` is written. Ends that fall inside a character are moved out
    /// to its edges, so a span never splits a character or an escape.
    pub fn span(&self, range: Range<usize>) -> Span {
        let len = self.text.len();
        let mut start = range.start.min(len);
        let mut end = range.end.clamp(start, len);
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }
        while !self.text.is_char_boundary(end) {
            end += 1;
        }
        if start == end || range.start >= range.end {
            let at = self.raw_start_of(start);
            return Span::new(at, at);
        }
        Span::new(self.raw_start_of(start), self.raw_end_of(end))
    }

    /// The source offset where the content at `offset` starts.
    fn raw_start_of(&self, offset: usize) -> u32 {
        let offset = offset as u32;
        let i = self.pieces.partition_point(|p| p.start <= offset);
        let Some(p) = i.checked_sub(1).map(|i| self.pieces[i]) else {
            return self.raw_start;
        };
        if offset as usize >= self.text.len() {
            p.raw_end
        } else if p.identity {
            (p.raw_start + (offset - p.start)).min(p.raw_end)
        } else {
            p.raw_start
        }
    }

    /// The source offset where the content just before `offset` ends.
    fn raw_end_of(&self, offset: usize) -> u32 {
        let Some(before) = (offset as u32).checked_sub(1) else {
            return self.raw_start_of(0);
        };
        let i = self.pieces.partition_point(|p| p.start <= before);
        match i.checked_sub(1).map(|i| self.pieces[i]) {
            Some(p) if p.identity => (p.raw_start + (before + 1 - p.start)).min(p.raw_end),
            Some(p) => p.raw_end,
            None => self.raw_start,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escape::chars;

    #[test]
    fn maps_plain_text_escapes_and_line_breaks() {
        // "aBc", a line break written as "\n * ", then "d".
        let source = "/**a\\u0042c\n * d*/";
        let mut content = Content::new(3);
        for c in chars(source, 3, 11) {
            content.push_char(c);
        }
        content.push_mapped('\n', 11..15);
        for c in chars(source, 15, 16) {
            content.push_char(c);
        }
        assert_eq!(content.text, "aBc\nd");
        let raw = |range: Range<usize>| &source[content.span(range).range()];
        assert_eq!(raw(0..1), "a");
        assert_eq!(raw(1..2), "\\u0042");
        assert_eq!(raw(0..3), "a\\u0042c");
        assert_eq!(raw(2..3), "c");
        assert_eq!(raw(3..4), "\n * ");
        assert_eq!(raw(2..5), "c\n * d");
        assert_eq!(raw(4..5), "d");
        assert_eq!(content.span(5..5), Span::new(16, 16));
        assert_eq!(content.span(1..1), Span::new(4, 4));
        // Out-of-range offsets are clamped.
        assert_eq!(content.span(4..99), Span::new(15, 16));
    }

    #[test]
    fn empty_content() {
        let content = Content::new(7);
        assert_eq!(content.span(0..0), Span::new(7, 7));
        assert_eq!(content.span(0..3), Span::new(7, 7));
    }

    #[test]
    fn spans_never_split_a_character() {
        let source = "é";
        let mut content = Content::new(0);
        for c in chars(source, 0, 2) {
            content.push_char(c);
        }
        assert_eq!(content.span(1..1), Span::new(0, 0));
        assert_eq!(content.span(1..2), Span::new(0, 2));
    }
}
