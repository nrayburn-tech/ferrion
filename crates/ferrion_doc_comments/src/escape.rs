//! Unicode escapes (JLS §3.3) in comment text.
//!
//! javac translates Unicode escapes before it reads a comment, so javadoc sees `\u007d` as a `}`
//! that can close an inline tag. The parsers do the same: they read a comment's characters through
//! [`chars`], which translates escapes and says where each character is written.
//!
//! Escapes are found by the lexer's [`unicode_escapes`], so the rules are the lexer's. One thing
//! differs: the lexer drops a malformed escape (`\u` not followed by four hex digits), but here it
//! stays as written, so no text of a comment is lost. The lexer has already reported it.

use std::iter::Peekable;

use ferrion_lexer::{Escapes, Span, unicode_escapes};

/// A character of a comment, and where it is written: its own bytes, or the escape for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Char {
    pub ch: char,
    pub start: u32,
    pub end: u32,
    /// Whether the character is written as a Unicode escape.
    pub escaped: bool,
}

/// The characters of `text[start..end]`, with Unicode escapes translated. Backslashes before
/// `start` count toward the even-backslash rule. `start` and `end` must be character boundaries.
pub(crate) fn chars(text: &str, start: usize, end: usize) -> Chars<'_> {
    Chars {
        text,
        pos: start,
        end,
        escapes: unicode_escapes(text, Span::new(start as u32, end as u32)).peekable(),
    }
}

pub(crate) struct Chars<'a> {
    text: &'a str,
    pos: usize,
    end: usize,
    escapes: Peekable<Escapes<'a>>,
}

impl Iterator for Chars<'_> {
    type Item = Char;

    fn next(&mut self) -> Option<Char> {
        let start = self.pos;
        let ch = self.text[start..self.end].chars().next()?;
        // A malformed escape is read as the characters it's written with.
        if let Some(escape) = self.escapes.next_if(|e| e.start as usize == start)
            && let Some(ch) = escape.ch
        {
            self.pos = escape.end as usize;
            return Some(Char {
                ch,
                start: start as u32,
                end: self.pos as u32,
                escaped: true,
            });
        }
        self.pos += ch.len_utf8();
        Some(Char {
            ch,
            start: start as u32,
            end: self.pos as u32,
            escaped: false,
        })
    }
}

/// `text[start..end]` with Unicode escapes translated.
pub(crate) fn cook(text: &str, start: usize, end: usize) -> String {
    chars(text, start, end).map(|c| c.ch).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(text: &str) -> Vec<(char, u32, u32, bool)> {
        chars(text, 0, text.len())
            .map(|c| (c.ch, c.start, c.end, c.escaped))
            .collect()
    }

    #[test]
    fn plain_and_escaped() {
        assert_eq!(all("é\\u0041"), [('é', 0, 2, false), ('A', 2, 8, true)]);
        assert_eq!(all("\\uuu007d"), [('}', 0, 8, true)]);
    }

    #[test]
    fn even_backslash_rule() {
        assert_eq!(cook("\\\\u0041", 0, 7), "\\\\u0041");
        assert_eq!(cook("\\\\\\u0041", 0, 8), "\\\\A");
        // The backslash an escape produces doesn't begin another escape.
        assert_eq!(cook("\\u005cu0041", 0, 11), "\\u0041");
        // Backslashes before the range count.
        assert_eq!(cook("\\\\u0041", 1, 7), "\\u0041");
    }

    #[test]
    fn malformed_escapes_stay_as_written() {
        assert_eq!(cook("\\u12;", 0, 5), "\\u12;");
        assert_eq!(cook("\\u", 0, 2), "\\u");
        // An escape isn't read past the end of the range.
        assert_eq!(cook("\\u0041", 0, 5), "\\u004");
    }

    #[test]
    fn surrogates() {
        assert_eq!(all("\\uD83D\\uDE00"), [('😀', 0, 12, true)]);
        assert_eq!(cook("\\uD83Dx", 0, 7), "\u{FFFD}x");
        assert_eq!(cook("\\uDE00\\uD83D", 0, 12), "\u{FFFD}\u{FFFD}");
    }
}
