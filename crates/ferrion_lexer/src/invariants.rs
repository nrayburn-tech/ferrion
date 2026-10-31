//! Checks that the lexer keeps the promises its API documents, for the fuzz targets in `fuzz/` and
//! the fuzz regression tests. Not part of the public API.
//!
//! Only documented promises are checked, so any failure is a bug in the lexer or its docs: tokens
//! cover the text without gaps or overlaps and end with one empty [`TokenKind::Eof`], errors are in
//! source order, no token boundary splits a character or a Unicode escape, [`cooked_text`]
//! translates exactly the escapes, and each token's cooked text fits its kind.

use std::borrow::Cow;

use ferrion_diagnostics::{LineCol, LineIndex, SourceText};

use crate::{LexError, Lexed, Span, Token, TokenKind, cooked_text, lex};

/// Reads `bytes` as a source file, and checks [`SourceText::to_bytes`] and [`check_text`]. Bytes
/// that aren't a supported source file are checked as their lossy UTF-8 text instead, so that
/// every input reaches the lexer.
pub fn check_bytes(bytes: &[u8]) -> Result<(), String> {
    match SourceText::decode(bytes) {
        Ok(source) => {
            if source.to_bytes() != bytes {
                return Err("SourceText::to_bytes doesn't reproduce the bytes".to_owned());
            }
            check_text(source.text())
        }
        Err(_) => check_text(&String::from_utf8_lossy(bytes)),
    }
}

/// Lexes `text` and checks the result with [`check`], and that [`LineIndex`] converts every token
/// boundary.
pub fn check_text(text: &str) -> Result<(), String> {
    let lexed = lex(text);
    check(text, &lexed)?;
    check_positions(text, &lexed.tokens)
}

/// Checks that `lexed` is a valid result of lexing `text`.
pub fn check(text: &str, lexed: &Lexed) -> Result<(), String> {
    let len = u32::try_from(text.len()).map_err(|_| "the text is too long".to_owned())?;
    check_tokens(text, len, &lexed.tokens)?;
    check_errors(text, len, &lexed.errors)
}

fn check_tokens(text: &str, len: u32, tokens: &[Token]) -> Result<(), String> {
    let escapes = unicode_escapes(text);
    let mut escapes = escapes.as_slice();
    let mut offset = 0;
    for (i, &token) in tokens.iter().enumerate() {
        let Span { start, end } = token.span;
        let fail = |problem: String| Err(format!("token {i}, {token:?}: {problem}"));
        if start > offset {
            return fail(format!(
                "gap after the previous token, which ends at {offset}"
            ));
        }
        if start < offset {
            return fail(format!(
                "overlaps the previous token, which ends at {offset}"
            ));
        }
        if end < start {
            return fail("ends before it starts".to_owned());
        }
        if end > len {
            return fail(format!("ends after the end of the text, {len}"));
        }
        if !text.is_char_boundary(end as usize) {
            return fail("ends inside a character".to_owned());
        }
        // The escapes that overlap the token, then the ones that end after it.
        let own = &escapes[..escapes.partition_point(|e| e.span.start < end)];
        escapes = &escapes[escapes.partition_point(|e| e.span.end <= end)..];
        if let Some(escape) = escapes.first()
            && escape.unit.is_some()
            && escape.span.start < end
        {
            return fail(format!(
                "ends inside the Unicode escape at {:?}",
                escape.span
            ));
        }
        let last = i + 1 == tokens.len();
        match (token.kind == TokenKind::Eof, last) {
            (true, true) if start == end => {}
            (true, true) => return fail("Eof isn't empty".to_owned()),
            (true, false) => return fail("Eof isn't the last token".to_owned()),
            (false, true) => return fail("the last token isn't Eof".to_owned()),
            (false, false) if start == end => return fail("empty".to_owned()),
            (false, false) => {
                check_token(text, token, own, cooked_text(text, token)).or_else(fail)?;
            }
        }
        offset = end;
    }
    if tokens.is_empty() {
        return Err("no tokens, not even Eof".to_owned());
    }
    if offset != len {
        return Err(format!(
            "the tokens end at {offset}, not at the end of the text, {len}"
        ));
    }
    let joined: String = tokens.iter().map(|t| &text[t.span.range()]).collect();
    if joined != text {
        return Err("concatenating the tokens doesn't reproduce the text".to_owned());
    }
    Ok(())
}

