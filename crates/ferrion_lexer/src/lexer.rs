//! The tokenizer: JLS chapter 3, over the characters [`Cursor`] reads.

use crate::cursor::Cursor;
use crate::escape::{Escape, Escapes, cook};
use crate::{LexError, LexErrorKind, Lexed, Span, Token, TokenKind, ident};

/// ASCII SUB (control-Z), ignored at the very end of the input (JLS §3.5).
const SUB: char = '\x1A';

pub(crate) fn lex(text: &str) -> Lexed {
    let escapes: Vec<Escape> = Escapes::new(text.as_bytes(), 0, text.len()).collect();
    let mut errors: Vec<LexError> = escapes
        .iter()
        .filter(|e| !e.is_valid())
        .map(|e| LexError {
            kind: LexErrorKind::InvalidUnicodeEscape,
            span: Span::new(e.start, e.end),
        })
        .collect();
    let mut lexer = Lexer {
        cursor: Cursor::new(text, &escapes),
        tokens: Vec::with_capacity(text.len() / 4 + 1),
        errors: Vec::new(),
        no_char_close_before: 0,
        no_text_block_close_from: usize::MAX,
    };
    lexer.run();
    errors.extend(lexer.errors);
    // A token's errors are found in the order it's lexed, and its own errors (such as an
    // unterminated literal) can come last. The sort is stable, so errors at the same offset stay in
    // the order they were found.
    errors.sort_by_key(|e| e.span.start);
    Lexed {
        tokens: lexer.tokens,
        errors,
    }
}

struct Lexer<'a> {
    cursor: Cursor<'a>,
    tokens: Vec<Token>,
    errors: Vec<LexError>,
    /// A character literal that isn't closed right after its first character is closed later on
    /// the line, if there is an unescaped `'` before the line ends. When there isn't, this is
    /// where the search stopped, so that another literal starting before it, which can only start
    /// at an escaped `'` the search already passed, doesn't search the same text again.
    no_char_close_before: usize,
    /// Where the content of a text block with no closing delimiter started, or `usize::MAX`.
    no_text_block_close_from: usize,
}

