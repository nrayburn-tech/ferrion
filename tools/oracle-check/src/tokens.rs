//! The token comparison: the lexer's tokens and comments against javac's, after the normalization
//! in docs/design/lexer-oracle-normalization.md, and the round trip from tokens back to the file.

use std::panic::{AssertUnwindSafe, catch_unwind};

use ferrion_diagnostics::{LineIndex, SourceText};
use ferrion_lexer::{Token, TokenKind};

use crate::allowed::{Allowed, Counts};
use crate::check::{Check, Located, Mismatch, Outcome, Status};
use crate::oracle::{Diagnostic, Output, Section, Spanned};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Items shown before and after the first difference.
const CONTEXT: usize = 3;

/// The prefixes of codes for javac's language-version errors. javac's scanner reports them for
/// text blocks before Java 15, for example, but lexes the text the same way.
const VERSION_ERRORS: &[&str] = &[
    "compiler.err.feature.not.supported",
    "compiler.err.preview.feature.disabled",
];

pub struct Tokens;

pub struct Prepared {
    text: String,
    index: LineIndex,
    bom: bool,
    /// The lexer's items, or its panic message.
    items: Result<Vec<Item<'static>>, String>,
    round_trip: Option<Located>,
    allowed: Counts,
}

impl Check for Tokens {
    type Prepared = Prepared;

    fn name(&self) -> &'static str {
        "tokens"
    }

    fn sections(&self) -> &'static [Section] {
        &[Section::Tokens, Section::Comments, Section::ScanDiagnostics]
    }

    fn prepare(&self, source: &SourceText, bytes: &[u8]) -> Prepared {
        let text = source.text();
        let index = LineIndex::new(text);
        let mut allowed = Counts::default();
        let (items, round_trip) = match catch_unwind(AssertUnwindSafe(|| ferrion_lexer::lex(text)))
        {
            Ok(lexed) => {
                let round_trip = round_trip(bytes, source, &lexed.tokens).err().map(|error| {
                    let at = index.line_col(error.offset);
                    Located {
                        line: at.line + 1,
                        column: at.col + 1,
                        message: error.message,
                    }
                });
                let items =
                    ferrion_items(text, &lexed.tokens, &index, source.has_bom(), &mut allowed);
                (Ok(items), round_trip)
            }
            Err(payload) => {
                let message = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "the lexer panicked".to_owned());
                (Err(message), None)
            }
        };
        Prepared {
            text: text.to_owned(),
            index,
            bom: source.has_bom(),
            items,
            round_trip,
            allowed,
        }
    }

    fn compare(&self, prepared: &Prepared, output: &Output) -> Outcome {
        let ours = match &prepared.items {
            Ok(items) => items,
            Err(panic) => return Outcome::with_status(Status::Panicked(panic.clone())),
        };
        let mut allowed = prepared.allowed;
        let javac = match javac_items(output, prepared.bom, &mut allowed) {
            Ok(javac) => javac,
            Err(message) => return Outcome::with_status(Status::JavacFailed(message)),
        };
        let cut = javac.error.as_ref().map(|error| error.at);
        let status = match first_difference(ours, &javac.items, cut) {
            None => Status::Matched,
            Some(at) => Status::Mismatched(Box::new(describe(prepared, ours, &javac.items, at))),
        };
        let javac_error = javac.error.map(|error| {
            let at = prepared.line_col(error.at);
            Located {
                line: at.0,
                column: at.1,
                message: format!("{} ({})", error.message, error.code),
            }
        });
        Outcome {
            status,
            round_trip: prepared.round_trip.clone(),
            javac_error,
            allowed,
        }
    }
}

impl Prepared {
    /// The 1-based line and column of a javac offset.
    fn line_col(&self, utf16: u32) -> (u32, u32) {
        let byte = self.byte_offset(utf16).unwrap_or(self.text.len() as u32);
        let at = self.index.line_col(byte);
        (at.line + 1, at.col + 1)
    }

    /// The byte offset in the text of a javac offset, which counts the byte-order mark. The mark
    /// itself is at 0.
    fn byte_offset(&self, utf16: u32) -> Option<u32> {
        self.index
            .byte_offset(utf16.saturating_sub(u32::from(self.bom)))
    }

