//! Inline content, inline tags, and block tag arguments, read from a comment's [`Content`].
//!
//! Traditional comments are read entirely here. Markdown comments use this for their inline tags
//! and block tag arguments, and leave the rest to the Markdown parser; in Markdown mode, HTML,
//! entities, and `@` escapes aren't read, since Markdown handles HTML and has its own escapes.
//!
//! What javadoc does where the specification is silent was found by running javadoc 27 on small
//! comments; the unit tests note each case.

use crate::content::Content;
use crate::model::{
    Attribute, AttributeValue, BlockTagKind, Diagnostic, DiagnosticKind, EndElement, Entity,
    Escape, HtmlComment, Inline, InlineTag, InlineTagContent, ParamName, Reference, Snippet,
    SnippetAttribute, StartElement, TagArgument, Text,
};
use crate::reference::{self, is_identifier_part, is_identifier_start};
use crate::{MAX_NESTING, Span};

/// Whitespace within a line, as javadoc reads it.
pub(crate) fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\x0C')
}

/// Characters a tag name may continue with. javadoc 27 accepts `@foo.bar`, `@foo:bar`, and
/// `@foo-bar` as names of custom tags.
fn is_tag_name_part(c: char) -> bool {
    is_identifier_part(c) || matches!(c, '.' | '-' | ':')
}

/// Where inline content ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Until {
    /// The end of the parser's range. A block tag at the start of a line ends it too when
    /// `tags_end` is set.
    End,
    /// The `}` that matches the opening brace of an inline tag.
    Brace,
    /// The end of the parser's range, which is set to the closing quote of an HTML attribute
    /// value. HTML isn't read in it.
    Attribute,
}

/// The start of a block tag: its name and argument. The body is what follows.
pub(crate) struct TagHead {
    pub start: usize,
    pub name: Text,
    pub kind: BlockTagKind,
    pub argument: Option<TagArgument>,
    /// The end of the name or the argument, whichever is later.
    pub end: usize,
}

