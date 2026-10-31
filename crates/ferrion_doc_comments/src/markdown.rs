//! Markdown documentation comments: runs of `///` lines (JEP 467, Java 23 and later).
//!
//! The Markdown is CommonMark with GFM tables, read by `pulldown-cmark`. On top of it come
//! javadoc's block tags, inline tags, and links to program elements:
//!
//! - **Inline tags** (`{@code ...}`, `{@link ...}`) are found first, anywhere but in code spans
//!   and code blocks, and an escaped `\{@` isn't one. A tag runs to its matching `}`, across lines
//!   and Markdown structure, and to the end of the comment when unclosed, as in javadoc 27. Its
//!   content is read as in a traditional comment, so `{@code a*b*c}` has no emphasis. Before the
//!   Markdown parser reads a section, each tag is replaced by inert text of the same length, which
//!   keeps offsets and stands in for the tag in the structure.
//! - **Block tags** start at an `@` that begins a line, after any whitespace, outside code spans,
//!   code blocks, and inline tags. HTML blocks don't hide them: javadoc 27 ends an HTML block's
//!   paragraph at an `@param` line inside it. `\@` and `&#064;` are a literal `@`.
//! - **Sections** (the main description and each block tag's body) are separate Markdown
//!   documents, so link reference definitions apply within their section, as the specification
//!   says.
//! - **Links to program elements** are reference links (`[List]`, `[List][]`, `[a list][List]`)
//!   whose label has no link reference definition and reads as a reference. Labels that don't,
//!   such as `[1]` or `[see above]`, stay text, as in javadoc 27.

use std::ops::Range;

use ferrion_lexer::{Token, TokenKind};
use pulldown_cmark::{
    BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser as CmarkParser, Tag,
    TextMergeWithOffset,
};

use crate::content::Content;
use crate::escape;
use crate::inline::{Parser, is_blank};
use crate::model::{BlockTag, Diagnostic, DiagnosticKind, InlineTag, Reference, Text};
use crate::{MAX_NESTING, Span, clamp_span, reference};

/// A parsed Markdown documentation comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownComment {
    /// From the first line's `///` to the end of the last line.
    pub span: Span,
    /// The main description: the blocks before the first block tag.
    pub description: Vec<Block>,
    /// Block tags in source order. Each one's body is Markdown.
    pub tags: Vec<BlockTag<Vec<Block>>>,
    /// Problems found, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// A Markdown block. Spans cover the block as written, with the `///` and indentation of the
/// lines it spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph {
        span: Span,
        content: Vec<Inline>,
    },
    /// An ATX (`# Title`) or setext (`Title` underlined with `=` or `-`) heading.
    Heading {
        span: Span,
        /// 1 to 6.
        level: u8,
        setext: bool,
        content: Vec<Inline>,
    },
    BlockQuote {
        span: Span,
        blocks: Vec<Block>,
    },
    List {
        span: Span,
        /// The number of the first item of an ordered list, or `None` for a bullet list.
        start: Option<u64>,
        /// Whether no item has blank lines between its blocks or after it (CommonMark's tight
        /// lists). The items of a tight list hold paragraphs too, as a loose list's do.
        tight: bool,
        items: Vec<ListItem>,
    },
    /// A fenced or indented code block.
    CodeBlock {
        span: Span,
        /// The info string of a fenced block, such as `java`, or `None` for an indented block.
        info: Option<String>,
        /// The code, without fences or indentation, one line ending in `\n` per line.
        text: String,
    },
    /// An HTML block: its lines as [`Inline::Html`], and any inline tags in it.
    Html {
        span: Span,
        content: Vec<Inline>,
    },
    ThematicBreak {
        span: Span,
    },
    /// A GFM table.
    Table {
        span: Span,
        alignments: Vec<Alignment>,
        head: TableRow,
        rows: Vec<TableRow>,
    },
    /// A link reference definition, `[label]: destination "title"`.
    LinkDefinition {
        span: Span,
        label: String,
        destination: String,
        title: Option<String>,
    },
}

