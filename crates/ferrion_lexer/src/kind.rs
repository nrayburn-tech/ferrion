//! Token kinds.

/// The kind of a [`Token`](crate::Token).
///
/// Every reserved keyword (JLS §3.9) has its own kind. Contextual keywords (`var`, `record`,
/// `yield`, ...) are [`Identifier`](TokenKind::Identifier)s; whether one acts as a keyword depends on
/// where it appears, which is the parser's business (see [`ContextualKeyword`]).
///
/// `>` never starts a longer token than `>=`: `>>`, `>>>`, `>>=`, and `>>>=` are lexed as `Gt` and
/// `GtEq` tokens with no gap between them, and the parser joins them. That way `List<List<T>>` needs
/// no splitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Trivia (JLS §3.6, §3.7).
    /// Spaces, tabs, and form feeds. Also an ASCII SUB (control-Z) that ends the text, which
    /// JLS §3.5 says to ignore.
    Whitespace,
    /// One CR, LF, or CRLF, written directly or as Unicode escapes.
    LineTerminator,
    /// `// ...`, up to but not including the line terminator.
    LineComment,
    /// `/// ...`, a Markdown documentation comment line in Java 23 and later, and a plain line
    /// comment before that. Like javac, any comment starting with three or more slashes is one.
    DocLineComment,
    /// `/* ... */`
    BlockComment,
    /// `/** ... */`, a traditional documentation comment. `/**/` is a `BlockComment`.
    DocComment,

    Identifier,

    // Reserved keywords (JLS §3.9).
    Abstract,
    Assert,
    Boolean,
    Break,
    Byte,
    Case,
    Catch,
    Char,
    Class,
    Const,
    Continue,
    Default,
    Do,
    Double,
    Else,
    Enum,
    Extends,
    Final,
    Finally,
    Float,
    For,
    Goto,
    If,
    Implements,
    Import,
    Instanceof,
    Int,
    Interface,
    Long,
    Native,
    New,
    Package,
    Private,
    Protected,
    Public,
    Return,
    Short,
    Static,
    Strictfp,
    Super,
    Switch,
    Synchronized,
    This,
    Throw,
    Throws,
    Transient,
    Try,
    Void,
    Volatile,
    While,
    /// `_`, a keyword since Java 9. In Java 8 it is an identifier; the version check handles that.
    Underscore,

    // Literals (JLS §3.10).
    True,
    False,
    Null,
    IntLiteral,
    /// An integer literal with an `l` or `L` suffix.
    LongLiteral,
    /// A floating-point literal with an `f` or `F` suffix.
    FloatLiteral,
    /// A floating-point literal without an `f` or `F` suffix.
    DoubleLiteral,
    CharLiteral,
    StringLiteral,
    /// `"""` ... `"""`
    TextBlock,

    // Separators (JLS §3.11).
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Dot,
    Ellipsis,
    At,
    ColonColon,

    // Operators (JLS §3.12), apart from the ones `>` starts (see above).
    Eq,
    Gt,
    Lt,
    Bang,
    Tilde,
    Question,
    Colon,
    Arrow,
    EqEq,
    GtEq,
    LtEq,
    BangEq,
    AmpAmp,
    PipePipe,
    PlusPlus,
    MinusMinus,
    Plus,
    Minus,
    Star,
    Slash,
    Amp,
    Pipe,
    Caret,
    Percent,
    LtLt,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    AmpEq,
    PipeEq,
    CaretEq,
    PercentEq,
    LtLtEq,

    /// Characters that can't start any token. The lexer reports an error for them.
    Error,
    /// The end of the text: an empty token, always last.
    Eof,
}

