//! A constant string's value: UTF-16 code units, as a Java string holds them, made by
//! concatenation without copying.
//!
//! A string constant folded from a long `+` chain has every prefix as a constant of its own
//! (lucene's JFlex tables join 2,670 literals into one 42,710-unit string, whose prefixes sum to
//! 57 million units), so a concatenation shares its operands instead of copying them, and a
//! chain is flattened every [`MAX_DEPTH`] levels, which keeps both the copying and the depth that
//! traversal and dropping walk small. Equality and hashing are by content: each string carries a
//! polynomial hash of its units, combined in constant time on concatenation.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// How deep a chain of concatenations grows before it is copied into one piece.
const MAX_DEPTH: u32 = 64;

/// The hash's base: odd, so multiplication by it is a bijection modulo 2^64.
const BASE: u64 = 0x0100_0000_01b3;

/// A Java string's code units: lone surrogates are kept.
#[derive(Clone)]
pub struct JavaString(Arc<Node>);

struct Node {
    len: usize,
    /// Σ unit[i] · BASE^(len - 1 - i), wrapping.
    hash: u64,
    depth: u32,
    kind: Kind,
}

enum Kind {
    Units(Box<[u16]>),
    Concat(JavaString, JavaString),
}

fn power(mut base: u64, mut exponent: usize) -> u64 {
    let mut result: u64 = 1;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exponent >>= 1;
    }
    result
}

impl JavaString {
    pub fn from_units(units: Vec<u16>) -> JavaString {
        let hash = units.iter().fold(0u64, |h, &u| {
            h.wrapping_mul(BASE).wrapping_add(u64::from(u))
        });
        JavaString(Arc::new(Node {
            len: units.len(),
            hash,
            depth: 0,
            kind: Kind::Units(units.into_boxed_slice()),
        }))
    }

    pub fn len(&self) -> usize {
        self.0.len
    }

    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }

    /// `self` followed by `other`, sharing both.
    pub fn concat(&self, other: &JavaString) -> JavaString {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let depth = self.0.depth.max(other.0.depth) + 1;
        if depth > MAX_DEPTH {
            let mut units = self.to_vec();
            other.extend_into(&mut units);
            return JavaString::from_units(units);
        }
        let hash = self
            .0
            .hash
            .wrapping_mul(power(BASE, other.len()))
            .wrapping_add(other.0.hash);
        JavaString(Arc::new(Node {
            len: self.len() + other.len(),
            hash,
            depth,
            kind: Kind::Concat(self.clone(), other.clone()),
        }))
    }

    /// The units, in order.
    pub fn to_vec(&self) -> Vec<u16> {
        let mut units = Vec::with_capacity(self.len());
        self.extend_into(&mut units);
        units
    }

    fn extend_into(&self, out: &mut Vec<u16>) {
        // In order, with an explicit stack: the depth is bounded, but needn't be small.
        let mut stack: Vec<&JavaString> = vec![self];
        while let Some(s) = stack.pop() {
            match &s.0.kind {
                Kind::Units(units) => out.extend_from_slice(units),
                Kind::Concat(a, b) => {
                    stack.push(b);
                    stack.push(a);
                }
            }
        }
    }
}

impl PartialEq for JavaString {
    fn eq(&self, other: &JavaString) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.len == other.0.len
                && self.0.hash == other.0.hash
                && self.to_vec() == other.to_vec())
    }
}

impl Eq for JavaString {}

impl Hash for JavaString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.0.len);
        state.write_u64(self.0.hash);
    }
}

impl fmt::Debug for JavaString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf16_lossy(&self.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> JavaString {
        JavaString::from_units(text.encode_utf16().collect())
    }

    #[test]
    fn concatenation_is_by_content() {
        let a = s("ab").concat(&s("c"));
        let b = s("a").concat(&s("bc"));
        assert_eq!(a, b);
        assert_eq!(a, s("abc"));
        assert_ne!(a, s("abd"));
        let hash = |x: &JavaString| crate::Fingerprint::of(x);
        assert_eq!(hash(&a), hash(&b));
        assert_eq!(a.to_vec(), "abc".encode_utf16().collect::<Vec<_>>());
    }

    #[test]
    fn long_chains_stay_shallow() {
        let mut chain = s("");
        let mut expected = String::new();
        for i in 0..5_000 {
            let part = format!("{i},");
            chain = chain.concat(&s(&part));
            expected.push_str(&part);
        }
        assert!(chain.0.depth <= MAX_DEPTH);
        assert_eq!(chain, s(&expected));
    }
}