    fn show(&self, item: &Item) -> String {
        let text = match (self.byte_offset(item.start), self.byte_offset(item.end)) {
            (Some(start), Some(end)) => self.text.get(start as usize..end as usize),
            _ => None,
        };
        let snippet = match text {
            Some(text) if text.chars().count() > 24 => {
                format!("{:?}…", text.chars().take(24).collect::<String>())
            }
            Some(text) => format!("{text:?}"),
            None => "?".to_owned(),
        };
        let class = match item.class {
            ItemClass::Token => "",
            ItemClass::Comment => " comment",
        };
        format!(
            "{}{class} {}..{} {snippet}",
            item.kind, item.start, item.end
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemClass {
    Token,
    Comment,
}

/// A token or comment in javac's terms: its kind as javac names it, and its UTF-16 span in the
/// file as javac reads it, which includes any byte-order mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item<'a> {
    pub class: ItemClass,
    pub kind: &'a str,
    pub start: u32,
    pub end: u32,
}

/// The lexer's tokens and comments, in javac's terms. Whitespace and line terminators are left
/// out, `>` and `>=` tokens are joined the way javac's scanner joins them, and each run of `///`
/// lines is one comment.
pub fn ferrion_items(
    text: &str,
    tokens: &[Token],
    index: &LineIndex,
    bom: bool,
    allowed: &mut Counts,
) -> Vec<Item<'static>> {
    let offset = u32::from(bom);
    let utf16 = |byte: u32| index.utf16_offset(byte) + offset;
    let mut items = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        let (class, kind, last) = match token.kind {
            TokenKind::Whitespace | TokenKind::LineTerminator => {
                i += 1;
                continue;
            }
            TokenKind::LineComment
                if tokens
                    .get(i + 1)
                    .is_some_and(|next| next.kind == TokenKind::Eof) =>
            {
                allowed.add(Allowed::LineCommentAtEndOfFile);
                i += 1;
                continue;
            }
            TokenKind::LineComment => (ItemClass::Comment, "LINE", i),
            TokenKind::BlockComment if text.get(token.span.range()) == Some("/**/") => {
                allowed.add(Allowed::EmptyBlockComment);
                (ItemClass::Comment, "JAVADOC_BLOCK", i)
            }
            TokenKind::BlockComment => (ItemClass::Comment, "BLOCK", i),
            TokenKind::DocComment => (ItemClass::Comment, "JAVADOC_BLOCK", i),
            TokenKind::DocLineComment => {
                (ItemClass::Comment, "JAVADOC_LINE", doc_lines_end(tokens, i))
            }
            TokenKind::Gt | TokenKind::GtEq => {
                let (kind, count) = join_gt(&tokens[i..]);
                (ItemClass::Token, kind, i + count - 1)
            }
            kind => (ItemClass::Token, javac_name(kind), i),
        };
        items.push(Item {
            class,
            kind,
            start: utf16(token.span.start),
            end: utf16(tokens[last].span.end),
        });
        i = last + 1;
    }
    items
}

/// javac's name for a token kind. Trivia have none, and `>` and `>=` get theirs from [`join_gt`].
fn javac_name(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Whitespace
        | TokenKind::LineTerminator
        | TokenKind::LineComment
        | TokenKind::DocLineComment
        | TokenKind::BlockComment
        | TokenKind::DocComment => "",
        TokenKind::Identifier => "IDENTIFIER",
        TokenKind::Abstract => "ABSTRACT",
        TokenKind::Assert => "ASSERT",
        TokenKind::Boolean => "BOOLEAN",
        TokenKind::Break => "BREAK",
        TokenKind::Byte => "BYTE",
        TokenKind::Case => "CASE",
        TokenKind::Catch => "CATCH",
        TokenKind::Char => "CHAR",
        TokenKind::Class => "CLASS",
        TokenKind::Const => "CONST",
        TokenKind::Continue => "CONTINUE",
        TokenKind::Default => "DEFAULT",
        TokenKind::Do => "DO",
        TokenKind::Double => "DOUBLE",
        TokenKind::Else => "ELSE",
        TokenKind::Enum => "ENUM",
        TokenKind::Extends => "EXTENDS",
        TokenKind::Final => "FINAL",
        TokenKind::Finally => "FINALLY",
        TokenKind::Float => "FLOAT",
        TokenKind::For => "FOR",
        TokenKind::Goto => "GOTO",
        TokenKind::If => "IF",
        TokenKind::Implements => "IMPLEMENTS",
        TokenKind::Import => "IMPORT",
        TokenKind::Instanceof => "INSTANCEOF",
        TokenKind::Int => "INT",
        TokenKind::Interface => "INTERFACE",
        TokenKind::Long => "LONG",
        TokenKind::Native => "NATIVE",
        TokenKind::New => "NEW",
        TokenKind::Package => "PACKAGE",
        TokenKind::Private => "PRIVATE",
        TokenKind::Protected => "PROTECTED",
        TokenKind::Public => "PUBLIC",
        TokenKind::Return => "RETURN",
        TokenKind::Short => "SHORT",
        TokenKind::Static => "STATIC",
        TokenKind::Strictfp => "STRICTFP",
        TokenKind::Super => "SUPER",
        TokenKind::Switch => "SWITCH",
        TokenKind::Synchronized => "SYNCHRONIZED",
        TokenKind::This => "THIS",
        TokenKind::Throw => "THROW",
        TokenKind::Throws => "THROWS",
        TokenKind::Transient => "TRANSIENT",
        TokenKind::Try => "TRY",
        TokenKind::Void => "VOID",
        TokenKind::Volatile => "VOLATILE",
        TokenKind::While => "WHILE",
        TokenKind::Underscore => "UNDERSCORE",
        TokenKind::True => "TRUE",
        TokenKind::False => "FALSE",
        TokenKind::Null => "NULL",
        TokenKind::IntLiteral => "INTLITERAL",
        TokenKind::LongLiteral => "LONGLITERAL",
        TokenKind::FloatLiteral => "FLOATLITERAL",
        TokenKind::DoubleLiteral => "DOUBLELITERAL",
        TokenKind::CharLiteral => "CHARLITERAL",
        // javac has no kind of its own for text blocks.
        TokenKind::StringLiteral | TokenKind::TextBlock => "STRINGLITERAL",
        TokenKind::LParen => "LPAREN",
        TokenKind::RParen => "RPAREN",
        TokenKind::LBrace => "LBRACE",
        TokenKind::RBrace => "RBRACE",
        TokenKind::LBracket => "LBRACKET",
        TokenKind::RBracket => "RBRACKET",
        TokenKind::Semi => "SEMI",
        TokenKind::Comma => "COMMA",
        TokenKind::Dot => "DOT",
        TokenKind::Ellipsis => "ELLIPSIS",
        TokenKind::At => "MONKEYS_AT",
        TokenKind::ColonColon => "COLCOL",
        TokenKind::Eq => "EQ",
        TokenKind::Gt => "GT",
        TokenKind::Lt => "LT",
        TokenKind::Bang => "BANG",
        TokenKind::Tilde => "TILDE",
        TokenKind::Question => "QUES",
        TokenKind::Colon => "COLON",
        TokenKind::Arrow => "ARROW",
        TokenKind::EqEq => "EQEQ",
        TokenKind::GtEq => "GTEQ",
        TokenKind::LtEq => "LTEQ",
        TokenKind::BangEq => "BANGEQ",
        TokenKind::AmpAmp => "AMPAMP",
        TokenKind::PipePipe => "BARBAR",
        TokenKind::PlusPlus => "PLUSPLUS",
        TokenKind::MinusMinus => "SUBSUB",
        TokenKind::Plus => "PLUS",
        TokenKind::Minus => "SUB",
        TokenKind::Star => "STAR",
        TokenKind::Slash => "SLASH",
        TokenKind::Amp => "AMP",
        TokenKind::Pipe => "BAR",
        TokenKind::Caret => "CARET",
        TokenKind::Percent => "PERCENT",
        TokenKind::LtLt => "LTLT",
        TokenKind::PlusEq => "PLUSEQ",
        TokenKind::MinusEq => "SUBEQ",
        TokenKind::StarEq => "STAREQ",
        TokenKind::SlashEq => "SLASHEQ",
        TokenKind::AmpEq => "AMPEQ",
        TokenKind::PipeEq => "BAREQ",
        TokenKind::CaretEq => "CARETEQ",
        TokenKind::PercentEq => "PERCENTEQ",
        TokenKind::LtLtEq => "LTLTEQ",
        TokenKind::Error => "ERROR",
        TokenKind::Eof => "EOF",
    }
}

