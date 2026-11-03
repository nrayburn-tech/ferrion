//! A JSONC parser that keeps every value's span.
//!
//! JSONC is JSON with `//` and `/* */` comments and trailing commas in arrays and objects, as VS
//! Code reads it, and nothing else of JSON5. A leading byte-order mark is skipped. A syntax error
//! stops the parse. A duplicate key doesn't: it is collected, and both members are kept, so that
//! the rest of the file can still be checked.

use ferrion_diagnostics::Span;

use crate::error::{ConfigError, ErrorCode};

/// How deep values may nest: an object or array inside 63 others is the deepest allowed.
pub const MAX_DEPTH: u32 = 64;

/// A JSON value and where it is in the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub kind: ValueKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    Null,
    Bool(bool),
    /// A number, as written.
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// The members in file order, duplicates included.
    Object(Vec<Member>),
}

/// A key and its value in an object.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub key: String,
    /// The key's span, quotes included.
    pub key_span: Span,
    pub value: Value,
}

impl Value {
    /// What the value is, for a message: "an object", "a string", and so on.
    pub fn describe(&self) -> &'static str {
        match self.kind {
            ValueKind::Null => "null",
            ValueKind::Bool(_) => "a boolean",
            ValueKind::Number(_) => "a number",
            ValueKind::String(_) => "a string",
            ValueKind::Array(_) => "an array",
            ValueKind::Object(_) => "an object",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            ValueKind::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[Member]> {
        match &self.kind {
            ValueKind::Object(members) => Some(members),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match &self.kind {
            ValueKind::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The value as compact JSON, without comments or trailing commas.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match &self.kind {
            ValueKind::Null => out.push_str("null"),
            ValueKind::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            ValueKind::Number(n) => out.push_str(n),
            ValueKind::String(s) => write_json_string(out, s),
            ValueKind::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            ValueKind::Object(members) => {
                if members.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{ ");
                for (i, member) in members.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_json_string(out, &member.key);
                    out.push_str(": ");
                    member.value.write_json(out);
                }
                out.push_str(" }");
            }
        }
    }
}

/// Writes `s` as a JSON string, quoted and escaped.
pub fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The result of a parse that found no syntax error: the root value, and any duplicate keys.
#[derive(Debug)]
pub struct Parsed {
    pub value: Value,
    pub duplicates: Vec<ConfigError>,
}

/// Parses JSONC text. A syntax error is returned alone; duplicate keys are collected.
pub fn parse(text: &str) -> Result<Parsed, Box<ConfigError>> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        pos: if text.starts_with('\u{FEFF}') { 3 } else { 0 },
        depth: 0,
        duplicates: Vec::new(),
    };
    parser.trivia()?;
    if parser.at_end() {
        return Err(
            ConfigError::new(ErrorCode::Syntax, "the config file is empty", parser.here())
                .help("a config that changes nothing is `{}`")
                .into(),
        );
    }
    let value = parser.value()?;
    parser.trivia()?;
    if !parser.at_end() {
        let (span, found) = parser.found();
        return Err(
            ConfigError::new(ErrorCode::Syntax, "expected the end of the file", span)
                .label(format!("found {found}"))
                .related(Span::empty(value.span.end), "the config ends here")
                .into(),
        );
    }
    Ok(Parsed {
        value,
        duplicates: parser.duplicates,
    })
}

struct Parser<'t> {
    text: &'t str,
    bytes: &'t [u8],
    pos: usize,
    depth: u32,
    duplicates: Vec<ConfigError>,
}

