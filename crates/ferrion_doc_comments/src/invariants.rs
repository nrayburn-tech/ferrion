//! Checks that the parsers keep the promises their API documents, for the fuzz targets in `fuzz/`,
//! the fuzz regression tests, and the unit tests. Not part of the public API.
//!
//! Only documented promises are checked, so any failure is a bug in a parser or its docs: every
//! span is within the comment, on character boundaries, and doesn't split a Unicode escape;
//! children are within their parents and siblings are in order without overlapping; text is never
//! empty, and a traditional comment's text is what its span covers with escapes translated;
//! diagnostics are within the comment and in order; and parsing is deterministic.

use ferrion_diagnostics::SourceText;
use ferrion_lexer::{TokenKind, lex};

use crate::escape;
use crate::model::{
    Attribute, BlockTag, Diagnostic, Inline, InlineTag, InlineTagContent, Reference, TagArgument,
    Text, TypeArgument,
};
use crate::{Javadoc, Span, parse_javadoc};

/// Reads `bytes` as a source file (or as lossy UTF-8 when it isn't one), wraps the text in `/**`
/// and `*/`, and checks every traditional comment the lexer finds with [`check_javadoc`]. The
/// whole text is parsed and checked as a comment too, to try spans that aren't comments.
pub fn check_javadoc_bytes(bytes: &[u8]) -> Result<(), String> {
    let text = decode(bytes);
    check_javadoc_text(&format!("/**{text}*/"))
}

/// Checks every traditional comment in `text`, and `text` as a whole read as one.
pub fn check_javadoc_text(text: &str) -> Result<(), String> {
    let lexed = lex(text);
    let whole = Span::new(0, text.len() as u32);
    let spans = lexed
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::DocComment)
        .map(|t| t.span)
        .chain([whole]);
    for span in spans {
        let javadoc = parse_javadoc(text, span);
        check_javadoc(text, &javadoc).map_err(|e| format!("comment at {span:?}: {e}"))?;
        if parse_javadoc(text, span) != javadoc {
            return Err(format!(
                "comment at {span:?}: parsing again gives another result"
            ));
        }
    }
    Ok(())
}