/// Checks a token's cooked text, given the Unicode escapes that overlap it.
fn check_token(
    text: &str,
    token: Token,
    escapes: &[Escape],
    cooked: Cow<'_, str>,
) -> Result<(), String> {
    // TODO: check Error tokens too once the lexer replaces the placeholder, whose cooked_text
    // doesn't translate escapes.
    if token.kind != TokenKind::Error {
        let raw = &text[token.span.range()];
        match translate(raw, token.span.start, escapes) {
            Some(expected) if expected != cooked => {
                return Err(format!("cooked text is {cooked:?}, not {expected:?}"));
            }
            _ => {}
        }
        if escapes.is_empty() && matches!(cooked, Cow::Owned(_)) {
            return Err("cooked text is owned, but the token has no Unicode escapes".to_owned());
        }
    }
    let cooked = cooked.as_ref();

    if let Some(fixed) = token.kind.fixed_text() {
        return if cooked == fixed {
            Ok(())
        } else {
            Err(format!(
                "cooked text {cooked:?} doesn't fit the kind, spelled {fixed:?}"
            ))
        };
    }
    let fits = match token.kind {
        TokenKind::Whitespace => cooked.chars().all(|c| matches!(c, ' ' | '\t' | '\x0C')),
        TokenKind::LineTerminator => matches!(cooked, "\n" | "\r" | "\r\n"),
        TokenKind::LineComment => cooked.starts_with("//") && !cooked.contains(['\r', '\n']),
        TokenKind::DocLineComment => cooked.starts_with("///") && !cooked.contains(['\r', '\n']),
        TokenKind::BlockComment => cooked.starts_with("/*"),
        TokenKind::DocComment => cooked.starts_with("/**") && cooked != "/**/",
        TokenKind::Identifier => TokenKind::from_keyword(cooked).is_none(),
        TokenKind::IntLiteral | TokenKind::DoubleLiteral => starts_number(cooked),
        TokenKind::LongLiteral => starts_number(cooked) && cooked.ends_with(['l', 'L']),
        TokenKind::FloatLiteral => starts_number(cooked) && cooked.ends_with(['f', 'F']),
        TokenKind::CharLiteral => cooked.starts_with('\''),
        TokenKind::StringLiteral => cooked.starts_with('"') && !cooked.starts_with("\"\"\""),
        TokenKind::TextBlock => cooked.starts_with("\"\"\""),
        _ => true,
    };
    if fits {
        Ok(())
    } else {
        Err(format!("cooked text {cooked:?} doesn't fit the kind"))
    }
}

fn starts_number(cooked: &str) -> bool {
    cooked.starts_with(|c: char| c.is_ascii_digit() || c == '.')
}

fn check_errors(text: &str, len: u32, errors: &[LexError]) -> Result<(), String> {
    let mut previous = 0;
    for (i, error) in errors.iter().enumerate() {
        let Span { start, end } = error.span;
        let fail = |problem: &str| Err(format!("error {i}, {error:?}: {problem}"));
        if end < start {
            return fail("ends before it starts");
        }
        if end > len {
            return fail("ends after the end of the text");
        }
        if !text.is_char_boundary(start as usize) || !text.is_char_boundary(end as usize) {
            return fail("splits a character");
        }
        if start < previous {
            return fail("starts before the previous error");
        }
        previous = start;
    }
    Ok(())
}