impl Block {
    pub fn span(&self) -> Span {
        match self {
            Block::Paragraph { span, .. }
            | Block::Heading { span, .. }
            | Block::BlockQuote { span, .. }
            | Block::List { span, .. }
            | Block::CodeBlock { span, .. }
            | Block::Html { span, .. }
            | Block::ThematicBreak { span }
            | Block::Table { span, .. }
            | Block::LinkDefinition { span, .. } => *span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub span: Span,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub span: Span,
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub span: Span,
    pub content: Vec<Inline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

/// Markdown inline content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    /// Text, with Markdown's backslash escapes and entities resolved, so its value can differ from
    /// what its span covers.
    Text(Text),
    /// A code span. The text is its content, as CommonMark reads it.
    Code(Text),
    Emphasis {
        span: Span,
        content: Vec<Inline>,
    },
    Strong {
        span: Span,
        content: Vec<Inline>,
    },
    Link {
        span: Span,
        kind: LinkKind,
        target: LinkTarget,
        title: String,
        /// The link text.
        content: Vec<Inline>,
    },
    Image {
        span: Span,
        kind: LinkKind,
        destination: String,
        title: String,
        /// The image description.
        content: Vec<Inline>,
    },
    /// Inline HTML, or a line of an HTML block.
    Html(Text),
    /// A line break within a paragraph: the line terminator and the next line's `///` and
    /// indentation.
    SoftBreak(Span),
    /// A hard line break: a backslash or two spaces, and the line break after it.
    HardBreak(Span),
    Tag(Box<InlineTag>),
}

impl Inline {
    pub fn span(&self) -> Span {
        match self {
            Inline::Text(text) | Inline::Code(text) | Inline::Html(text) => text.span,
            Inline::Emphasis { span, .. }
            | Inline::Strong { span, .. }
            | Inline::Link { span, .. }
            | Inline::Image { span, .. }
            | Inline::SoftBreak(span)
            | Inline::HardBreak(span) => *span,
            Inline::Tag(tag) => tag.span,
        }
    }
}

/// How a link is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// `[text](destination)`
    Inline,
    /// `[text][label]`
    Reference,
    /// `[label][]`
    Collapsed,
    /// `[label]`
    Shortcut,
    /// `<https://example.com>`
    Autolink,
    /// `<user@example.com>`
    Email,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A URL, from the link itself or a link reference definition.
    Url(String),
    /// A program element, from a label with no link reference definition, such as `[List]`.
    Element(Box<Reference>),
}

/// Groups the [`DocLineComment`](TokenKind::DocLineComment) tokens in `tokens` into the runs that
/// form Markdown documentation comments, returning each run's spans for [`parse_markdown`].
///
/// A run continues while the next line, after any whitespace, starts with a `///` comment, as
/// JEP 467 says and javadoc 27 does: a blank line, a line of code, or any other comment ends it,
/// and a `///` comment after code on the same line starts one. Whether a run documents anything,
/// and whether `///` comments are documentation at all (Java 23 and later), is for the caller to
/// decide.
pub fn markdown_comment_runs(tokens: &[Token]) -> Vec<Vec<Span>> {
    let mut runs = Vec::new();
    let mut run: Vec<Span> = Vec::new();
    // Line terminators since the last `///` comment, or `None` if anything else came between.
    let mut breaks = Some(0);
    for token in tokens {
        match token.kind {
            TokenKind::DocLineComment => {
                if breaks != Some(1) && !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
                run.push(token.span);
                breaks = Some(0);
            }
            TokenKind::Whitespace => {}
            TokenKind::LineTerminator => breaks = breaks.map(|n| n + 1),
            _ => breaks = None,
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

/// Parses the Markdown documentation comment whose lines are at `lines` in `text`.
///
/// Each span is a [`DocLineComment`](TokenKind::DocLineComment) token's span, as
/// [`markdown_comment_runs`] groups them, in source order. The `///` (possibly written with
/// Unicode escapes) and the lines' common indentation are taken out, as JEP 467 describes; what is
/// between one line's span and the next stands for a line break. Spans that are out of bounds or
/// split a character are shrunk to fit, and a span that starts before the previous one ends is
/// ignored.
pub fn parse_markdown(text: &str, lines: &[Span]) -> MarkdownComment {
    let (content, span) = content(text, lines);
    let len = content.len();
    let code = code_ranges(&content.text);
    let mut diagnostics = Vec::new();
    let tags = find_inline_tags(text, &content, &code, &mut diagnostics);
    let starts = block_tag_starts(&content.text, &code, &tags);
    // Each block tag's head is read from the content, and its body as Markdown.
    let (tag_ranges, tags): (Vec<_>, Vec<_>) = tags
        .into_iter()
        .map(|(range, tag)| (range, Some(tag)))
        .unzip();
    let mut sections = Sections {
        content: &content,
        tag_ranges,
        tags,
        diagnostics: &mut diagnostics,
    };
    let description = sections.blocks(0..starts.first().copied().unwrap_or(len), false);
    let mut block_tags = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(len);
        let mut parser = Parser::new(text, &content, true);
        parser.pos = start;
        parser.end = end;
        let head = parser.block_tag_head();
        sections.diagnostics.append(&mut parser.diagnostics);
        let body = sections.blocks(head.end..end, true);
        let head_span = content.span(head.start..head.end);
        let end = body
            .last()
            .map_or(head_span.end, |block| block.span().end.max(head_span.end));
        block_tags.push(BlockTag {
            span: Span::new(head_span.start, end),
            name: head.name,
            kind: head.kind,
            argument: head.argument,
            body,
        });
    }
    diagnostics.sort_by_key(|d| (d.span.start, d.span.end));
    MarkdownComment {
        span,
        description,
        tags: block_tags,
        diagnostics,
    }
}

/// The content of the comment: each line after its `///`, without the lines' common indentation,
/// joined by line breaks. Also returns the comment's span.
fn content(text: &str, lines: &[Span]) -> (Content, Span) {
    let mut kept: Vec<Vec<escape::Char>> = Vec::new();
    let mut bounds: Vec<(usize, usize)> = Vec::new();
    for &line in lines {
        let (start, end) = clamp_span(text, line);
        if bounds.last().is_some_and(|&(_, previous)| start < previous) {
            continue;
        }
        let mut chars: Vec<_> = escape::chars(text, start, end).collect();
        let blanks = chars.iter().take_while(|c| is_blank(c.ch)).count();
        let slashes = chars[blanks..]
            .iter()
            .take(3)
            .take_while(|c| c.ch == '/')
            .count();
        chars.drain(..blanks + slashes);
        kept.push(chars);
        bounds.push((start, end));
    }
    // JEP 467: lines are shifted left by the least indentation of the lines that aren't blank.
    let indent = kept
        .iter()
        .filter(|chars| chars.iter().any(|c| !is_blank(c.ch)))
        .map(|chars| chars.iter().take_while(|c| is_blank(c.ch)).count())
        .min()
        .unwrap_or(0);
    let first =
        |i: usize, chars: &[escape::Char]| chars.first().map_or(bounds[i].1, |c| c.start as usize);
    let mut content = Content::new(0);
    for (i, chars) in kept.iter().enumerate() {
        let blanks = chars.iter().take_while(|c| is_blank(c.ch)).count();
        let chars = &chars[blanks.min(indent)..];
        if i == 0 {
            content = Content::new(first(i, chars) as u32);
        } else {
            let line_start = first(i, chars);
            content.push_mapped('\n', bounds[i - 1].1 as u32..line_start as u32);
        }
        for &c in chars {
            content.push_char(c);
        }
    }
    let span = match (bounds.first(), bounds.last()) {
        (Some(&(start, _)), Some(&(_, end))) => Span::new(start as u32, end as u32),
        _ => {
            let at = lines
                .first()
                .map_or(0, |&line| clamp_span(text, line).0 as u32);
            Span::new(at, at)
        }
    };
    (content, span)
}

/// The options javadoc's Markdown has: CommonMark and GFM tables.
fn options() -> Options {
    Options::ENABLE_TABLES
}

/// The ranges of code spans and code blocks in `text`, sorted and merged, where inline tags and
/// block tags aren't recognized.
fn code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (event, range) in CmarkParser::new_ext(text, options()).into_offset_iter() {
        if matches!(event, Event::Code(_) | Event::Start(Tag::CodeBlock(_))) {
            ranges.push(range);
        }
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

fn in_ranges(ranges: &[Range<usize>], offset: usize) -> bool {
    let i = ranges.partition_point(|r| r.start <= offset);
    i > 0 && offset < ranges[i - 1].end
}

/// Finds the inline tags outside code, in order, with their content ranges.
fn find_inline_tags(
    source: &str,
    content: &Content,
    code: &[Range<usize>],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(Range<usize>, Box<InlineTag>)> {
    let text = &content.text;
    let mut tags = Vec::new();
    let mut parser = Parser::new(source, content, true);
    let mut from = 0;
    while let Some(i) = text[from..].find("{@") {
        let start = from + i;
        from = start + 2;
        let backslashes = text[..start]
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count();
        if backslashes % 2 == 1 || in_ranges(code, start) {
            continue;
        }
        parser.pos = start;
        if let crate::Inline::Tag(tag) = parser.inline_tag(0) {
            from = parser.pos;
            tags.push((start..parser.pos, tag));
        }
    }
    diagnostics.append(&mut parser.diagnostics);
    tags
}

/// The offsets of the `@`s that start block tags.
fn block_tag_starts(
    text: &str,
    code: &[Range<usize>],
    tags: &[(Range<usize>, Box<InlineTag>)],
) -> Vec<usize> {
    let in_tag = |offset: usize| {
        let i = tags.partition_point(|(range, _)| range.start <= offset);
        i > 0 && offset < tags[i - 1].0.end
    };
    let mut starts = Vec::new();
    let line_starts = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1));
    for line in line_starts {
        let at = line + text[line..].len() - text[line..].trim_start_matches(is_blank).len();
        if text[at..].starts_with('@') && !in_ranges(code, at) && !in_tag(at) {
            starts.push(at);
        }
    }
    starts
}

/// Reads sections of a comment as Markdown.
struct Sections<'a> {
    content: &'a Content,
    /// The content ranges of the inline tags, in order.
    tag_ranges: Vec<Range<usize>>,
    /// The inline tags, taken as they are placed in the structure.
    tags: Vec<Option<Box<InlineTag>>>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

/// The text that stands in for an inline tag of `len` bytes while the Markdown parser reads a
/// section: a character from the Private Use Area, which Markdown treats like a letter, repeated,
/// and padded with `x`s to the same length. Inline tags are at least 3 bytes (`{@a`), so there is
/// always one.
fn placeholder(len: usize) -> String {
    let mut text = "\u{E000}".repeat(len / 3);
    text.push_str(&"x".repeat(len % 3));
    text
}

impl Sections<'_> {
    /// Reads `range` of the content as Markdown blocks, leaving out leading blank lines, and
    /// with `after_tag`, the whitespace after a block tag's name or argument too.
    fn blocks(&mut self, range: Range<usize>, after_tag: bool) -> Vec<Block> {
        let text = &self.content.text;
        let start = range.start.min(text.len());
        let end = range.end.clamp(start, text.len());
        let section = &text[start..end];
        let start = if after_tag {
            start + section.len() - section.trim_start_matches([' ', '\t', '\x0C', '\n']).len()
        } else {
            // The first line's indentation can matter, as in an indented code block.
            let blank = section.len() - section.trim_start_matches([' ', '\t', '\x0C', '\n']).len();
            start + section[..blank].rfind('\n').map_or(0, |i| i + 1)
        };
        let mut filled = String::with_capacity(end - start);
        let mut copied = start;
        for tag_range in &self.tag_ranges {
            if start <= tag_range.start && tag_range.end <= end {
                filled.push_str(&text[copied..tag_range.start]);
                filled.push_str(&placeholder(tag_range.len()));
                copied = tag_range.end;
            }
        }
        filled.push_str(&text[copied..end]);
        // A label with no definition that reads as a reference links to a program element. The
        // callback makes it a link, which the parser then marks as resolved this way.
        let callback = |link: BrokenLink<'_>| {
            reference::is_reference(link.reference.trim())
                .then_some((CowStr::Borrowed(""), CowStr::Borrowed("")))
        };
        let parser = CmarkParser::new_with_broken_link_callback(&filled, options(), Some(callback));
        let mut definitions: Vec<Definition> = parser
            .reference_definitions()
            .iter()
            .map(|(label, def)| {
                (
                    def.span.clone(),
                    label.to_owned(),
                    def.dest.to_string(),
                    def.title.as_ref().map(|t| t.to_string()),
                )
            })
            .collect();
        definitions.sort_by_key(|d| d.0.start);
        let mut builder = Builder {
            sections: self,
            filled: &filled,
            offset: start,
            stack: vec![Frame::new(FrameKind::Root, 0..filled.len())],
            suppressed: 0,
            definitions: definitions.into_iter().peekable(),
        };
        for (event, range) in TextMergeWithOffset::new(parser.into_offset_iter()) {
            builder.event(event, range);
        }
        builder.finish()
    }
}

#[derive(Debug)]
enum FrameKind {
    Root,
    Paragraph,
    Heading(u8),
    BlockQuote,
    CodeBlock(Option<String>),
    Html,
    List(Option<u64>),
    Item,
    Table(Vec<Alignment>),
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Link {
        kind: LinkKind,
        target: LinkTarget,
        title: String,
    },
    Image {
        kind: LinkKind,
        destination: String,
        title: String,
    },
    /// A construct javadoc's Markdown doesn't have; its content goes to its parent.
    Transparent,
}

impl FrameKind {
    fn is_inline(&self) -> bool {
        matches!(
            self,
            FrameKind::Emphasis
                | FrameKind::Strong
                | FrameKind::Link { .. }
                | FrameKind::Image { .. }
        )
    }
}

/// A node on its way into the structure.
enum Node {
    Block(Block),
    Inline(Inline),
    Item(ListItem, bool),
    Row(TableRow, bool),
    Cell(TableCell),
}

impl Node {
    fn span(&self) -> Span {
        match self {
            Node::Block(block) => block.span(),
            Node::Inline(inline) => inline.span(),
            Node::Item(item, _) => item.span,
            Node::Row(row, _) => row.span,
            Node::Cell(cell) => cell.span,
        }
    }
}

struct Frame {
    kind: FrameKind,
    range: Range<usize>,
    children: Vec<Node>,
    code: String,
}

impl Frame {
    fn new(kind: FrameKind, range: Range<usize>) -> Frame {
        Frame {
            kind,
            range,
            children: Vec::new(),
            code: String::new(),
        }
    }
}

/// A link reference definition: its range, label, destination, and title.
type Definition = (Range<usize>, String, String, Option<String>);

/// Builds the structure of a section from the Markdown parser's events, with an explicit stack.
struct Builder<'a, 'b> {
    sections: &'a mut Sections<'b>,
    /// The section's text, with placeholders for inline tags.
    filled: &'a str,
    /// Where the section starts in the content.
    offset: usize,
    stack: Vec<Frame>,
    /// The number of open containers nested too deeply to have frames.
    suppressed: usize,
    /// The section's link reference definitions, which the parser's events leave out, in order.
    definitions: std::iter::Peekable<std::vec::IntoIter<Definition>>,
}

impl Builder<'_, '_> {
    fn span(&self, range: Range<usize>) -> Span {
        self.sections
            .content
            .span(self.offset + range.start..self.offset + range.end)
    }

