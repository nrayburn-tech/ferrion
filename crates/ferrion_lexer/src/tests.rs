//! Lexer tests. Where javac's behavior is cited, it was observed by running javac 27 on the input.

use super::*;
use LexErrorKind as E;
use TokenKind as K;

/// Lexes `text`, checks the invariants every result must satisfy, and returns it.
fn lex_checked(text: &str) -> Lexed {
    let lexed = lex(text);
    let mut offset = 0;
    for token in &lexed.tokens {
        assert_eq!(
            token.span.start, offset,
            "gap or overlap before {token:?} in {text:?}"
        );
        assert!(token.span.end >= token.span.start);
        assert!(text.is_char_boundary(token.span.end as usize));
        offset = token.span.end;
    }
    assert_eq!(offset as usize, text.len(), "tokens don't cover {text:?}");
    let last = lexed.tokens.last().expect("no tokens");
    assert_eq!(last.kind, K::Eof);
    assert!(last.span.is_empty());
    assert!(
        lexed.tokens[..lexed.tokens.len() - 1]
            .iter()
            .all(|t| t.kind != K::Eof && !t.span.is_empty()),
        "an empty token or an early Eof in {text:?}"
    );
    assert!(
        lexed
            .errors
            .windows(2)
            .all(|w| w[0].span.start <= w[1].span.start),
        "errors out of order in {text:?}"
    );
    for error in &lexed.errors {
        assert!(error.span.end as usize <= text.len());
    }
    lexed
}

/// Every token but `Eof`, with its text.
fn all(text: &str) -> Vec<(TokenKind, &str)> {
    let lexed = lex_checked(text);
    lexed.tokens[..lexed.tokens.len() - 1]
        .iter()
        .map(|t| (t.kind, &text[t.span.range()]))
        .collect()
}

/// Tokens that aren't whitespace or line terminators (comments included), with their text.
fn toks(text: &str) -> Vec<(TokenKind, &str)> {
    all(text)
        .into_iter()
        .filter(|(kind, _)| !matches!(kind, K::Whitespace | K::LineTerminator))
        .collect()
}

fn kinds(text: &str) -> Vec<TokenKind> {
    toks(text).into_iter().map(|(kind, _)| kind).collect()
}

fn errs(text: &str) -> Vec<(LexErrorKind, &str)> {
    lex_checked(text)
        .errors
        .iter()
        .map(|e| (e.kind, &text[e.span.range()]))
        .collect()
}

/// Asserts that `text` is a single token of `kind` with no errors.
#[track_caller]
fn single(text: &str, kind: TokenKind) {
    assert_eq!(toks(text), [(kind, text)], "{text:?}");
    assert_eq!(errs(text), [], "{text:?}");
}

/// The cooked text of each token but `Eof`.
fn cooked(text: &str) -> Vec<String> {
    let lexed = lex_checked(text);
    lexed.tokens[..lexed.tokens.len() - 1]
        .iter()
        .map(|&t| cooked_text(text, t).into_owned())
        .collect()
}

mod basics {
    use super::*;

    #[test]
    fn empty() {
        let lexed = lex_checked("");
        assert_eq!(
            lexed.tokens,
            [Token {
                kind: K::Eof,
                span: Span::empty(0)
            }]
        );
        assert_eq!(lexed.errors, []);
    }

    #[test]
    fn class_declaration() {
        assert_eq!(
            all("public class A {\n  int x = 1;\n}\n"),
            [
                (K::Public, "public"),
                (K::Whitespace, " "),
                (K::Class, "class"),
                (K::Whitespace, " "),
                (K::Identifier, "A"),
                (K::Whitespace, " "),
                (K::LBrace, "{"),
                (K::LineTerminator, "\n"),
                (K::Whitespace, "  "),
                (K::Int, "int"),
                (K::Whitespace, " "),
                (K::Identifier, "x"),
                (K::Whitespace, " "),
                (K::Eq, "="),
                (K::Whitespace, " "),
                (K::IntLiteral, "1"),
                (K::Semi, ";"),
                (K::LineTerminator, "\n"),
                (K::RBrace, "}"),
                (K::LineTerminator, "\n"),
            ]
        );
    }

    #[test]
    fn eof_is_at_the_end() {
        let lexed = lex_checked("a ");
        assert_eq!(lexed.tokens.last().unwrap().span, Span::empty(2));
    }
}

mod whitespace_and_line_terminators {
    use super::*;

    #[test]
    fn whitespace_runs() {
        assert_eq!(all(" \t\x0C "), [(K::Whitespace, " \t\x0C ")]);
        assert_eq!(
            all("a \t b"),
            [
                (K::Identifier, "a"),
                (K::Whitespace, " \t "),
                (K::Identifier, "b")
            ]
        );
    }

    #[test]
    fn one_token_per_line_terminator() {
        assert_eq!(
            all("\n\r\r\n\n\r"),
            [
                (K::LineTerminator, "\n"),
                (K::LineTerminator, "\r"),
                (K::LineTerminator, "\r\n"),
                (K::LineTerminator, "\n"),
                (K::LineTerminator, "\r"),
            ]
        );
        assert_eq!(
            all("\n\n"),
            [(K::LineTerminator, "\n"), (K::LineTerminator, "\n")]
        );
    }

    #[test]
    fn escaped_line_terminators() {
        assert_eq!(
            all(r"\u000a\u000d\u000a\u000d"),
            [
                (K::LineTerminator, r"\u000a"),
                (K::LineTerminator, r"\u000d\u000a"),
                (K::LineTerminator, r"\u000d"),
            ]
        );
        assert_eq!(all("\r\\u000a"), [(K::LineTerminator, "\r\\u000a")]);
        assert_eq!(all(r"\u0020\u0009"), [(K::Whitespace, r"\u0020\u0009")]);
    }

    #[test]
    fn other_space_characters_are_illegal() {
        // javac reports these as illegal characters: vertical tab, no-break space, line separator.
        for text in ["\x0B", "\u{A0}", "\u{2028}", "\u{3000}"] {
            assert_eq!(toks(text), [(K::Error, text)]);
            assert_eq!(errs(text), [(E::IllegalCharacter, text)]);
        }
    }

