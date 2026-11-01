//! Unicode escapes (JLS §3.3).
//!
//! A `\` as written is eligible to begin a Unicode escape unless it is the second of a pair of
//! backslashes. Backslashes pair up from the left in a run of them, so `\\u0041` is a backslash,
//! a backslash, and `u0041`, but `\\\u0041` is `\\` and `A`. An eligible `\` followed by
//! one or more `u`s begins an escape, which must go on to four hex digits. A backslash produced by
//! an escape (`\u005c`) never begins another escape, but it pairs like any other, except that a
//! `\` right after it is always eligible: `\u005c\u0041` is a backslash and `A`, and
//! `\u005c\\u0041` is a backslash, a backslash, and `A`. That is how javac reads them
//! (observed with the oracle; the JLS counts only backslashes as written, which would make the last
//! one `\`, `\`, and `u0041`).
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
    /// Whether the character just before `pos` is a backslash that doesn't pair with one before
    /// it, so that a `\` written at `pos` would be the second of a pair.
    unpaired: bool,
    /// Whether the character just before `pos` came from an escape.
    translated: bool,
}

impl<'a> Escapes<'a> {
    /// Escapes in `bytes[start..end]`. Backslashes before `start` count toward the pairing rule,
    /// but escapes that begin before `start` aren't found.
    pub(crate) fn new(bytes: &'a [u8], start: usize, end: usize) -> Escapes<'a> {
        // Read the run of backslashes before `start`, if any, for its pairing.
        let mut escapes = Escapes {
            bytes,
            pos: backslash_run_start(bytes, start),
            end: start,
            unpaired: false,
            translated: false,
        };
        while escapes.next().is_some() {}
        escapes.pos = start;
        escapes.end = end;
        escapes
    }
}

/// Where the run of backslashes, as written or as escapes (`\u005c`), that ends at `pos` starts.
/// The character before it isn't a backslash, so pairing starts afresh there. Linear in the run's
/// length.
fn backslash_run_start(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        if pos > 0 && bytes[pos - 1] == b'\\' {
            pos -= 1;
            continue;
        }
        if pos >= 6 && bytes[pos - 4..pos].eq_ignore_ascii_case(b"005c") {
            let digits = pos - 4;
            let us = bytes[..digits]
                .iter()
                .rev()
                .take_while(|&&b| b == b'u')
                .count();
            if us > 0 && digits > us && bytes[digits - us - 1] == b'\\' {
                pos = digits - us - 1;
                continue;
            }
        }
        return pos;
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
                self.unpaired = false;
                self.translated = false;
                self.pos += offset;
            }
            let start = self.pos;
            let eligible = !self.unpaired || self.translated;
            if !eligible || self.bytes.get(start + 1) != Some(&b'u') {
                self.unpaired = !self.unpaired;
                self.translated = false;
                self.pos += 1;
                continue;
            }
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
            // A backslash from an escape pairs with an unpaired one before it, and the next `\`
            // after any escape is eligible.
            self.unpaired = ch == Some('\\') && !self.unpaired;
            self.translated = true;
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
        // The backslash an escape produces doesn't start another escape, and a backslash right
        // after it is eligible.
        assert_eq!(escapes(r"\u005cu0041"), [(0, 6, Some('\\'))]);
        assert_eq!(
            escapes(r"\u005c\u005c"),
            [(0, 6, Some('\\')), (6, 12, Some('\\'))]
        );
        assert_eq!(
            escapes(r"\u005c\u0041"),
            [(0, 6, Some('\\')), (6, 12, Some('A'))]
        );
    }

    #[test]
    fn escaped_backslashes_pair_with_written_ones() {
        // The escape's backslash and the one after it pair, which leaves the next eligible.
        assert_eq!(
            escapes(r"\u005c\\u0041"),
            [(0, 6, Some('\\')), (7, 13, Some('A'))]
        );
        assert_eq!(escapes(r"\u005c\\\u0041"), [(0, 6, Some('\\'))]);
        assert_eq!(
            escapes(r"\u005c\\u005c\]"),
            [(0, 6, Some('\\')), (7, 13, Some('\\'))]
        );
        assert_eq!(
            escapes(r"\\\u005c\u0041"),
            [(2, 8, Some('\\')), (8, 14, Some('A'))]
        );
        assert_eq!(
            escapes(r"\u005c\u005c\u005cu0041"),
            [
                (0, 6, Some('\\')),
                (6, 12, Some('\\')),
                (12, 18, Some('\\'))
            ]
        );
    }

    #[test]
    fn pairing_before_the_range_counts() {
        // From the written backslash: the escape before it pairs with it.
        let text = r"\u005c\\u0041";
        let found: Vec<_> = Escapes::new(text.as_bytes(), 6, text.len())
            .map(|e| e.start)
            .collect();
        assert_eq!(found, [7]);
        let text = r"\u005c\\\u0041";
        let found: Vec<_> = Escapes::new(text.as_bytes(), 8, text.len())
            .map(|e| e.start)
            .collect();
        assert_eq!(found, [] as [u32; 0]);
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