/// What may follow a value in an array or object, for the error when something else does.
enum Closing {
    Array,
    Object,
}

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn here(&self) -> Span {
        Span::empty(self.pos as u32)
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(start as u32, self.pos as u32)
    }

    /// Skips whitespace and comments.
    fn trivia(&mut self) -> Result<(), Box<ConfigError>> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'/') if self.bytes.get(self.pos + 1) == Some(&b'/') => {
                    while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.bytes.get(self.pos + 1) == Some(&b'*') => {
                    let start = self.pos;
                    match self.text[start + 2..].find("*/") {
                        Some(end) => self.pos = start + 2 + end + 2,
                        None => {
                            return Err(ConfigError::new(
                                ErrorCode::Syntax,
                                "unterminated comment",
                                Span::new(start as u32, start as u32 + 2),
                            )
                            .label("this comment has no `*/`")
                            .into());
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// The token at the current position, for an error about it: its span and "a string", "`]`",
    /// "the end of the file", and so on.
    fn found(&self) -> (Span, String) {
        let rest = &self.text[self.pos..];
        let Some(c) = rest.chars().next() else {
            return (self.here(), "the end of the file".to_owned());
        };
        if c == '"' {
            return (self.span_one(c), "a string".to_owned());
        }
        if c == '-' || c.is_ascii_digit() {
            let len = word_len(rest);
            return (
                Span::new(self.pos as u32, (self.pos + len.max(1)) as u32),
                "a number".to_owned(),
            );
        }
        let len = word_len(rest);
        if len > 0 {
            return (
                Span::new(self.pos as u32, (self.pos + len) as u32),
                format!("`{}`", &rest[..len]),
            );
        }
        (self.span_one(c), format!("`{c}`"))
    }

    fn span_one(&self, c: char) -> Span {
        Span::new(self.pos as u32, (self.pos + c.len_utf8()) as u32)
    }

    fn value(&mut self) -> Result<Value, Box<ConfigError>> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => {
                let (string, span) = self.string()?;
                Ok(Value {
                    kind: ValueKind::String(string),
                    span,
                })
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.word(),
        }
    }

    /// `true`, `false`, or `null`, or the error for anything else where a value belongs.
    fn word(&mut self) -> Result<Value, Box<ConfigError>> {
        let start = self.pos;
        let len = word_len(&self.text[start..]);
        let word = &self.text[start..start + len];
        let kind = match word {
            "true" => Some(ValueKind::Bool(true)),
            "false" => Some(ValueKind::Bool(false)),
            "null" => Some(ValueKind::Null),
            _ => None,
        };
        if let Some(kind) = kind {
            self.pos += len;
            return Ok(Value {
                kind,
                span: self.span_from(start),
            });
        }
        let (span, found) = self.found();
        let mut error = ConfigError::new(ErrorCode::Syntax, "expected a value", span)
            .label(format!("found {found}"));
        let lower = word.to_ascii_lowercase();
        if let Some(literal) = ["true", "false", "null"].into_iter().find(|l| *l == lower) {
            error = error.help(format!("did you mean `{literal}`?"));
            error.suggestion = Some(literal.to_owned());
        } else if matches!(word, "NaN" | "Infinity" | "undefined") {
            error = error.help(format!("`{word}` isn't a JSON value"));
        } else if !word.is_empty() {
            error = error.help("strings must be in double quotes");
        } else if self.peek() == Some(b'\'') {
            error = error.help("strings must be in double quotes, not single quotes");
        }
        Err(error.into())
    }

    fn number(&mut self) -> Result<Value, Box<ConfigError>> {
        let start = self.pos;
        let len = word_len(&self.text[start..]).max(1);
        self.pos += len;
        let raw = &self.text[start..self.pos];
        if !is_json_number(raw) {
            return Err(ConfigError::new(
                ErrorCode::Syntax,
                format!("`{raw}` isn't a valid number"),
                self.span_from(start),
            )
            .help("numbers are written as in JSON: no leading zeros, `+`, hexadecimal, or `.` without digits on both sides").into());
        }
        Ok(Value {
            kind: ValueKind::Number(raw.to_owned()),
            span: self.span_from(start),
        })
    }

    /// A string, from its opening quote: its value, and its span, quotes included.
    fn string(&mut self) -> Result<(String, Span), Box<ConfigError>> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let rest = &self.text[self.pos..];
            let Some(c) = rest.chars().next() else {
                return Err(self.unterminated(start).into());
            };
            match c {
                '"' => {
                    self.pos += 1;
                    return Ok((out, self.span_from(start)));
                }
                '\\' => out.push(self.escape()?),
                '\n' | '\r' => return Err(self.unterminated(start).into()),
                c if (c as u32) < 0x20 => {
                    return Err(ConfigError::new(
                        ErrorCode::Syntax,
                        "a string can't contain a control character",
                        self.span_one(c),
                    )
                    .label(format!("escape it as `\\u{:04x}`", c as u32))
                    .into());
                }
                c => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn unterminated(&self, start: usize) -> ConfigError {
        ConfigError::new(
            ErrorCode::Syntax,
            "unterminated string",
            Span::new(start as u32, self.pos as u32),
        )
        .label("this string has no closing `\"` on its line")
    }

    /// An escape, from its backslash.
    fn escape(&mut self) -> Result<char, Box<ConfigError>> {
        let start = self.pos;
        self.pos += 1;
        let Some(c) = self.text[self.pos..].chars().next() else {
            return Err(self.unterminated(start).into());
        };
        self.pos += c.len_utf8();
        let simple = match c {
            '"' => Some('"'),
            '\\' => Some('\\'),
            '/' => Some('/'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'u' => None,
            _ => {
                return Err(ConfigError::new(
                    ErrorCode::Syntax,
                    format!("`\\{c}` isn't an escape"),
                    self.span_from(start),
                )
                .help(concat!(
                    "the escapes are `\\\"`, `\\\\`, `\\/`, `\\b`, `\\f`, `\\n`, `\\r`, `\\t`, and `\\",
                    "u` with four hexadecimal digits"
                )).into());
            }
        };
        if let Some(c) = simple {
            return Ok(c);
        }
        let high = self.hex4(start)?;
        if !(0xD800..0xE000).contains(&high) {
            return Ok(char::from_u32(high).unwrap());
        }
        // A surrogate: a high one must be followed by an escaped low one.
        if high < 0xDC00 && self.text[self.pos..].starts_with("\\u") {
            let low_start = self.pos;
            self.pos += 2;
            let low = self.hex4(low_start)?;
            if (0xDC00..0xE000).contains(&low) {
                let c = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                return Ok(char::from_u32(c).unwrap());
            }
        }
        Err(ConfigError::new(
            ErrorCode::Syntax,
            "a string can't contain half of a surrogate pair",
            self.span_from(start),
        )
        .label("this escape is half of a UTF-16 surrogate pair, without its other half")
        .into())
    }

    /// The four hexadecimal digits of a `\u` escape that starts at `start`.
    fn hex4(&mut self, start: usize) -> Result<u32, Box<ConfigError>> {
        let digits = self.text.get(self.pos..self.pos + 4).unwrap_or("");
        match u32::from_str_radix(digits, 16) {
            Ok(n) if digits.bytes().all(|b| b.is_ascii_hexdigit()) => {
                self.pos += 4;
                Ok(n)
            }
            _ => Err(ConfigError::new(
                ErrorCode::Syntax,
                "a `\\u` escape needs four hexadecimal digits",
                Span::new(start as u32, self.pos as u32),
            )
            .into()),
        }
    }

    fn enter(&mut self) -> Result<(), Box<ConfigError>> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ConfigError::new(
                ErrorCode::TooDeep,
                format!("values can't nest more than {MAX_DEPTH} deep"),
                Span::new(self.pos as u32, self.pos as u32 + 1),
            )
            .label(format!("this is level {}", self.depth))
            .into());
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Value, Box<ConfigError>> {
        let start = self.pos;
        self.enter()?;
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.trivia()?;
            if self.peek() == Some(b']') {
                break;
            }
            let item = self.value()?;
            let end = item.span.end;
            items.push(item);
            self.trivia()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => break,
                _ => return Err(self.expected_separator(end, Closing::Array).into()),
            }
        }
        self.pos += 1;
        self.depth -= 1;
        Ok(Value {
            kind: ValueKind::Array(items),
            span: self.span_from(start),
        })
    }

    fn object(&mut self) -> Result<Value, Box<ConfigError>> {
        let start = self.pos;
        self.enter()?;
        self.pos += 1;
        let mut members: Vec<Member> = Vec::new();
        loop {
            self.trivia()?;
            match self.peek() {
                Some(b'}') => break,
                Some(b'"') => {}
                _ => {
                    let (span, found) = self.found();
                    let mut error =
                        ConfigError::new(ErrorCode::Syntax, "expected a key or `}`", span)
                            .label(format!("found {found}"));
                    if self.peek() == Some(b'\'') {
                        error = error.help("keys must be in double quotes, not single quotes");
                    } else if word_len(&self.text[self.pos..]) > 0 {
                        error = error.help("keys must be in double quotes");
                    }
                    return Err(error.into());
                }
            }
            let (key, key_span) = self.string()?;
            self.trivia()?;
            if self.peek() != Some(b':') {
                let (span, found) = self.found();
                return Err(ConfigError::new(
                    ErrorCode::Syntax,
                    "expected `:` after this key",
                    span,
                )
                .label(format!("found {found}"))
                .related(key_span, "the key")
                .into());
            }
            self.pos += 1;
            self.trivia()?;
            let value = self.value()?;
            if let Some(first) = members.iter().find(|m| m.key == key) {
                self.duplicates.push(
                    ConfigError::new(
                        ErrorCode::DuplicateKey,
                        format!("duplicate key `{key}`"),
                        key_span,
                    )
                    .label("given again here")
                    .related(first.key_span, "first given here")
                    .help("only one of them would be used: remove the other"),
                );
            }
            let end = value.span.end;
            members.push(Member {
                key,
                key_span,
                value,
            });
            self.trivia()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => break,
                _ => return Err(self.expected_separator(end, Closing::Object).into()),
            }
        }
        self.pos += 1;
        self.depth -= 1;
        Ok(Value {
            kind: ValueKind::Object(members),
            span: self.span_from(start),
        })
    }

    /// The error for something other than `,` or the closing bracket after a value that ends at
    /// `end`. When it looks like the next member, a missing comma is likely, and the error says so,
    /// marking the value's last character, which every value has.
    fn expected_separator(&self, end: u32, closing: Closing) -> ConfigError {
        let (span, found) = self.found();
        let (message, next) = match closing {
            Closing::Array => (
                "expected `,` or `]` after this value",
                "the next value starts here",
            ),
            Closing::Object => (
                "expected `,` or `}` after this value",
                "the next key starts here",
            ),
        };
        let looks_like_next = match closing {
            Closing::Object => self.peek() == Some(b'"'),
            Closing::Array => matches!(
                self.peek(),
                Some(b'"' | b'{' | b'[' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n')
            ),
        };
        let error = ConfigError::new(ErrorCode::Syntax, message, span);
        if looks_like_next {
            error
                .label(next)
                .related(Span::new(end - 1, end), "add `,` here")
        } else {
            error.label(format!("found {found}"))
        }
    }
}

/// The length of the run of ASCII letters, digits, `_`, `$`, `.`, `+`, and `-` at the start of
/// `text`: a word, or a number as far as it can be told apart from what follows it.
fn word_len(text: &str) -> usize {
    text.bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'.' | b'+' | b'-'))
        .count()
}

