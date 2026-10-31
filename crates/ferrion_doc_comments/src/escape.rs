//! Unicode escapes (JLS §3.3) in comment text.
//!
//! javac translates Unicode escapes before it reads a comment, so javadoc sees `\u007d` as a `}`
//! that can close an inline tag. The parsers do the same: they read a comment's characters through
//! [`chars`], which translates escapes and says where each character is written.
//!
//! The rules match the lexer's: a `\` begins an escape when it is preceded by an even number of
//! contiguous backslashes as written, a backslash produced by an escape never begins another one,
//! two escapes that form a UTF-16 surrogate pair are one character, and a surrogate that isn't part
//! of a pair is U+FFFD. One thing differs: the lexer drops a malformed escape (`\u` not followed by
//! four hex digits), but here it stays as written, so no text of a comment is lost. The lexer has
//! already reported it.

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
    let backslashes = text.as_bytes()[..start]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count();
    Chars {
        text,
        pos: start,
        end,
        backslashes,
    }
}

pub(crate) struct Chars<'a> {
    text: &'a str,
    pos: usize,
    end: usize,
    /// The number of contiguous backslashes as written just before `pos`.
    backslashes: usize,
}

impl Iterator for Chars<'_> {
    type Item = Char;

    fn next(&mut self) -> Option<Char> {
        let start = self.pos;
        let ch = self.text[start..self.end].chars().next()?;
        if ch == '\\'
            && self.backslashes.is_multiple_of(2)
            && let Some((end, unit)) = scan(self.text.as_bytes(), start, self.end)
        {
            self.backslashes = 0;
            let ch = match unit {
                0xD800..=0xDBFF => match scan(self.text.as_bytes(), end, self.end) {
                    Some((low_end, low @ 0xDC00..=0xDFFF)) => {
                        self.pos = low_end;
                        let c = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                        char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER)
                    }
                    _ => {
                        self.pos = end;
                        char::REPLACEMENT_CHARACTER
                    }
                },
                _ => {
                    self.pos = end;
                    char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER)
                }
            };
            return Some(Char {
                ch,
                start: start as u32,
                end: self.pos as u32,
                escaped: true,
            });
        }
        self.backslashes = if ch == '\\' { self.backslashes + 1 } else { 0 };
        self.pos += ch.len_utf8();
        Some(Char {
            ch,
            start: start as u32,
            end: self.pos as u32,
            escaped: false,
        })
    }
}

/// Reads a well-formed escape starting at `start` and not reading past `end`: where it ends and
/// the UTF-16 code unit it stands for. The caller has checked the even-backslash rule.
fn scan(bytes: &[u8], start: usize, end: usize) -> Option<(usize, u32)> {
    if bytes.get(start) != Some(&b'\\') || start + 1 >= end || bytes[start + 1] != b'u' {
        return None;
    }
    let mut pos = start + 1;
    while pos < end && bytes[pos] == b'u' {
        pos += 1;
    }
    let mut unit = 0;
    for _ in 0..4 {
        let digit = bytes[..end]
            .get(pos)
            .and_then(|&b| (b as char).to_digit(16))?;
        unit = unit * 16 + digit;
        pos += 1;
    }
    Some((pos, unit))
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