    #[test]
    fn sub_at_the_end_is_whitespace() {
        // JLS §3.5; javac ignores a SUB only if it is the last character as written.
        assert_eq!(
            all("a\n\x1A"),
            [
                (K::Identifier, "a"),
                (K::LineTerminator, "\n"),
                (K::Whitespace, "\x1A")
            ]
        );
        assert_eq!(errs("a\n\x1A"), []);
        assert_eq!(all("\x1A"), [(K::Whitespace, "\x1A")]);
    }

    #[test]
    fn sub_elsewhere() {
        assert_eq!(
            all("\x1A\x1A"),
            [(K::Error, "\x1A"), (K::Whitespace, "\x1A")]
        );
        assert_eq!(errs("\x1A\x1A"), [(E::IllegalCharacter, "\x1A")]);
        assert_eq!(
            toks("{\x1A}"),
            [(K::LBrace, "{"), (K::Error, "\x1A"), (K::RBrace, "}")]
        );
        // Escaped, it's never ignored.
        assert_eq!(errs("a \\u001a"), [(E::IllegalCharacter, "\\u001a")]);
        // SUB is identifier-ignorable, so javac takes it into an identifier, even at the end.
        assert_eq!(toks("a\x1A"), [(K::Identifier, "a\x1A")]);
        // In comments and literals, it's content.
        assert_eq!(toks("// a\x1A"), [(K::LineComment, "// a\x1A")]);
        assert_eq!(toks("/* a\x1A"), [(K::BlockComment, "/* a\x1A")]);
        assert_eq!(errs("/* a\x1A"), [(E::UnterminatedComment, "/*")]);
        assert_eq!(toks("\"a\x1A"), [(K::StringLiteral, "\"a\x1A")]);
    }
}

mod unicode_escapes {
    use super::*;

    #[test]
    fn escapes_are_translated_before_tokens_are_formed() {
        assert_eq!(toks(r"\u0063lass"), [(K::Class, r"\u0063lass")]);
        assert_eq!(toks(r"\uuuu0063lass"), [(K::Class, r"\uuuu0063lass")]);
        assert_eq!(toks(r"\u005f"), [(K::Underscore, r"\u005f")]);
        assert_eq!(
            toks(r"a\u002bb"),
            [
                (K::Identifier, "a"),
                (K::Plus, r"\u002b"),
                (K::Identifier, "b")
            ]
        );
        assert_eq!(toks(r"\u0031\u0032L"), [(K::LongLiteral, r"\u0031\u0032L")]);
        assert_eq!(toks(r"-\u003e"), [(K::Arrow, r"-\u003e")]);
        assert_eq!(
            toks(r"\u003e\u003e"),
            [(K::Gt, r"\u003e"), (K::Gt, r"\u003e")]
        );
    }

    #[test]
    fn escapes_form_comment_delimiters() {
        assert_eq!(
            toks(r"\u002f\u002f x"),
            [(K::LineComment, r"\u002f\u002f x")]
        );
        assert_eq!(toks(r"/\u002a x */"), [(K::BlockComment, r"/\u002a x */")]);
        assert_eq!(toks(r"/* x *\u002f"), [(K::BlockComment, r"/* x *\u002f")]);
        assert_eq!(
            toks(r"/* \u002a/ a */"),
            [
                (K::BlockComment, r"/* \u002a/"),
                (K::Identifier, "a"),
                (K::Star, "*"),
                (K::Slash, "/")
            ]
        );
        assert_eq!(
            toks(r"/\u002a\u002a doc */"),
            [(K::DocComment, r"/\u002a\u002a doc */")]
        );
    }

    #[test]
    fn escaped_line_terminator_ends_a_line_comment() {
        assert_eq!(
            all(r"// x\u000aint b;"),
            [
                (K::LineComment, "// x"),
                (K::LineTerminator, r"\u000a"),
                (K::Int, "int"),
                (K::Whitespace, " "),
                (K::Identifier, "b"),
                (K::Semi, ";"),
            ]
        );
    }