pub(crate) struct Parser<'a> {
    source: &'a str,
    content: &'a Content,
    text: &'a str,
    pub pos: usize,
    /// The end of the range being read.
    pub end: usize,
    markdown: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Recent searches by [`Parser::find`]: the pattern, where the search started, and what it
    /// found.
    found: Vec<(&'static str, usize, Option<usize>)>,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str, content: &'a Content, markdown: bool) -> Parser<'a> {
        Parser {
            source,
            content,
            text: &content.text,
            pos: 0,
            end: content.len(),
            markdown,
            diagnostics: Vec::new(),
            found: Vec::new(),
        }
    }

    // Reading text.

    fn peek(&self) -> Option<char> {
        self.text[self.pos..self.end].chars().next()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.text.get(offset..self.end)?.chars().next()
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..self.end]
    }

    fn at_line_start(&self, offset: usize) -> bool {
        offset == 0 || self.text.as_bytes()[offset - 1] == b'\n'
    }

    /// Whether only whitespace is between the start of the line and `offset`.
    fn after_leading_blanks(&self, offset: usize) -> bool {
        // Only the blanks before `offset` are read, so asking at each `@` of a long line takes
        // linear time in all.
        self.at_line_start(self.text[..offset].trim_end_matches(is_blank).len())
    }

    /// Whether a block tag starts on the line that starts at `offset`. Only traditional comments
    /// ask: Markdown comments find their block tags before reading them.
    pub fn block_tag_at(&self, offset: usize) -> bool {
        if self.markdown || !self.at_line_start(offset) {
            return false;
        }
        let line = &self.text[offset..self.end];
        let line = line.trim_start_matches(is_blank);
        // `@@` and `@*` are escapes for a literal `@` or `*` at the start of a line.
        line.starts_with('@') && !line.starts_with("@@") && !line.starts_with("@*")
    }

    fn skip_blanks(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start_matches(is_blank).len();
    }

    /// Skips whitespace and line breaks, but not a line break before a block tag when
    /// `tags_end` is set.
    fn skip_whitespace(&mut self, tags_end: bool) {
        loop {
            self.skip_blanks();
            if self.peek() == Some('\n') && !(tags_end && self.block_tag_at(self.pos + 1)) {
                self.pos += 1;
            } else {
                return;
            }
        }
    }

    /// Advances over characters for which `f` holds.
    fn take_while(&mut self, f: impl Fn(char) -> bool) -> usize {
        let start = self.pos;
        let rest = self.rest();
        self.pos += rest.find(|c| !f(c)).unwrap_or(rest.len());
        start
    }

    fn span(&self, start: usize, end: usize) -> Span {
        self.content.span(start..end)
    }

    fn text_node(&self, start: usize, end: usize) -> Text {
        Text {
            span: self.span(start, end),
            text: self.text[start..end].to_owned(),
        }
    }

    fn report(&mut self, kind: DiagnosticKind, span: Span) {
        self.diagnostics.push(Diagnostic { kind, span });
    }

    // Inline content.

    /// Reads inline content up to the end of the parser's range, or a block tag when `tags_end`
    /// is set, trimming leading and trailing whitespace.
    pub fn section(&mut self, tags_end: bool) -> Vec<Inline> {
        let (mut nodes, _) = self.content_until(Until::End, 0, tags_end);
        self.trim(&mut nodes);
        nodes
    }

    /// Reads inline content until `until`, returning it and whether `until` was found (and
    /// consumed).
    fn content_until(&mut self, until: Until, depth: usize, tags_end: bool) -> (Vec<Inline>, bool) {
        let mut nodes = Vec::new();
        let mut run = self.pos;
        let mut braces = 0usize;
        let flush = |parser: &Parser, nodes: &mut Vec<Inline>, run: usize| {
            if run < parser.pos {
                nodes.push(Inline::Text(parser.text_node(run, parser.pos)));
            }
        };
        while let Some(c) = self.peek() {
            match c {
                '\n' => {
                    flush(self, &mut nodes, run);
                    nodes.push(Inline::LineBreak(self.span(self.pos, self.pos + 1)));
                    self.pos += 1;
                    run = self.pos;
                    if tags_end && self.block_tag_at(self.pos) {
                        return (nodes, false);
                    }
                    continue;
                }
                '}' if until == Until::Brace => {
                    if braces == 0 {
                        flush(self, &mut nodes, run);
                        self.pos += 1;
                        return (nodes, true);
                    }
                    braces -= 1;
                }
                '{' if self.peek_at(self.pos + 1) == Some('@')
                    && !(!self.markdown && self.peek_at(self.pos + 2) == Some('@')) =>
                {
                    flush(self, &mut nodes, run);
                    nodes.push(self.inline_tag(depth));
                    run = self.pos;
                    continue;
                }
                '{' if until == Until::Brace => braces += 1,
                // HTML isn't read inside an attribute value.
                '<' if !self.markdown && until != Until::Attribute => {
                    let start = self.pos;
                    let errors = self.diagnostics.len();
                    match self.html(depth, tags_end) {
                        Some(node) => {
                            if run < start {
                                nodes.push(Inline::Text(self.text_node(run, start)));
                            }
                            nodes.push(node);
                            run = self.pos;
                            continue;
                        }
                        None => {
                            // The `<` is text.
                            self.pos = start;
                            self.diagnostics.truncate(errors);
                            self.report(DiagnosticKind::MalformedHtml, self.span(start, start + 1));
                        }
                    }
                }
                '&' if !self.markdown => {
                    if let Some(entity) = self.entity() {
                        flush(self, &mut nodes, run);
                        self.pos += entity.len();
                        let start = self.pos - entity.len();
                        nodes.push(Inline::Entity(Entity {
                            span: self.span(start, self.pos),
                            name: entity[1..entity.len() - 1].to_owned(),
                        }));
                        run = self.pos;
                        continue;
                    }
                    self.report(
                        DiagnosticKind::MalformedEntity,
                        self.span(self.pos, self.pos + 1),
                    );
                }
                '@' if !self.markdown => {
                    if let Some(ch) = self.escape() {
                        flush(self, &mut nodes, run);
                        nodes.push(Inline::Escape(Escape {
                            span: self.span(self.pos, self.pos + 2),
                            ch,
                        }));
                        self.pos += 2;
                        run = self.pos;
                        continue;
                    }
                }
                _ => {}
            }
            self.pos += c.len_utf8();
        }
        flush(self, &mut nodes, run);
        (nodes, false)
    }

    /// The character an `@` escape at the current position stands for, if it is one. javadoc
    /// reads these only where the escaped character would mean something else: `@@` where `@`
    /// would begin a tag (at the start of a line, or after `{`), `@*` where `*` would be a
    /// leading asterisk, and `@/` after `*`, where `/` would end the comment.
    fn escape(&self) -> Option<char> {
        let next = self.peek_at(self.pos + 1)?;
        let start_of_line = || self.after_leading_blanks(self.pos);
        let after = |c: char| self.text[..self.pos].ends_with(c);
        match next {
            '@' if start_of_line() || after('{') => Some('@'),
            '*' if start_of_line() => Some('*'),
            '/' if after('*') => Some('/'),
            _ => None,
        }
    }

    /// The text of an entity at the current position, `&...;`, if there is a well-formed one.
    fn entity(&self) -> Option<&'a str> {
        let rest = self.rest();
        let body = rest.strip_prefix('&')?;
        let len = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            2 + hex
                .find(|c: char| !c.is_ascii_hexdigit())
                .filter(|&n| n > 0)?
        } else if let Some(digits) = body.strip_prefix('#') {
            1 + digits
                .find(|c: char| !c.is_ascii_digit())
                .filter(|&n| n > 0)?
        } else if body.starts_with(|c: char| c.is_ascii_alphabetic()) {
            body.find(|c: char| !c.is_ascii_alphanumeric())?
        } else {
            return None;
        };
        body[len..].starts_with(';').then(|| &rest[..len + 2])
    }

    /// Reads text up to the `}` that matches an inline tag's opening brace, as written: text and
    /// line breaks, with nested braces balanced. Returns the content and whether the `}` was
    /// found.
    fn literal_text(&mut self) -> (Vec<Inline>, bool) {
        let mut nodes = Vec::new();
        let mut run = self.pos;
        let mut braces = 0usize;
        while let Some(c) = self.peek() {
            match c {
                '\n' => {
                    if run < self.pos {
                        nodes.push(Inline::Text(self.text_node(run, self.pos)));
                    }
                    nodes.push(Inline::LineBreak(self.span(self.pos, self.pos + 1)));
                    run = self.pos + 1;
                }
                '{' => braces += 1,
                '}' if braces == 0 => {
                    if run < self.pos {
                        nodes.push(Inline::Text(self.text_node(run, self.pos)));
                    }
                    self.pos += 1;
                    return (nodes, true);
                }
                '}' => braces -= 1,
                _ => {}
            }
            self.pos += c.len_utf8();
        }
        if run < self.pos {
            nodes.push(Inline::Text(self.text_node(run, self.pos)));
        }
        (nodes, false)
    }

    /// Reads an inline tag at `{@`. Returns the `{@` as text when no name follows, as javadoc
    /// does.
    pub fn inline_tag(&mut self, depth: usize) -> Inline {
        let start = self.pos;
        self.pos += 2;
        let name_start = self.pos;
        if self.peek().is_some_and(is_identifier_start) {
            self.take_while(is_tag_name_part);
        }
        if self.pos == name_start {
            self.report(DiagnosticKind::MissingTagName, self.span(start, name_start));
            return Inline::Text(self.text_node(start, name_start));
        }
        let name = self.text_node(name_start, self.pos);
        let name_end = self.pos;
        let (content, closed) = if depth >= MAX_NESTING {
            self.report(DiagnosticKind::NestingTooDeep, self.span(start, name_end));
            self.skip_whitespace(false);
            let (text, closed) = self.literal_text();
            (InlineTagContent::Other(text), closed)
        } else {
            self.tag_content(&name, depth)
        };
        if !closed {
            self.report(
                DiagnosticKind::UnclosedInlineTag,
                self.span(start, name_end),
            );
        }
        Inline::Tag(Box::new(InlineTag {
            span: self.span(start, self.pos),
            name,
            content,
            closed,
        }))
    }

    /// Reads an inline tag's content after its name, through the closing `}`.
    fn tag_content(&mut self, name: &Text, depth: usize) -> (InlineTagContent, bool) {
        let name_span = name.span;
        match name.text.as_str() {
            "code" | "literal" => {
                // javadoc 27 drops one space or tab after the name: `{@code   x}` is `  x`.
                // A line break stays.
                if self.peek().is_some_and(|c| c == ' ' || c == '\t') {
                    self.pos += 1;
                }
                let (text, closed) = self.literal_text();
                let content = if name.text == "code" {
                    InlineTagContent::Code(text)
                } else {
                    InlineTagContent::Literal(text)
                };
                (content, closed)
            }
            "docRoot" => (InlineTagContent::DocRoot, self.no_more_content()),
            "inheritDoc" => {
                self.skip_whitespace(false);
                let supertype = self.reference_unless_brace();
                (
                    InlineTagContent::InheritDoc { supertype },
                    self.no_more_content(),
                )
            }
            "link" | "linkplain" => {
                self.skip_whitespace(false);
                let reference = self.reference_unless_brace();
                if reference.is_none() {
                    self.report(DiagnosticKind::MissingArgument, name_span);
                }
                let (label, closed) = self.tag_inline_content(depth);
                let content = if name.text == "link" {
                    InlineTagContent::Link { reference, label }
                } else {
                    InlineTagContent::LinkPlain { reference, label }
                };
                (content, closed)
            }
            "value" => {
                self.skip_whitespace(false);
                let format = match self.peek() {
                    Some('"') => self.quoted(),
                    Some('%') => self.word(),
                    _ => None,
                };
                self.skip_whitespace(false);
                let reference = self.reference_unless_brace();
                (
                    InlineTagContent::Value { format, reference },
                    self.no_more_content(),
                )
            }
            "index" => {
                self.skip_whitespace(false);
                let term = match self.peek() {
                    Some('"') => self.quoted(),
                    _ => self.word(),
                };
                if term.is_none() {
                    self.report(DiagnosticKind::MissingArgument, name_span);
                }
                let (description, closed) = self.tag_inline_content(depth);
                (InlineTagContent::Index { term, description }, closed)
            }
            "return" | "summary" => {
                let (description, closed) = self.tag_inline_content(depth);
                let content = if name.text == "return" {
                    InlineTagContent::Return(description)
                } else {
                    InlineTagContent::Summary(description)
                };
                (content, closed)
            }
            "systemProperty" => {
                self.skip_whitespace(false);
                let start = self.take_while(|c| is_identifier_part(c) || c == '.');
                let name = (self.pos > start).then(|| self.text_node(start, self.pos));
                if name.is_none() {
                    self.report(DiagnosticKind::MissingArgument, name_span);
                }
                (
                    InlineTagContent::SystemProperty { name },
                    self.no_more_content(),
                )
            }
            "snippet" => {
                let (snippet, closed) = self.snippet();
                (InlineTagContent::Snippet(snippet), closed)
            }
            _ => {
                self.skip_whitespace(false);
                let (text, closed) = self.literal_text();
                (InlineTagContent::Other(text), closed)
            }
        }
    }

    /// Reads the inline content of a tag such as `{@link}` or `{@summary}` through its `}`.
    fn tag_inline_content(&mut self, depth: usize) -> (Vec<Inline>, bool) {
        self.skip_whitespace(false);
        let (mut nodes, closed) = self.content_until(Until::Brace, depth + 1, false);
        self.trim(&mut nodes);
        (nodes, closed)
    }

    /// Reads to the end of a tag that takes no more content, reporting any content before the
    /// `}`. Returns whether the `}` was found.
    fn no_more_content(&mut self) -> bool {
        self.skip_whitespace(false);
        if self.peek() == Some('}') {
            self.pos += 1;
            return true;
        }
        let start = self.pos;
        let (_, closed) = self.literal_text();
        let end = if closed { self.pos - 1 } else { self.pos };
        if end > start {
            let span = self.span(start, end);
            self.report(DiagnosticKind::UnexpectedContent, span);
        }
        closed
    }

    /// A word: characters up to whitespace or `}`.
    fn word(&mut self) -> Option<Text> {
        let start = self.take_while(|c| !c.is_whitespace() && c != '}');
        (self.pos > start).then(|| self.text_node(start, self.pos))
    }

    /// A string in double quotes, on one line. Its text leaves out the quotes, and its span
    /// covers them. An unclosed string runs to the end of the line.
    fn quoted(&mut self) -> Option<Text> {
        let start = self.pos;
        self.pos += 1;
        let inner = self.take_while(|c| c != '"' && c != '\n');
        let inner_end = self.pos;
        if self.peek() == Some('"') {
            self.pos += 1;
        } else {
            self.report(
                DiagnosticKind::MalformedArgument,
                self.span(start, inner_end),
            );
        }
        Some(Text {
            span: self.span(start, self.pos),
            text: self.text[inner..inner_end].to_owned(),
        })
    }

    /// A reference at the current position, unless the tag ends here.
    fn reference_unless_brace(&mut self) -> Option<Reference> {
        if self.peek() == Some('}') {
            return None;
        }
        self.reference()
    }

    /// Reads a reference: everything up to whitespace outside parentheses, or a `}`.
    pub fn reference(&mut self) -> Option<Reference> {
        let start = self.pos;
        let mut parens = 0usize;
        while let Some(c) = self.peek() {
            match c {
                '(' => parens += 1,
                ')' => parens = parens.saturating_sub(1),
                '}' => break,
                c if c.is_whitespace() && parens == 0 => break,
                _ => {}
            }
            self.pos += c.len_utf8();
        }
        if self.pos == start {
            return None;
        }
        let (reference, ok) = reference::parse(self.content, start, self.pos);
        if !ok {
            self.report(DiagnosticKind::MalformedReference, reference.span);
        }
        Some(reference)
    }

    /// `{@snippet attributes : body}`, after the name.
    fn snippet(&mut self) -> (Snippet, bool) {
        let mut snippet = Snippet {
            attributes: Vec::new(),
            body: None,
        };
        loop {
            self.skip_whitespace(false);
            let Some(c) = self.peek() else {
                return (snippet, false);
            };
            match c {
                '}' => {
                    self.pos += 1;
                    return (snippet, true);
                }
                ':' => break,
                c if is_identifier_start(c) => {
                    let attribute = self.snippet_attribute();
                    snippet.attributes.push(attribute);
                }
                c => {
                    self.report(
                        DiagnosticKind::MalformedArgument,
                        self.span(self.pos, self.pos + c.len_utf8()),
                    );
                    self.pos += c.len_utf8();
                }
            }
        }
        // The body starts on the line after the `:`.
        self.pos += 1;
        self.skip_blanks();
        match self.peek() {
            Some('\n') => self.pos += 1,
            Some(_) => {
                let start = self.pos;
                let line_end = self
                    .rest()
                    .find(['\n', '}'])
                    .map_or(self.end, |i| start + i);
                if line_end > start {
                    self.report(
                        DiagnosticKind::UnexpectedContent,
                        self.span(start, line_end),
                    );
                }
            }
            None => {}
        }
        let (body, closed) = self.literal_text();
        snippet.body = Some(body);
        (snippet, closed)
    }

    fn snippet_attribute(&mut self) -> SnippetAttribute {
        let start = self.take_while(|c| is_identifier_part(c) || c == '-');
        let name = self.text_node(start, self.pos);
        let after_name = self.pos;
        self.skip_blanks();
        if self.peek() != Some('=') {
            self.pos = after_name;
            return SnippetAttribute {
                span: name.span,
                name,
                value: None,
            };
        }
        self.pos += 1;
        self.skip_blanks();
        let value = match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                let value_start = self.pos;
                self.pos += 1;
                let inner = self.take_while(|c| c != quote && c != '\n');
                let inner_end = self.pos;
                if self.peek() == Some(quote) {
                    self.pos += 1;
                } else {
                    self.report(
                        DiagnosticKind::MalformedArgument,
                        self.span(value_start, inner_end),
                    );
                }
                Text {
                    span: self.span(value_start, self.pos),
                    text: self.text[inner..inner_end].to_owned(),
                }
            }
            // javadoc 27 reads `a=b:` as `a=b` and a colon, and `a=:` as an empty value.
            _ => {
                let value_start = self.take_while(|c| !c.is_whitespace() && c != ':' && c != '}');
                self.text_node(value_start, self.pos)
            }
        };
        SnippetAttribute {
            span: self.span(start, self.pos),
            name,
            value: Some(value),
        }
    }

    // HTML.

    /// Reads an HTML start tag, end tag, or comment at `<`, or returns `None` if there isn't a
    /// well-formed one. The caller restores the position.
    fn html(&mut self, depth: usize, tags_end: bool) -> Option<Inline> {
        let start = self.pos;
        if self.rest().starts_with("<!--") {
            let inner = start + 4;
            let close = self.find("-->", inner)?;
            let content = self.plain_lines(inner, close);
            self.pos = close + 3;
            return Some(Inline::HtmlComment(HtmlComment {
                span: self.span(start, self.pos),
                content,
            }));
        }
        let end_tag = self.rest().starts_with("</");
        self.pos += if end_tag { 2 } else { 1 };
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let name_start = self.take_while(|c| c.is_ascii_alphanumeric());
        let name = self.text_node(name_start, self.pos);
        if end_tag {
            self.skip_html_whitespace(tags_end)?;
            if self.peek() != Some('>') {
                return None;
            }
            self.pos += 1;
            return Some(Inline::EndElement(EndElement {
                span: self.span(start, self.pos),
                name,
            }));
        }
        let mut attributes = Vec::new();
        loop {
            let before = self.pos;
            self.skip_html_whitespace(tags_end)?;
            let separated = self.pos > before;
            match self.peek()? {
                '>' => {
                    self.pos += 1;
                    break;
                }
                '/' if self.peek_at(self.pos + 1) == Some('>') => {
                    self.pos += 2;
                    return Some(Inline::StartElement(StartElement {
                        span: self.span(start, self.pos),
                        name,
                        attributes,
                        self_closing: true,
                    }));
                }
                c if separated && is_attribute_name_char(c) => {
                    attributes.push(self.attribute(depth, tags_end)?);
                }
                _ => return None,
            }
        }
        Some(Inline::StartElement(StartElement {
            span: self.span(start, self.pos),
            name,
            attributes,
            self_closing: false,
        }))
    }

    /// Skips whitespace and line breaks inside an HTML tag, or returns `None` at a line where a
    /// block tag starts, which ends the tag unclosed.
    fn skip_html_whitespace(&mut self, tags_end: bool) -> Option<()> {
        self.skip_whitespace(tags_end);
        (self.peek() != Some('\n')).then_some(())
    }

    fn attribute(&mut self, depth: usize, tags_end: bool) -> Option<Attribute> {
        let start = self.take_while(is_attribute_name_char);
        let name = self.text_node(start, self.pos);
        let after_name = self.pos;
        self.skip_html_whitespace(tags_end)?;
        if self.peek() != Some('=') {
            self.pos = after_name;
            return Some(Attribute {
                span: name.span,
                name,
                value: None,
            });
        }
        self.pos += 1;
        self.skip_html_whitespace(tags_end)?;
        let value_start = self.pos;
        let value = match self.peek()? {
            quote @ ('"' | '\'') => {
                // The value ends at the next quote, whatever is in between, so a malformed tag
                // fails here instead of after reading nested content, which would make reading
                // nested malformed tags take exponential time.
                let close = self.find(if quote == '"' { "\"" } else { "'" }, self.pos + 1)?;
                if depth >= MAX_NESTING {
                    return None;
                }
                self.pos += 1;
                let end = std::mem::replace(&mut self.end, close);
                let (content, _) = self.content_until(Until::Attribute, depth + 1, tags_end);
                self.end = end;
                if self.pos != close {
                    // A block tag starts inside the value.
                    return None;
                }
                self.pos += 1;
                AttributeValue {
                    span: self.span(value_start, self.pos),
                    quote: Some(quote),
                    content,
                }
            }
            _ => {
                self.take_while(|c| {
                    !c.is_whitespace() && !matches!(c, '>' | '"' | '\'' | '=' | '<' | '`')
                });
                if self.pos == value_start {
                    return None;
                }
                AttributeValue {
                    span: self.span(value_start, self.pos),
                    quote: None,
                    content: vec![Inline::Text(self.text_node(value_start, self.pos))],
                }
            }
        };
        Some(Attribute {
            span: self.span(start, self.pos),
            name,
            value: Some(value),
        })
    }

    /// The offset of the first `c` at or after `from`, before the end of the range. Searches for
    /// the same character from offsets that only move forward are answered from a cache, so
    /// repeated failed searches (such as for the closing quote of many unclosed attribute values)
    /// take linear time in all.
    fn find(&mut self, pattern: &'static str, from: usize) -> Option<usize> {
        let end = self.end;
        let fits = |found: Option<usize>| found.filter(|&found| found + pattern.len() <= end);
        if let Some(&(_, _, found)) = self.found.iter().find(|&&(p, searched_from, found)| {
            p == pattern && searched_from <= from && found.is_none_or(|found| from <= found)
        }) {
            return fits(found);
        }
        let from = from.min(self.text.len());
        let found = self.text[from..].find(pattern).map(|i| from + i);
        self.found.retain(|&(p, ..)| p != pattern);
        self.found.push((pattern, from, found));
        fits(found)
    }

    /// The text and line breaks of `text[start..end]`.
    fn plain_lines(&self, start: usize, end: usize) -> Vec<Inline> {
        let mut nodes = Vec::new();
        let mut run = start;
        for (i, c) in self.text[start..end].char_indices() {
            if c == '\n' {
                let i = start + i;
                if run < i {
                    nodes.push(Inline::Text(self.text_node(run, i)));
                }
                nodes.push(Inline::LineBreak(self.span(i, i + 1)));
                run = i + 1;
            }
        }
        if run < end {
            nodes.push(Inline::Text(self.text_node(run, end)));
        }
        nodes
    }

    // Block tags.

    /// Reads a block tag's `@`, name, and argument, at the start of a line or after leading
    /// whitespace.
    pub fn block_tag_head(&mut self) -> TagHead {
        self.skip_blanks();
        let start = self.pos;
        self.pos += 1;
        let name_start = self.pos;
        if self.peek().is_some_and(is_identifier_start) {
            self.take_while(is_tag_name_part);
        }
        let name = self.text_node(name_start, self.pos);
        if name.text.is_empty() {
            // javadoc 27 reports "no tag name after '@'" for `@ x` and `@1x`, and ends the
            // previous tag there.
            self.report(DiagnosticKind::MissingTagName, self.span(start, name_start));
        }
        let kind = if name.text.is_empty() {
            BlockTagKind::Other
        } else {
            BlockTagKind::from_name(&name.text)
        };
        let name_end = self.pos;
        let argument = self.tag_argument(kind, name.span);
        let end = if argument.is_some() {
            self.pos
        } else {
            self.pos = name_end;
            name_end
        };
        TagHead {
            start,
            name,
            kind,
            argument,
            end,
        }
    }

    fn tag_argument(&mut self, kind: BlockTagKind, name: Span) -> Option<TagArgument> {
        let tags_end = !self.markdown;
        match kind {
            BlockTagKind::Param => {
                self.skip_whitespace(tags_end);
                let argument = self.param_name();
                if argument.is_none() {
                    self.report(DiagnosticKind::MissingArgument, name);
                }
                argument.map(TagArgument::Param)
            }
            BlockTagKind::Throws
            | BlockTagKind::Exception
            | BlockTagKind::Provides
            | BlockTagKind::Uses => {
                self.skip_whitespace(tags_end);
                let argument = self.reference();
                if argument.is_none() {
                    self.report(DiagnosticKind::MissingArgument, name);
                }
                argument.map(TagArgument::Reference)
            }
            BlockTagKind::See => {
                self.skip_whitespace(tags_end);
                match self.peek() {
                    Some('"') => self.quoted().map(TagArgument::Quoted),
                    // `@see <a href="...">label</a>`: the link is the body.
                    Some('<') => None,
                    _ => {
                        let argument = self.reference();
                        if argument.is_none() {
                            self.report(DiagnosticKind::MissingArgument, name);
                        }
                        argument.map(TagArgument::Reference)
                    }
                }
            }
            BlockTagKind::SerialField => {
                self.skip_whitespace(tags_end);
                let start = self.take_while(is_identifier_part);
                if self.pos == start {
                    self.report(DiagnosticKind::MissingArgument, name);
                    return None;
                }
                let field = self.text_node(start, self.pos);
                let after_field = self.pos;
                self.skip_whitespace(tags_end);
                let type_start = self.take_while(|c| !c.is_whitespace());
                let ty = if self.pos > type_start {
                    let (ty, ok) = reference::parse_type(self.content, type_start, self.pos);
                    if !ok {
                        let span = self.span(type_start, self.pos);
                        self.report(DiagnosticKind::MalformedReference, span);
                    }
                    ty
                } else {
                    self.report(DiagnosticKind::MissingArgument, name);
                    self.pos = after_field;
                    None
                };
                Some(TagArgument::SerialField { name: field, ty })
            }
            BlockTagKind::Spec => {
                self.skip_whitespace(tags_end);
                let url = self.word();
                if url.is_none() {
                    self.report(DiagnosticKind::MissingArgument, name);
                }
                url.map(TagArgument::Url)
            }
            _ => None,
        }
    }

    /// `name` or `<T>`.
    fn param_name(&mut self) -> Option<ParamName> {
        let start = self.pos;
        let type_parameter = self.peek() == Some('<');
        if type_parameter {
            self.pos += 1;
        }
        let name_start = self.take_while(is_identifier_part);
        if self.pos == name_start || !self.text[name_start..].starts_with(is_identifier_start) {
            self.pos = start;
            return None;
        }
        let name = self.text_node(name_start, self.pos);
        if type_parameter {
            if self.peek() == Some('>') {
                self.pos += 1;
            } else {
                // javadoc 27 reports "'>' expected".
                let span = self.span(start, self.pos);
                self.report(DiagnosticKind::MalformedArgument, span);
            }
        }
        Some(ParamName {
            span: self.span(start, self.pos),
            name,
            type_parameter,
        })
    }

    // Trimming.

    /// Removes leading and trailing whitespace and line breaks.
    pub fn trim(&self, nodes: &mut Vec<Inline>) {
        let blank = |node: &Inline| match node {
            Inline::LineBreak(_) => true,
            Inline::Text(text) => text.text.chars().all(is_blank),
            _ => false,
        };
        let leading = nodes.iter().take_while(|n| blank(n)).count();
        nodes.drain(..leading);
        while nodes.last().is_some_and(blank) {
            nodes.pop();
        }
        if let Some(Inline::Text(text)) = nodes.first_mut() {
            self.trim_text(text, true);
        }
        if let Some(Inline::Text(text)) = nodes.last_mut() {
            self.trim_text(text, false);
        }
    }

    /// Trims blanks from one end of `text`. Only blanks written as themselves are trimmed, since
    /// the span can't end inside a Unicode escape.
    fn trim_text(&self, text: &mut Text, start: bool) {
        let raw = self.source[text.span.range()].as_bytes();
        let cooked = text.text.as_bytes();
        // Blanks are ASCII, so bytes can be compared. One character always stays: text that is
        // all blanks has been removed already, and a blank written as an escape isn't trimmed.
        let count = |raw: &mut dyn Iterator<Item = &u8>, cooked: &mut dyn Iterator<Item = &u8>| {
            raw.zip(cooked)
                .take_while(|&(r, c)| r == c && is_blank(*c as char))
                .count()
                .min(text.text.len().saturating_sub(1))
        };
        if start {
            let n = count(&mut raw.iter(), &mut cooked.iter());
            text.text.drain(..n);
            text.span.start += n as u32;
        } else {
            let n = count(&mut raw.iter().rev(), &mut cooked.iter().rev());
            text.text.truncate(text.text.len() - n);
            text.span.end -= n as u32;
        }
    }
}

fn is_attribute_name_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !matches!(c, '"' | '\'' | '>' | '/' | '=' | '<' | '}')
}