fn decode(bytes: &[u8]) -> String {
    match SourceText::decode(bytes) {
        Ok(source) => source.text().to_owned(),
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Checks a traditional comment parsed from `text`.
pub fn check_javadoc(text: &str, javadoc: &Javadoc) -> Result<(), String> {
    let checker = Checker::new(text, javadoc.span, true, true)?;
    let mut siblings = Siblings::new(javadoc.span);
    for node in &javadoc.description {
        checker.inline(node, javadoc.span, 0)?;
        siblings.next(node.span(), "description node")?;
    }
    for tag in &javadoc.tags {
        checker.block_tag(tag, javadoc.span, |body, span| {
            let mut siblings = Siblings::new(span);
            for node in body {
                checker.inline(node, span, 0)?;
                siblings.next(node.span(), "body node")?;
            }
            Ok(())
        })?;
        siblings.next(tag.span, "block tag")?;
    }
    checker.diagnostics(&javadoc.diagnostics)
}

/// How deep the structure may go: the parsers' limit, and a few levels for the nodes around
/// each level of nesting.
const MAX_DEPTH: usize = 4 * crate::MAX_NESTING + 16;

/// Checks the spans of a comment parsed from `text`, which is at `bounds`.
struct Checker<'a> {
    text: &'a str,
    bounds: Span,
    /// The Unicode escapes in the comment, which no span may split.
    escapes: Vec<(u32, u32)>,
    /// Whether text values are what their spans cover, as in traditional comments.
    exact_text: bool,
    /// Whether a line break covers one line terminator only.
    one_line_break: bool,
}

/// Checks that spans come in order without overlapping, within a parent.
struct Siblings {
    parent: Span,
    end: u32,
}

impl Siblings {
    fn new(parent: Span) -> Siblings {
        Siblings {
            parent,
            end: parent.start,
        }
    }

    fn next(&mut self, span: Span, what: &str) -> Result<(), String> {
        if span.start < self.end {
            return Err(format!(
                "{what} at {span:?} starts before the previous one ends, at {}",
                self.end
            ));
        }
        if span.end > self.parent.end || span.start < self.parent.start {
            return Err(format!(
                "{what} at {span:?} is outside its parent, {:?}",
                self.parent
            ));
        }
        self.end = span.end;
        Ok(())
    }
}

impl<'a> Checker<'a> {
    fn new(
        text: &'a str,
        bounds: Span,
        exact_text: bool,
        one_line_break: bool,
    ) -> Result<Checker<'a>, String> {
        let (start, end) = (bounds.start as usize, bounds.end as usize);
        if start > end || end > text.len() {
            return Err(format!("the comment's span {bounds:?} is out of bounds"));
        }
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return Err(format!("the comment's span {bounds:?} splits a character"));
        }
        let escapes = escape::chars(text, start, end)
            .filter(|c| c.escaped)
            .map(|c| (c.start, c.end))
            .collect();
        Ok(Checker {
            text,
            bounds,
            escapes,
            exact_text,
            one_line_break,
        })
    }

    /// Checks that `span` is in the comment, on character boundaries, doesn't split an escape,
    /// and is within `parent`.
    fn span(&self, span: Span, parent: Span, what: &str) -> Result<(), String> {
        let fail = |problem: &str| Err(format!("{what} at {span:?}: {problem}"));
        if span.start > span.end {
            return fail("ends before it starts");
        }
        if span.start < self.bounds.start || span.end > self.bounds.end {
            return fail(&format!("outside the comment, {:?}", self.bounds));
        }
        if span.start < parent.start || span.end > parent.end {
            return fail(&format!("outside its parent, {parent:?}"));
        }
        let (start, end) = (span.start as usize, span.end as usize);
        if !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return fail("splits a character");
        }
        for offset in [span.start, span.end] {
            let i = self.escapes.partition_point(|&(s, _)| s < offset);
            if let Some(&(s, e)) = i.checked_sub(1).map(|i| &self.escapes[i])
                && s < offset
                && offset < e
            {
                return fail(&format!("splits the Unicode escape at {s}..{e}"));
            }
        }
        Ok(())
    }

    fn depth(&self, depth: usize, span: Span) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("nested more than {MAX_DEPTH} deep at {span:?}"));
        }
        Ok(())
    }

    fn text_node(&self, text: &Text, parent: Span, what: &str) -> Result<(), String> {
        self.span(text.span, parent, what)?;
        if text.text.contains(['\n', '\r']) {
            return Err(format!(
                "{what} at {:?} has a line break: {:?}",
                text.span, text.text
            ));
        }
        if self.exact_text {
            let cooked = escape::cook(self.text, text.span.start as usize, text.span.end as usize);
            if cooked != text.text {
                return Err(format!(
                    "{what} at {:?} is {:?}, but its span reads {cooked:?}",
                    text.span, text.text
                ));
            }
        }
        Ok(())
    }

    /// A name or other text that is part of a larger node. It may be empty.
    fn part(&self, text: &Text, parent: Span, what: &str) -> Result<(), String> {
        self.span(text.span, parent, what)
    }

    fn inline(&self, node: &Inline, parent: Span, depth: usize) -> Result<(), String> {
        let span = node.span();
        self.depth(depth, span)?;
        self.span(span, parent, "inline node")?;
        match node {
            Inline::Text(text) => {
                if text.text.is_empty() {
                    return Err(format!("empty text at {span:?}"));
                }
                self.text_node(text, parent, "text")
            }
            Inline::LineBreak(span) => {
                let raw = escape::cook(self.text, span.start as usize, span.end as usize);
                let rest = raw
                    .strip_prefix("\r\n")
                    .or_else(|| raw.strip_prefix(['\r', '\n']));
                match rest {
                    Some(rest) if !(self.one_line_break && rest.contains(['\r', '\n'])) => Ok(()),
                    _ => Err(format!("line break at {span:?} reads {raw:?}")),
                }
            }
            Inline::Escape(escape) => {
                let raw = escape::cook(self.text, span.start as usize, span.end as usize);
                if raw == format!("@{}", escape.ch) && matches!(escape.ch, '@' | '*' | '/') {
                    Ok(())
                } else {
                    Err(format!("escape at {span:?} reads {raw:?}"))
                }
            }
            Inline::Entity(entity) => {
                let raw = escape::cook(self.text, span.start as usize, span.end as usize);
                if raw == format!("&{};", entity.name) {
                    Ok(())
                } else {
                    Err(format!("entity at {span:?} reads {raw:?}"))
                }
            }
            Inline::StartElement(element) => {
                self.part(&element.name, span, "element name")?;
                let mut siblings = Siblings::new(span);
                siblings.next(element.name.span, "element name")?;
                for attribute in &element.attributes {
                    self.attribute(attribute, span, depth + 1)?;
                    siblings.next(attribute.span, "attribute")?;
                }
                Ok(())
            }
            Inline::EndElement(element) => self.part(&element.name, span, "element name"),
            Inline::HtmlComment(comment) => self.inlines(&comment.content, span, depth + 1),
            Inline::Tag(tag) => self.inline_tag(tag, depth),
        }
    }

    fn inlines(&self, nodes: &[Inline], parent: Span, depth: usize) -> Result<(), String> {
        let mut siblings = Siblings::new(parent);
        for node in nodes {
            self.inline(node, parent, depth)?;
            siblings.next(node.span(), "inline node")?;
        }
        Ok(())
    }

    fn attribute(&self, attribute: &Attribute, parent: Span, depth: usize) -> Result<(), String> {
        self.span(attribute.span, parent, "attribute")?;
        self.part(&attribute.name, attribute.span, "attribute name")?;
        if let Some(value) = &attribute.value {
            self.span(value.span, attribute.span, "attribute value")?;
            self.inlines(&value.content, value.span, depth + 1)?;
        }
        Ok(())
    }

    fn inline_tag(&self, tag: &InlineTag, depth: usize) -> Result<(), String> {
        let span = tag.span;
        self.part(&tag.name, span, "tag name")?;
        let mut siblings = Siblings::new(span);
        siblings.next(tag.name.span, "tag name")?;
        let mut content = |nodes: &[Inline]| -> Result<(), String> {
            for node in nodes {
                self.inline(node, span, depth + 1)?;
                siblings.next(node.span(), "tag content")?;
            }
            Ok(())
        };
        match &tag.content {
            InlineTagContent::Code(nodes)
            | InlineTagContent::Literal(nodes)
            | InlineTagContent::Return(nodes)
            | InlineTagContent::Summary(nodes)
            | InlineTagContent::Other(nodes) => content(nodes),
            InlineTagContent::DocRoot => Ok(()),
            InlineTagContent::Index { term, description } => {
                if let Some(term) = term {
                    self.part(term, span, "index term")?;
                }
                content(description)
            }
            InlineTagContent::InheritDoc { supertype } => match supertype {
                Some(reference) => self.reference(reference, span),
                None => Ok(()),
            },
            InlineTagContent::Link { reference, label }
            | InlineTagContent::LinkPlain { reference, label } => {
                if let Some(reference) = reference {
                    self.reference(reference, span)?;
                }
                content(label)
            }
            InlineTagContent::Snippet(snippet) => {
                for attribute in &snippet.attributes {
                    self.span(attribute.span, span, "snippet attribute")?;
                    self.part(&attribute.name, attribute.span, "snippet attribute name")?;
                    if let Some(value) = &attribute.value {
                        self.part(value, attribute.span, "snippet attribute value")?;
                    }
                }
                match &snippet.body {
                    Some(body) => content(body),
                    None => Ok(()),
                }
            }
            InlineTagContent::SystemProperty { name } => match name {
                Some(name) => self.part(name, span, "property name"),
                None => Ok(()),
            },
            InlineTagContent::Value { format, reference } => {
                if let Some(format) = format {
                    self.part(format, span, "value format")?;
                }
                match reference {
                    Some(reference) => self.reference(reference, span),
                    None => Ok(()),
                }
            }
        }
    }

    fn reference(&self, reference: &Reference, parent: Span) -> Result<(), String> {
        let span = reference.span;
        self.span(span, parent, "reference")?;
        if let Some(module) = &reference.module {
            self.span(module.span, span, "module")?;
        }
        for ty in reference.types() {
            self.span(ty.span, span, "type")?;
            self.span(ty.name.span, ty.span, "type name")?;
            let mut siblings = Siblings::new(ty.name.span);
            for segment in &ty.name.segments {
                self.part(segment, ty.name.span, "name segment")?;
                siblings.next(segment.span, "name segment")?;
            }
            for argument in &ty.type_arguments {
                if let TypeArgument::Wildcard { span: wildcard, .. } = argument {
                    self.span(*wildcard, ty.span, "wildcard")?;
                }
            }
        }
        if let Some(member) = &reference.member {
            self.part(&member.name, span, "member name")?;
            if let Some(parameters) = &member.parameters {
                self.span(parameters.span, span, "parameters")?;
                let mut siblings = Siblings::new(parameters.span);
                for parameter in &parameters.list {
                    self.span(parameter.span, parameters.span, "parameter")?;
                    siblings.next(parameter.span, "parameter")?;
                }
            }
        }
        if let Some(fragment) = &reference.fragment {
            self.part(fragment, span, "fragment")?;
        }
        Ok(())
    }

    fn block_tag<B>(
        &self,
        tag: &BlockTag<B>,
        parent: Span,
        body: impl FnOnce(&B, Span) -> Result<(), String>,
    ) -> Result<(), String> {
        let span = tag.span;
        self.span(span, parent, "block tag")?;
        self.part(&tag.name, span, "block tag name")?;
        let mut siblings = Siblings::new(span);
        siblings.next(tag.name.span, "block tag name")?;
        if let Some(argument) = &tag.argument {
            let argument_span = match argument {
                TagArgument::Param(param) => {
                    self.part(&param.name, param.span, "parameter name")?;
                    param.span
                }
                TagArgument::Reference(reference) => {
                    self.reference(reference, span)?;
                    reference.span
                }
                TagArgument::Quoted(text) | TagArgument::Url(text) => text.span,
                TagArgument::SerialField { name, ty } => {
                    self.part(name, span, "field name")?;
                    match ty {
                        Some(ty) => name.span.cover(ty.span),
                        None => name.span,
                    }
                }
            };
            self.span(argument_span, span, "argument")?;
            siblings.next(argument_span, "argument")?;
        }
        let body_span = Span::new(siblings.end, span.end);
        body(&tag.body, body_span)
    }

    fn diagnostics(&self, diagnostics: &[Diagnostic]) -> Result<(), String> {
        let mut previous = self.bounds.start;
        for diagnostic in diagnostics {
            self.span(
                diagnostic.span,
                self.bounds,
                &format!("{:?}", diagnostic.kind),
            )?;
            if diagnostic.span.start < previous {
                return Err(format!("{diagnostic:?} is out of order"));
            }
            previous = diagnostic.span.start;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagnosticKind;

    #[test]
    fn accepts_the_parsers() {
        for text in [
            "",
            "x",
            "/** a {@code b} <i>c</i> &amp; @@ */",
            "/**\n * @param x y\n * {@link A#b(int) c}\n */",
            "/** \\u0041 {@code \\u007d */",
            "/** {@summary {@summary {@link X}}} */",
        ] {
            check_javadoc_text(text).unwrap();
            check_javadoc_bytes(text.as_bytes()).unwrap();
        }
    }

    #[test]
    fn rejects_bad_spans() {
        let text = "/** abc \\u0041 */";
        let mut javadoc = parse_javadoc(text, Span::new(0, text.len() as u32));
        javadoc.diagnostics.push(Diagnostic {
            kind: DiagnosticKind::MalformedHtml,
            span: Span::new(10, 11),
        });
        let message = check_javadoc(text, &javadoc).unwrap_err();
        assert!(message.contains("splits the Unicode escape"), "{message}");

        let mut javadoc = parse_javadoc(text, Span::new(0, text.len() as u32));
        if let Some(Inline::Text(t)) = javadoc.description.first_mut() {
            t.text.push('!');
        }
        let message = check_javadoc(text, &javadoc).unwrap_err();
        assert!(message.contains("but its span reads"), "{message}");

        let mut javadoc = parse_javadoc(text, Span::new(0, text.len() as u32));
        javadoc.description.push(javadoc.description[0].clone());
        let message = check_javadoc(text, &javadoc).unwrap_err();
        assert!(
            message.contains("starts before the previous one ends"),
            "{message}"
        );
    }
}
