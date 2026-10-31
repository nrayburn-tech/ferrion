//! Reading a source file's bytes into text: encoding detection, byte-order mark, and line endings.

use std::fmt;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Byte-order marks of encodings that aren't supported, longest first so that UTF-32LE is not
/// mistaken for UTF-16LE.
const UNSUPPORTED_ENCODING_BOMS: &[(&[u8], &str)] = &[
    (b"\xFF\xFE\x00\x00", "UTF-32LE"),
    (b"\x00\x00\xFE\xFF", "UTF-32BE"),
    (b"\xFF\xFE", "UTF-16LE"),
    (b"\xFE\xFF", "UTF-16BE"),
];

/// How much of the file is sampled for NUL bytes, which mark UTF-16 or UTF-32 without a
/// byte-order mark.
const NUL_SAMPLE: usize = 1024;

/// A source file's encoding, detected from its content, with what the formatter needs to write the
/// file back in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Every byte is ASCII. ASCII has no byte-order mark: a file with one is [`Encoding::Utf8`].
    Ascii,
    /// Valid UTF-8 with at least one non-ASCII byte, or a byte-order mark.
    Utf8 {
        /// Whether the file starts with a UTF-8 byte-order mark.
        bom: bool,
    },
}

/// A line terminator (JLS §3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// Why a file was skipped instead of being read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The file is not UTF-8 or ASCII.
    UnsupportedEncoding {
        /// The encoding, when it could be identified.
        detected: Option<&'static str>,
        /// The offset of the first byte that is not valid UTF-8, if that is how it was found.
        invalid_at: Option<usize>,
    },
    /// The file has more than `u32::MAX` bytes.
    TooLarge { len: usize },
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::UnsupportedEncoding {
                detected: Some(name),
                ..
            } => write!(
                f,
                "the file is encoded as {name}; only UTF-8 and ASCII are supported"
            ),
            SkipReason::UnsupportedEncoding {
                detected: None,
                invalid_at: Some(offset),
            } => write!(
                f,
                "the file is not valid UTF-8 (invalid byte at offset {offset}); only UTF-8 and ASCII \
                 are supported"
            ),
            SkipReason::UnsupportedEncoding {
                detected: None,
                invalid_at: None,
            } => write!(f, "only UTF-8 and ASCII are supported"),
            SkipReason::TooLarge { len } => write!(
                f,
                "the file is too large ({len} bytes; the limit is {} bytes)",
                u32::MAX
            ),
        }
    }
}

impl std::error::Error for SkipReason {}

/// A source file's text, with what the formatter needs to write it back: its encoding, including
/// whether it had a byte-order mark, and its first line ending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceText {
    text: String,
    encoding: Encoding,
    line_ending: Option<LineEnding>,
}

impl SourceText {
    /// Reads a file's bytes. Files that are not UTF-8 or ASCII are skipped.
    pub fn decode(bytes: impl Into<Vec<u8>>) -> Result<SourceText, SkipReason> {
        let mut bytes = bytes.into();
        if bytes.len() > u32::MAX as usize {
            return Err(SkipReason::TooLarge { len: bytes.len() });
        }
        if let Some(name) = foreign_encoding(&bytes) {
            return Err(SkipReason::UnsupportedEncoding {
                detected: Some(name),
                invalid_at: None,
            });
        }
        let bom = bytes.starts_with(UTF8_BOM);
        if bom {
            bytes.drain(..UTF8_BOM.len());
        }
        let text = String::from_utf8(bytes).map_err(|error| SkipReason::UnsupportedEncoding {
            detected: None,
            invalid_at: Some(
                error.utf8_error().valid_up_to() + if bom { UTF8_BOM.len() } else { 0 },
            ),
        })?;
        let encoding = if bom || !text.is_ascii() {
            Encoding::Utf8 { bom }
        } else {
            Encoding::Ascii
        };
        let line_ending = first_line_ending(text.as_bytes());
        Ok(SourceText {
            text,
            encoding,
            line_ending,
        })
    }

    /// The text after any byte-order mark. [`Span`](crate::Span)s are offsets into it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the file starts with a byte-order mark.
    pub fn has_bom(&self) -> bool {
        matches!(self.encoding, Encoding::Utf8 { bom: true })
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The first line ending in the file, or `None` if it has a single line. Unicode escapes
    /// (`\u000a`) are not line endings here: this is about the bytes the formatter writes.
    pub fn line_ending(&self) -> Option<LineEnding> {
        self.line_ending
    }

    /// The file's original bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let bom: &[u8] = if self.has_bom() { UTF8_BOM } else { &[] };
        [bom, self.text.as_bytes()].concat()
    }
}

