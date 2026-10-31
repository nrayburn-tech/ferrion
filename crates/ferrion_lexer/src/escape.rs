//! Unicode escapes (JLS §3.3).
//!
//! A `\` is eligible to begin a Unicode escape when it is preceded by an even number of contiguous
//! backslashes as written, so `\\u0041` is a backslash, a backslash, and `u0041`. An eligible `\`
//! followed by one or more `u`s begins an escape, which must go on to four hex digits. A backslash
//! produced by an escape (`\u005c`) never begins another escape.
//!
//! Escapes are found in a single pass over the raw text, before tokens are formed, so the lexer
//! sees the translated characters while spans keep covering the escapes as written.

use std::borrow::Cow;

use crate::{Span, ident};

/// A Unicode escape: `\`, one or more `u`s, and four hex digits, or, when invalid, as much of that
/// as is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Escape {
    /// The offset of the `\`.
    pub start: u32,
    pub end: u32,
    /// The character the escape stands for, or `None` if it's invalid (`\u` not followed by four
    /// hex digits). javac drops an invalid escape after reporting it, and so does the lexer.
    ///
    /// Two escapes that form a UTF-16 surrogate pair (`\uD83D\uDE00`) are one `Escape` whose
    /// character is the supplementary character. A surrogate that isn't part of a pair, which only
    /// a character or string literal can usefully contain, is U+FFFD here; it behaves the same way
    /// in every lexical context, since neither can start or continue an identifier.
    pub ch: Option<char>,
}

impl Escape {
    pub fn is_valid(&self) -> bool {
        self.ch.is_some()
    }
}

/// Iterates over the Unicode escapes in a range of a text, in order. Made by
/// [`unicode_escapes`](crate::unicode_escapes).
pub struct Escapes<'a> {
    bytes: &'a [u8],
    pos: usize,
    end: usize,
    /// The number of contiguous raw backslashes just before `pos`.
    backslashes: usize,
}

impl<'a> Escapes<'a> {
    /// Escapes in `bytes[start..end]`. Backslashes before `start` count toward the even-backslash
    /// rule, but escapes that begin before `start` aren't found.
    pub(crate) fn new(bytes: &'a [u8], start: usize, end: usize) -> Escapes<'a> {
        let backslashes = bytes[..start]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count();
        Escapes {
            bytes,
            pos: start,
            end,
            backslashes,
        }
    }
}

impl Iterator for Escapes<'_> {
    type Item = Escape;

    fn next(&mut self) -> Option<Escape> {
        while self.pos < self.end {
            let Some(offset) = self.bytes[self.pos..self.end]
                .iter()
                .position(|&b| b == b'\\')
            else {
                self.pos = self.end;
                return None;
            };
            if offset > 0 {
                self.backslashes = 0;
                self.pos += offset;
            }
            let start = self.pos;
            if self.backslashes % 2 == 1 || self.bytes.get(start + 1) != Some(&b'u') {
                self.backslashes += 1;
                self.pos += 1;
                continue;
            }
            // An escape always ends with a `u` or a hex digit, so the next backslash is eligible.
            self.backslashes = 0;
            let (end, unit) = scan(self.bytes, start, self.end);
            self.pos = end;
            let ch = match unit {
                None => None,
                Some(high @ 0xD800..=0xDBFF) => {
                    // A high surrogate pairs with a low surrogate escape right after it.
                    let low = (self.bytes.get(end) == Some(&b'\\')
                        && self.bytes.get(end + 1) == Some(&b'u'))
                    .then(|| scan(self.bytes, end, self.end))
                    .and_then(|(low_end, low)| match low {
                        Some(low @ 0xDC00..=0xDFFF) => Some((low_end, low)),
                        _ => None,
                    });
                    match low {
                        Some((low_end, low)) => {
                            self.pos = low_end;
                            let c = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                            char::from_u32(c)
                        }
                        None => Some(char::REPLACEMENT_CHARACTER),
                    }
                }
                Some(unit) => Some(char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER)),
            };
            return Some(Escape {
                start: start as u32,
                end: self.pos as u32,
                ch,
            });
        }
        None
    }
}

/// Scans the escape whose `\u` is at `start`, not reading past `end`: returns where it ends and
/// the UTF-16 code unit it stands for, or `None` if fewer than four hex digits follow the `u`s.
fn scan(bytes: &[u8], start: usize, end: usize) -> (usize, Option<u32>) {
    let mut pos = start + 1;
    while pos < end && bytes[pos] == b'u' {
        pos += 1;
    }
    let mut unit = 0;
    for _ in 0..4 {
        match bytes[..end]
            .get(pos)
            .and_then(|&b| (b as char).to_digit(16))
        {
            Some(digit) => {
                unit = unit * 16 + digit;
                pos += 1;
            }
            None => return (pos, None),
        }
    }
    (pos, Some(unit))
}