    fn push(&mut self, node: Node) {
        if let Some(frame) = self.stack.last_mut() {
            frame.children.push(node);
        }
    }

    /// Adds the link reference definitions that start before `offset` to the open container.
    fn definitions_before(&mut self, offset: usize) {
        while let Some((range, ..)) = self.definitions.peek() {
            if range.start >= offset {
                return;
            }
            let Some((range, label, destination, title)) = self.definitions.next() else {
                return;
            };
            let span = self.span(range.start..range.end);
            let block = Block::LinkDefinition {
                span,
                label,
                destination: self.restore(&destination, range.clone()),
                title: title.map(|title| self.restore(&title, range)),
            };
            self.push_definition(block);
        }
    }

    /// Adds a link reference definition to the open container. A list holds items, so a
    /// definition the parser places in a list but outside its items (as it can after an empty
    /// item) goes into the last item.
    fn push_definition(&mut self, block: Block) {
        let Some(frame) = self.stack.last_mut() else {
            return;
        };
        if !matches!(frame.kind, FrameKind::List(_)) {
            frame.children.push(Node::Block(block));
            return;
        }
        let span = block.span();
        match frame
            .children
            .iter_mut()
            .rev()
            .find_map(|child| match child {
                Node::Item(item, _) => Some(item),
                _ => None,
            }) {
            Some(item) => {
                item.span = item.span.cover(span);
                item.blocks.push(block);
            }
            None => frame.children.push(Node::Item(
                ListItem {
                    span,
                    blocks: vec![block],
                },
                false,
            )),
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::End(_) => {
                self.definitions_before(range.end);
                if self.suppressed > 0 {
                    self.suppressed -= 1;
                } else if self.stack.len() > 1
                    && let Some(frame) = self.stack.pop()
                {
                    self.close(frame);
                }
                return;
            }
            _ => self.definitions_before(range.start),
        }
        match event {
            Event::Start(tag) => {
                let kind = self.frame_kind(tag, &range);
                if self.stack.len() > MAX_NESTING {
                    if self.suppressed == 0 {
                        let span = self.span(range.clone());
                        self.sections.diagnostics.push(Diagnostic {
                            kind: DiagnosticKind::NestingTooDeep,
                            span: Span::new(span.start, span.start),
                        });
                    }
                    self.suppressed += 1;
                } else {
                    self.stack.push(Frame::new(kind, range));
                }
            }
            Event::Text(text) => {
                if let Some(frame) = self.stack.last_mut()
                    && matches!(frame.kind, FrameKind::CodeBlock(_))
                {
                    let text = self.restore(&text, range);
                    if let Some(frame) = self.stack.last_mut() {
                        frame.code.push_str(&text);
                    }
                    return;
                }
                for node in self.split(&text, range, false) {
                    self.push(Node::Inline(node));
                }
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                for node in self.split(&text, range, true) {
                    self.push(Node::Inline(node));
                }
            }
            Event::Code(text) => {
                let text = self.restore(&text, range.clone());
                let span = self.span(range);
                self.push(Node::Inline(Inline::Code(Text { span, text })));
            }
            Event::SoftBreak => {
                let span = self.span(range);
                self.push(Node::Inline(Inline::SoftBreak(span)));
            }
            Event::HardBreak => {
                let span = self.span(range);
                self.push(Node::Inline(Inline::HardBreak(span)));
            }
            Event::Rule => {
                let span = self.span(range);
                self.push(Node::Block(Block::ThematicBreak { span }));
            }
            // Not enabled: math, footnotes, and task lists. Kept as text, should they appear.
            Event::InlineMath(text) | Event::DisplayMath(text) | Event::FootnoteReference(text) => {
                for node in self.split(&text, range, false) {
                    self.push(Node::Inline(node));
                }
            }
            Event::TaskListMarker(_) | Event::End(_) => {}
        }
    }