/// Joins the `>` and `>=` tokens at the start of `tokens` the way javac's scanner reads the
/// characters: the longest of `>>>=`, `>>>`, `>>=`, `>>`, `>=`, and `>`. Returns javac's kind and
/// how many tokens it covers.
fn join_gt(tokens: &[Token]) -> (&'static str, usize) {
    let next_is = |i: usize, kind: TokenKind| {
        tokens.get(i).is_some_and(|token| {
            token.kind == kind && (i == 0 || tokens[i - 1].span.end == token.span.start)
        })
    };
    let mut gts = 0;
    while gts < 3 && next_is(gts, TokenKind::Gt) {
        gts += 1;
    }
    if gts < 3 && next_is(gts, TokenKind::GtEq) {
        return (["GTEQ", "GTGTEQ", "GTGTGTEQ"][gts], gts + 1);
    }
    (["", "GT", "GTGT", "GTGTGT"][gts], gts)
}

/// The index of the last `///` line in the run starting at `first`. javac reads lines that start
/// with `///`, after any whitespace, as one comment until a line that doesn't, such as a blank one.
fn doc_lines_end(tokens: &[Token], first: usize) -> usize {
    let is = |i: usize, kind: TokenKind| tokens.get(i).is_some_and(|token| token.kind == kind);
    let skip_whitespace = |mut i: usize| {
        while is(i, TokenKind::Whitespace) {
            i += 1;
        }
        i
    };
    let mut last = first;
    loop {
        let i = skip_whitespace(last + 1);
        if !is(i, TokenKind::LineTerminator) {
            return last;
        }
        let i = skip_whitespace(i + 1);
        if !is(i, TokenKind::DocLineComment) {
            return last;
        }
        last = i;
    }
}

pub struct Javac<'a> {
    /// Tokens and comments in source order.
    pub items: Vec<Item<'a>>,
    pub error: Option<LexicalError>,
}

/// The first lexical error javac's scanner reported.
#[derive(Debug, PartialEq, Eq)]
pub struct LexicalError {
    pub code: String,
    pub message: String,
    /// Where the token holding the error starts. What javac produces from there on depends on how
    /// it recovers, so it isn't compared.
    pub at: u32,
}

