//! Text in class files: modified UTF-8 (JVMS 4.4.7).
//!
//! NUL is `c0 80`, and characters outside the Basic Multilingual Plane are two three-byte
//! surrogates. A raw zero byte, a byte from `f0` to `ff`, and a sequence cut short are malformed.
//! Overlong forms (`e0 80 80` for NUL) are read as the character they spell, as javap reads
//! them: class files in the corpus have them (Xerces' `Token`).

use std::borrow::Cow;
use std::fmt;

use crate::error::{Error, ErrorKind, Result};

/// A name or other text from the constant pool: modified UTF-8 bytes, decoded on request. Used
/// for class, member, package, and module names, descriptors, and signatures.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Name<'a>(pub(crate) &'a [u8]);

impl<'a> Name<'a> {
    pub fn new(bytes: &'a [u8]) -> Name<'a> {
        Name(bytes)
    }

    pub fn as_bytes(&self) -> &'a [u8] {
        self.0
    }

    /// The text as a Rust string, borrowed when the bytes are plain UTF-8, which they are for
    /// nearly every name. A name holding a lone surrogate is `Unsupported`, since a Rust string
    /// can't hold one.
    pub fn to_str(&self) -> Result<Cow<'a, str>> {
        if let Some(text) = plain(self.0) {
            return Ok(Cow::Borrowed(text));
        }
        let units = decode_utf16(self.0)?;
        String::from_utf16(&units).map(Cow::Owned).map_err(|_| {
            Error::new(
                ErrorKind::Unsupported,
                "a name holds a lone surrogate, which isn't supported",
            )
        })
    }

    /// Whether the text equals `text`, without allocating for plain names.
    pub fn eq_str(&self, text: &str) -> bool {
        if self.0 == text.as_bytes() && plain(self.0).is_some() {
            return true;
        }
        matches!(self.to_str(), Ok(decoded) if decoded == text)
    }
}

impl fmt::Debug for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_str() {
            Ok(text) => write!(f, "{text:?}"),
            Err(_) => write!(f, "{:?}", JavaStr(self.0).to_string_lossy()),
        }
    }
}

/// A string constant (a `String` field's `ConstantValue`, a string annotation value). It can hold
/// any UTF-16 sequence, lone surrogates included, which constant folding needs exactly.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct JavaStr<'a>(pub(crate) &'a [u8]);

impl<'a> JavaStr<'a> {
    pub fn new(bytes: &'a [u8]) -> JavaStr<'a> {
        JavaStr(bytes)
    }

    /// The modified UTF-8 bytes.
    pub fn as_bytes(&self) -> &'a [u8] {
        self.0
    }

    /// The UTF-16 code units, exactly.
    pub fn to_utf16(&self) -> Result<Vec<u16>> {
        decode_utf16(self.0)
    }

    /// The text, with each lone surrogate replaced by U+FFFD, or an error when the bytes aren't
    /// modified UTF-8.
    pub fn to_string_lossy(&self) -> String {
        match plain(self.0) {
            Some(text) => text.to_owned(),
            None => match decode_utf16(self.0) {
                Ok(units) => String::from_utf16_lossy(&units),
                Err(_) => String::from_utf8_lossy(self.0).into_owned(),
            },
        }
    }
}

impl fmt::Debug for JavaStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

/// The bytes as a `str` when modified UTF-8 and UTF-8 agree on them: valid UTF-8 with no zero
/// byte and no four-byte sequence.
fn plain(bytes: &[u8]) -> Option<&str> {
    if bytes.iter().any(|&b| b == 0 || b >= 0xf0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

/// Decodes modified UTF-8 into UTF-16 code units.
pub(crate) fn decode_utf16(bytes: &[u8]) -> Result<Vec<u16>> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let bad = |i: usize| {
        Error::malformed(
            i as u64,
            format!(
                "text isn't valid modified UTF-8 at byte {i} of {}",
                bytes.len()
            ),
        )
    };
    let continuation = |i: usize| match bytes.get(i) {
        Some(&b) if b & 0xc0 == 0x80 => Ok(u16::from(b & 0x3f)),
        _ => Err(bad(i)),
    };
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x01..=0x7f => {
                units.push(u16::from(b));
                i += 1;
            }
            0xc0..=0xdf => {
                units.push(u16::from(b & 0x1f) << 6 | continuation(i + 1)?);
                i += 2;
            }
            0xe0..=0xef => {
                units.push(
                    u16::from(b & 0x0f) << 12 | continuation(i + 1)? << 6 | continuation(i + 2)?,
                );
                i += 3;
            }
            _ => return Err(bad(i)),
        }
    }
    Ok(units)
}

/// Encodes text as modified UTF-8, for tests and seeds.
pub fn encode(text: &str) -> Vec<u8> {
    encode_utf16(&text.encode_utf16().collect::<Vec<_>>())
}

/// Encodes UTF-16 code units, lone surrogates included, as modified UTF-8.
pub fn encode_utf16(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(units.len());
    for &unit in units {
        match unit {
            0x01..=0x7f => bytes.push(unit as u8),
            0 | 0x80..=0x7ff => {
                bytes.push(0xc0 | (unit >> 6) as u8);
                bytes.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                bytes.push(0xe0 | (unit >> 12) as u8);
                bytes.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_are_borrowed() {
        let name = Name(b"java/lang/Object");
        assert!(matches!(
            name.to_str().unwrap(),
            Cow::Borrowed("java/lang/Object")
        ));
        assert!(name.eq_str("java/lang/Object"));
        assert!(!name.eq_str("java/lang/Objec"));
    }

    #[test]
    fn nul_and_supplementary_characters() {
        let text = "a\0b\u{e9}\u{1f600}";
        let bytes = encode(text);
        assert_eq!(&bytes[1..3], [0xc0, 0x80]);
        // The emoji is two three-byte surrogates.
        assert_eq!(bytes.len(), 1 + 2 + 1 + 2 + 6);
        let name = Name(&bytes);
        assert!(matches!(name.to_str().unwrap(), Cow::Owned(ref s) if s == text));
        assert!(name.eq_str(text));
    }

    #[test]
    fn lone_surrogates() {
        let bytes = encode_utf16(&[0x61, 0xd800, 0x62]);
        let error = Name(&bytes).to_str().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        let string = JavaStr(&bytes);
        assert_eq!(string.to_utf16().unwrap(), [0x61, 0xd800, 0x62]);
        assert_eq!(string.to_string_lossy(), "a\u{fffd}b");
    }

    #[test]
    fn malformed() {
        for bytes in [
            &b"a\0"[..],
            b"\xc0",
            b"\xe0\x80",
            b"\xf0\x9f\x98\x80",
            b"\x80",
            b"\xe0\x80\x41",
        ] {
            let error = Name(bytes).to_str().unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Malformed, "{bytes:?}");
        }
    }
}