    fn frame_kind(&mut self, tag: Tag<'_>, range: &Range<usize>) -> FrameKind {
        match tag {
            Tag::Paragraph => FrameKind::Paragraph,
            Tag::Heading { level, .. } => FrameKind::Heading(level as u8),
            Tag::BlockQuote(_) => FrameKind::BlockQuote,
            Tag::CodeBlock(CodeBlockKind::Indented) => FrameKind::CodeBlock(None),
            Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                FrameKind::CodeBlock(Some(self.restore(&info, range.clone())))
            }
            Tag::HtmlBlock => FrameKind::Html,
            Tag::List(start) => FrameKind::List(start),
            Tag::Item => FrameKind::Item,
            Tag::Table(alignments) => FrameKind::Table(
                alignments
                    .into_iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::None => Alignment::None,
                        pulldown_cmark::Alignment::Left => Alignment::Left,
                        pulldown_cmark::Alignment::Center => Alignment::Center,
                        pulldown_cmark::Alignment::Right => Alignment::Right,
                    })
                    .collect(),
            ),
            Tag::TableHead => FrameKind::TableHead,
            Tag::TableRow => FrameKind::TableRow,
            Tag::TableCell => FrameKind::TableCell,
            Tag::Emphasis => FrameKind::Emphasis,
            Tag::Strong => FrameKind::Strong,
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let kind = link_kind(link_type);
                let target = if is_unknown(link_type) {
                    LinkTarget::Element(Box::new(self.element_reference(link_type, range.clone())))
                } else {
                    LinkTarget::Url(self.restore(&dest_url, range.clone()))
                };
                FrameKind::Link {
                    kind,
                    target,
                    title: self.restore(&title, range.clone()),
                }
            }
            Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            } => FrameKind::Image {
                kind: link_kind(link_type),
                destination: self.restore(&dest_url, range.clone()),
                title: self.restore(&title, range.clone()),
            },
            _ => FrameKind::Transparent,
        }
    }

    /// The reference of a link to a program element: its label, the last bracketed part of the
    /// link.
    fn element_reference(&mut self, link_type: LinkType, range: Range<usize>) -> Reference {
        let link = self.filled.get(range.clone()).unwrap_or_default();
        let label = match link_type {
            LinkType::ReferenceUnknown => last_brackets(link),
            _ => first_brackets(link),
        };
        let (start, end) = match label {
            Some((start, end)) => {
                let inner = &link[start..end];
                let trimmed = inner.trim_start();
                let start = start + inner.len() - trimmed.len();
                (start, start + trimmed.trim_end().len())
            }
            None => (0, link.len()),
        };
        let (start, end) = (
            self.offset + range.start + start,
            self.offset + range.start + end,
        );
        let (reference, ok) = reference::parse(self.sections.content, start, end);
        if !ok {
            self.sections.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::MalformedReference,
                span: reference.span,
            });
        }
        reference
    }

    /// Text from an event, with its inline tag placeholders replaced by the tags. `html` makes the
    /// text [`Inline::Html`], one node per line.
    fn split(&mut self, text: &str, range: Range<usize>, html: bool) -> Vec<Inline> {
        let mut nodes = Vec::new();
        let mut text_pos = 0;
        let mut source_pos = range.start;
        for i in self.tags_within(range.clone()) {
            let tag_range = &self.sections.tag_ranges[i];
            let local = tag_range.start - self.offset..tag_range.end - self.offset;
            if local.start < source_pos || self.sections.tags[i].is_none() {
                continue;
            }
            let Some(found) = text[text_pos..].find(&placeholder(local.len())) else {
                continue;
            };
            let before = &text[text_pos..text_pos + found];
            self.text_nodes(before, source_pos..local.start, html, &mut nodes);
            if let Some(tag) = self.sections.tags[i].take() {
                nodes.push(Inline::Tag(tag));
            }
            text_pos += found + local.len();
            source_pos = local.end;
        }
        self.text_nodes(&text[text_pos..], source_pos..range.end, html, &mut nodes);
        nodes
    }

    fn text_nodes(&self, text: &str, range: Range<usize>, html: bool, nodes: &mut Vec<Inline>) {
        if text.is_empty() {
            return;
        }
        if !html {
            let span = self.span(range);
            nodes.push(Inline::Text(Text {
                span,
                text: text.to_owned(),
            }));
            return;
        }
        // HTML is passed through as written, so it splits into lines where its source does.
        let same = self.filled.get(range.clone()) == Some(text);
        if !same {
            let text = text.trim_end_matches('\n');
            if !text.is_empty() {
                let span = self.span(range);
                nodes.push(Inline::Html(Text {
                    span,
                    text: text.to_owned(),
                }));
            }
            return;
        }
        let mut start = range.start;
        for line in text.split_inclusive('\n') {
            let line_text = line.strip_suffix('\n').unwrap_or(line);
            if !line_text.is_empty() {
                let span = self.span(start..start + line_text.len());
                nodes.push(Inline::Html(Text {
                    span,
                    text: line_text.to_owned(),
                }));
            }
            start += line.len();
        }
    }

    /// The indices of the inline tags within `range` of the section.
    fn tags_within(&self, range: Range<usize>) -> Range<usize> {
        let (start, end) = (self.offset + range.start, self.offset + range.end);
        let ranges = &self.sections.tag_ranges;
        let first = ranges.partition_point(|r| r.start < start);
        let last = ranges.partition_point(|r| r.start < end);
        let last = (first..last).find(|&i| ranges[i].end > end).unwrap_or(last);
        first..last
    }

    /// `text`, from an event at `range`, with any inline tag placeholders in it replaced by the
    /// tags' text as written. For places the structure has no room for a tag, such as a link's
    /// destination.
    fn restore(&self, text: &str, range: Range<usize>) -> String {
        let mut restored = text.to_owned();
        if !text.contains('\u{E000}') {
            return restored;
        }
        let content = &self.sections.content.text;
        let mut from = 0;
        for i in self.tags_within(range) {
            let tag_range = self.sections.tag_ranges[i].clone();
            let placeholder = placeholder(tag_range.len());
            if let Some(found) = restored[from..].find(&placeholder) {
                let found = from + found;
                restored.replace_range(
                    found..found + placeholder.len(),
                    &content[tag_range.clone()],
                );
                from = found + tag_range.len();
            }
        }
        restored
    }

    /// Turns a closed frame into a node of its parent.
    fn close(&mut self, frame: Frame) {
        if matches!(frame.kind, FrameKind::Transparent) {
            for child in frame.children {
                self.push(child);
            }
            return;
        }
        let node = self.node(frame);
        self.push(node);
    }

    fn node(&mut self, frame: Frame) -> Node {
        let mut range = frame.range.clone();
        if !frame.kind.is_inline() {
            // Block ranges end after the line break that ends their last line; spans don't.
            let text = self.filled.get(range.clone()).unwrap_or_default();
            range.end -= text.len() - text.trim_end_matches([' ', '\t', '\x0C', '\n']).len();
        }
        let mut span = self.span(range);
        // But they still cover their content, such as an HTML block's trailing spaces.
        span.end = frame
            .children
            .iter()
            .map(|child| child.span().end)
            .fold(span.end, u32::max);
        match frame.kind {
            FrameKind::Root | FrameKind::BlockQuote | FrameKind::Transparent => {
                let (blocks, _) = blocks(frame.children);
                Node::Block(Block::BlockQuote { span, blocks })
            }
            FrameKind::Paragraph => Node::Block(Block::Paragraph {
                span,
                content: inlines(frame.children),
            }),
            FrameKind::Heading(level) => {
                let setext = !self
                    .filled
                    .get(frame.range.clone())
                    .unwrap_or_default()
                    .trim_start()
                    .starts_with('#');
                Node::Block(Block::Heading {
                    span,
                    level,
                    setext,
                    content: inlines(frame.children),
                })
            }
            FrameKind::CodeBlock(info) => Node::Block(Block::CodeBlock {
                span,
                info,
                text: frame.code,
            }),
            FrameKind::Html => Node::Block(Block::Html {
                span,
                content: inlines(frame.children),
            }),
            FrameKind::List(start) => {
                let mut tight = false;
                let mut items = Vec::new();
                for child in frame.children {
                    match child {
                        Node::Item(item, item_tight) => {
                            tight |= item_tight;
                            items.push(item);
                        }
                        other => {
                            let span = other.span();
                            let (blocks, _) = blocks(vec![other]);
                            items.push(ListItem { span, blocks });
                        }
                    }
                }
                Node::Block(Block::List {
                    span,
                    start,
                    tight,
                    items,
                })
            }
            FrameKind::Item => {
                let (blocks, tight) = blocks(frame.children);
                Node::Item(ListItem { span, blocks }, tight)
            }
            FrameKind::Table(alignments) => {
                let mut head = None;
                let mut rows = Vec::new();
                for child in frame.children {
                    match child {
                        Node::Row(row, true) if head.is_none() => head = Some(row),
                        Node::Row(row, _) => rows.push(row),
                        _ => {}
                    }
                }
                Node::Block(Block::Table {
                    span,
                    alignments,
                    head: head.unwrap_or(TableRow {
                        span: Span::new(span.start, span.start),
                        cells: Vec::new(),
                    }),
                    rows,
                })
            }
            FrameKind::TableHead | FrameKind::TableRow => {
                let cells = frame
                    .children
                    .into_iter()
                    .filter_map(|child| match child {
                        Node::Cell(cell) => Some(cell),
                        _ => None,
                    })
                    .collect();
                Node::Row(
                    TableRow { span, cells },
                    matches!(frame.kind, FrameKind::TableHead),
                )
            }
            FrameKind::TableCell => Node::Cell(TableCell {
                span,
                content: inlines(frame.children),
            }),
            FrameKind::Emphasis => Node::Inline(Inline::Emphasis {
                span,
                content: inlines(frame.children),
            }),
            FrameKind::Strong => Node::Inline(Inline::Strong {
                span,
                content: inlines(frame.children),
            }),
            FrameKind::Link {
                kind,
                target,
                title,
            } => Node::Inline(Inline::Link {
                span,
                kind,
                target,
                title,
                content: inlines(frame.children),
            }),
            FrameKind::Image {
                kind,
                destination,
                title,
            } => Node::Inline(Inline::Image {
                span,
                kind,
                destination,
                title,
                content: inlines(frame.children),
            }),
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.definitions_before(usize::MAX);
        while self.stack.len() > 1 {
            if let Some(frame) = self.stack.pop() {
                self.close(frame);
            }
        }
        let root = self.stack.pop().map(|f| f.children).unwrap_or_default();
        blocks(root).0
    }
}

