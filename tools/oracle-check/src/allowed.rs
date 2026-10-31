//! The differences from javac that comparisons allow.
//!
//! SCOPE.md allows disagreements with javac only as a documented, categorized list. This is that
//! list for the comparisons in this tool, and docs/design/lexer-oracle-normalization.md explains
//! each entry. An entry needs a reason: observed javac behavior that Ferrion deliberately doesn't
//! follow, or javac behavior that isn't meaningful to compare. Comparisons count how often each
//! entry applies, and the report shows the counts.

use serde::Serialize;

/// The categories SCOPE.md allows differences in. Not every one has an entry yet.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// What javac produces after an error, which depends on how it recovers.
    JavacErrorRecovery,
    /// javac behavior that the specification doesn't call for.
    JavacQuirk,
    /// Ferrion deliberately behaves differently from javac.
    DeliberateDeparture,
}

/// An allowed difference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allowed {
    /// javac reads a UTF-8 byte-order mark as the character U+FEFF, reports it as an illegal
    /// character, and counts it in every position after it. Ferrion records the mark and lexes the
    /// text after it.
    ByteOrderMark,
    /// javac styles `/**/` as a documentation comment (`JAVADOC_BLOCK`). It has no content, and the
    /// lexer makes it a block comment.
    EmptyBlockComment,
}

impl Allowed {
    pub const ALL: [Allowed; 2] = [Allowed::ByteOrderMark, Allowed::EmptyBlockComment];

    pub const fn id(self) -> &'static str {
        match self {
            Allowed::ByteOrderMark => "byte-order-mark",
            Allowed::EmptyBlockComment => "empty-block-comment",
        }
    }

    pub const fn category(self) -> Category {
        match self {
            Allowed::ByteOrderMark | Allowed::EmptyBlockComment => Category::DeliberateDeparture,
        }
    }
}

/// How many times each allowed difference applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts([u64; Allowed::ALL.len()]);

impl Counts {
    pub fn add(&mut self, allowed: Allowed) {
        self.0[allowed as usize] += 1;
    }

    pub fn merge(&mut self, other: &Counts) {
        for (count, other) in self.0.iter_mut().zip(other.0) {
            *count += other;
        }
    }

    pub fn get(&self, allowed: Allowed) -> u64 {
        self.0[allowed as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&count| count == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_entry_in_order() {
        for (i, allowed) in Allowed::ALL.into_iter().enumerate() {
            assert_eq!(allowed as usize, i);
        }
    }

    #[test]
    fn counts() {
        let mut counts = Counts::default();
        assert!(counts.is_empty());
        counts.add(Allowed::EmptyBlockComment);
        let mut total = Counts::default();
        total.merge(&counts);
        total.merge(&counts);
        assert_eq!(total.get(Allowed::EmptyBlockComment), 2);
        assert_eq!(total.get(Allowed::ByteOrderMark), 0);
    }
}