/// Identifies UTF-16 and UTF-32, by byte-order mark or by a high share of NUL bytes.
fn foreign_encoding(bytes: &[u8]) -> Option<&'static str> {
    if let Some((_, name)) = UNSUPPORTED_ENCODING_BOMS
        .iter()
        .find(|(bom, _)| bytes.starts_with(bom))
    {
        return Some(name);
    }
    let sample = &bytes[..bytes.len().min(NUL_SAMPLE)];
    let nuls = sample.iter().filter(|&&b| b == 0).count();
    (nuls > 0 && nuls * 4 >= sample.len()).then_some("UTF-16 or UTF-32 without a byte-order mark")
}

fn first_line_ending(bytes: &[u8]) -> Option<LineEnding> {
    let i = bytes.iter().position(|&b| b == b'\n' || b == b'\r')?;
    Some(match (bytes[i], bytes.get(i + 1)) {
        (b'\n', _) => LineEnding::Lf,
        (_, Some(b'\n')) => LineEnding::CrLf,
        _ => LineEnding::Cr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_utf8() {
        let ascii = SourceText::decode("class A {}").unwrap();
        assert_eq!(ascii.encoding(), Encoding::Ascii);
        assert!(!ascii.has_bom());
        let utf8 = SourceText::decode("class Ä {}").unwrap();
        assert_eq!(utf8.encoding(), Encoding::Utf8 { bom: false });
        assert!(!utf8.has_bom());
    }

    #[test]
    fn bom_is_recorded_and_restored() {
        let bytes = b"\xEF\xBB\xBFclass A {}\n".to_vec();
        let source = SourceText::decode(bytes.clone()).unwrap();
        assert!(source.has_bom());
        // The content is ASCII, but ASCII has no byte-order mark.
        assert_eq!(source.encoding(), Encoding::Utf8 { bom: true });
        assert_eq!(source.text(), "class A {}\n");
        assert_eq!(source.to_bytes(), bytes);
    }

    #[test]
    fn invalid_utf8_is_skipped() {
        // "café" in ISO-8859-1.
        let error = SourceText::decode(b"// caf\xE9\n".to_vec()).unwrap_err();
        assert_eq!(
            error,
            SkipReason::UnsupportedEncoding {
                detected: None,
                invalid_at: Some(6)
            }
        );
        let error = SourceText::decode(b"\xEF\xBB\xBF\xFF".to_vec()).unwrap_err();
        assert_eq!(
            error,
            SkipReason::UnsupportedEncoding {
                detected: None,
                invalid_at: Some(3)
            }
        );
    }

    #[test]
    fn utf16_and_utf32_are_identified() {
        let detected = |bytes: &[u8]| match SourceText::decode(bytes) {
            Err(SkipReason::UnsupportedEncoding { detected, .. }) => detected,
            other => panic!("expected a skip, got {other:?}"),
        };
        assert_eq!(detected(b"\xFF\xFEc\x00"), Some("UTF-16LE"));
        assert_eq!(detected(b"\xFE\xFF\x00c"), Some("UTF-16BE"));
        assert_eq!(detected(b"\xFF\xFE\x00\x00c\x00\x00\x00"), Some("UTF-32LE"));
        assert_eq!(detected(b"\x00\x00\xFE\xFF"), Some("UTF-32BE"));
        assert!(detected(b"c\x00l\x00a\x00s\x00s\x00").is_some());
    }

    #[test]
    fn a_stray_nul_is_not_utf16() {
        let text = format!("class A {{ char c = '\0'; {} }}", " ".repeat(64));
        assert!(SourceText::decode(text).is_ok());
    }

    #[test]
    fn first_line_ending_wins() {
        let ending = |text: &str| SourceText::decode(text).unwrap().line_ending();
        assert_eq!(ending("a\nb\r\n"), Some(LineEnding::Lf));
        assert_eq!(ending("a\r\nb\n"), Some(LineEnding::CrLf));
        assert_eq!(ending("a\rb\n"), Some(LineEnding::Cr));
        assert_eq!(ending("a\r"), Some(LineEnding::Cr));
        assert_eq!(ending("a \\u000a b"), None);
        assert_eq!(ending(""), None);
    }
}
