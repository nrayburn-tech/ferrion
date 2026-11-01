//! Fingerprints: 64-bit hashes of values, compared to tell whether a value changed without
//! keeping or comparing the old one (docs/design/semantic-incremental.md).

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use ferrion_syntax::SyntaxNode;

/// A 64-bit hash of a value. Equal values have equal fingerprints; different values almost
/// always have different ones, and the engine treats them as if they always did.
///
/// Fingerprints use the standard library's `DefaultHasher`, whose keys are fixed, so they repeat
/// within a process and across runs of the same build. They aren't persisted, so a change of
/// hasher between Rust releases doesn't matter.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(u64);

impl Fingerprint {
    pub fn of<T: Hash + ?Sized>(value: &T) -> Fingerprint {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        Fingerprint(hasher.finish())
    }

    /// The fingerprint of the tokens in `node`, trivia left out: each token's kind and its text
    /// with Unicode escapes translated. Comments and whitespace don't change it; any other edit
    /// does. A loop over the node's tokens, so a deeply nested expression doesn't recurse.
    pub fn of_tokens(node: SyntaxNode<'_>) -> Fingerprint {
        let tree = node.tree();
        let mut hasher = DefaultHasher::new();
        for token in node.tokens() {
            if token.is_trivia() {
                continue;
            }
            token.kind().hash(&mut hasher);
            let lexed = tree.lexer_tokens()[token.id().index()];
            ferrion_lexer::cooked_text(tree.text(), lexed).hash(&mut hasher);
        }
        Fingerprint(hasher.finish())
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({:016x})", self.0)
    }
}
