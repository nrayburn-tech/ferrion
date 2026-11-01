//! A bounds-checked reader over bytes. Every read in the crate goes through it, so a lying length
//! or a truncated structure is `Malformed`, never a panic.

use crate::error::{Error, Result};

#[derive(Clone)]
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    /// Added to positions in errors, so they are offsets in the whole input.
    base: u64,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8], base: u64) -> Cursor<'a> {
        Cursor { data, pos: 0, base }
    }

    /// The offset of the next byte in the whole input.
    pub(crate) fn offset(&self) -> u64 {
        self.base + self.pos as u64
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pos == self.data.len()
    }

    pub(crate) fn error(&self, message: impl Into<String>) -> Error {
        Error::malformed(self.offset(), message)
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.remaining() {
            return Err(self.error(format!(
                "truncated: {len} bytes needed, {} left",
                self.remaining()
            )));
        }
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(bytes)
    }

    pub(crate) fn skip(&mut self, len: usize) -> Result<()> {
        self.bytes(len).map(|_| ())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut array = [0; N];
        array.copy_from_slice(self.bytes(N)?);
        Ok(array)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_bounds() {
        let mut cursor = Cursor::new(&[1, 2, 3, 4, 5, 6, 7], 100);
        assert_eq!(cursor.u16().unwrap(), 0x0102);
        assert_eq!(cursor.u16().unwrap(), 0x0304);
        assert_eq!(cursor.offset(), 104);
        let error = cursor.u32().unwrap_err();
        assert_eq!(error.offset(), Some(104));
        assert_eq!(cursor.remaining(), 3);
        assert_eq!(cursor.bytes(3).unwrap(), [5, 6, 7]);
        assert!(cursor.is_empty());
        assert!(cursor.u8().is_err());
    }
}
