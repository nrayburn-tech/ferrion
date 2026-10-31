//! The kinds of tokens and nodes. The enum is generated from `java.grammar`.

mod generated;

pub use generated::SyntaxKind;
pub use generated::token_sets;

impl SyntaxKind {
    /// A token kind: one the lexer produces, or a contextual keyword's.
    pub fn is_token(self) -> bool {
        self < SyntaxKind::FIRST_NODE
    }

    pub fn is_contextual_keyword(self) -> bool {
        (SyntaxKind::FIRST_CONTEXTUAL..SyntaxKind::FIRST_NODE).contains(&self)
    }

    pub fn is_node(self) -> bool {
        self >= SyntaxKind::FIRST_NODE
    }
}

#[cfg(test)]
mod tests {
    use ferrion_lexer::{ContextualKeyword, TokenKind};

    use super::*;

    #[test]
    fn kinds_are_in_order() {
        for (i, kind) in SyntaxKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, i);
        }
        assert!(SyntaxKind::Eof.is_token() && !SyntaxKind::Eof.is_contextual_keyword());
        assert!(SyntaxKind::RecordKw.is_token() && SyntaxKind::RecordKw.is_contextual_keyword());
        assert!(SyntaxKind::CompilationUnit.is_node() && !SyntaxKind::CompilationUnit.is_token());
    }

    #[test]
    fn token_kinds_match_the_lexer() {
        for (kind, text) in [
            (TokenKind::Class, "class"),
            (TokenKind::Underscore, "_"),
            (TokenKind::GtEq, ">="),
            (TokenKind::True, "true"),
        ] {
            assert_eq!(SyntaxKind::from_token_kind(kind).fixed_text(), Some(text));
            assert_eq!(kind.fixed_text(), Some(text));
        }
        assert_eq!(
            SyntaxKind::from_token_kind(TokenKind::Error),
            SyntaxKind::ErrorToken
        );
        assert!(SyntaxKind::from_token_kind(TokenKind::DocComment).is_trivia());
        assert!(!SyntaxKind::from_token_kind(TokenKind::Identifier).is_trivia());
    }

    #[test]
    fn contextual_keywords() {
        for keyword in [
            ContextualKeyword::Exports,
            ContextualKeyword::Module,
            ContextualKeyword::NonSealed,
            ContextualKeyword::Open,
            ContextualKeyword::Opens,
            ContextualKeyword::Permits,
            ContextualKeyword::Provides,
            ContextualKeyword::Record,
            ContextualKeyword::Requires,
            ContextualKeyword::Sealed,
            ContextualKeyword::To,
            ContextualKeyword::Transitive,
            ContextualKeyword::Uses,
            ContextualKeyword::When,
            ContextualKeyword::With,
            ContextualKeyword::Yield,
        ] {
            let kind = SyntaxKind::from_contextual_keyword(keyword).unwrap();
            assert!(kind.is_contextual_keyword());
            let text = kind.fixed_text().unwrap();
            if keyword != ContextualKeyword::NonSealed {
                assert_eq!(ContextualKeyword::from_identifier(text), Some(keyword));
            }
        }
        assert_eq!(
            SyntaxKind::from_contextual_keyword(ContextualKeyword::Var),
            None
        );
    }
}