/// The text of `span` with Unicode escapes translated, and invalid ones dropped. With
/// `strip_ignorable`, identifier-ignorable characters are removed too.
pub(crate) fn cook(text: &str, span: Span, strip_ignorable: bool) -> Cow<'_, str> {
    let (start, end) = (span.start as usize, span.end as usize);
    let raw = &text[start..end];
    let may_have_escapes = raw.as_bytes().windows(2).any(|w| w == b"\\u");
    if !may_have_escapes && !(strip_ignorable && raw.chars().any(ident::is_ignorable)) {
        return Cow::Borrowed(raw);
    }
    let mut cooked = String::with_capacity(raw.len());
    let mut copied = start;
    if may_have_escapes {
        for escape in Escapes::new(text.as_bytes(), start, end) {
            cooked.push_str(&text[copied..escape.start as usize]);
            cooked.extend(escape.ch);
            copied = escape.end as usize;
        }
    }
    cooked.push_str(&text[copied..end]);
    if strip_ignorable {
        cooked.retain(|c| !ident::is_ignorable(c));
    }
    if cooked == raw {
        Cow::Borrowed(raw)
    } else {
        Cow::Owned(cooked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escapes(text: &str) -> Vec<(u32, u32, Option<char>)> {
        Escapes::new(text.as_bytes(), 0, text.len())
            .map(|e| (e.start, e.end, e.ch))
            .collect()
    }

    #[test]
    fn simple_and_multiple_u() {
        assert_eq!(escapes(r"a\u0041b"), [(1, 7, Some('A'))]);
        assert_eq!(escapes(r"\uuuu0041"), [(0, 9, Some('A'))]);
        assert_eq!(
            escapes(r"\u00e9\u00E9"),
            [(0, 6, Some('é')), (6, 12, Some('é'))]
        );
    }

    #[test]
    fn even_backslash_rule() {
        assert_eq!(escapes(r"\\u0041"), []);
        assert_eq!(escapes(r"\\\u0041"), [(2, 8, Some('A'))]);
        assert_eq!(escapes(r"\\\\u0041"), []);
        // The backslash an escape produces doesn't start another escape, and doesn't count as a
        // backslash before the next one.
        assert_eq!(escapes(r"\u005cu0041"), [(0, 6, Some('\\'))]);
        assert_eq!(
            escapes(r"\u005c\u005c"),
            [(0, 6, Some('\\')), (6, 12, Some('\\'))]
        );
    }

    #[test]
    fn backslashes_before_the_range_count() {
        let text = r"\\u0041 \u0041";
        let found: Vec<_> = Escapes::new(text.as_bytes(), 1, text.len())
            .map(|e| e.start)
            .collect();
        assert_eq!(found, [8]);
    }

    #[test]
    fn invalid_escapes() {
        assert_eq!(escapes(r"\u12;"), [(0, 4, None)]);
        assert_eq!(escapes(r"\uu00g1"), [(0, 5, None)]);
        assert_eq!(escapes(r"\u"), [(0, 2, None)]);
        assert_eq!(escapes(r"\uuu"), [(0, 4, None)]);
        assert_eq!(escapes(r"\u\u0031"), [(0, 2, None), (2, 8, Some('1'))]);
        assert_eq!(escapes(r"\x \"), []);
    }

    #[test]
    fn surrogates() {
        assert_eq!(escapes(r"\uD83D\uDE00"), [(0, 12, Some('😀'))]);
        assert_eq!(
            escapes(r"\uD83D\uuDE00x"),
            [(0, 13, Some('😀'))],
            "multiple u's in the low half"
        );
        assert_eq!(
            escapes(r"\uD800x"),
            [(0, 6, Some(char::REPLACEMENT_CHARACTER))]
        );
        assert_eq!(
            escapes(r"\uDE00\uD83D"),
            [
                (0, 6, Some(char::REPLACEMENT_CHARACTER)),
                (6, 12, Some(char::REPLACEMENT_CHARACTER))
            ]
        );
        assert_eq!(
            escapes(r"\uD83D\u0041"),
            [
                (0, 6, Some(char::REPLACEMENT_CHARACTER)),
                (6, 12, Some('A'))
            ]
        );
    }

    #[test]
    fn long_backslash_runs_are_linear() {
        let text = "\\".repeat(100_001) + "u0041";
        assert_eq!(escapes(&text), [(100_000, 100_006, Some('A'))]);
        let text = "\\".repeat(100_000) + "u0041";
        assert_eq!(escapes(&text), []);
    }
}