/// Checks that line and UTF-16 positions convert at every token boundary, and that a line
/// terminator written directly ends a line.
fn check_positions(text: &str, tokens: &[Token]) -> Result<(), String> {
    let index = LineIndex::new(text);
    let mut previous = LineCol { line: 0, col: 0 };
    for token in tokens {
        let offset = token.span.start;
        let position = index.line_col(offset);
        let fail = |problem: String| Err(format!("offset {offset}, {position:?}: {problem}"));
        if position < previous {
            return fail(format!(
                "before the previous token's position, {previous:?}"
            ));
        }
        previous = position;
        match index.line_start(position.line) {
            Some(start) if start <= offset => {}
            start => return fail(format!("the line starts at {start:?}")),
        }
        let utf16 = index.utf16_offset(offset);
        if index.byte_offset(utf16) != Some(offset) {
            return fail(format!("UTF-16 offset {utf16} doesn't convert back"));
        }
        if token.kind == TokenKind::LineTerminator
            && matches!(&text[token.span.range()], "\n" | "\r" | "\r\n")
        {
            let next = index.line_col(token.span.end);
            if next
                != (LineCol {
                    line: position.line + 1,
                    col: 0,
                })
            {
                return fail(format!("the line terminator is followed by {next:?}"));
            }
        }
    }
    let utf16_len = text.encode_utf16().count();
    if index.utf16_len() as usize != utf16_len {
        return Err(format!(
            "the UTF-16 length is {}, not {utf16_len}",
            index.utf16_len()
        ));
    }
    Ok(())
}

/// A Unicode escape (JLS §3.3) as written.
#[derive(Clone, Copy, Debug)]
struct Escape {
    span: Span,
    /// The UTF-16 code unit, or `None` if the `u`s aren't followed by four hex digits.
    unit: Option<u16>,
}

/// The text's Unicode escapes. A `\` begins one when it is followed by `u` and preceded by an even
/// number of `\`s.
fn unicode_escapes(text: &str) -> Vec<Escape> {
    let bytes = text.as_bytes();
    let mut escapes = Vec::new();
    let mut backslashes = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            backslashes = 0;
            i += 1;
            continue;
        }
        if backslashes % 2 == 1 || bytes.get(i + 1) != Some(&b'u') {
            backslashes += 1;
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while bytes.get(i) == Some(&b'u') {
            i += 1;
        }
        let digits = bytes[i..]
            .iter()
            .take(4)
            .take_while(|b| b.is_ascii_hexdigit());
        let digits = digits.count();
        let unit = (digits == 4)
            .then(|| u16::from_str_radix(&text[i..i + 4], 16).ok())
            .flatten();
        i += digits;
        escapes.push(Escape {
            span: Span::new(start as u32, i as u32),
            unit,
        });
        backslashes = 0;
    }
    escapes
}