impl TokenKind {
    /// Whitespace, line terminators, and comments.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            TokenKind::Whitespace
                | TokenKind::LineTerminator
                | TokenKind::LineComment
                | TokenKind::DocLineComment
                | TokenKind::BlockComment
                | TokenKind::DocComment
        )
    }

    pub fn is_comment(self) -> bool {
        self.is_trivia() && !matches!(self, TokenKind::Whitespace | TokenKind::LineTerminator)
    }

    /// The reserved keyword or literal keyword (`true`, `false`, `null`) spelled `text`.
    pub fn from_keyword(text: &str) -> Option<TokenKind> {
        Some(match text {
            "abstract" => TokenKind::Abstract,
            "assert" => TokenKind::Assert,
            "boolean" => TokenKind::Boolean,
            "break" => TokenKind::Break,
            "byte" => TokenKind::Byte,
            "case" => TokenKind::Case,
            "catch" => TokenKind::Catch,
            "char" => TokenKind::Char,
            "class" => TokenKind::Class,
            "const" => TokenKind::Const,
            "continue" => TokenKind::Continue,
            "default" => TokenKind::Default,
            "do" => TokenKind::Do,
            "double" => TokenKind::Double,
            "else" => TokenKind::Else,
            "enum" => TokenKind::Enum,
            "extends" => TokenKind::Extends,
            "final" => TokenKind::Final,
            "finally" => TokenKind::Finally,
            "float" => TokenKind::Float,
            "for" => TokenKind::For,
            "goto" => TokenKind::Goto,
            "if" => TokenKind::If,
            "implements" => TokenKind::Implements,
            "import" => TokenKind::Import,
            "instanceof" => TokenKind::Instanceof,
            "int" => TokenKind::Int,
            "interface" => TokenKind::Interface,
            "long" => TokenKind::Long,
            "native" => TokenKind::Native,
            "new" => TokenKind::New,
            "package" => TokenKind::Package,
            "private" => TokenKind::Private,
            "protected" => TokenKind::Protected,
            "public" => TokenKind::Public,
            "return" => TokenKind::Return,
            "short" => TokenKind::Short,
            "static" => TokenKind::Static,
            "strictfp" => TokenKind::Strictfp,
            "super" => TokenKind::Super,
            "switch" => TokenKind::Switch,
            "synchronized" => TokenKind::Synchronized,
            "this" => TokenKind::This,
            "throw" => TokenKind::Throw,
            "throws" => TokenKind::Throws,
            "transient" => TokenKind::Transient,
            "try" => TokenKind::Try,
            "void" => TokenKind::Void,
            "volatile" => TokenKind::Volatile,
            "while" => TokenKind::While,
            "_" => TokenKind::Underscore,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            _ => return None,
        })
    }

    /// The fixed text of keywords, separators, and operators, or `None` for kinds whose text
    /// varies.
    pub fn fixed_text(self) -> Option<&'static str> {
        Some(match self {
            TokenKind::Abstract => "abstract",
            TokenKind::Assert => "assert",
            TokenKind::Boolean => "boolean",
            TokenKind::Break => "break",
            TokenKind::Byte => "byte",
            TokenKind::Case => "case",
            TokenKind::Catch => "catch",
            TokenKind::Char => "char",
            TokenKind::Class => "class",
            TokenKind::Const => "const",
            TokenKind::Continue => "continue",
            TokenKind::Default => "default",
            TokenKind::Do => "do",
            TokenKind::Double => "double",
            TokenKind::Else => "else",
            TokenKind::Enum => "enum",
            TokenKind::Extends => "extends",
            TokenKind::Final => "final",
            TokenKind::Finally => "finally",
            TokenKind::Float => "float",
            TokenKind::For => "for",
            TokenKind::Goto => "goto",
            TokenKind::If => "if",
            TokenKind::Implements => "implements",
            TokenKind::Import => "import",
            TokenKind::Instanceof => "instanceof",
            TokenKind::Int => "int",
            TokenKind::Interface => "interface",
            TokenKind::Long => "long",
            TokenKind::Native => "native",
            TokenKind::New => "new",
            TokenKind::Package => "package",
            TokenKind::Private => "private",
            TokenKind::Protected => "protected",
            TokenKind::Public => "public",
            TokenKind::Return => "return",
            TokenKind::Short => "short",
            TokenKind::Static => "static",
            TokenKind::Strictfp => "strictfp",
            TokenKind::Super => "super",
            TokenKind::Switch => "switch",
            TokenKind::Synchronized => "synchronized",
            TokenKind::This => "this",
            TokenKind::Throw => "throw",
            TokenKind::Throws => "throws",
            TokenKind::Transient => "transient",
            TokenKind::Try => "try",
            TokenKind::Void => "void",
            TokenKind::Volatile => "volatile",
            TokenKind::While => "while",
            TokenKind::Underscore => "_",
            TokenKind::True => "true",
            TokenKind::False => "false",
            TokenKind::Null => "null",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Semi => ";",
            TokenKind::Comma => ",",
            TokenKind::Dot => ".",
            TokenKind::Ellipsis => "...",
            TokenKind::At => "@",
            TokenKind::ColonColon => "::",
            TokenKind::Eq => "=",
            TokenKind::Gt => ">",
            TokenKind::Lt => "<",
            TokenKind::Bang => "!",
            TokenKind::Tilde => "~",
            TokenKind::Question => "?",
            TokenKind::Colon => ":",
            TokenKind::Arrow => "->",
            TokenKind::EqEq => "==",
            TokenKind::GtEq => ">=",
            TokenKind::LtEq => "<=",
            TokenKind::BangEq => "!=",
            TokenKind::AmpAmp => "&&",
            TokenKind::PipePipe => "||",
            TokenKind::PlusPlus => "++",
            TokenKind::MinusMinus => "--",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Amp => "&",
            TokenKind::Pipe => "|",
            TokenKind::Caret => "^",
            TokenKind::Percent => "%",
            TokenKind::LtLt => "<<",
            TokenKind::PlusEq => "+=",
            TokenKind::MinusEq => "-=",
            TokenKind::StarEq => "*=",
            TokenKind::SlashEq => "/=",
            TokenKind::AmpEq => "&=",
            TokenKind::PipeEq => "|=",
            TokenKind::CaretEq => "^=",
            TokenKind::PercentEq => "%=",
            TokenKind::LtLtEq => "<<=",
            _ => return None,
        })
    }
}