/// javac's tokens and comments, in source order, after the allowed differences.
pub fn javac_items<'a>(
    output: &'a Output,
    bom: bool,
    allowed: &mut Counts,
) -> Result<Javac<'a>, String> {
    let missing = |section: Section| format!("the oracle's output has no {}", section.name());
    let mut tokens: &[Spanned] = output.tokens.as_ref().ok_or(missing(Section::Tokens))?;
    let comments = output.comments.as_ref().ok_or(missing(Section::Comments))?;
    let diagnostics = output
        .scan_diagnostics
        .as_ref()
        .ok_or(missing(Section::ScanDiagnostics))?;

    // javac reads a byte-order mark as an illegal character: one ERROR token, and an error.
    let mut bom_token = false;
    if bom
        && let Some((kind, 0, 1)) = tokens.first()
        && kind == "ERROR"
    {
        tokens = &tokens[1..];
        allowed.add(Allowed::ByteOrderMark);
        bom_token = true;
    }

    let mut items: Vec<Item> = tokens
        .iter()
        .map(|(kind, start, end)| (ItemClass::Token, kind, *start, *end))
        .chain(
            comments
                .iter()
                .map(|(kind, start, end)| (ItemClass::Comment, kind, *start, *end)),
        )
        .map(|(class, kind, start, end)| Item {
            class,
            kind,
            start,
            end,
        })
        .collect();
    items.sort_by_key(|item| (item.start, item.end));

    let error = diagnostics
        .iter()
        .filter(|diagnostic| is_lexical_error(diagnostic))
        .filter(|d| !(bom_token && d.code == "compiler.err.illegal.char" && d.start == 0))
        .filter_map(|diagnostic| {
            let at = [diagnostic.start, diagnostic.position]
                .into_iter()
                .filter(|&offset| offset >= 0)
                .min()?;
            Some((diagnostic, u32::try_from(at).ok()?))
        })
        .min_by_key(|&(_, at)| at)
        .map(|(diagnostic, at)| LexicalError {
            code: diagnostic.code.clone(),
            message: diagnostic.message.lines().next().unwrap_or("").to_owned(),
            at: tokens
                .iter()
                .find(|&&(_, start, end)| start <= at && at < end)
                .map_or(at, |&(_, start, _)| start),
        });
    Ok(Javac { items, error })
}

fn is_lexical_error(diagnostic: &Diagnostic) -> bool {
    diagnostic.kind == "ERROR"
        && !VERSION_ERRORS
            .iter()
            .any(|prefix| diagnostic.code.starts_with(prefix))
}

/// The index of the first item that differs, comparing only items that start before `cut`.
pub fn first_difference(ours: &[Item], javac: &[Item], cut: Option<u32>) -> Option<usize> {
    let before_cut = |items: &[Item]| match cut {
        Some(cut) => items.partition_point(|item| item.start < cut),
        None => items.len(),
    };
    let ours = &ours[..before_cut(ours)];
    let javac = &javac[..before_cut(javac)];
    (0..ours.len().max(javac.len())).find(|&i| ours.get(i) != javac.get(i))
}

fn describe(prepared: &Prepared, ours: &[Item], javac: &[Item], at: usize) -> Mismatch {
    let summary = match (ours.get(at), javac.get(at)) {
        (Some(a), Some(b)) if (a.class, a.kind) != (b.class, b.kind) => {
            format!("ferrion has {}, javac has {}", a.kind, b.kind)
        }
        (Some(a), Some(_)) => format!("the spans of {} differ", a.kind),
        (Some(a), None) => format!("ferrion has {} after javac's last item", a.kind),
        (None, Some(b)) => format!("ferrion has nothing where javac has {}", b.kind),
        (None, None) => unreachable!("items differ at {at}"),
    };
    let position = javac.get(at).or(ours.get(at)).map_or(0, |item| item.start);
    let (line, column) = prepared.line_col(position);
    let start = at.saturating_sub(CONTEXT);
    let end = at + CONTEXT + 1;
    let window = |items: &[Item]| -> Vec<Option<String>> {
        (start..end)
            .map(|i| items.get(i).map(|item| prepared.show(item)))
            .collect()
    };
    Mismatch {
        line,
        column,
        summary,
        ferrion: window(ours),
        javac: window(javac),
        at: at - start,
    }
}

pub struct RoundTripError {
    /// The byte offset in the text where the problem is.
    pub offset: u32,
    pub message: String,
}