    #[test]
    fn escapes_form_quotes() {
        assert_eq!(toks(r#"\u0022abc""#), [(K::StringLiteral, r#"\u0022abc""#)]);
        assert_eq!(toks(r"\u0027a'"), [(K::CharLiteral, r"\u0027a'")]);
        // `\u0027` is the closing quote here, so the literal is empty, and the second `'` opens
        // another literal (as in javac).
        assert_eq!(
            toks(r"'\u0027'"),
            [(K::CharLiteral, r"'\u0027"), (K::CharLiteral, "'")]
        );
        assert_eq!(
            errs(r"'\u0027'"),
            [
                (E::EmptyCharLiteral, r"'\u0027"),
                (E::UnterminatedCharLiteral, "'")
            ]
        );
    }

    #[test]
    fn escaped_backslash_starts_an_escape_sequence_but_no_unicode_escape() {
        // `\u005c"` is `\"`, so the string doesn't end there.
        assert_eq!(toks(r#""\u005c" ""#), [(K::StringLiteral, r#""\u005c" ""#)]);
        single(r#""\u005c\u005c""#, K::StringLiteral);
        single(r"'\u005c\u005c'", K::CharLiteral);
        // `\u005cu0041` is `\`, `u`, `0`, `0`, `4`, `1`: in a string, an invalid escape `\u`.
        assert_eq!(errs(r#""\u005cu0041""#), [(E::InvalidEscape, r"\u005cu")]);
        assert_eq!(
            toks(r"\u005cu0041"),
            [(K::Error, r"\u005c"), (K::Identifier, "u0041")]
        );
    }

    #[test]
    fn even_backslash_rule() {
        single(r#""\\u0041""#, K::StringLiteral);
        assert_eq!(cooked(r#""\\u0041""#), [r#""\\u0041""#]);
        assert_eq!(
            toks(r"\\u0041"),
            [(K::Error, "\\"), (K::Error, "\\"), (K::Identifier, "u0041")]
        );
        assert_eq!(
            toks(r"\\\u0041"),
            [
                (K::Error, "\\"),
                (K::Error, "\\"),
                (K::Identifier, r"\u0041")
            ]
        );
    }

    #[test]
    fn surrogate_pairs() {
        assert_eq!(toks(r"\uD835\uDC00"), [(K::Identifier, r"\uD835\uDC00")]);
        assert_eq!(cooked(r"\uD835\uDC00"), ["𝐀"]);
        single("𝐀", K::Identifier);
        // A lone surrogate is fine in a literal, and isn't a Java letter.
        single(r"'\uD800'", K::CharLiteral);
        assert_eq!(cooked(r"'\uD800'"), ["'\u{FFFD}'"]);
        assert_eq!(errs(r"\uD800"), [(E::IllegalCharacter, r"\uD800")]);
    }

    #[test]
    fn invalid_escapes_are_reported_and_skipped() {
        // javac drops an invalid escape after reporting it; its text joins the token before it.
        assert_eq!(errs(r"\u12;"), [(E::InvalidUnicodeEscape, r"\u12")]);
        assert_eq!(toks(r"\u12;"), [(K::Error, r"\u12"), (K::Semi, ";")]);
        assert_eq!(
            toks(r"a = 1\u12;"),
            [
                (K::Identifier, "a"),
                (K::Eq, "="),
                (K::IntLiteral, r"1\u12"),
                (K::Semi, ";")
            ]
        );
        assert_eq!(
            toks(r"x \uu00g1"),
            [
                (K::Identifier, "x"),
                (K::Error, r"\uu00"),
                (K::Identifier, "g1")
            ]
        );
        assert_eq!(
            toks(r"\u12\u0031"),
            [(K::Error, r"\u12"), (K::IntLiteral, r"\u0031")]
        );
        assert_eq!(
            toks(r"ab\u12cd"),
            [(K::Identifier, r"ab\u12cd")],
            "a valid escape"
        );
        assert_eq!(toks(r"ab\u12xy"), [(K::Identifier, r"ab\u12xy")]);
        assert_eq!(cooked(r"ab\u12xy"), ["abxy"]);
        assert_eq!(toks(r#""a\u12b""#), [(K::StringLiteral, r#""a\u12b""#)]);
        assert_eq!(toks(r"/* \u12 */"), [(K::BlockComment, r"/* \u12 */")]);
        assert_eq!(toks(r"// \u12"), [(K::LineComment, r"// \u12")]);
        assert_eq!(errs(r"// \u12"), [(E::InvalidUnicodeEscape, r"\u12")]);
        assert_eq!(toks(r"\u"), [(K::Error, r"\u")]);
        assert_eq!(toks(r"\uuuu"), [(K::Error, r"\uuuu")]);
        assert_eq!(
            errs(r"\u\u"),
            [
                (E::InvalidUnicodeEscape, r"\u"),
                (E::InvalidUnicodeEscape, r"\u")
            ]
        );
    }

    #[test]
    fn invalid_escapes_inside_tokens() {
        // javac reads past an invalid escape as if it weren't there.
        assert_eq!(
            toks(r"b +\u12+ c"),
            [
                (K::Identifier, "b"),
                (K::PlusPlus, r"+\u12+"),
                (K::Identifier, "c")
            ]
        );
        assert_eq!(toks(r"0\u12x1"), [(K::IntLiteral, r"0\u12x1")]);
        assert_eq!(toks(r"1\u12.5"), [(K::DoubleLiteral, r"1\u12.5")]);
        assert_eq!(toks(r"'a\u12'"), [(K::CharLiteral, r"'a\u12'")]);
        assert_eq!(toks(r"'\u12'"), [(K::CharLiteral, r"'\u12'")]);
        assert_eq!(
            errs(r"'\u12'"),
            [
                (E::EmptyCharLiteral, r"'\u12'"),
                (E::InvalidUnicodeEscape, r"\u12")
            ]
        );
        assert_eq!(
            toks(r"a.\u12;"),
            [(K::Identifier, "a"), (K::Dot, r".\u12"), (K::Semi, ";")]
        );
    }

    #[test]
    fn invalid_escapes_stay_out_of_whitespace_and_line_terminators() {
        assert_eq!(
            all("a \\u12 \n\\u12z"),
            [
                (K::Identifier, "a"),
                (K::Whitespace, " "),
                (K::Error, r"\u12"),
                (K::Whitespace, " "),
                (K::LineTerminator, "\n"),
                (K::Error, r"\u12"),
                (K::Identifier, "z"),
            ]
        );
        assert_eq!(
            all("\r\\u12\n"),
            [
                (K::LineTerminator, "\r"),
                (K::Error, r"\u12"),
                (K::LineTerminator, "\n")
            ]
        );
    }

    #[test]
    fn errors_are_in_source_order() {
        assert_eq!(
            errs(r#"# "\q" \u12 #"#),
            [
                (E::IllegalCharacter, "#"),
                (E::InvalidEscape, r"\q"),
                (E::InvalidUnicodeEscape, r"\u12"),
                (E::IllegalCharacter, "#"),
            ]
        );
    }

    #[test]
    fn cooked_text_borrows_without_escapes() {
        let text = "abc \"\\n\"";
        let lexed = lex_checked(text);
        for &token in &lexed.tokens {
            assert!(matches!(cooked_text(text, token), Cow::Borrowed(_)));
        }
        let text = r"\u0061";
        let lexed = lex_checked(text);
        assert!(matches!(cooked_text(text, lexed.tokens[0]), Cow::Owned(_)));
    }

    #[test]
    fn cooked_text_translates_escapes() {
        assert_eq!(cooked(r"\u0061\uu0062c"), ["abc"]);
        assert_eq!(cooked(r#""\u0041\n""#), [r#""A\n""#]);
        // Text after a backslash inside a token is counted from the start of the text.
        assert_eq!(cooked(r#""\\\u0041""#), [r#""\\A""#]);
    }

    #[test]
    fn identifier_ignorable_characters() {
        // javac ignores them when comparing names, so these are the keyword `class`.
        assert_eq!(toks("cl\u{AD}ass"), [(K::Class, "cl\u{AD}ass")]);
        assert_eq!(toks(r"cl\u00ADass"), [(K::Class, r"cl\u00ADass")]);
        assert_eq!(cooked("a\u{200B}b\0c"), ["abc"]);
        assert_eq!(toks("a\0b"), [(K::Identifier, "a\0b")]);
        assert_eq!(toks("class\0"), [(K::Class, "class\0")]);
        // They don't start identifiers, though.
        assert_eq!(
            toks("\u{200B}b"),
            [(K::Error, "\u{200B}"), (K::Identifier, "b")]
        );
        // And only names drop them.
        assert_eq!(cooked("\"\u{AD}\""), ["\"\u{AD}\""]);
    }
}

mod comments {
    use super::*;

    #[test]
    fn line_comments() {
        assert_eq!(
            all("// a\n/// b\r\n//// c\n//"),
            [
                (K::LineComment, "// a"),
                (K::LineTerminator, "\n"),
                (K::DocLineComment, "/// b"),
                (K::LineTerminator, "\r\n"),
                (K::DocLineComment, "//// c"),
                (K::LineTerminator, "\n"),
                (K::LineComment, "//"),
            ]
        );
        single("///", K::DocLineComment);
        assert_eq!(
            toks("a // b"),
            [(K::Identifier, "a"), (K::LineComment, "// b")]
        );
    }

    #[test]
    fn block_comments() {
        single("/* a */", K::BlockComment);
        single("/**/", K::BlockComment);
        single("/*/ */", K::BlockComment);
        single("/* a **/", K::BlockComment);
        single("/* /* */", K::BlockComment);
        single("/*\n * a\r\n */", K::BlockComment);
        assert_eq!(
            toks("/* a */ */"),
            [
                (K::BlockComment, "/* a */"),
                (K::Star, "*"),
                (K::Slash, "/")
            ]
        );
    }

    #[test]
    fn doc_comments() {
        single("/** a */", K::DocComment);
        single("/***/", K::DocComment);
        single("/** */", K::DocComment);
        single("/**\n * @param a b\n */", K::DocComment);
    }

    #[test]
    fn unterminated() {
        for (text, opening) in [
            ("/*", "/*"),
            ("/* a", "/*"),
            ("/* a *", "/*"),
            ("/**", "/**"),
            ("/** a\n", "/**"),
            ("/*/", "/*"),
        ] {
            assert_eq!(all(text).len(), 1, "{text:?}");
            assert_eq!(errs(text), [(E::UnterminatedComment, opening)], "{text:?}");
        }
        assert_eq!(kinds("/** a"), [K::DocComment]);
    }

    #[test]
    fn slash_operators() {
        assert_eq!(
            toks("a / b /= c"),
            [
                (K::Identifier, "a"),
                (K::Slash, "/"),
                (K::Identifier, "b"),
                (K::SlashEq, "/="),
                (K::Identifier, "c")
            ]
        );
        assert_eq!(toks("/"), [(K::Slash, "/")]);
    }
}

mod identifiers_and_keywords {
    use super::*;

    #[test]
    fn identifiers() {
        for text in [
            "a", "A1", "$", "_a", "__", "a$b_c", "été", "名前", "€uro", "a\u{301}", "x٣",
        ] {
            single(text, K::Identifier);
        }
        assert_eq!(
            toks("1a"),
            [(K::IntLiteral, "1"), (K::Identifier, "a")],
            "no error here; the parser reports it"
        );
    }

    #[test]
    fn every_keyword() {
        let keywords = [
            "abstract",
            "assert",
            "boolean",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "class",
            "const",
            "continue",
            "default",
            "do",
            "double",
            "else",
            "enum",
            "extends",
            "final",
            "finally",
            "float",
            "for",
            "goto",
            "if",
            "implements",
            "import",
            "instanceof",
            "int",
            "interface",
            "long",
            "native",
            "new",
            "package",
            "private",
            "protected",
            "public",
            "return",
            "short",
            "static",
            "strictfp",
            "super",
            "switch",
            "synchronized",
            "this",
            "throw",
            "throws",
            "transient",
            "try",
            "void",
            "volatile",
            "while",
            "_",
            "true",
            "false",
            "null",
        ];
        for text in keywords {
            let kind = TokenKind::from_keyword(text).unwrap();
            single(text, kind);
            assert_eq!(
                toks(&format!("{text}x")),
                [(K::Identifier, &*format!("{text}x"))]
            );
        }
    }

    #[test]
    fn contextual_keywords_are_identifiers() {
        for text in [
            "var", "record", "yield", "sealed", "permits", "module", "open", "when", "exports",
        ] {
            single(text, K::Identifier);
        }
        assert_eq!(
            toks("non-sealed"),
            [
                (K::Identifier, "non"),
                (K::Minus, "-"),
                (K::Identifier, "sealed")
            ]
        );
    }

    #[test]
    fn keywords_are_case_sensitive() {
        single("Class", K::Identifier);
        single("NULL", K::Identifier);
    }
}

mod numbers {
    use super::*;

    #[test]
    fn integers() {
        for text in [
            "0",
            "7",
            "123",
            "1_000",
            "1__2",
            "00",
            "007",
            "0_7",
            "0__7",
            "0x0",
            "0xFF",
            "0XfF",
            "0x1_F",
            "0xCAFE_BABE",
            "0b0",
            "0B101",
            "0b1__0",
            "2147483648",
            "99999999999",
        ] {
            single(text, K::IntLiteral);
        }
        for text in ["0L", "0l", "1_000L", "0xFFL", "0xFF_FFl", "0b1L", "017L"] {
            single(text, K::LongLiteral);
        }
    }

    #[test]
    fn floating_point() {
        for text in [
            "1.",
            "1.5",
            ".5",
            "0.0",
            "1e5",
            "1E5",
            "1e+5",
            "1e-5",
            "1.e5",
            ".5e1",
            "1d",
            "1D",
            "1.5d",
            "09.5",
            "09e1",
            "09d",
            "1_0.0_1",
            "0x1p3",
            "0X1P-3",
            "0x1.8p1",
            "0x.8p1",
            "0x1.p1",
            "0x1_0p1_0",
            "1e1_0",
        ] {
            single(text, K::DoubleLiteral);
        }
        for text in [
            "1f", "1F", "1.5f", ".5F", "1e5f", "09f", "0.f", "0x1p3f", "0x1P-3F",
        ] {
            single(text, K::FloatLiteral);
        }
    }

    #[test]
    fn malformed() {
        // (text, kind, the text of each error), with boundaries as javac lexes them.
        let cases: &[(&str, TokenKind, &[&str])] = &[
            ("0x", K::IntLiteral, &["0x"]),
            ("0xL", K::LongLiteral, &["0xL"]),
            ("0x_1", K::IntLiteral, &["_"]),
            ("0x1_", K::IntLiteral, &["_"]),
            ("0x1.", K::DoubleLiteral, &["0x1."]),
            ("0x1.8", K::DoubleLiteral, &["0x1.8"]),
            ("0x1p", K::DoubleLiteral, &["0x1p"]),
            ("0x1p+", K::DoubleLiteral, &["0x1p+"]),
            ("0x1pd", K::DoubleLiteral, &["0x1pd"]),
            ("0x1.8f", K::DoubleLiteral, &["0x1.8f"]),
            ("0x1.8pf", K::FloatLiteral, &["0x1.8pf"]),
            ("0x.", K::DoubleLiteral, &["0x."]),
            ("0b", K::IntLiteral, &["0b"]),
            ("0b2", K::IntLiteral, &["2"]),
            ("0b1012", K::IntLiteral, &["2"]),
            ("0b_1", K::IntLiteral, &["_"]),
            ("0b1_", K::IntLiteral, &["_"]),
            ("0b1__", K::IntLiteral, &["__"]),
            ("08", K::IntLiteral, &["8"]),
            ("0_8", K::IntLiteral, &["8"]),
            ("0779", K::IntLiteral, &["9"]),
            ("08L", K::LongLiteral, &["8"]),
            ("0_", K::IntLiteral, &["_"]),
            ("1_", K::IntLiteral, &["_"]),
            ("1__", K::IntLiteral, &["__"]),
            ("01_", K::IntLiteral, &["_"]),
            ("1_L", K::LongLiteral, &["_"]),
            ("1_.5", K::DoubleLiteral, &["_"]),
            ("1._5", K::DoubleLiteral, &["_"]),
            ("1.5_", K::DoubleLiteral, &["_"]),
            ("1e", K::DoubleLiteral, &["1e"]),
            ("1e+", K::DoubleLiteral, &["1e+"]),
            ("1e-f", K::FloatLiteral, &["1e-f"]),
            ("1e_5", K::DoubleLiteral, &["_"]),
            ("1e5_", K::DoubleLiteral, &["_"]),
            ("1_e5", K::DoubleLiteral, &["_"]),
        ];
        for &(text, kind, errors) in cases {
            assert_eq!(toks(text), [(kind, text)], "{text:?}");
            let expected: Vec<_> = errors.iter().map(|&e| (E::MalformedNumber, e)).collect();
            assert_eq!(errs(text), expected, "{text:?}");
        }
    }

    #[test]
    fn where_literals_end() {
        let cases: &[(&str, &[(TokenKind, &str)])] = &[
            ("1.5L", &[(K::DoubleLiteral, "1.5"), (K::Identifier, "L")]),
            ("0x1G", &[(K::IntLiteral, "0x1"), (K::Identifier, "G")]),
            ("0xp1", &[(K::IntLiteral, "0x"), (K::Identifier, "p1")]),
            ("0x.p1", &[(K::DoubleLiteral, "0x."), (K::Identifier, "p1")]),
            ("0b1.5", &[(K::IntLiteral, "0b1"), (K::DoubleLiteral, ".5")]),
            ("0b1e5", &[(K::IntLiteral, "0b1"), (K::Identifier, "e5")]),
            ("0b1f", &[(K::IntLiteral, "0b1"), (K::Identifier, "f")]),
            ("0x1e5", &[(K::IntLiteral, "0x1e5")]),
            (
                "1e1.5",
                &[(K::DoubleLiteral, "1e1"), (K::DoubleLiteral, ".5")],
            ),
            (
                "1..2",
                &[(K::DoubleLiteral, "1."), (K::DoubleLiteral, ".2")],
            ),
            // javac lexes `..` as one illegal token here.
            (
                "1...2",
                &[
                    (K::DoubleLiteral, "1."),
                    (K::Dot, "."),
                    (K::DoubleLiteral, ".2"),
                ],
            ),
            ("1ee5", &[(K::DoubleLiteral, "1e"), (K::Identifier, "e5")]),
            (
                "1e+-5",
                &[
                    (K::DoubleLiteral, "1e+"),
                    (K::Minus, "-"),
                    (K::IntLiteral, "5"),
                ],
            ),
            ("0_x1", &[(K::IntLiteral, "0_"), (K::Identifier, "x1")]),
            ("1LL", &[(K::LongLiteral, "1L"), (K::Identifier, "L")]),
            ("1fd", &[(K::FloatLiteral, "1f"), (K::Identifier, "d")]),
            ("1.f2", &[(K::FloatLiteral, "1.f"), (K::IntLiteral, "2")]),
            ("a.5", &[(K::Identifier, "a"), (K::DoubleLiteral, ".5")]),
            ("._5", &[(K::Dot, "."), (K::Identifier, "_5")]),
            ("-1", &[(K::Minus, "-"), (K::IntLiteral, "1")]),
        ];
        for &(text, expected) in cases {
            assert_eq!(toks(text), expected, "{text:?}");
        }
    }
}

mod char_literals {
    use super::*;

    #[test]
    fn valid() {
        for text in [
            "'a'",
            "' '",
            "'\"'",
            "'é'",
            r"'\n'",
            r"'\s'",
            r"'\b'",
            r"'\t'",
            r"'\f'",
            r"'\r'",
            r"'\''",
            r#"'\"'"#,
            r"'\\'",
            r"'\0'",
            r"'\7'",
            r"'\77'",
            r"'\377'",
            r"'\u0041'",
            r"'\u005c\u005c'",
            r"'\uFFFF'",
        ] {
            single(text, K::CharLiteral);
        }
    }

    #[test]
    fn empty() {
        assert_eq!(toks("''"), [(K::CharLiteral, "''")]);
        assert_eq!(errs("''"), [(E::EmptyCharLiteral, "''")]);
    }

    #[test]
    fn more_than_one_character() {
        for text in [
            "'ab'",
            "'abc'",
            r"'\400'",
            r"'\1234'",
            "'😀'",
            r"'\uD83D\uDE00'",
            r"'a\''",
        ] {
            assert_eq!(toks(text), [(K::CharLiteral, text)], "{text:?}");
            assert_eq!(
                errs(text),
                [(E::MultipleCharsInCharLiteral, text)],
                "{text:?}"
            );
        }
        assert_eq!(
            toks("'ab' + 'c'"),
            [
                (K::CharLiteral, "'ab'"),
                (K::Plus, "+"),
                (K::CharLiteral, "'c'")
            ]
        );
    }

    #[test]
    fn unterminated() {
        // Without a closing quote on the line, the literal ends after its character, as in javac.
        let cases: &[(&str, &[(TokenKind, &str)])] = &[
            ("'", &[(K::CharLiteral, "'")]),
            ("'a", &[(K::CharLiteral, "'a")]),
            ("'a ;", &[(K::CharLiteral, "'a"), (K::Semi, ";")]),
            (
                "'ab\n'",
                &[
                    (K::CharLiteral, "'a"),
                    (K::Identifier, "b"),
                    (K::CharLiteral, "'"),
                ],
            ),
            (r"'\'", &[(K::CharLiteral, r"'\'")]),
            ("'\n", &[(K::CharLiteral, "'")]),
        ];
        for &(text, expected) in cases {
            assert_eq!(toks(text), expected, "{text:?}");
            assert_eq!(
                errs(text)[0],
                (E::UnterminatedCharLiteral, expected[0].1),
                "{text:?}"
            );
        }
    }

    #[test]
    fn line_terminator() {
        assert_eq!(toks("'\n'"), [(K::CharLiteral, "'\n'")]);
        assert_eq!(errs("'\n'"), [(E::LineTerminatorInCharLiteral, "\n")]);
        assert_eq!(toks("'\r\n'"), [(K::CharLiteral, "'\r\n'")]);
        assert_eq!(toks(r"'\u000a'"), [(K::CharLiteral, r"'\u000a'")]);
        assert_eq!(
            toks(r"'\u000d\u000a'"),
            [(K::CharLiteral, r"'\u000d\u000a'")]
        );
        assert_eq!(
            errs(r"'\u000a'"),
            [(E::LineTerminatorInCharLiteral, r"\u000a")]
        );
    }

    #[test]
    fn invalid_escapes() {
        assert_eq!(toks(r"'\q'"), [(K::CharLiteral, r"'\q'")]);
        assert_eq!(errs(r"'\q'"), [(E::InvalidEscape, r"\q")]);
    }

    #[test]
    fn escaped_line_terminator_closed_on_the_next_line() {
        // javac makes each of these one literal, with `illegal.esc.char` at the line terminator,
        // and keeps the `;`. A backslash written as an escape behaves the same.
        let cases = [
            ("'\\\n';", "\\\n"),
            ("'\\\r\n';", "\\\r\n"),
            ("'\\\r';", "\\\r"),
            ("'\\u005c\n';", "\\u005c\n"),
            ("'\\u005c\\u000a\\u0027;", "\\u005c\\u000a"),
        ];
        for (text, escape) in cases {
            let literal = &text[..text.len() - 1];
            assert_eq!(
                toks(text),
                [(K::CharLiteral, literal), (K::Semi, ";")],
                "{text:?}"
            );
            assert_eq!(errs(text), [(E::InvalidEscape, escape)], "{text:?}");
        }
    }

    #[test]
    fn escaped_line_terminator_not_closed_on_the_next_line() {
        // Without a `'` right at the start of the next line, the literal ends before the line
        // terminator, as `'` and a line terminator does. (javac's literal runs on, to the next `'`
        // or the end of the file.)
        let cases: &[(&str, &[(TokenKind, &str)])] = &[
            (
                "'\\\nx';",
                &[
                    (K::CharLiteral, "'\\"),
                    (K::Identifier, "x"),
                    (K::CharLiteral, "';"),
                ],
            ),
            (
                "'\\\n ';",
                &[(K::CharLiteral, "'\\"), (K::CharLiteral, "';")],
            ),
            ("'\\\n", &[(K::CharLiteral, "'\\")]),
            ("'\\\r\n", &[(K::CharLiteral, "'\\")]),
        ];
        for &(text, expected) in cases {
            assert_eq!(toks(text), expected, "{text:?}");
            assert_eq!(
                errs(text)[..2],
                [
                    (E::UnterminatedCharLiteral, "'\\"),
                    (E::InvalidEscape, "\\")
                ],
                "{text:?}"
            );
            // The same as without the backslash, apart from the backslash.
            let unescaped = text.replacen('\\', "", 1);
            let mut unescaped_expected = expected.to_vec();
            unescaped_expected[0].1 = "'";
            assert_eq!(toks(&unescaped), unescaped_expected, "{unescaped:?}");
        }
    }

    #[test]
    fn many_unterminated_literals_on_a_line_are_linear() {
        // Each `'` after an `\` could start a literal that looks for a closing quote.
        let text = "'a".to_string() + &r"\'".repeat(100_000);
        let lexed = lex_checked(&text);
        assert!(lexed.tokens.len() > 100_000);
    }
}

mod string_literals {
    use super::*;

    #[test]
    fn valid() {
        for text in [
            r#""""#,
            r#""abc""#,
            r#""'""#,
            r#""\s\b\t\n\f\r\"\'\\""#,
            r#""\7\77\377\400""#,
            r#""\u0041""#,
            "\"é😀\"",
        ] {
            single(text, K::StringLiteral);
        }
    }

    #[test]
    fn unterminated_ends_before_the_line_terminator() {
        assert_eq!(
            all("\"abc ;\n}"),
            [
                (K::StringLiteral, "\"abc ;"),
                (K::LineTerminator, "\n"),
                (K::RBrace, "}")
            ]
        );
        assert_eq!(
            errs("\"abc ;\n}"),
            [(E::UnterminatedStringLiteral, "\"abc ;")]
        );
        assert_eq!(toks("\""), [(K::StringLiteral, "\"")]);
        assert_eq!(
            kinds(r#""a\u000ab""#),
            [K::StringLiteral, K::Identifier, K::StringLiteral]
        );
    }

    #[test]
    fn invalid_escapes() {
        assert_eq!(
            errs(r#""\q\x""#),
            [(E::InvalidEscape, r"\q"), (E::InvalidEscape, r"\x")]
        );
        // A line continuation is only allowed in text blocks.
        assert_eq!(
            errs("\"\\\n\""),
            [
                (E::UnterminatedStringLiteral, "\"\\"),
                (E::InvalidEscape, "\\"),
                (E::UnterminatedStringLiteral, "\"")
            ]
        );
        assert_eq!(
            errs("\"\\"),
            [
                (E::UnterminatedStringLiteral, "\"\\"),
                (E::InvalidEscape, "\\")
            ]
        );
    }
}

mod text_blocks {
    use super::*;

    #[test]
    fn valid() {
        for text in [
            "\"\"\"\nabc\"\"\"",
            "\"\"\"\n\"\"\"",
            "\"\"\" \t\x0C\nabc\n\"\"\"",
            "\"\"\"\r\nabc\r\n\"\"\"",
            "\"\"\"\rabc\"\"\"",
            "\"\"\"\n \" \"\" \\\"\"\" \"\"\"",
            "\"\"\"\n\\\nabc\\\r\n\"\"\"",
            "\"\"\"\n\\s\\t\\u0041\\\\\\'\\\"\\0\\377\n\"\"\"",
            r#""""\u000aabc""""#,
            r#"\u0022\u0022\u0022
abc""""#,
        ] {
            single(text, K::TextBlock);
        }
    }

    #[test]
    fn closing_delimiter_is_the_first_three_quotes() {
        assert_eq!(
            toks("\"\"\"\na\"\"\"\""),
            [(K::TextBlock, "\"\"\"\na\"\"\""), (K::StringLiteral, "\"")]
        );
    }

    #[test]
    fn opening_delimiter_not_on_its_own_line() {
        // javac lexes the text after `"""` as tokens; the lexer keeps it in the text block.
        let cases: &[(&str, &str)] = &[
            ("\"\"\"abc\"\"\"", "abc"),
            ("\"\"\" abc\n\"\"\"", "abc"),
            ("\"\"\"a\nb\"\"\"", "a"),
            ("\"\"\"\"\"\"", "\"\"\""),
            ("\"\"\" \"\"\"", "\"\"\""),
            ("\"\"\"\\\nabc\"\"\"", "\"\"\""),
        ];
        for &(text, error) in cases {
            assert_eq!(toks(text), [(K::TextBlock, text)], "{text:?}");
            assert_eq!(
                errs(text),
                [(E::TextBlockOpeningNotOnOwnLine, error)],
                "{text:?}"
            );
        }
        assert_eq!(
            errs("\"\"\"abc"),
            [
                (E::UnterminatedTextBlock, "\"\"\""),
                (E::TextBlockOpeningNotOnOwnLine, "abc")
            ]
        );
    }

    #[test]
    fn unterminated() {
        for text in ["\"\"\"", "\"\"\"\n", "\"\"\"\nabc\"\"", "\"\"\"\n\\\"\"\""] {
            assert_eq!(kinds(text), [K::TextBlock], "{text:?}");
            assert!(
                errs(text).contains(&(E::UnterminatedTextBlock, "\"\"\"")),
                "{text:?}"
            );
        }
        assert_eq!(
            errs("\"\"\"\n\\"),
            [
                (E::UnterminatedTextBlock, "\"\"\""),
                (E::InvalidEscape, "\\")
            ]
        );
    }

    #[test]
    fn unterminated_ends_after_the_first_line() {
        // As in javac, so the rest of the file is still lexed.
        assert_eq!(
            toks("\"\"\"\na \\q\nb\n} x"),
            [
                (K::TextBlock, "\"\"\"\na \\q\n"),
                (K::Identifier, "b"),
                (K::RBrace, "}"),
                (K::Identifier, "x")
            ]
        );
        assert_eq!(
            errs("\"\"\"\na \\q\nb \\q"),
            [
                (E::UnterminatedTextBlock, "\"\"\""),
                (E::InvalidEscape, "\\q"),
                (E::IllegalCharacter, "\\")
            ]
        );
        // A line continuation doesn't end the line.
        assert_eq!(
            toks("\"\"\"\na \\\nb\r\nc"),
            [(K::TextBlock, "\"\"\"\na \\\nb\r\n"), (K::Identifier, "c")]
        );
        assert_eq!(
            toks("\"\"\"abc\nd"),
            [(K::TextBlock, "\"\"\"abc\n"), (K::Identifier, "d")]
        );
    }

    #[test]
    fn many_unterminated_text_blocks_are_linear() {
        // Each line holds an escaped quote and then a `"""` that opens another text block.
        let text = "\"\"\"\n".to_string() + &"\\\"\"\"\n".repeat(100_000);
        let lexed = lex_checked(&text);
        assert!(lexed.tokens.len() > 100_000);
    }

    #[test]
    fn invalid_escapes() {
        assert_eq!(errs("\"\"\"\n\\q\n\"\"\""), [(E::InvalidEscape, r"\q")]);
        assert_eq!(
            errs("\"\"\"\n\\u12\"\"\""),
            [(E::InvalidUnicodeEscape, r"\u12")]
        );
    }
}

mod operators {
    use super::*;

    #[test]
    fn separators() {
        assert_eq!(
            kinds("( ) { } [ ] ; , . ... @ ::"),
            [
                K::LParen,
                K::RParen,
                K::LBrace,
                K::RBrace,
                K::LBracket,
                K::RBracket,
                K::Semi,
                K::Comma,
                K::Dot,
                K::Ellipsis,
                K::At,
                K::ColonColon,
            ]
        );
    }

    #[test]
    fn every_operator() {
        for kind in [
            K::Eq,
            K::Gt,
            K::Lt,
            K::Bang,
            K::Tilde,
            K::Question,
            K::Colon,
            K::Arrow,
            K::EqEq,
            K::GtEq,
            K::LtEq,
            K::BangEq,
            K::AmpAmp,
            K::PipePipe,
            K::PlusPlus,
            K::MinusMinus,
            K::Plus,
            K::Minus,
            K::Star,
            K::Slash,
            K::Amp,
            K::Pipe,
            K::Caret,
            K::Percent,
            K::LtLt,
            K::PlusEq,
            K::MinusEq,
            K::StarEq,
            K::SlashEq,
            K::AmpEq,
            K::PipeEq,
            K::CaretEq,
            K::PercentEq,
            K::LtLtEq,
            K::LParen,
            K::Ellipsis,
            K::ColonColon,
        ] {
            single(kind.fixed_text().unwrap(), kind);
        }
    }

    #[test]
    fn greater_than_never_forms_longer_tokens() {
        let cases: &[(&str, &[TokenKind])] = &[
            (">>", &[K::Gt, K::Gt]),
            (">>>", &[K::Gt, K::Gt, K::Gt]),
            (">>=", &[K::Gt, K::GtEq]),
            (">>>=", &[K::Gt, K::Gt, K::GtEq]),
            (">==", &[K::GtEq, K::Eq]),
            (
                "List<List<T>>",
                &[
                    K::Identifier,
                    K::Lt,
                    K::Identifier,
                    K::Lt,
                    K::Identifier,
                    K::Gt,
                    K::Gt,
                ],
            ),
        ];
        for &(text, expected) in cases {
            assert_eq!(kinds(text), expected, "{text:?}");
        }
        // No gaps, so the parser can join them.
        let lexed = lex_checked(">>>=");
        assert_eq!(lexed.tokens[2].span, Span::new(2, 4));
    }

    #[test]
    fn maximal_munch() {
        let cases: &[(&str, &[TokenKind])] = &[
            ("<<<=", &[K::LtLt, K::LtEq]),
            ("a--b", &[K::Identifier, K::MinusMinus, K::Identifier]),
            (
                "a---b",
                &[K::Identifier, K::MinusMinus, K::Minus, K::Identifier],
            ),
            (
                "a+++b",
                &[K::Identifier, K::PlusPlus, K::Plus, K::Identifier],
            ),
            ("->-", &[K::Arrow, K::Minus]),
            ("===", &[K::EqEq, K::Eq]),
            ("&&&", &[K::AmpAmp, K::Amp]),
            ("|||=", &[K::PipePipe, K::PipeEq]),
            (":::", &[K::ColonColon, K::Colon]),
            ("....", &[K::Ellipsis, K::Dot]),
            // `..` isn't a token in the JLS (javac reports it as one illegal token).
            ("..", &[K::Dot, K::Dot]),
            ("!==", &[K::BangEq, K::Eq]),
        ];
        for &(text, expected) in cases {
            assert_eq!(kinds(text), expected, "{text:?}");
        }
    }

    #[test]
    fn illegal_characters() {
        for text in ["#", "`", "\\", "\0", "\u{7F}", "\u{FEFF}", "→", "\u{1F600}"] {
            assert_eq!(toks(text), [(K::Error, text)], "{text:?}");
            assert_eq!(errs(text), [(E::IllegalCharacter, text)], "{text:?}");
        }
        // One per character, as in javac.
        assert_eq!(kinds(r"\\\\ ##"), [K::Error; 6]);
    }
}

mod robustness {
    use super::*;

    /// A small deterministic generator, so the test needs no dependencies and is reproducible.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    const PIECES: &[&str] = &[
        "\\", "u", "\\u", "\\u0041", "\\u000a", "\\u000d", "\\u002f", "\\u002a", "\\u0022",
        "\\u0027", "\\u005c", "\\uD83D", "\\uDE00", "\\u12", "'", "\"", "\"\"\"", "/", "*", "/*",
        "*/", "//", "\n", "\r", " ", "\t", "0", "1", "x", "e", "p", ".", "_", "L", "f", "a", "é",
        "😀", "\x1A", "\0", "#", ">", "=", "-", "\u{AD}", "\u{2028}",
    ];

    #[test]
    fn random_inputs_keep_the_invariants() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..20_000 {
            let len = (rng.next() % 16) as usize;
            let text: String = (0..len)
                .map(|_| PIECES[(rng.next() % PIECES.len() as u64) as usize])
                .collect();
            let lexed = lex_checked(&text);
            for &token in &lexed.tokens {
                cooked_text(&text, token);
            }
        }
    }

    #[test]
    fn long_pathological_inputs() {
        let n = 200_000;
        for text in [
            "\\".repeat(n),
            "\\".repeat(n) + "u0041",
            r"\u".repeat(n),
            r"\u12".repeat(n),
            "/*".repeat(n),
            "'".repeat(n),
            "\"".repeat(n),
            "\"\"\"".repeat(n),
            "'a".repeat(n),
            r"'\".repeat(n),
            "0x".repeat(n),
            "1_".repeat(n),
            "a".repeat(n),
            ">".repeat(n),
            "\u{AD}".repeat(n),
            "x".to_string() + &"\u{AD}".repeat(n),
            "\r".repeat(n),
            "\r\\u000a".repeat(n),
        ] {
            let lexed = lex_checked(&text);
            for &token in &lexed.tokens {
                cooked_text(&text, token);
            }
        }
    }
}