/// `raw`, which starts at `offset`, with `escapes` translated, or `None` if one of them is
/// malformed or a surrogate, whose cooked text the API leaves open.
fn translate(raw: &str, offset: u32, escapes: &[Escape]) -> Option<String> {
    let mut cooked = String::with_capacity(raw.len());
    let mut copied = 0;
    for escape in escapes {
        let unit = escape.unit?;
        let start = escape.span.start.checked_sub(offset)? as usize;
        cooked.push_str(raw.get(copied..start)?);
        cooked.push(char::from_u32(unit.into())?);
        copied = (escape.span.end - offset) as usize;
    }
    cooked.push_str(raw.get(copied..)?);
    Some(cooked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LexErrorKind, TokenKind::*};

    /// Tokens of the given kinds and lengths, then Eof.
    fn lexed(tokens: &[(TokenKind, u32)]) -> Lexed {
        let mut offset = 0;
        let mut lexed = Lexed::default();
        for &(kind, len) in tokens {
            lexed.tokens.push(token(kind, offset, offset + len));
            offset += len;
        }
        lexed.tokens.push(token(Eof, offset, offset));
        lexed
    }

    fn token(kind: TokenKind, start: u32, end: u32) -> Token {
        Token {
            kind,
            span: Span::new(start, end),
        }
    }

    fn error(start: u32, end: u32) -> LexError {
        LexError {
            kind: LexErrorKind::IllegalCharacter,
            span: Span::new(start, end),
        }
    }

    fn rejects(text: &str, lexed: &Lexed, problem: &str) {
        match check(text, lexed) {
            Ok(()) => panic!("accepted {lexed:?} for {text:?}"),
            Err(message) => assert!(
                message.contains(problem),
                "expected {problem:?} in {message:?}"
            ),
        }
    }

    const CLASS: &str = "class A {\n  // hi\r\n}";

    fn class() -> Lexed {
        lexed(&[
            (Class, 5),
            (Whitespace, 1),
            (Identifier, 1),
            (Whitespace, 1),
            (LBrace, 1),
            (LineTerminator, 1),
            (Whitespace, 2),
            (LineComment, 5),
            (LineTerminator, 2),
            (RBrace, 1),
        ])
    }

    #[test]
    fn accepts_valid_tokens() {
        check(CLASS, &class()).unwrap();
        check_positions(CLASS, &class().tokens).unwrap();
        check("", &lexed(&[])).unwrap();
        let text = "/**/ /** a */ /// b\n'c' \"\" \"\"\"\n\"\"\" 1L 2f .5 0x1f é";
        let lexed = lexed(&[
            (BlockComment, 4),
            (Whitespace, 1),
            (DocComment, 8),
            (Whitespace, 1),
            (DocLineComment, 5),
            (LineTerminator, 1),
            (CharLiteral, 3),
            (Whitespace, 1),
            (StringLiteral, 2),
            (Whitespace, 1),
            (TextBlock, 7),
            (Whitespace, 1),
            (LongLiteral, 2),
            (Whitespace, 1),
            (FloatLiteral, 2),
            (Whitespace, 1),
            (DoubleLiteral, 2),
            (Whitespace, 1),
            (IntLiteral, 4),
            (Whitespace, 1),
            (Identifier, 2),
        ]);
        check(text, &lexed).unwrap();
        check_positions(text, &lexed.tokens).unwrap();
    }

    #[test]
    fn accepts_the_lexer() {
        for text in ["", "class A {}", "\u{FEFF}é\\u0041\r\n\\", "/* \"\"\" '"] {
            check_text(text).unwrap();
            check_bytes(text.as_bytes()).unwrap();
        }
        check_bytes(b"\xFF\xFE\x00").unwrap();
    }

    #[test]
    fn rejects_a_missing_or_misplaced_eof() {
        rejects(CLASS, &Lexed::default(), "no tokens");
        let mut lexed = class();
        lexed.tokens.pop();
        rejects(CLASS, &lexed, "the last token isn't Eof");
        let mut lexed = class();
        lexed.tokens.insert(0, token(Eof, 0, 0));
        rejects(CLASS, &lexed, "Eof isn't the last token");
        let mut lexed = class();
        lexed.tokens.remove(9);
        lexed.tokens[9].span = Span::new(19, 20);
        rejects(CLASS, &lexed, "Eof isn't empty");
        rejects(&format!("{CLASS} "), &class(), "not at the end of the text");
    }

    #[test]
    fn rejects_gaps_overlaps_and_bad_spans() {
        let mut lexed = class();
        lexed.tokens[1].span.start += 1;
        rejects(CLASS, &lexed, "gap");
        let mut lexed = class();
        lexed.tokens[1].span.start -= 1;
        rejects(CLASS, &lexed, "overlaps");
        let mut lexed = class();
        lexed.tokens[1].span = Span::new(5, 4);
        rejects(CLASS, &lexed, "ends before it starts");
        let mut lexed = class();
        lexed.tokens[9].span.end += 1;
        rejects(CLASS, &lexed, "ends after the end");
        let mut lexed = class();
        lexed.tokens.insert(1, token(Whitespace, 5, 5));
        rejects(CLASS, &lexed, "empty");
        rejects(
            "é",
            &self::lexed(&[(Error, 1), (Error, 1)]),
            "inside a character",
        );
    }

    #[test]
    fn rejects_split_unicode_escapes() {
        let text = "a\\u0062";
        let lexed = lexed(&[(Identifier, 3), (Identifier, 4)]);
        rejects(text, &lexed, "inside the Unicode escape at 1..7");
        // Malformed escapes may be split, and `\\u` isn't an escape.
        let text = "\\u00 \\\\u0062";
        let lexed = self::lexed(&[
            (Error, 1),
            (Identifier, 3),
            (Whitespace, 1),
            (Error, 1),
            (Error, 1),
            (Identifier, 5),
        ]);
        check(text, &lexed).unwrap();
    }

    #[test]
    fn rejects_wrong_cooked_text() {
        let cooked = |text: &'static str, cooked: Cow<'static, str>| {
            let token = token(Identifier, 0, text.len() as u32);
            check_token(text, token, &unicode_escapes(text), cooked)
        };
        let text = "\\u0061\\uuu0062";
        cooked(text, Cow::Owned("ab".to_owned())).unwrap();
        let message = cooked(text, Cow::Borrowed(text)).unwrap_err();
        assert!(message.contains("not \"ab\""), "{message}");
        let message = cooked("ab", Cow::Owned("ab".to_owned())).unwrap_err();
        assert!(message.contains("owned"), "{message}");
        // A lone surrogate has no cooked text in a `str`, so anything goes.
        cooked("\\uD800", Cow::Owned("\u{FFFD}".to_owned())).unwrap();
        cooked("\\uD800", Cow::Borrowed("\\uD800")).unwrap();
    }

    #[test]
    fn rejects_text_that_doesnt_fit_the_kind() {
        for (text, kind) in [
            ("clas", Class),
            (">>", Gt),
            ("class", Identifier),
            ("_", Identifier),
            ("x", Whitespace),
            (" \n", Whitespace),
            ("\n\n", LineTerminator),
            ("/* a", LineComment),
            ("// a", DocLineComment),
            ("// a", DocComment),
            ("/**/", DocComment),
            ("/ * */", BlockComment),
            ("a", IntLiteral),
            ("1", LongLiteral),
            ("1d", FloatLiteral),
            ("\"\"\"", StringLiteral),
            ("\"", CharLiteral),
            ("\"\"", TextBlock),
        ] {
            let len = text.len() as u32;
            rejects(text, &lexed(&[(kind, len)]), "doesn't fit the kind");
        }
        let text = "// a\nb";
        rejects(text, &lexed(&[(LineComment, 6)]), "doesn't fit the kind");
        rejects(text, &lexed(&[(Class, 6)]), "spelled \"class\"");
    }

    #[test]
    fn rejects_bad_errors() {
        let mut lexed = class();
        lexed.errors = vec![error(0, 1), error(0, 3), error(2, 2), error(20, 20)];
        check(CLASS, &lexed).unwrap();
        lexed.errors = vec![error(3, 4), error(2, 5)];
        rejects(CLASS, &lexed, "before the previous error");
        lexed.errors = vec![error(3, 2)];
        rejects(CLASS, &lexed, "ends before it starts");
        lexed.errors = vec![error(19, 21)];
        rejects(CLASS, &lexed, "ends after the end");
        let mut lexed = self::lexed(&[(Identifier, 2)]);
        lexed.errors = vec![error(0, 1)];
        rejects("é", &lexed, "splits a character");
    }

    #[test]
    fn rejects_a_split_crlf() {
        let text = "a\r\nb";
        let lexed = lexed(&[
            (Identifier, 1),
            (LineTerminator, 1),
            (LineTerminator, 1),
            (Identifier, 1),
        ]);
        check(text, &lexed).unwrap();
        let message = check_positions(text, &lexed.tokens).unwrap_err();
        assert!(message.contains("followed by"), "{message}");
    }

    #[test]
    fn finds_unicode_escapes() {
        let found = |text| -> Vec<_> {
            unicode_escapes(text)
                .iter()
                .map(|e| (e.span.start, e.span.end, e.unit))
                .collect()
        };
        assert_eq!(found("\\u0041"), [(0, 6, Some(0x41))]);
        assert_eq!(
            found("\\uuu005c\\u0041"),
            [(0, 8, Some(0x5C)), (8, 14, Some(0x41))]
        );
        assert_eq!(found("\\\\u0041 \\\\\\u0041"), [(10, 16, Some(0x41))]);
        assert_eq!(found("\\u00g \\u"), [(0, 4, None), (6, 8, None)]);
        assert_eq!(found("\\x \\"), []);
    }
}