/// Checks that the tokens cover the text without gaps or overlaps, end with an empty
/// [`TokenKind::Eof`], and, with the byte-order mark, reproduce `bytes`.
pub fn round_trip(
    bytes: &[u8],
    source: &SourceText,
    tokens: &[Token],
) -> Result<(), RoundTripError> {
    let text = source.text();
    let error = |offset: u32, message: String| Err(RoundTripError { offset, message });
    let mut rebuilt = Vec::with_capacity(bytes.len());
    if source.has_bom() {
        rebuilt.extend_from_slice(UTF8_BOM);
    }
    let mut offset = 0;
    for (i, token) in tokens.iter().enumerate() {
        if token.span.start != offset {
            return error(
                offset.min(token.span.start),
                format!(
                    "token {i} ({:?} at {:?}) doesn't start where the previous token ends, at byte \
                     {offset}",
                    token.kind, token.span
                ),
            );
        }
        let Some(token_text) = text.get(token.span.range()) else {
            return error(
                offset,
                format!(
                    "token {i} ({:?} at {:?}) isn't on character boundaries within the text",
                    token.kind, token.span
                ),
            );
        };
        rebuilt.extend_from_slice(token_text.as_bytes());
        offset = token.span.end;
        if token.kind == TokenKind::Eof && (i + 1 != tokens.len() || !token.span.is_empty()) {
            return error(
                offset,
                format!("token {i} is an Eof that isn't empty and last"),
            );
        }
    }
    if tokens.last().map(|token| token.kind) != Some(TokenKind::Eof) {
        return error(offset, "the last token isn't Eof".to_owned());
    }
    if offset as usize != text.len() {
        return error(
            offset,
            format!("the tokens end at byte {offset}, before the end of the text"),
        );
    }
    if rebuilt != bytes {
        let first = rebuilt.iter().zip(bytes).position(|(a, b)| a != b);
        let at = first.unwrap_or(rebuilt.len().min(bytes.len()));
        let in_text = at.saturating_sub(rebuilt.len() - text.len());
        return error(
            in_text as u32,
            format!("the tokens differ from the file at byte {at}"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ferrion_diagnostics::Span;

    use super::*;
    use crate::oracle::Diagnostic;

    /// Tokens from `(kind, text)` pairs, laid end to end.
    fn tokens(parts: &[(TokenKind, &str)]) -> (String, Vec<Token>) {
        let mut text = String::new();
        let mut tokens = Vec::new();
        for &(kind, part) in parts {
            let start = text.len() as u32;
            text.push_str(part);
            tokens.push(Token {
                kind,
                span: Span::new(start, text.len() as u32),
            });
        }
        tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::empty(text.len() as u32),
        });
        (text, tokens)
    }

    fn ours(parts: &[(TokenKind, &str)], bom: bool) -> (Vec<Item<'static>>, Counts) {
        let (text, tokens) = tokens(parts);
        let mut allowed = Counts::default();
        let items = ferrion_items(&text, &tokens, &LineIndex::new(&text), bom, &mut allowed);
        (items, allowed)
    }

    fn token(kind: &str, start: u32, end: u32) -> Item<'_> {
        Item {
            class: ItemClass::Token,
            kind,
            start,
            end,
        }
    }

    fn comment(kind: &str, start: u32, end: u32) -> Item<'_> {
        Item {
            class: ItemClass::Comment,
            kind,
            start,
            end,
        }
    }

    fn output(json: &str) -> Output {
        serde_json::from_str(json).unwrap()
    }

    use TokenKind::*;

    #[test]
    fn trivia_is_left_out_and_kinds_use_javac_names() {
        let (items, allowed) = ours(
            &[
                (Class, "class"),
                (Whitespace, " "),
                (Identifier, "A"),
                (LineTerminator, "\n"),
                (At, "@"),
                (TextBlock, "\"\"\"\n\"\"\""),
            ],
            false,
        );
        assert_eq!(
            items,
            [
                token("CLASS", 0, 5),
                token("IDENTIFIER", 6, 7),
                token("MONKEYS_AT", 8, 9),
                token("STRINGLITERAL", 9, 16),
                token("EOF", 16, 16),
            ]
        );
        assert!(allowed.is_empty());
    }

    #[test]
    fn greater_than_signs_are_joined_like_javac() {
        let joined = |parts: &[(TokenKind, &str)]| -> Vec<&str> {
            ours(parts, false).0.iter().map(|item| item.kind).collect()
        };
        assert_eq!(joined(&[(Gt, ">"), (Gt, ">")]), ["GTGT", "EOF"]);
        assert_eq!(joined(&[(Gt, ">"), (GtEq, ">=")]), ["GTGTEQ", "EOF"]);
        assert_eq!(
            joined(&[(Gt, ">"), (Gt, ">"), (Gt, ">")]),
            ["GTGTGT", "EOF"]
        );
        assert_eq!(
            joined(&[(Gt, ">"), (Gt, ">"), (GtEq, ">=")]),
            ["GTGTGTEQ", "EOF"]
        );
        // The longest match wins, then the rest starts a new token.
        assert_eq!(
            joined(&[(Gt, ">"), (Gt, ">"), (Gt, ">"), (Gt, ">")]),
            ["GTGTGT", "GT", "EOF"]
        );
        assert_eq!(
            joined(&[(Gt, ">"), (Gt, ">"), (Gt, ">"), (GtEq, ">=")]),
            ["GTGTGT", "GTEQ", "EOF"]
        );
        assert_eq!(joined(&[(GtEq, ">="), (Gt, ">")]), ["GTEQ", "GT", "EOF"]);
        // Whitespace keeps them apart.
        assert_eq!(
            joined(&[(Gt, ">"), (Whitespace, " "), (Gt, ">")]),
            ["GT", "GT", "EOF"]
        );
        let (items, _) = ours(&[(Identifier, "a"), (Gt, ">"), (Gt, ">"), (Gt, ">")], false);
        assert_eq!(items[1], token("GTGTGT", 1, 4));
    }

    #[test]
    fn runs_of_doc_lines_are_one_comment() {
        let (items, _) = ours(
            &[
                (DocLineComment, "/// a"),
                (LineTerminator, "\n"),
                (Whitespace, "  "),
                (DocLineComment, "///"),
                (LineTerminator, "\r\n"),
                (DocLineComment, "/// b"),
                (LineTerminator, "\n"),
                // A blank line, even with whitespace on it, ends the run.
                (Whitespace, " "),
                (LineTerminator, "\n"),
                (DocLineComment, "/// c"),
                (LineTerminator, "\n"),
                (LineComment, "// d"),
                (LineTerminator, "\n"),
                (DocLineComment, "/// e"),
            ],
            false,
        );
        assert_eq!(
            items,
            [
                comment("JAVADOC_LINE", 0, 18),
                comment("JAVADOC_LINE", 21, 26),
                comment("LINE", 27, 31),
                comment("JAVADOC_LINE", 32, 37),
                token("EOF", 37, 37),
            ]
        );
    }

    #[test]
    fn a_line_comment_ending_the_file_is_left_out() {
        let (items, allowed) = ours(&[(Semi, ";"), (LineComment, "// a")], false);
        let kinds: Vec<_> = items.iter().map(|item| item.kind).collect();
        assert_eq!(kinds, ["SEMI", "EOF"]);
        assert_eq!(allowed.get(Allowed::LineCommentAtEndOfFile), 1);

        let (items, allowed) = ours(
            &[
                (LineComment, "// a"),
                (LineTerminator, "\n"),
                (DocLineComment, "/// b"),
            ],
            false,
        );
        let kinds: Vec<_> = items.iter().map(|item| item.kind).collect();
        assert_eq!(kinds, ["LINE", "JAVADOC_LINE", "EOF"]);
        assert!(allowed.is_empty());
    }

    #[test]
    fn comment_styles() {
        let (items, allowed) = ours(
            &[
                (BlockComment, "/* a */"),
                (DocComment, "/** b */"),
                (BlockComment, "/**/"),
                (DocComment, "/***/"),
            ],
            false,
        );
        let kinds: Vec<_> = items.iter().map(|item| item.kind).collect();
        assert_eq!(
            kinds,
            [
                "BLOCK",
                "JAVADOC_BLOCK",
                "JAVADOC_BLOCK",
                "JAVADOC_BLOCK",
                "EOF"
            ]
        );
        assert_eq!(allowed.get(Allowed::EmptyBlockComment), 1);
    }

    #[test]
    fn offsets_are_utf16_and_count_the_byte_order_mark() {
        // "é" is 2 bytes and 1 UTF-16 unit; "😀" is 4 bytes and 2 units.
        let parts = [
            (StringLiteral, "\"é😀\""),
            (Whitespace, " "),
            (Identifier, "x"),
        ];
        let (items, _) = ours(&parts, false);
        assert_eq!(items[0], token("STRINGLITERAL", 0, 5));
        assert_eq!(items[1], token("IDENTIFIER", 6, 7));
        let (items, _) = ours(&parts, true);
        assert_eq!(items[1], token("IDENTIFIER", 7, 8));
    }

    #[test]
    fn every_javac_kind_has_a_lexer_kind() {
        // javac's token kinds, listed by running javac; STRINGFRAGMENT and CUSTOM are never
        // produced for Java source.
        let javac = "EOF ERROR IDENTIFIER ABSTRACT ASSERT BOOLEAN BREAK BYTE CASE CATCH CHAR CLASS \
                     CONST CONTINUE DEFAULT DO DOUBLE ELSE ENUM EXTENDS FINAL FINALLY FLOAT FOR \
                     GOTO IF IMPLEMENTS IMPORT INSTANCEOF INT INTERFACE LONG NATIVE NEW PACKAGE \
                     PRIVATE PROTECTED PUBLIC RETURN SHORT STATIC STRICTFP SUPER SWITCH \
                     SYNCHRONIZED THIS THROW THROWS TRANSIENT TRY VOID VOLATILE WHILE INTLITERAL \
                     LONGLITERAL FLOATLITERAL DOUBLELITERAL CHARLITERAL STRINGLITERAL TRUE FALSE \
                     NULL UNDERSCORE ARROW COLCOL LPAREN RPAREN LBRACE RBRACE LBRACKET RBRACKET \
                     SEMI COMMA DOT ELLIPSIS EQ GT LT BANG TILDE QUES COLON EQEQ LTEQ GTEQ BANGEQ \
                     AMPAMP BARBAR PLUSPLUS SUBSUB PLUS SUB STAR SLASH AMP BAR CARET PERCENT LTLT \
                     GTGT GTGTGT PLUSEQ SUBEQ STAREQ SLASHEQ AMPEQ BAREQ CARETEQ PERCENTEQ LTLTEQ \
                     GTGTEQ GTGTGTEQ MONKEYS_AT";
        let mut names: Vec<&str> = ALL_KINDS
            .iter()
            .map(|&kind| javac_name(kind))
            .chain(["GTGT", "GTGTGT", "GTGTEQ", "GTGTGTEQ"])
            .filter(|name| !name.is_empty())
            .collect();
        names.sort_unstable();
        names.dedup();
        let mut expected: Vec<&str> = javac.split_whitespace().collect();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    /// Every lexer kind. A new kind fails to compile in `javac_name` first.
    #[rustfmt::skip]
    const ALL_KINDS: &[TokenKind] = &[
        Whitespace, LineTerminator, LineComment, DocLineComment, BlockComment, DocComment,
        Identifier, Abstract, Assert, Boolean, Break, Byte, Case, Catch, Char, Class, Const,
        Continue, Default, Do, Double, Else, Enum, Extends, Final, Finally, Float, For, Goto, If,
        Implements, Import, Instanceof, Int, Interface, Long, Native, New, Package, Private,
        Protected, Public, Return, Short, Static, Strictfp, Super, Switch, Synchronized, This,
        Throw, Throws, Transient, Try, Void, Volatile, While, Underscore, True, False, Null,
        IntLiteral, LongLiteral, FloatLiteral, DoubleLiteral, CharLiteral, StringLiteral,
        TextBlock, LParen, RParen, LBrace, RBrace, LBracket, RBracket, Semi, Comma, Dot, Ellipsis,
        At, ColonColon, Eq, Gt, Lt, Bang, Tilde, Question, Colon, Arrow, EqEq, GtEq, LtEq, BangEq,
        AmpAmp, PipePipe, PlusPlus, MinusMinus, Plus, Minus, Star, Slash, Amp, Pipe, Caret,
        Percent, LtLt, PlusEq, MinusEq, StarEq, SlashEq, AmpEq, PipeEq, CaretEq, PercentEq,
        LtLtEq, Error, Eof,
    ];

    #[test]
    fn javac_items_are_in_source_order() {
        let output = output(
            r#"{"tokens":[["INT",0,3],["IDENTIFIER",13,14],["EOF",15,15]],
                "comments":[["BLOCK",4,11]],"scanDiagnostics":[]}"#,
        );
        let mut allowed = Counts::default();
        let javac = javac_items(&output, false, &mut allowed).unwrap();
        assert_eq!(
            javac.items,
            [
                token("INT", 0, 3),
                comment("BLOCK", 4, 11),
                token("IDENTIFIER", 13, 14),
                token("EOF", 15, 15),
            ]
        );
        assert_eq!(javac.error, None);
        let missing: Output = serde_json::from_str(r#"{"tokens":[]}"#).unwrap();
        assert!(javac_items(&missing, false, &mut allowed).is_err());
    }

    #[test]
    fn the_byte_order_mark_token_is_dropped() {
        let json = r#"{"tokens":[["ERROR",0,1],["CLASS",1,6],["EOF",6,6]],"comments":[],
            "scanDiagnostics":[{"kind":"ERROR","code":"compiler.err.illegal.char","start":0,
            "position":0,"end":0,"message":"illegal character: '\\ufeff'"}]}"#;
        let output = output(json);
        let mut allowed = Counts::default();
        let javac = javac_items(&output, true, &mut allowed).unwrap();
        assert_eq!(javac.items, [token("CLASS", 1, 6), token("EOF", 6, 6)]);
        assert_eq!(javac.error, None);
        assert_eq!(allowed.get(Allowed::ByteOrderMark), 1);
        let (ours, _) = ours(&[(Class, "class")], true);
        assert_eq!(first_difference(&ours, &javac.items, None), None);

        // Without a byte-order mark, the same output is an illegal character.
        let javac = javac_items(&output, false, &mut allowed).unwrap();
        assert_eq!(javac.items[0], token("ERROR", 0, 1));
        assert_eq!(javac.error.unwrap().at, 0);
    }

    #[test]
    fn lexical_errors_stop_the_comparison_at_their_token() {
        let diagnostic = |kind: &str, code: &str, at: i64| Diagnostic {
            kind: kind.into(),
            code: code.into(),
            start: at,
            position: at,
            end: at,
            message: "message\ndetail".into(),
        };
        let mut output = Output {
            tokens: Some(vec![
                ("STRINGLITERAL".into(), 0, 10),
                ("ERROR".into(), 11, 20),
                ("SEMI".into(), 20, 21),
                ("EOF".into(), 21, 21),
            ]),
            comments: Some(vec![]),
            scan_diagnostics: Some(vec![
                diagnostic("WARNING", "compiler.warn.something", 1),
                diagnostic(
                    "ERROR",
                    "compiler.err.feature.not.supported.in.source.plural",
                    2,
                ),
                diagnostic("ERROR", "compiler.err.illegal.esc.char", 15),
            ]),
            ..Output::default()
        };
        let mut allowed = Counts::default();
        let javac = javac_items(&output, false, &mut allowed).unwrap();
        let error = javac.error.as_ref().unwrap();
        assert_eq!(
            (error.code.as_str(), error.message.as_str(), error.at),
            ("compiler.err.illegal.esc.char", "message", 11)
        );
        // Only the string literal is compared, so the lexer's different recovery is no mismatch.
        let ours = [
            token("STRINGLITERAL", 0, 10),
            token("STRINGLITERAL", 11, 20),
            token("EOF", 20, 20),
        ];
        assert_eq!(first_difference(&ours, &javac.items, Some(error.at)), None);
        assert_eq!(first_difference(&ours, &javac.items, None), Some(1));
        let wrong = [token("IDENTIFIER", 0, 10)];
        assert_eq!(
            first_difference(&wrong, &javac.items, Some(error.at)),
            Some(0)
        );

        // Version errors alone don't stop it.
        output.scan_diagnostics.as_mut().unwrap().pop();
        assert_eq!(
            javac_items(&output, false, &mut allowed).unwrap().error,
            None
        );
    }

    #[test]
    fn first_difference_finds_kind_span_and_length_differences() {
        let javac = [token("CLASS", 0, 5), token("EOF", 5, 5)];
        assert_eq!(first_difference(&javac, &javac, None), None);
        assert_eq!(
            first_difference(&[token("CLASS", 0, 4), token("EOF", 5, 5)], &javac, None),
            Some(0)
        );
        assert_eq!(
            first_difference(&[token("CLASS", 0, 5), comment("EOF", 5, 5)], &javac, None),
            Some(1)
        );
        assert_eq!(first_difference(&javac[..1], &javac, None), Some(1));
    }

    fn prepare(text: &str) -> Prepared {
        let source = SourceText::decode(text).unwrap();
        Tokens.prepare(&source, text.as_bytes())
    }

    #[test]
    fn mismatches_are_described_with_both_sides() {
        // As the placeholder lexer does, one ERROR token for the whole text.
        let prepared = Prepared {
            items: Ok(vec![token("ERROR", 0, 18), token("EOF", 18, 18)]),
            ..prepare("class A {}\nint x;\n")
        };
        let output = output(
            r#"{"tokens":[["CLASS",0,5],["IDENTIFIER",6,7],["LBRACE",8,9],["RBRACE",9,10],
                ["INT",11,14],["IDENTIFIER",15,16],["SEMI",16,17],["EOF",18,18]],
                "comments":[],"scanDiagnostics":[]}"#,
        );
        let outcome = Tokens.compare(&prepared, &output);
        assert_eq!(outcome.round_trip, None);
        let Status::Mismatched(mismatch) = outcome.status else {
            panic!("expected a mismatch, got {:?}", outcome.status);
        };
        assert_eq!((mismatch.line, mismatch.column), (1, 1));
        assert_eq!(mismatch.summary, "ferrion has ERROR, javac has CLASS");
        assert_eq!(mismatch.at, 0);
        assert_eq!(
            mismatch.ferrion[0].as_deref(),
            Some(r#"ERROR 0..18 "class A {}\nint x;\n""#)
        );
        assert_eq!(mismatch.ferrion[2], None);
        assert_eq!(mismatch.javac[0].as_deref(), Some(r#"CLASS 0..5 "class""#));
        assert_eq!(mismatch.javac.len(), CONTEXT + 1);
    }

    #[test]
    fn a_matching_file() {
        let prepared = Prepared {
            items: Ok(vec![token("CLASS", 0, 5), token("EOF", 5, 5)]),
            ..prepare("class")
        };
        let output =
            output(r#"{"tokens":[["CLASS",0,5],["EOF",5,5]],"comments":[],"scanDiagnostics":[]}"#);
        let outcome = Tokens.compare(&prepared, &output);
        assert_eq!(outcome, Outcome::default());
    }

    #[test]
    fn a_later_mismatch_is_located_by_line_and_utf16_column() {
        let prepared = Prepared {
            items: Ok(vec![token("STRINGLITERAL", 6, 10), token("EOF", 11, 11)]),
            ..prepare("\n\"é\" x\n")
        };
        let output = output(
            r#"{"tokens":[["STRINGLITERAL",1,4],["IDENTIFIER",5,6],["EOF",7,7]],"comments":[],
                "scanDiagnostics":[]}"#,
        );
        let Status::Mismatched(mismatch) = Tokens.compare(&prepared, &output).status else {
            panic!("expected a mismatch");
        };
        assert_eq!((mismatch.line, mismatch.column), (2, 1));
        assert_eq!(mismatch.summary, "the spans of STRINGLITERAL differ");
        assert_eq!(
            mismatch.javac[0].as_deref(),
            Some(r#"STRINGLITERAL 1..4 "\"é\"""#)
        );
    }

    #[test]
    fn round_trips() {
        let source = SourceText::decode(b"\xEF\xBB\xBFab".to_vec()).unwrap();
        let bytes = b"\xEF\xBB\xBFab";
        let good = [
            Token {
                kind: Identifier,
                span: Span::new(0, 2),
            },
            Token {
                kind: Eof,
                span: Span::empty(2),
            },
        ];
        assert!(round_trip(bytes, &source, &good).is_ok());
        let check = |tokens: &[Token]| round_trip(bytes, &source, tokens).err().unwrap().message;
        let gap = [
            Token {
                kind: Identifier,
                span: Span::new(0, 1),
            },
            good[1],
        ];
        assert!(
            check(&gap).contains("doesn't start where"),
            "{}",
            check(&gap)
        );
        assert!(check(&good[..1]).contains("isn't Eof"));
        let past_end = [Token {
            kind: Identifier,
            span: Span::new(0, 3),
        }];
        assert!(check(&past_end).contains("character boundaries"));
        let early_eof = [
            Token {
                kind: Eof,
                span: Span::empty(0),
            },
            good[0],
            good[1],
        ];
        assert!(check(&early_eof).contains("empty and last"));
        let short = [Token {
            kind: Eof,
            span: Span::empty(0),
        }];
        assert!(check(&short).contains("before the end"));
    }

    #[test]
    fn the_lexer_round_trips() {
        let text = "class A {}\r\n/** é */ int x = 1 >>> 2;\n";
        let source = SourceText::decode(text).unwrap();
        let lexed = ferrion_lexer::lex(text);
        assert!(round_trip(text.as_bytes(), &source, &lexed.tokens).is_ok());
    }
}