impl<'a> Lexer<'a> {
    fn run(&mut self) {
        loop {
            let start = self.cursor.pos();
            if self.cursor.invalid_escape_here().is_some() {
                // Invalid escapes join the token before them, as in javac, except when that's
                // trivia (or there's no token before them). They were reported already.
                self.cursor.skip_invalid_escapes();
                self.push(TokenKind::Error, start);
                continue;
            }
            if self.cursor.is_at_end() {
                break;
            }
            let mut kind = self.token(start);
            if self.cursor.pos() == start {
                // Every branch of `token` consumes a character; this keeps a bug from looping.
                debug_assert!(false, "no progress at {start}");
                let len = self.cursor.text()[start..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
                self.cursor = self.cursor_at(start + len);
                kind = TokenKind::Error;
            }
            if !matches!(kind, TokenKind::Whitespace | TokenKind::LineTerminator) {
                self.cursor.skip_invalid_escapes();
            }
            self.push(kind, start);
        }
        let end = self.cursor.text().len() as u32;
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::empty(end),
        });
    }

    fn cursor_at(&self, pos: usize) -> Cursor<'a> {
        let mut cursor = self.cursor.clone();
        cursor.set_pos(pos);
        cursor
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token {
            kind,
            span: self.span_from(start),
        });
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(start as u32, self.cursor.pos() as u32)
    }

    fn error(&mut self, kind: LexErrorKind, span: Span) {
        self.errors.push(LexError { kind, span });
    }

    fn error_from(&mut self, kind: LexErrorKind, start: usize) {
        let span = self.span_from(start);
        self.error(kind, span);
    }

    /// Lexes one token starting at `start`, which is neither the end nor an invalid escape.
    fn token(&mut self, start: usize) -> TokenKind {
        let Some(c) = self.cursor.peek() else {
            return TokenKind::Eof;
        };
        match c {
            // JLS §3.6. Whitespace never takes in an invalid escape after it.
            ' ' | '\t' | '\x0C' => {
                self.cursor.eat_here_while(is_whitespace);
                TokenKind::Whitespace
            }
            // JLS §3.4.
            '\n' => {
                self.cursor.bump();
                TokenKind::LineTerminator
            }
            '\r' => {
                self.cursor.bump();
                self.cursor.eat_here_if(|c| c == '\n');
                TokenKind::LineTerminator
            }
            SUB if self.cursor.raw_byte_is(0x1A) && start + 1 == self.cursor.text().len() => {
                // As javac does, only a SUB written as is, where a token would start, counts.
                // Elsewhere it is an illegal character, or part of a comment, literal, or (as an
                // identifier-ignorable character) identifier.
                self.cursor.bump();
                TokenKind::Whitespace
            }
            '/' => match self.cursor.peek2() {
                Some('/') => self.line_comment(),
                Some('*') => self.block_comment(start),
                _ => self.operator(&[("/=", TokenKind::SlashEq), ("/", TokenKind::Slash)]),
            },
            '"' if self.cursor.peek2() == Some('"') && self.cursor.peek_nth(2) == Some('"') => {
                self.text_block(start)
            }
            '"' => self.string_literal(start),
            '\'' => self.char_literal(start),
            '0'..='9' => self.number(start),
            '.' if self.cursor.peek2().is_some_and(|c| c.is_ascii_digit()) => self.number(start),
            _ if ident::is_start(c) => self.identifier(start),
            _ => self.separator_or_operator(start, c),
        }
    }

    /// JLS §3.11, §3.12. `>` only ever forms `>` and `>=` (see [`TokenKind`]).
    fn separator_or_operator(&mut self, start: usize, c: char) -> TokenKind {
        use TokenKind as K;
        let ops: &[(&str, TokenKind)] = match c {
            '(' => &[("(", K::LParen)],
            ')' => &[(")", K::RParen)],
            '{' => &[("{", K::LBrace)],
            '}' => &[("}", K::RBrace)],
            '[' => &[("[", K::LBracket)],
            ']' => &[("]", K::RBracket)],
            ';' => &[(";", K::Semi)],
            ',' => &[(",", K::Comma)],
            // `..` is two dots: javac reports it as one illegal token, but it isn't one in the JLS,
            // and the parser reports the error.
            '.' => &[("...", K::Ellipsis), (".", K::Dot)],
            '@' => &[("@", K::At)],
            ':' => &[("::", K::ColonColon), (":", K::Colon)],
            '=' => &[("==", K::EqEq), ("=", K::Eq)],
            '>' => &[(">=", K::GtEq), (">", K::Gt)],
            '<' => &[
                ("<<=", K::LtLtEq),
                ("<<", K::LtLt),
                ("<=", K::LtEq),
                ("<", K::Lt),
            ],
            '!' => &[("!=", K::BangEq), ("!", K::Bang)],
            '~' => &[("~", K::Tilde)],
            '?' => &[("?", K::Question)],
            '-' => &[
                ("->", K::Arrow),
                ("--", K::MinusMinus),
                ("-=", K::MinusEq),
                ("-", K::Minus),
            ],
            '+' => &[("++", K::PlusPlus), ("+=", K::PlusEq), ("+", K::Plus)],
            '*' => &[("*=", K::StarEq), ("*", K::Star)],
            '&' => &[("&&", K::AmpAmp), ("&=", K::AmpEq), ("&", K::Amp)],
            '|' => &[("||", K::PipePipe), ("|=", K::PipeEq), ("|", K::Pipe)],
            '^' => &[("^=", K::CaretEq), ("^", K::Caret)],
            '%' => &[("%=", K::PercentEq), ("%", K::Percent)],
            _ => {
                self.cursor.bump();
                self.error_from(LexErrorKind::IllegalCharacter, start);
                return TokenKind::Error;
            }
        };
        self.operator(ops)
    }

    /// Lexes the first of `ops`, longest first, that the input continues with. The last of `ops`
    /// is the single character already peeked.
    fn operator(&mut self, ops: &[(&str, TokenKind)]) -> TokenKind {
        let Some((&(_, single), longer)) = ops.split_last() else {
            return TokenKind::Error;
        };
        for &(text, kind) in longer {
            if text
                .chars()
                .enumerate()
                .all(|(i, c)| self.cursor.peek_nth(i) == Some(c))
            {
                for _ in text.chars() {
                    self.cursor.bump();
                }
                return kind;
            }
        }
        self.cursor.bump();
        single
    }

    /// JLS §3.7: `//` up to the line terminator. `///` starts a documentation comment (JEP 467),
    /// and javac treats any longer run of slashes the same way.
    fn line_comment(&mut self) -> TokenKind {
        self.cursor.bump();
        self.cursor.bump();
        let doc = self.cursor.peek() == Some('/');
        self.cursor.eat_while(|c| !is_line_terminator(c));
        if doc {
            TokenKind::DocLineComment
        } else {
            TokenKind::LineComment
        }
    }

    /// JLS §3.7: `/*` up to the first `*/`. `/**` starts a documentation comment, unless it's the
    /// empty comment `/**/`.
    fn block_comment(&mut self, start: usize) -> TokenKind {
        self.cursor.bump();
        self.cursor.bump();
        let doc = self.cursor.peek() == Some('*') && self.cursor.peek2() != Some('/');
        if doc {
            self.cursor.bump();
        }
        let opening = self.span_from(start);
        loop {
            self.cursor.eat_while(|c| c != '*');
            match self.cursor.peek() {
                None => {
                    self.error(LexErrorKind::UnterminatedComment, opening);
                    break;
                }
                Some('*') => {
                    self.cursor.bump();
                    if self.cursor.eat('/') {
                        break;
                    }
                }
                Some(_) => self.cursor.bump(),
            }
        }
        if doc {
            TokenKind::DocComment
        } else {
            TokenKind::BlockComment
        }
    }

    /// JLS §3.8, §3.9. Keywords are recognized by the identifier's text with escapes translated
    /// and identifier-ignorable characters removed, as javac compares names.
    fn identifier(&mut self, start: usize) -> TokenKind {
        self.cursor.bump();
        self.cursor.eat_while(ident::is_part);
        let raw = &self.cursor.text()[start..self.cursor.pos()];
        let kind = if raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
        {
            TokenKind::from_keyword(raw)
        } else {
            TokenKind::from_keyword(&cook(self.cursor.text(), self.span_from(start), true))
        };
        kind.unwrap_or(TokenKind::Identifier)
    }

    /// JLS §3.10.1, §3.10.2.
    ///
    /// Malformed literals are lexed as far as javac lexes them, so `0x`, `1e+`, `0b12`, and `1_`
    /// are each one token, while `1.5L` is `1.5` and `L`, and `0b1e5` is `0b1` and `e5`.
    fn number(&mut self, start: usize) -> TokenKind {
        if self.cursor.peek() == Some('0') {
            match self.cursor.peek2() {
                Some('x' | 'X') => {
                    self.cursor.bump();
                    self.cursor.bump();
                    return self.hex_number(start);
                }
                Some('b' | 'B') => {
                    self.cursor.bump();
                    self.cursor.bump();
                    return self.binary_number(start);
                }
                _ => {}
            }
        }
        self.decimal_number(start)
    }

    fn decimal_number(&mut self, start: usize) -> TokenKind {
        let int_digits = if self.cursor.peek() == Some('.') {
            0
        } else {
            self.digits(|c| c.is_ascii_digit())
        };
        let mut float = false;
        let mut malformed = false;
        if self.cursor.eat('.') {
            float = true;
            self.digits(|c| c.is_ascii_digit());
        }
        if self.cursor.eat_if(|c| matches!(c, 'e' | 'E')).is_some() {
            float = true;
            malformed |= !self.exponent();
        }
        let kind = if self.cursor.eat_if(|c| matches!(c, 'f' | 'F')).is_some() {
            TokenKind::FloatLiteral
        } else if self.cursor.eat_if(|c| matches!(c, 'd' | 'D')).is_some() || float {
            TokenKind::DoubleLiteral
        } else if self.cursor.eat_if(|c| matches!(c, 'l' | 'L')).is_some() {
            TokenKind::LongLiteral
        } else {
            TokenKind::IntLiteral
        };
        if malformed {
            self.error_from(LexErrorKind::MalformedNumber, start);
        }
        if matches!(kind, TokenKind::IntLiteral | TokenKind::LongLiteral) && int_digits > 1 {
            // A leading zero makes it octal (JLS §3.10.1). `09` is only allowed as the start of a
            // floating-point literal, like `09.5`.
            let mut cursor = self.cursor_at(start);
            if cursor.peek() == Some('0') {
                while cursor.pos() < self.cursor.pos() {
                    let before = cursor.pos();
                    if cursor.eat_if(|c| matches!(c, '8' | '9')).is_some() {
                        let span = Span::new(before as u32, cursor.pos() as u32);
                        self.error(LexErrorKind::MalformedNumber, span);
                        break;
                    }
                    cursor.bump();
                }
            }
        }
        kind
    }

    /// After `0x`: a hexadecimal integer, or a hexadecimal floating-point literal, which needs a
    /// binary exponent (`0x1.8p1`).
    fn hex_number(&mut self, start: usize) -> TokenKind {
        let is_hex = |c: char| c.is_ascii_hexdigit();
        let int_digits = self.digits(is_hex);
        let mut float = false;
        let mut malformed = false;
        let mut frac_digits = 0;
        if self.cursor.peek() == Some('.') {
            if int_digits == 0 && !self.cursor.peek2().is_some_and(is_hex) {
                // `0x.` with no digits on either side: javac stops after the dot.
                self.cursor.bump();
                self.error_from(LexErrorKind::MalformedNumber, start);
                return TokenKind::DoubleLiteral;
            }
            self.cursor.bump();
            float = true;
            frac_digits = self.digits(is_hex);
        }
        let has_digits = int_digits > 0 || frac_digits > 0;
        if has_digits && self.cursor.eat_if(|c| matches!(c, 'p' | 'P')).is_some() {
            float = true;
            malformed = !self.exponent();
        } else if float || !has_digits {
            // A fraction with no exponent, or `0x` with no digits.
            malformed = true;
        }
        let kind = if !float {
            if self.cursor.eat_if(|c| matches!(c, 'l' | 'L')).is_some() {
                TokenKind::LongLiteral
            } else {
                TokenKind::IntLiteral
            }
        } else if self.cursor.eat_if(|c| matches!(c, 'f' | 'F')).is_some() {
            TokenKind::FloatLiteral
        } else {
            self.cursor.eat_if(|c| matches!(c, 'd' | 'D'));
            TokenKind::DoubleLiteral
        };
        if malformed {
            self.error_from(LexErrorKind::MalformedNumber, start);
        }
        kind
    }

    /// After `0b`. Like javac, any decimal digits are taken in, and the ones that aren't binary
    /// are reported.
    fn binary_number(&mut self, start: usize) -> TokenKind {
        let digits_start = self.cursor.pos();
        let digits = self.digits(|c| c.is_ascii_digit());
        let mut cursor = self.cursor_at(digits_start);
        while cursor.pos() < self.cursor.pos() {
            let before = cursor.pos();
            if cursor.eat_if(|c| matches!(c, '2'..='9')).is_some() {
                let span = Span::new(before as u32, cursor.pos() as u32);
                self.error(LexErrorKind::MalformedNumber, span);
                break;
            }
            cursor.bump();
        }
        if digits == 0 {
            self.error_from(LexErrorKind::MalformedNumber, start);
        }
        if self.cursor.eat_if(|c| matches!(c, 'l' | 'L')).is_some() {
            TokenKind::LongLiteral
        } else {
            TokenKind::IntLiteral
        }
    }

    /// The rest of an exponent after `e` or `p`: an optional sign and decimal digits. Returns
    /// whether there were digits.
    fn exponent(&mut self) -> bool {
        self.cursor.eat_if(|c| matches!(c, '+' | '-'));
        self.digits(|c| c.is_ascii_digit()) > 0
    }

    /// Takes in digits that `is_digit` accepts, with underscores (JLS §3.10.1), and returns how
    /// many digits there were. Underscores are only allowed between digits; others are taken in
    /// too, as javac does, and reported.
    fn digits(&mut self, is_digit: impl Fn(char) -> bool) -> usize {
        let mut digits = 0;
        let mut underscores: Option<usize> = None;
        loop {
            let before = self.cursor.pos();
            match self.cursor.eat_if(|c| c == '_' || is_digit(c)) {
                Some('_') => {
                    underscores.get_or_insert(before);
                }
                Some(_) => {
                    if let Some(from) = underscores.take()
                        && digits == 0
                    {
                        let span = Span::new(from as u32, before as u32);
                        self.error(LexErrorKind::MalformedNumber, span);
                    }
                    digits += 1;
                }
                None => break,
            }
        }
        if let Some(from) = underscores {
            self.error_from(LexErrorKind::MalformedNumber, from);
        }
        digits
    }

    /// JLS §3.10.4. A literal that isn't closed right after its character is closed at the next
    /// `'` on the line, as one malformed literal, or if there isn't one, ends after the character,
    /// as in javac.
    fn char_literal(&mut self, start: usize) -> TokenKind {
        self.cursor.bump();
        let units = match self.cursor.peek() {
            None => {
                self.error_from(LexErrorKind::UnterminatedCharLiteral, start);
                return TokenKind::CharLiteral;
            }
            Some(c) if is_line_terminator(c) => {
                // javac takes `'`, a line terminator, and `'` as one literal.
                self.cursor.skip_invalid_escapes();
                let lt_start = self.cursor.pos();
                let crlf = c == '\r' && self.cursor.peek2() == Some('\n');
                let after = self.cursor.peek_nth(if crlf { 2 } else { 1 });
                if after != Some('\'') {
                    self.error_from(LexErrorKind::UnterminatedCharLiteral, start);
                    return TokenKind::CharLiteral;
                }
                self.cursor.bump();
                if crlf {
                    self.cursor.bump();
                }
                self.error_from(LexErrorKind::LineTerminatorInCharLiteral, lt_start);
                self.cursor.bump();
                return TokenKind::CharLiteral;
            }
            Some('\'') => {
                self.cursor.bump();
                self.error_from(LexErrorKind::EmptyCharLiteral, start);
                return TokenKind::CharLiteral;
            }
            Some('\\') => {
                self.escape_sequence(false);
                1
            }
            Some(c) => {
                self.cursor.bump();
                c.len_utf16()
            }
        };
        if self.cursor.eat('\'') {
            if units > 1 {
                self.error_from(LexErrorKind::MultipleCharsInCharLiteral, start);
            }
        } else if self.closes_later_on_line() {
            while !self.cursor.eat('\'') {
                if self.cursor.peek() == Some('\\') {
                    self.escape_sequence(false);
                } else {
                    self.cursor.bump();
                }
            }
            self.error_from(LexErrorKind::MultipleCharsInCharLiteral, start);
        } else {
            self.error_from(LexErrorKind::UnterminatedCharLiteral, start);
        }
        TokenKind::CharLiteral
    }

    /// Whether an unescaped `'` comes before the end of the line.
    fn closes_later_on_line(&mut self) -> bool {
        if self.cursor.pos() < self.no_char_close_before {
            return false;
        }
        let mut cursor = self.cursor.clone();
        loop {
            match cursor.peek() {
                Some('\'') => return true,
                Some('\\') => {
                    cursor.bump();
                    cursor.eat_if(|c| !is_line_terminator(c));
                }
                Some(c) if !is_line_terminator(c) => cursor.bump(),
                _ => {
                    self.no_char_close_before = cursor.pos();
                    return false;
                }
            }
        }
    }

    /// JLS §3.10.5. An unterminated string literal ends before the line terminator.
    fn string_literal(&mut self, start: usize) -> TokenKind {
        self.cursor.bump();
        loop {
            self.cursor
                .eat_while(|c| !matches!(c, '"' | '\\' | '\n' | '\r'));
            match self.cursor.peek() {
                Some('"') => {
                    self.cursor.bump();
                    break;
                }
                Some('\\') => self.escape_sequence(false),
                Some(c) if !is_line_terminator(c) => self.cursor.bump(),
                _ => {
                    self.error_from(LexErrorKind::UnterminatedStringLiteral, start);
                    break;
                }
            }
        }
        TokenKind::StringLiteral
    }

    /// JLS §3.10.6. The opening `"""` must be followed by a line terminator, after optional
    /// whitespace. When it isn't, the text block is reported and lexed anyway, so that the error
    /// doesn't spread to the rest of the line. (javac lexes the text after the `"""` as tokens
    /// instead.)
    fn text_block(&mut self, start: usize) -> TokenKind {
        for _ in 0..3 {
            self.cursor.bump();
        }
        let opening = self.span_from(start);
        self.cursor.eat_here_while(is_whitespace);
        let mut not_own_line = match self.cursor.peek() {
            Some('\r') => {
                self.cursor.bump();
                self.cursor.eat_here_if(|c| c == '\n');
                None
            }
            Some('\n') => {
                self.cursor.bump();
                None
            }
            _ => Some(self.cursor.pos()),
        };
        let content_start = self.cursor.pos();
        // A text block that isn't closed means there's no closing delimiter after it, so a text
        // block opened later (at an escaped quote it passed over) isn't closed either.
        let known_unclosed = content_start >= self.no_text_block_close_from;
        let mut first_line_end = None;
        loop {
            self.cursor
                .eat_while(|c| !matches!(c, '"' | '\\' | '\n' | '\r'));
            let c = self.cursor.peek();
            let ends_line = match c {
                None => true,
                Some('"') => {
                    self.cursor.peek2() == Some('"') && self.cursor.peek_nth(2) == Some('"')
                }
                Some('\\') => self.cursor.peek2().is_some_and(is_line_terminator),
                Some(c) => is_line_terminator(c),
            };
            if ends_line && let Some(from) = not_own_line.take() {
                let to = self.cursor.pos();
                let span = if to > from {
                    Span::new(from as u32, to as u32)
                } else {
                    opening
                };
                self.error(LexErrorKind::TextBlockOpeningNotOnOwnLine, span);
            }
            match c {
                None => break,
                Some('"') if ends_line => {
                    for _ in 0..3 {
                        self.cursor.bump();
                    }
                    return TokenKind::TextBlock;
                }
                Some('\\') => self.escape_sequence(true),
                Some(c) => {
                    self.cursor.bump();
                    if c == '\r' {
                        self.cursor.eat_here_if(|c| c == '\n');
                    }
                    if is_line_terminator(c) && first_line_end.is_none() {
                        first_line_end = Some((self.cursor.pos(), self.errors.len()));
                        if known_unclosed {
                            break;
                        }
                    }
                }
            }
        }
        // Unterminated. Like javac, end the text block after its first line, if it has more than
        // one, rather than at the end of the file.
        if let Some((end, errors)) = first_line_end {
            self.cursor.set_pos(end);
            self.errors.truncate(errors);
        }
        self.no_text_block_close_from = self.no_text_block_close_from.min(content_start);
        self.error(LexErrorKind::UnterminatedTextBlock, opening);
        TokenKind::TextBlock
    }

    /// JLS §3.10.7, at a `\` in a character literal, string literal, or text block. In a text
    /// block, `\` can also escape a line terminator.
    fn escape_sequence(&mut self, text_block: bool) {
        self.cursor.skip_invalid_escapes();
        let start = self.cursor.pos();
        self.cursor.bump();
        match self.cursor.peek() {
            Some('b' | 's' | 't' | 'n' | 'f' | 'r' | '"' | '\'' | '\\') => self.cursor.bump(),
            Some(first @ '0'..='7') => {
                // `\0` to `\377`.
                self.cursor.bump();
                if self.cursor.eat_if(is_octal_digit).is_some() && first <= '3' {
                    self.cursor.eat_if(is_octal_digit);
                }
            }
            Some('\n') if text_block => self.cursor.bump(),
            Some('\r') if text_block => {
                self.cursor.bump();
                self.cursor.eat_here_if(|c| c == '\n');
            }
            Some(c) if !is_line_terminator(c) => {
                self.cursor.bump();
                self.error_from(LexErrorKind::InvalidEscape, start);
            }
            // The line terminator, or the end, belongs to whatever comes next.
            _ => self.error_from(LexErrorKind::InvalidEscape, start),
        }
    }
}

/// JLS §3.6, apart from line terminators.
fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\x0C')
}

/// JLS §3.4.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r')
}

fn is_octal_digit(c: char) -> bool {
    matches!(c, '0'..='7')
}