/// The blocks among `children`, with inline content between them made into paragraphs, as the
/// items of a tight list have. Also returns whether there was any such inline content.
fn blocks(children: Vec<Node>) -> (Vec<Block>, bool) {
    let mut blocks = Vec::new();
    let mut pending: Vec<Inline> = Vec::new();
    let mut loose_inlines = false;
    let flush = |blocks: &mut Vec<Block>, pending: &mut Vec<Inline>| {
        let content = std::mem::take(pending);
        let (Some(first), Some(last)) = (content.first(), content.last()) else {
            return;
        };
        let span = first.span().cover(last.span());
        blocks.push(Block::Paragraph { span, content });
    };
    for child in children {
        match child {
            Node::Block(block) => {
                flush(&mut blocks, &mut pending);
                blocks.push(block);
            }
            Node::Inline(Inline::Text(text)) if text.text.is_empty() => {}
            Node::Inline(inline) => {
                loose_inlines = true;
                pending.push(inline);
            }
            Node::Item(item, _) => {
                flush(&mut blocks, &mut pending);
                blocks.extend(item.blocks);
            }
            Node::Row(row, _) => {
                for cell in row.cells {
                    pending.extend(cell.content);
                }
            }
            Node::Cell(cell) => pending.extend(cell.content),
        }
    }
    flush(&mut blocks, &mut pending);
    (blocks, loose_inlines)
}

