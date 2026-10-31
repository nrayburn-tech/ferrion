//! Converting byte offsets to lines, columns, and UTF-16 offsets.
//!
//! javac, Google Java Format, and the Language Server Protocol count positions in UTF-16 code
//! units. Ferrion counts bytes internally and converts at those boundaries.

/// A zero-based line and column. The column counts UTF-16 code units from the start of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// An index of a text's line starts and non-ASCII characters.
///
/// Lines end at CR, LF, or CRLF (JLS §3.4). Unicode escapes such as `\u000a` don't end lines here,
/// since positions describe the file as written.
#[derive(Clone, Debug)]
pub struct LineIndex {
    line_starts: Vec<u32>,
    /// For each character outside ASCII: where it ends, in bytes and in UTF-16 code units.
    wide: Vec<Wide>,
    len: u32,
}

#[derive(Clone, Copy, Debug)]
struct Wide {
    byte_end: u32,
    utf16_end: u32,
    surrogate_pair: bool,
}

impl LineIndex {
    /// Indexes `text`, which must be at most `u32::MAX` bytes, as a
    /// [`SourceText`](crate::SourceText) guarantees.
    pub fn new(text: &str) -> LineIndex {
        let mut line_starts = vec![0];
        let mut wide = Vec::new();
        let bytes = text.as_bytes();
        let mut utf16 = 0u32;
        for (i, c) in text.char_indices() {
            let i = i as u32;
            let end = i + c.len_utf8() as u32;
            utf16 += c.len_utf16() as u32;
            match c {
                '\n' => line_starts.push(end),
                '\r' if bytes.get(end as usize) != Some(&b'\n') => line_starts.push(end),
                _ if !c.is_ascii() => wide.push(Wide {
                    byte_end: end,
                    utf16_end: utf16,
                    surrogate_pair: c.len_utf16() == 2,
                }),
                _ => {}
            }
        }
        LineIndex {
            line_starts,
            wide,
            len: text.len() as u32,
        }
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// The byte offset where `line` starts, or `None` past the last line.
    pub fn line_start(&self, line: u32) -> Option<u32> {
        self.line_starts.get(line as usize).copied()
    }

    /// The line and column of a byte offset. Offsets past the end are clamped to the end.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = offset.min(self.len);
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let start = self.line_starts[line];
        LineCol {
            line: line as u32,
            col: self.utf16_offset(offset) - self.utf16_offset(start),
        }
    }

    /// The UTF-16 offset of a byte offset. Offsets past the end are clamped to the end, and an
    /// offset inside a character counts the code units of the characters before it.
    pub fn utf16_offset(&self, offset: u32) -> u32 {
        let offset = offset.min(self.len);
        let i = self.wide.partition_point(|w| w.byte_end <= offset);
        match i.checked_sub(1).map(|i| self.wide[i]) {
            Some(w) => w.utf16_end + (offset - w.byte_end),
            None => offset,
        }
    }

    /// The byte offset of a UTF-16 offset, or `None` if it is past the end or falls inside a
    /// character.
    pub fn byte_offset(&self, utf16_offset: u32) -> Option<u32> {
        let i = self.wide.partition_point(|w| w.utf16_end <= utf16_offset);
        let offset = match i.checked_sub(1).map(|i| self.wide[i]) {
            Some(w) => w.byte_end + (utf16_offset - w.utf16_end),
            None => utf16_offset,
        };
        // Only a surrogate pair can be split: the offset is then one unit before its end.
        let inside_char = self
            .wide
            .get(i)
            .is_some_and(|next| next.surrogate_pair && next.utf16_end - 1 == utf16_offset);
        (offset <= self.len && !inside_char).then_some(offset)
    }

    /// The UTF-16 length of the text.
    pub fn utf16_len(&self) -> u32 {
        self.utf16_offset(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_end_at_cr_lf_and_crlf() {
        let index = LineIndex::new("a\nb\r\nc\rd");
        assert_eq!(index.line_count(), 4);
        let starts: Vec<_> = (0..4).map(|l| index.line_start(l).unwrap()).collect();
        assert_eq!(starts, [0, 2, 5, 7]);
        assert_eq!(index.line_col(3), LineCol { line: 1, col: 1 });
        assert_eq!(index.line_col(4), LineCol { line: 1, col: 2 });
        assert_eq!(index.line_col(100), LineCol { line: 3, col: 1 });
    }

    #[test]
    fn escaped_line_feed_is_not_a_line_end() {
        assert_eq!(LineIndex::new("a\\u000ab").line_count(), 1);
    }

    #[test]
    fn utf16_offsets() {
        // 'é' is 2 bytes and 1 code unit; '😀' is 4 bytes and 2 code units.
        let text = "aé😀b\n😀c";
        let index = LineIndex::new(text);
        for (byte, utf16) in [
            (0, 0),
            (1, 1),
            (3, 2),
            (7, 4),
            (8, 5),
            (9, 6),
            (13, 8),
            (14, 9),
        ] {
            assert_eq!(index.utf16_offset(byte), utf16, "byte {byte}");
            assert_eq!(index.byte_offset(utf16), Some(byte), "utf16 {utf16}");
        }
        assert_eq!(index.utf16_len(), 9);
        assert_eq!(index.line_col(13), LineCol { line: 1, col: 2 });
    }

    #[test]
    fn utf16_offset_inside_a_surrogate_pair() {
        let index = LineIndex::new("a😀b");
        assert_eq!(index.byte_offset(2), None);
        assert_eq!(index.byte_offset(3), Some(5));
        assert_eq!(index.byte_offset(5), None);
    }

    #[test]
    fn utf16_round_trip_matches_std() {
        let text = "x\u{7FF}\u{800}\u{FFFF}\u{10000}\u{10FFFF}é\r\nz";
        let index = LineIndex::new(text);
        for (byte, _) in text.char_indices().chain([(text.len(), ' ')]) {
            let utf16 = text[..byte].encode_utf16().count() as u32;
            assert_eq!(index.utf16_offset(byte as u32), utf16);
            assert_eq!(index.byte_offset(utf16), Some(byte as u32));
        }
    }
}
