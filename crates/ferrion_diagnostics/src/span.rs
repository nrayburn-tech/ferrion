//! Byte ranges in a source text.

use std::fmt;
use std::ops::Range;

/// A range of bytes, `start..end`, in a [`SourceText`](crate::SourceText)'s text.
///
/// Offsets count bytes of the text after any byte-order mark. Source texts are limited to
/// `u32::MAX` bytes, so offsets are stored as `u32`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    /// An empty span at `offset`.
    pub const fn empty(offset: u32) -> Span {
        Span::new(offset, offset)
    }

    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both `self` and `other`.
    pub fn cover(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    pub fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    pub fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Range<usize> {
        span.range()
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_and_contains() {
        let a = Span::new(2, 5);
        let b = Span::new(7, 9);
        assert_eq!(a.cover(b), Span::new(2, 9));
        assert!(a.contains(2));
        assert!(!a.contains(5));
        assert!(Span::empty(3).is_empty());
        assert_eq!(&"abcdefg"[a.range()], "cde");
    }
}