/// The inline content among `children`. Blocks can only be here when nesting was too deep;
/// their inline content is kept.
fn inlines(children: Vec<Node>) -> Vec<Inline> {
    let mut inlines = Vec::new();
    for child in children {
        match child {
            Node::Inline(Inline::Text(text)) if text.text.is_empty() => {}
            Node::Inline(inline) => inlines.push(inline),
            Node::Block(
                Block::Paragraph { content, .. }
                | Block::Heading { content, .. }
                | Block::Html { content, .. },
            ) => inlines.extend(content),
            Node::Cell(cell) => inlines.extend(cell.content),
            Node::Block(_) | Node::Item(..) | Node::Row(..) => {}
        }
    }
    inlines
}

fn link_kind(link_type: LinkType) -> LinkKind {
    match link_type {
        LinkType::Reference | LinkType::ReferenceUnknown => LinkKind::Reference,
        LinkType::Collapsed | LinkType::CollapsedUnknown => LinkKind::Collapsed,
        LinkType::Shortcut | LinkType::ShortcutUnknown => LinkKind::Shortcut,
        LinkType::Autolink => LinkKind::Autolink,
        LinkType::Email => LinkKind::Email,
        LinkType::Inline | LinkType::WikiLink { .. } => LinkKind::Inline,
    }
}

/// Whether a link's label had no definition, so the broken-link callback made it a link to a
/// program element.
fn is_unknown(link_type: LinkType) -> bool {
    matches!(
        link_type,
        LinkType::ReferenceUnknown | LinkType::CollapsedUnknown | LinkType::ShortcutUnknown
    )
}

/// The content of the first `[...]` in `text`, as a range: from after `[` to before the first `]`
/// that isn't escaped.
fn first_brackets(text: &str) -> Option<(usize, usize)> {
    let open = text.find('[')? + 1;
    let mut escaped = false;
    for (i, c) in text[open..].char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            ']' if !escaped => return Some((open, open + i)),
            _ => escaped = false,
        }
    }
    None
}

/// The content of the last `[...]` in `text`, which ends with `]`.
fn last_brackets(text: &str) -> Option<(usize, usize)> {
    let close = text.strip_suffix(']')?.len();
    let bytes = text.as_bytes();
    let mut i = close;
    while i > 0 {
        i -= 1;
        if bytes[i] == b'[' {
            let backslashes = bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count();
            if backslashes.is_multiple_of(2) {
                return Some((i + 1, close));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