/// Identifiers that act as keywords in some contexts (JLS §3.9).
///
/// `non-sealed` is lexed as three tokens, `non`, `-`, and `sealed`, with no gaps; the parser joins
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextualKeyword {
    Exports,
    Module,
    NonSealed,
    Open,
    Opens,
    Permits,
    Provides,
    Record,
    Requires,
    Sealed,
    To,
    Transitive,
    Uses,
    Var,
    When,
    With,
    Yield,
}

impl ContextualKeyword {
    /// The contextual keyword an identifier spells, given its text after Unicode escapes are
    /// translated (see [`cooked_text`](crate::cooked_text)). `non-sealed` is never returned, since
    /// it isn't a single identifier.
    pub fn from_identifier(text: &str) -> Option<ContextualKeyword> {
        Some(match text {
            "exports" => ContextualKeyword::Exports,
            "module" => ContextualKeyword::Module,
            "open" => ContextualKeyword::Open,
            "opens" => ContextualKeyword::Opens,
            "permits" => ContextualKeyword::Permits,
            "provides" => ContextualKeyword::Provides,
            "record" => ContextualKeyword::Record,
            "requires" => ContextualKeyword::Requires,
            "sealed" => ContextualKeyword::Sealed,
            "to" => ContextualKeyword::To,
            "transitive" => ContextualKeyword::Transitive,
            "uses" => ContextualKeyword::Uses,
            "var" => ContextualKeyword::Var,
            "when" => ContextualKeyword::When,
            "with" => ContextualKeyword::With,
            "yield" => ContextualKeyword::Yield,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_round_trip() {
        for text in ["class", "while", "_", "true", "null", "strictfp", "goto"] {
            let kind = TokenKind::from_keyword(text).unwrap();
            assert_eq!(kind.fixed_text(), Some(text));
        }
        for text in ["var", "record", "Class", "", "non-sealed"] {
            assert_eq!(TokenKind::from_keyword(text), None);
        }
    }

    #[test]
    fn contextual_keywords() {
        assert_eq!(
            ContextualKeyword::from_identifier("yield"),
            Some(ContextualKeyword::Yield)
        );
        assert_eq!(ContextualKeyword::from_identifier("non-sealed"), None);
        assert_eq!(ContextualKeyword::from_identifier("class"), None);
    }
}