/// Whether `s` is a number as JSON writes it: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            digits(&mut i);
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backslash, so that no escape in a test is read as one before the parser sees it.
    const BS: &str = "\\";

    fn ok(text: &str) -> Value {
        parse(text).unwrap().value
    }

    fn err(text: &str) -> ConfigError {
        *parse(text).unwrap_err()
    }

    fn slice(text: &str, span: Span) -> &str {
        &text[span.range()]
    }

    #[test]
    fn values_and_spans() {
        let text = r#"{ "a": [1, -2.5e3, true, false, null], "b": { "c": "d" } }"#;
        let value = ok(text);
        assert_eq!(value.span, Span::new(0, text.len() as u32));
        let members = value.as_object().unwrap();
        assert_eq!(members[0].key, "a");
        assert_eq!(slice(text, members[0].key_span), r#""a""#);
        let items = members[0].value.as_array().unwrap();
        assert_eq!(items.len(), 5);
        assert_eq!(items[1].kind, ValueKind::Number("-2.5e3".into()));
        assert_eq!(slice(text, items[1].span), "-2.5e3");
        assert_eq!(items[2].kind, ValueKind::Bool(true));
        assert_eq!(items[4].kind, ValueKind::Null);
        assert_eq!(slice(text, members[1].value.span), r#"{ "c": "d" }"#);
        assert_eq!(
            value.to_json(),
            r#"{ "a": [1, -2.5e3, true, false, null], "b": { "c": "d" } }"#
        );
    }

    #[test]
    fn comments_and_trailing_commas() {
        let text = "// leading\n{ /* a */ \"a\": [1, 2,], // b\n \"b\": {\"c\": 1,}, }\n/* end */";
        let value = ok(text);
        assert_eq!(value.to_json(), r#"{ "a": [1, 2], "b": { "c": 1 } }"#);
        assert_eq!(ok("{}").as_object().unwrap().len(), 0);
        assert_eq!(ok("[]").as_array().unwrap().len(), 0);
        // A comment can end at the end of the file, and `/*/` doesn't close itself.
        assert!(parse("{} // end").is_ok());
        assert_eq!(err("{} /*/").message, "unterminated comment");
    }

    #[test]
    fn byte_order_mark() {
        let value = ok("\u{FEFF}{}");
        assert_eq!(value.span, Span::new(3, 5));
    }

    #[test]
    fn strings_and_escapes() {
        let text = format!(
            r#""a{BS}"{BS}{BS}{BS}/{BS}b{BS}f{BS}n{BS}r{BS}t{BS}u00e9{BS}ud83d{BS}ude00é""#
        );
        assert_eq!(
            ok(&text).kind,
            ValueKind::String("a\"\\/\u{8}\u{c}\n\r\té😀é".into())
        );
        let mut json = String::new();
        write_json_string(&mut json, "a\"\\\n\u{1}");
        assert_eq!(json, format!(r#""a{BS}"{BS}{BS}{BS}n{BS}u0001""#));
    }

    #[test]
    fn string_errors() {
        assert_eq!(err(r#""abc"#).message, "unterminated string");
        assert_eq!(err("\"ab\ncd\"").message, "unterminated string");
        assert_eq!(
            err("\"a\tb\"").message,
            "a string can't contain a control character"
        );
        assert_eq!(
            err(&format!(r#""{BS}x""#)).message,
            format!("`{BS}x` isn't an escape")
        );
        assert_eq!(
            err(&format!(r#""{BS}u12g4""#)).message,
            format!("a `{BS}u` escape needs four hexadecimal digits")
        );
        for lone in [
            format!(r#""{BS}ud83d""#),
            format!(r#""{BS}ude00""#),
            format!(r#""{BS}ud83d{BS}u0041""#),
        ] {
            assert_eq!(
                err(&lone).message,
                "a string can't contain half of a surrogate pair",
                "{lone}"
            );
        }
    }

    #[test]
    fn numbers() {
        for good in ["0", "-0", "12", "1.5", "1e9", "1E+9", "-1.25e-3"] {
            assert_eq!(ok(good).kind, ValueKind::Number(good.into()), "{good}");
        }
        for bad in ["01", "1.", ".5", "1e", "0x1F", "-", "1.2.3", "1e+"] {
            let error = parse(bad).unwrap_err();
            assert_eq!(error.code, ErrorCode::Syntax, "{bad}");
        }
        assert_eq!(err("[+1]").message, "expected a value");
    }

    #[test]
    fn words() {
        let error = err("[True]");
        assert_eq!(error.message, "expected a value");
        assert_eq!(error.suggestion.as_deref(), Some("true"));
        assert_eq!(err("NaN").help.as_deref(), Some("`NaN` isn't a JSON value"));
        assert_eq!(
            err("[abc]").help.as_deref(),
            Some("strings must be in double quotes")
        );
        assert_eq!(
            err("['a']").help.as_deref(),
            Some("strings must be in double quotes, not single quotes")
        );
    }

    #[test]
    fn structure_errors() {
        let text = "{\n  \"a\": 1\n  \"b\": 2\n}";
        let error = err(text);
        assert_eq!(error.message, "expected `,` or `}` after this value");
        assert_eq!(slice(text, error.span), "\"");
        assert_eq!(slice(text, error.related.unwrap().span), "1");
        let error = err("[1 2]");
        assert_eq!(error.message, "expected `,` or `]` after this value");
        assert_eq!(error.label.as_deref(), Some("the next value starts here"));
        let error = err("[1 }");
        assert_eq!(error.label.as_deref(), Some("found `}`"));
        assert!(error.related.is_none());
        assert_eq!(
            err("{a: 1}").help.as_deref(),
            Some("keys must be in double quotes")
        );
        assert_eq!(err("{\"a\" 1}").message, "expected `:` after this key");
        assert_eq!(err("[1,,2]").message, "expected a value");
        assert_eq!(err("{,}").message, "expected a key or `}`");
        assert_eq!(err("{\"a\":}").message, "expected a value");
        assert_eq!(err("{} {}").message, "expected the end of the file");
        assert_eq!(err("[").label.as_deref(), Some("found the end of the file"));
        assert_eq!(
            err(" // only a comment\n").message,
            "the config file is empty"
        );
        assert_eq!(err("").message, "the config file is empty");
    }

    #[test]
    fn duplicate_keys_are_collected() {
        let text = r#"{ "a": 1, "b": { "c": 1, "c": 2 }, "a": 3 }"#;
        let parsed = parse(text).unwrap();
        let messages: Vec<_> = parsed.duplicates.iter().map(|e| &e.message).collect();
        assert_eq!(messages, ["duplicate key `c`", "duplicate key `a`"]);
        assert_eq!(parsed.value.as_object().unwrap().len(), 3);
        let first = parsed.duplicates[1].related.as_ref().unwrap().span;
        assert_eq!(first, Span::new(2, 5));
    }

    #[test]
    fn depth_limit() {
        let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(&nested(MAX_DEPTH as usize)).is_ok());
        let error = err(&nested(MAX_DEPTH as usize + 1));
        assert_eq!(error.code, ErrorCode::TooDeep);
        assert_eq!(error.span, Span::new(MAX_DEPTH, MAX_DEPTH + 1));
        // Far deeper input stops at the limit rather than overflowing the stack.
        assert_eq!(err(&"{\"a\":".repeat(100_000)).code, ErrorCode::TooDeep);
    }
}
