//! Reading a text's characters after Unicode escapes are translated, at raw offsets.

use crate::escape::Escape;

/// A position in a text, reading characters with Unicode escapes translated (JLS §3.3).
///
/// Offsets are always raw byte offsets, so an escape such as `A` is one character `A` that
/// spans six bytes. Invalid escapes are skipped over as if they weren't there, which is what javac
/// does after reporting them; the lexer decides which token their text belongs to.
#[derive(Clone)]
pub(crate) struct Cursor<'a> {
    text: &'a str,
    escapes: &'a [Escape],
    has_invalid_escapes: bool,
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(text: &'a str, escapes: &'a [Escape]) -> Cursor<'a> {
        Cursor {
            text,
            escapes,
            has_invalid_escapes: escapes.iter().any(|e| !e.is_valid()),
            pos: 0,
        }
    }

    pub fn text(&self) -> &'a str {
        self.text
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn is_at_end(&self) -> bool {
        self.pos >= self.text.len()
    }

    /// The next character.
    pub fn peek(&self) -> Option<char> {
        self.char_at(self.pos).map(|(c, _)| c)
    }

    /// The character after the next one.
    pub fn peek2(&self) -> Option<char> {
        let (_, end) = self.char_at(self.pos)?;
        self.char_at(end).map(|(c, _)| c)
    }

    /// The `n`th character from here, counting from 0.
    pub fn peek_nth(&self, n: usize) -> Option<char> {
        let mut pos = self.pos;
        for _ in 0..n {
            pos = self.char_at(pos)?.1;
        }
        self.char_at(pos).map(|(c, _)| c)
    }

    /// Moves past the next character, and any invalid escapes before it.
    pub fn bump(&mut self) {
        if let Some((_, end)) = self.char_at(self.pos) {
            self.pos = end;
        }
    }

    /// Moves past the next character if it is `c`.
    pub fn eat(&mut self, c: char) -> bool {
        let matches = self.peek() == Some(c);
        if matches {
            self.bump();
        }
        matches
    }

    /// Moves past the next character if `pred` accepts it, and returns it.
    pub fn eat_if(&mut self, pred: impl FnOnce(char) -> bool) -> Option<char> {
        let (c, end) = self.char_at(self.pos)?;
        if !pred(c) {
            return None;
        }
        self.pos = end;
        Some(c)
    }

    /// Moves past the next character if `pred` accepts it, unless an invalid escape comes first.
    pub fn eat_here_if(&mut self, pred: impl FnOnce(char) -> bool) -> bool {
        if self.invalid_escape_here().is_some() {
            return false;
        }
        self.eat_if(pred).is_some()
    }

    /// Moves past characters while `pred` accepts them, and any invalid escapes between them.
    pub fn eat_while(&mut self, pred: impl Fn(char) -> bool) {
        loop {
            self.eat_ascii_while(&pred);
            if self.eat_if(&pred).is_none() {
                break;
            }
        }
    }

    /// Moves past characters while `pred` accepts them, stopping at an invalid escape.
    pub fn eat_here_while(&mut self, pred: impl Fn(char) -> bool) {
        loop {
            self.eat_ascii_while(&pred);
            if !self.eat_here_if(&pred) {
                break;
            }
        }
    }

    /// The fast path of `eat_while`: ASCII characters written as is, which can't be part of an
    /// escape unless they're a backslash.
    fn eat_ascii_while(&mut self, pred: &impl Fn(char) -> bool) {
        let bytes = self.text.as_bytes();
        while let Some(&b) = bytes.get(self.pos) {
            if !b.is_ascii() || b == b'\\' || !pred(b as char) {
                break;
            }
            self.pos += 1;
        }
    }

    /// The invalid escape at the current position, if any.
    pub fn invalid_escape_here(&self) -> Option<&'a Escape> {
        self.escape_at(self.pos).filter(|e| !e.is_valid())
    }

    /// Moves past any invalid escapes at the current position.
    pub fn skip_invalid_escapes(&mut self) {
        self.pos = self.skip_invalid(self.pos);
    }

    /// Whether the raw byte at the current position is `b`, as written rather than as an escape.
    pub fn raw_byte_is(&self, b: u8) -> bool {
        self.text.as_bytes().get(self.pos) == Some(&b)
    }

    /// The character at `pos` and where it ends, skipping any invalid escapes first.
    fn char_at(&self, pos: usize) -> Option<(char, usize)> {
        let pos = self.skip_invalid(pos);
        let b = *self.text.as_bytes().get(pos)?;
        if b == b'\\'
            && let Some(escape) = self.escape_at(pos)
        {
            // `skip_invalid` already moved past invalid escapes.
            return Some((escape.ch?, escape.end as usize));
        }
        if b.is_ascii() {
            return Some((b as char, pos + 1));
        }
        let c = self.text[pos..].chars().next()?;
        Some((c, pos + c.len_utf8()))
    }

    fn skip_invalid(&self, mut pos: usize) -> usize {
        if !self.has_invalid_escapes {
            return pos;
        }
        while let Some(escape) = self.escape_at(pos).filter(|e| !e.is_valid()) {
            pos = escape.end as usize;
        }
        pos
    }

    fn escape_at(&self, pos: usize) -> Option<&'a Escape> {
        if self.escapes.is_empty() || self.text.as_bytes().get(pos) != Some(&b'\\') {
            return None;
        }
        let i = self.escapes.partition_point(|e| (e.start as usize) < pos);
        self.escapes.get(i).filter(|e| e.start as usize == pos)
    }
}
