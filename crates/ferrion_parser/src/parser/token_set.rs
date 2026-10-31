//! Sets of token kinds, for lookahead and recovery sets.

use crate::SyntaxKind;

/// A set of token kinds (contextual keyword kinds included), as a bit set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TokenSet([u64; 4]);

impl TokenSet {
    #[allow(dead_code, reason = "for the grammar areas still to come")]
    pub const EMPTY: TokenSet = TokenSet([0; 4]);

    /// Panics, at compile time for a constant, if a kind isn't a token kind.
    pub const fn new(kinds: &[SyntaxKind]) -> TokenSet {
        let mut bits = [0; 4];
        let mut i = 0;
        while i < kinds.len() {
            let kind = kinds[i] as usize;
            assert!(kind < 256, "a token set holds only token kinds");
            bits[kind / 64] |= 1 << (kind % 64);
            i += 1;
        }
        TokenSet(bits)
    }

    #[allow(dead_code, reason = "for the grammar areas still to come")]
    pub const fn union(self, other: TokenSet) -> TokenSet {
        let mut bits = self.0;
        let mut i = 0;
        while i < 4 {
            bits[i] |= other.0[i];
            i += 1;
        }
        TokenSet(bits)
    }

    pub const fn contains(self, kind: SyntaxKind) -> bool {
        let kind = kind as usize;
        kind < 256 && self.0[kind / 64] & (1 << (kind % 64)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrion_syntax::token_sets;

    #[test]
    fn sets() {
        const IDENT: TokenSet = TokenSet::new(token_sets::IDENT);
        let set = IDENT.union(TokenSet::new(&[SyntaxKind::YieldKw, SyntaxKind::Eof]));
        for kind in [
            SyntaxKind::Identifier,
            SyntaxKind::Underscore,
            SyntaxKind::YieldKw,
            SyntaxKind::Eof,
        ] {
            assert!(set.contains(kind));
        }
        assert!(!set.contains(SyntaxKind::Whitespace));
        assert!(!set.contains(SyntaxKind::CompilationUnit));
        assert!(!TokenSet::EMPTY.contains(SyntaxKind::Whitespace));
        assert!(SyntaxKind::ALL.iter().filter(|k| k.is_token()).count() <= 256);
    }
}
