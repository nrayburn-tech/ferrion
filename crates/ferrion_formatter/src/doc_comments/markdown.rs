//! Markdown doc comments, runs of `///` lines from Java 23 (docs/design/formatter-completeness.md,
//! "Markdown"): the layout built from `ferrion_doc_comments`'s parse, and the comment's `///`
//! lines around the filled text.
//!
//! [`layout`] walks the parsed blocks in order, writing each as paragraphs of the [`Layout`]:
//!
//! - **Text** (a paragraph, an ATX heading, a block tag's name, argument, and first paragraph) is
//!   cut into words at whitespace, from the source of each of its lines past the `///`, the
//!   comment's common indentation, and its block quotes' `>`. A code span is never cut. A hard
//!   break ends a paragraph of the layout, so the line still breaks there, and one written with
//!   two trailing spaces, which go, is written with a backslash instead.
//! - **Copied blocks** (fenced code, tables, and HTML blocks) are a paragraph of the layout per
//!   line, holding the line's text past its containers' markers and indentation, as written.
//! - **Containers:** a list item's marker is the first word of its first line, and its later lines
//!   and blocks are indented past it, as are a block tag's; a block quote's `>` starts each of its
//!   lines. These are each paragraph's [`Prefix`].
//!
//! A comment the rules don't reflow has no layout. Its lines are then printed as line comments
//! are, each `///` at the comment's column, so every line is moved by the same amount (the
//! design's decision 4).

use ferrion_doc_comments::markdown::{Block, Inline, LinkKind, ListItem, parse_markdown};
use ferrion_doc_comments::{BlockTag, DiagnosticKind, InlineTagContent, Span, TagArgument};
use ferrion_syntax::comments::{DocCommentKind, MARKDOWN_DOC_COMMENT_RELEASE};
use ferrion_syntax::comments::{can_have_doc_comment, doc_comment};
use ferrion_syntax::{SyntaxKind, SyntaxTree, TokenId};
use pulldown_cmark::{Event, Options, Parser, Tag};

use super::javadoc::{ends_with_hyphen, has_escape};
use super::{Columns, Glue, Layout, Paragraph, Prefix, Word, WordText, fill};
use crate::document::{LINE_WIDTH, width};
use crate::regions::Region;

/// A run of `///` lines the rules reflow: from its `first` token to its `last`, printed as one
/// piece of copied text from `layout`.
#[derive(Debug)]
pub struct Run {
    pub first: TokenId,
    pub last: TokenId,
    pub layout: Layout,
}

/// The runs of `///` lines in `tree` that the rules reflow, in source order
/// (docs/design/formatter-completeness.md, "Which comments"). From release 23, a run is reflowed
/// when it is a declaration's doc comment, as javac finds it, with no blank line between them, as
/// GJF reflows it (observed), and when its [`layout`] doesn't fall back. A run in a disable region
/// (`regions`) is copied, and GJF leaves the doc comment of a field declaring several variables
/// as it is (observed), so Ferrion does too. Other `///` lines are line comments.
pub fn runs(tree: &SyntaxTree, regions: &[Region]) -> Vec<Run> {
    let mut runs = Vec::new();
    if tree.release() < MARKDOWN_DOC_COMMENT_RELEASE || !tree.text().contains("///") {
        return runs;
    }
    for node in tree.nodes() {
        if !can_have_doc_comment(node.kind()) {
            continue;
        }
        let Some(doc) = doc_comment(node) else {
            continue;
        };
        if doc.kind() != DocCommentKind::Markdown {
            continue;
        }
        let (first, last) = (doc.first_token().id(), doc.last_token().id());
        if runs.last().is_some_and(|run: &Run| run.first == first)
            || regions
                .iter()
                .any(|r| r.start <= last && r.end.is_none_or(|end| end >= first))
        {
            continue;
        }
        let declarators = node
            .children()
            .filter(|n| n.kind() == SyntaxKind::VariableDeclarator)
            .count();
        if declarators > 1 {
            continue;
        }
        // A blank line between the run and the declaration.
        let mut breaks = 0;
        let mut blank = false;
        for index in last.index() + 1..node.first_token().id().index() {
            match tree.token(TokenId::from_index(index)).kind() {
                SyntaxKind::Whitespace => {}
                SyntaxKind::LineTerminator => {
                    breaks += 1;
                    blank |= breaks >= 2;
                }
                _ => breaks = 0,
            }
        }
        if blank {
            continue;
        }
        let spans: Vec<Span> = doc.tokens().map(|t| t.span()).collect();
        if let Some(layout) = layout(tree.text(), &spans) {
            runs.push(Run {
                first,
                last,
                layout,
            });
        }
    }
    runs.sort_by_key(|run| run.first);
    runs.dedup_by_key(|run| run.first);
    runs
}

/// The layout of the Markdown doc comment whose `///` lines are at `lines` in `text`, in order,
/// or `None` if it isn't reflowed. GJF leaves a comment with a setext heading, an indented code
/// block, a thematic break, a link reference definition, or an unclosed inline tag as it is
/// (observed), and so does Ferrion. So it does a comment that the layout can't keep safely: one
/// with a Unicode escape for other than a letter or digit, a `{@snippet}`,
/// a tab in a list item's or block quote's indentation, a tag argument or reference javadoc can't
/// read, a code span with whitespace at the end of a line, an inline tag written across lines in
/// a block quote, a link destination in `<>`, a block tag whose body starts with fenced code, or an
/// HTML block in a block tag. A run whose first line starts with `////` isn't
/// a doc comment to GJF, and isn't reflowed either.
pub fn layout(text: &str, lines: &[Span]) -> Option<Layout> {
    let (first, last) = (lines.first()?, lines.last()?);
    let comment = text.get(first.start as usize..last.end as usize)?;
    if comment.starts_with("////") || has_escape(comment) {
        return None;
    }
    let source = source_lines(text, lines)?;
    let parsed = parse_markdown(text, lines);
    if parsed.diagnostics.iter().any(|d| {
        matches!(
            d.kind,
            DiagnosticKind::UnclosedInlineTag
                | DiagnosticKind::NestingTooDeep
                | DiagnosticKind::MissingTagName
                | DiagnosticKind::MalformedArgument
                | DiagnosticKind::MalformedReference
        )
    }) {
        return None;
    }
    let mut writer = Writer {
        text,
        lines: source,
        layout: Layout::default(),
        failed: false,
        gap: 0,
        blank: String::new(),
        rest: String::new(),
        lead: None,
        containers: Vec::new(),
        in_tag: false,
    };
    writer.blocks(&parsed.description);
    for (i, tag) in parsed.tags.iter().enumerate() {
        let before = i.checked_sub(1).map(|i| &parsed.tags[i]);
        writer.tag(tag, before);
    }
    (!writer.failed).then_some(writer.layout)
}

/// The comment's lines at `column`, the column its first `///` is at: the first without
/// indentation, the others with `column` spaces before their `///`. Each line is `///`, a space,
/// and its paragraph's prefix and text, or `///` alone when those are empty. An empty comment is
/// `///`.
pub fn render(layout: &Layout, text: &str, column: u32) -> Vec<String> {
    let filled = fill(
        layout,
        text,
        Columns {
            text: column + 4,
            copied: column + 4,
            limit: LINE_WIDTH,
        },
    );
    if filled.is_empty() {
        return vec!["///".to_owned()];
    }
    let margin = " ".repeat(column as usize);
    let mut out = Vec::with_capacity(filled.len());
    // The paragraph of the last line, and how many of its lines, blank lines before it included,
    // came before.
    let mut current = None;
    let mut seen = 0;
    for line in &filled {
        if current != Some(line.paragraph) {
            current = Some(line.paragraph);
            seen = 0;
        }
        let index = line.paragraph as usize;
        let prefix = &layout.prefixes[index];
        let blanks = if index == 0 {
            0
        } else {
            usize::from(layout.paragraphs[index].gap.saturating_sub(1))
        };
        let body = if seen < blanks {
            prefix.blank.to_string()
        } else if seen == blanks {
            format!("{}{}", prefix.first, line.text)
        } else {
            format!("{}{}", prefix.rest, line.text)
        };
        seen += 1;
        let body = body.trim_end_matches([' ', '\t', '\x0c']);
        let start = if out.is_empty() { "" } else { margin.as_str() };
        out.push(if body.is_empty() {
            format!("{start}///")
        } else {
            format!("{start}/// {body}")
        });
    }
    out
}

/// A line of the comment in the source.
#[derive(Clone, Copy, Debug)]
struct SourceLine {
    /// Where its `///` starts.
    start: u32,
    /// Where its content starts: past the `///` and the comment's common indentation, as the
    /// parser reads it (JEP 467).
    content: u32,
    /// Where it ends.
    end: u32,
}

/// The lines at `spans`, each a `///` comment, or `None` if one isn't.
fn source_lines(text: &str, spans: &[Span]) -> Option<Vec<SourceLine>> {
    let mut lines = Vec::with_capacity(spans.len());
    let mut indents = Vec::with_capacity(spans.len());
    let mut previous = 0;
    for span in spans {
        let line = text.get(span.start as usize..span.end as usize)?;
        if !line.starts_with("///") || span.start < previous || line.contains(['\n', '\r']) {
            return None;
        }
        previous = span.end;
        let blanks = line[3..].bytes().take_while(|&b| is_blank(b)).count();
        indents.push((blanks, 3 + blanks < line.len()));
        lines.push(SourceLine {
            start: span.start,
            content: span.start + 3,
            end: span.end,
        });
    }
    let common = indents
        .iter()
        .filter(|(_, text)| *text)
        .map(|&(blanks, _)| blanks)
        .min()
        .unwrap_or(0);
    for (line, (blanks, _)) in lines.iter_mut().zip(indents) {
        line.content += blanks.min(common) as u32;
    }
    Some(lines)
}

/// Space, tab, and form feed, as the parser reads indentation.
fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0c')
}

/// Whether a block gets a blank line before and after it, as GJF writes it (observed): headings,
/// fenced code, tables, and HTML blocks.
fn spaced(block: &Block) -> bool {
    matches!(
        block,
        Block::Heading { .. } | Block::CodeBlock { .. } | Block::Table { .. } | Block::Html { .. }
    )
}

/// Whether `item` ends with a block that gets a blank line after it, in it or in a list in it.
fn ends_spaced(item: &ListItem) -> bool {
    match item.blocks.last() {
        Some(Block::List { items, .. }) => items.last().is_some_and(ends_spaced),
        Some(block) => spaced(block),
        None => false,
    }
}

/// The span of a block tag's argument.
fn argument_span(argument: &TagArgument) -> Span {
    match argument {
        TagArgument::Param(name) => name.span,
        TagArgument::Reference(reference) => reference.span,
        TagArgument::Quoted(text) | TagArgument::Url(text) => text.span,
        TagArgument::SerialField { name, ty } => Span::new(
            name.span.start,
            ty.as_ref().map_or(name.span.end, |t| t.span.end),
        ),
    }
}

/// Whether `word`, at the start of a line in a paragraph, could start a block there, so that a
/// line may not break before it (the design's first Markdown departure). `pulldown-cmark` is
/// asked whether the word, alone or followed by text, starts a block after a paragraph's line.
/// Words made only of the characters of thematic breaks, setext underlines, table delimiter rows,
/// list markers, headings, quotes, and fences count too, since with the words after them on the
/// line they could start one.
pub(crate) fn starts_block(word: &str) -> bool {
    let Some(&first) = word.as_bytes().first() else {
        return false;
    };
    if !(first.is_ascii_punctuation() || first.is_ascii_digit()) {
        return false;
    }
    if word.bytes().all(|b| b"-_*=|:+#>~`".contains(&b)) {
        return true;
    }
    !one_paragraph(&format!("x\n{word}\n")) || !one_paragraph(&format!("x\n{word} x\n"))
}

/// Whether `text` is one paragraph to `pulldown-cmark`, with GFM tables as the parser reads them.
fn one_paragraph(text: &str) -> bool {
    let mut paragraphs = 0;
    for event in Parser::new_ext(text, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::Paragraph) => paragraphs += 1,
            Event::Start(
                Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::List(_)
                | Tag::Item
                | Tag::Table(_),
            )
            | Event::Rule
            | Event::Html(_) => return false,
            _ => {}
        }
    }
    paragraphs == 1
}

/// The words that start the next paragraph's first line, before its own.
struct Lead {
    /// The prefix before them.
    prefix: String,
    words: Vec<Word>,
    /// The glue between them and the paragraph's first word.
    glue: Glue,
}

/// A container a block is in, as the source writes it, for finding the text of its lines.
#[derive(Clone, Copy, Debug)]
enum Container {
    /// A block quote: each of its lines has a `>` after up to three spaces, and maybe a space.
    Quote,
    /// A list item, whose first line is `line`, where its content starts at `content`. Its later
    /// lines are indented by `width` past the start of the container it is in.
    Item {
        line: usize,
        content: u32,
        width: u32,
    },
}

/// The layout being written.
struct Writer<'a> {
    text: &'a str,
    lines: Vec<SourceLine>,
    layout: Layout,
    /// Whether the comment can't be reflowed after all.
    failed: bool,
    /// The line breaks before the next paragraph: 2 for a blank line, and 1 or 0 for a line break.
    gap: u8,
    /// The prefix of the blank lines before the next paragraph.
    blank: String,
    /// The prefix of the lines of the containers the next block is in, after their first.
    rest: String,
    /// What starts the next paragraph's first line: the markers of the list items and block quotes
    /// that start there, or a block tag's name and argument.
    lead: Option<Lead>,
    /// The containers the next block is in, from the outermost.
    containers: Vec<Container>,
    /// Whether the next block is in a block tag's body, which is indented past the tag.
    in_tag: bool,
}

impl Writer<'_> {
    fn word(&self, start: u32, end: u32, glue: Glue) -> Word {
        Word {
            text: WordText::Source(start, end),
            width: width(&self.text[start as usize..end as usize]),
            glue,
        }
    }

    /// The index of the source line `offset` is on. Offsets between two lines are on the first.
    fn line_at(&self, offset: u32) -> usize {
        self.lines
            .partition_point(|l| l.start <= offset)
            .saturating_sub(1)
    }

    /// The last source line `span` has text on, past its containers' markers.
    fn last_line(&self, span: Span) -> usize {
        let first = self.line_at(span.start);
        let mut line = self.line_at(span.end.saturating_sub(1).max(span.start));
        while line > first {
            let l = self.lines[line];
            let end = span.end.min(l.end).max(l.content);
            let text = &self.text[l.content as usize..end as usize];
            if text.trim_matches([' ', '\t', '\x0c', '>']).is_empty() {
                line -= 1;
            } else {
                break;
            }
        }
        line
    }

    /// Whether a blank line comes between the blocks at `before` and `after` in the source.
    fn blank_between(&self, before: Span, after: Span) -> bool {
        self.line_at(after.start) > self.last_line(before) + 1
    }

    /// Asks for `breaks` line breaks before the next paragraph: 2 for a blank line, whose prefix
    /// is the block quotes' `>` of the container the next block is in.
    fn gap(&mut self, breaks: u8) {
        if breaks > self.gap {
            self.gap = breaks;
            self.blank = self.rest.trim_end().to_owned();
        }
    }

    /// Writes a paragraph of the layout holding `words`, after the lead, if any.
    fn paragraph(&mut self, words: Vec<Word>) {
        let start = self.layout.words.len() as u32;
        let (first, glue) = match self.lead.take() {
            Some(lead) => {
                self.layout.words.extend(lead.words);
                (lead.prefix, Some(lead.glue))
            }
            None => (self.rest.clone(), None),
        };
        for (i, mut word) in words.into_iter().enumerate() {
            if i == 0
                && let Some(glue) = glue
                && word.glue != Glue::Bound
            {
                word.glue = glue;
            }
            self.layout.words.push(word);
        }
        let end = self.layout.words.len() as u32;
        let gap = if self.layout.paragraphs.is_empty() {
            0
        } else {
            self.gap.max(1)
        };
        self.layout.paragraphs.push(Paragraph {
            gap,
            indent: width(&first),
            hanging: width(&self.rest),
            words: (start, end),
        });
        self.layout.prefixes.push(Prefix {
            first: first.into(),
            rest: self.rest.as_str().into(),
            blank: std::mem::take(&mut self.blank).into(),
        });
        self.gap = 0;
    }

    /// Writes `blocks`, siblings in a container, with the line breaks between them.
    fn blocks(&mut self, blocks: &[Block]) {
        for (i, block) in blocks.iter().enumerate() {
            if let Some(before) = i.checked_sub(1).map(|i| &blocks[i]) {
                let blank = spaced(before)
                    || spaced(block)
                    || self.blank_between(before.span(), block.span());
                self.gap(if blank { 2 } else { 1 });
            }
            self.block(block);
            if self.failed {
                return;
            }
        }
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Paragraph { span, content } => self.text(*span, content, false),
            Block::Heading {
                span,
                setext: false,
                content,
                ..
            } => self.text(*span, content, true),
            Block::BlockQuote { blocks, .. } => self.quote(blocks),
            Block::List { items, .. } => self.list(items),
            Block::CodeBlock {
                span,
                info: Some(_),
                ..
            } => self.copied(*span, true),
            // An HTML block's lines would gain the indentation of a block tag's body.
            Block::Html { .. } if self.in_tag => self.failed = true,
            Block::Html { span, .. } | Block::Table { span, .. } => self.copied(*span, false),
            // A setext heading, an indented code block, a thematic break, or a link reference
            // definition, which GJF leaves unreflowed.
            _ => self.failed = true,
        }
    }

    /// Where the text of source line `line` starts in the containers the next block is in, or
    /// `None` if a container's marker or indentation isn't there, as on a paragraph's lazy
    /// continuation line.
    fn strip(&mut self, line: usize) -> Option<u32> {
        let bytes = self.text.as_bytes();
        let SourceLine { content, end, .. } = self.lines[line];
        let end = end as usize;
        let mut at = content as usize;
        for &container in &self.containers {
            match container {
                Container::Quote => {
                    let mut spaces = 0;
                    while spaces < 3 && at < end && bytes[at] == b' ' {
                        at += 1;
                        spaces += 1;
                    }
                    if at >= end || bytes[at] != b'>' {
                        self.failed |= at < end && bytes[at] == b'\t';
                        return None;
                    }
                    at += 1;
                    match bytes.get(at) {
                        Some(b' ') if at < end => at += 1,
                        Some(b'\t') if at < end => self.failed = true,
                        _ => {}
                    }
                }
                Container::Item {
                    line: first,
                    content,
                    width,
                } => {
                    if first == line {
                        at = (content as usize).min(end);
                        continue;
                    }
                    let mut spaces = 0;
                    while spaces < width && at < end && bytes[at] == b' ' {
                        at += 1;
                        spaces += 1;
                    }
                    if spaces < width && at < end {
                        self.failed |= bytes[at] == b'\t';
                        return None;
                    }
                }
            }
        }
        Some(at as u32)
    }

    /// Where the text of source line `line`, a paragraph's continuation line, starts: past its
    /// whitespace and the block quotes' `>`, which a lazy line leaves out.
    fn continuation(&mut self, line: usize) -> u32 {
        let bytes = self.text.as_bytes();
        let SourceLine { content, end, .. } = self.lines[line];
        let mut quotes = self
            .containers
            .iter()
            .filter(|c| matches!(c, Container::Quote))
            .count();
        let mut at = content;
        loop {
            while at < end && is_blank(bytes[at as usize]) {
                self.failed |= bytes[at as usize] == b'\t' && !self.containers.is_empty();
                at += 1;
            }
            if quotes > 0 && at < end && bytes[at as usize] == b'>' {
                at += 1;
                quotes -= 1;
                continue;
            }
            return at;
        }
    }

    /// Where `start..end` ends without trailing whitespace.
    fn trim_end(&self, start: u32, end: u32) -> u32 {
        let bytes = self.text.as_bytes();
        let mut end = end;
        while end > start && is_blank(bytes[end as usize - 1]) {
            end -= 1;
        }
        end
    }

    /// Writes a block quote holding `blocks`.
    fn quote(&mut self, blocks: &[Block]) {
        let saved = self.rest.len();
        let starts_line = match &mut self.lead {
            Some(lead) => {
                lead.words.push(Word {
                    text: WordText::Static(">"),
                    width: 1,
                    glue: Glue::Bound,
                });
                false
            }
            None => true,
        };
        self.rest.push_str("> ");
        self.containers.push(Container::Quote);
        if starts_line {
            // GJF starts a quote whose first block isn't a paragraph with empty `>` lines
            // (observed): one before a list or a quote, two before the other blocks.
            let empty = match blocks.first() {
                Some(Block::List { .. } | Block::BlockQuote { .. }) => 1,
                Some(block) if spaced(block) => 2,
                _ => 0,
            };
            for _ in 0..empty {
                self.paragraph(Vec::new());
            }
        }
        if blocks.is_empty() {
            self.paragraph(Vec::new());
        } else {
            self.blocks(blocks);
        }
        self.containers.pop();
        self.rest.truncate(saved);
    }

    /// Writes a list's items, with a blank line between two where the source has one, and before
    /// and after a block that gets them.
    fn list(&mut self, items: &[ListItem]) {
        for (i, item) in items.iter().enumerate() {
            if let Some(before) = i.checked_sub(1).map(|i| &items[i]) {
                let blank = ends_spaced(before)
                    || item.blocks.first().is_some_and(spaced)
                    || self.blank_between(before.span, item.span);
                self.gap(if blank { 2 } else { 1 });
            }
            self.item(item);
            if self.failed {
                return;
            }
        }
    }

    /// Writes a list item: its marker as written, a space, and its blocks, the first on the
    /// marker's line and the others, with its later lines, indented past the marker.
    fn item(&mut self, item: &ListItem) {
        let bytes = self.text.as_bytes();
        let mut start = item.span.start as usize;
        while start < bytes.len() && bytes[start] == b' ' {
            start += 1;
        }
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        match bytes.get(end) {
            Some(b'.' | b')') if end > start => end += 1,
            Some(b'-' | b'+' | b'*') if end == start => end += 1,
            _ => {
                self.failed = true;
                return;
            }
        }
        let line = self.line_at(start as u32);
        let Some(parent) = self.strip(line) else {
            self.failed = true;
            return;
        };
        // The content starts past one to four spaces after the marker, or one if there are more
        // (an indented code block, which falls back) or no text (the item starts with a blank
        // line), as CommonMark reads it.
        let line_end = self.lines[line].end as usize;
        let spaces = bytes[end..line_end]
            .iter()
            .take_while(|&&b| b == b' ')
            .count();
        if bytes.get(end + spaces) == Some(&b'\t') && end + spaces < line_end {
            self.failed = true;
            return;
        }
        let after = if (1..=4).contains(&spaces) && end + spaces < line_end {
            spaces
        } else {
            1
        };
        let marker = self.word(start as u32, end as u32, Glue::Bound);
        let marker_width = marker.width;
        match &mut self.lead {
            Some(lead) => lead.words.push(marker),
            None => {
                self.lead = Some(Lead {
                    prefix: self.rest.clone(),
                    words: vec![marker],
                    glue: Glue::Bound,
                });
            }
        }
        let saved = self.rest.len();
        self.rest
            .extend(std::iter::repeat_n(' ', marker_width as usize + 1));
        self.containers.push(Container::Item {
            line,
            content: (end + after) as u32,
            width: end as u32 - parent + after as u32,
        });
        if item.blocks.is_empty() {
            self.paragraph(Vec::new());
        } else {
            self.blocks(&item.blocks);
        }
        self.containers.pop();
        self.rest.truncate(saved);
    }

    /// Writes a fenced code block, a table, or an HTML block, a line of the layout for each of its
    /// lines, as written past its containers' markers and indentation. A fenced code block the
    /// comment ends before its closing fence gets one, as GJF adds it (observed), without the
    /// blank lines before it.
    fn copied(&mut self, span: Span, fence: bool) {
        let first = self.line_at(span.start);
        let last = self.last_line(span);
        let mut rows = Vec::new();
        for line in first..=last {
            let end = self.lines[line].end;
            let row = match self.strip(line) {
                Some(start) => (start, self.trim_end(start, end)),
                None => {
                    let content = self.lines[line].content;
                    if self.trim_end(content, end) > content || self.failed {
                        self.failed = true;
                        return;
                    }
                    (end, end)
                }
            };
            rows.push(row);
        }
        let mut closing = None;
        if fence && let Some(&(start, end)) = rows.first() {
            let opening = &self.text[start as usize..end as usize];
            let indent = opening.len() - opening.trim_start_matches(' ').len();
            let marks = &opening[indent..];
            let mark = marks.as_bytes().first().copied().unwrap_or(b'`');
            let count = marks.bytes().take_while(|&b| b == mark).count();
            let closed = rows.len() > 1 && {
                let (start, end) = rows[rows.len() - 1];
                let line = self.text[start as usize..end as usize].trim_start_matches(' ');
                line.len() >= count && line.bytes().all(|b| b == mark)
            };
            if !closed {
                while rows.len() > 1 && rows.last().is_some_and(|&(start, end)| start == end) {
                    rows.pop();
                }
                let at = start + indent as u32;
                closing = Some((at, at + count as u32));
            }
        }
        for (start, end) in rows.into_iter().chain(closing) {
            let words = if start < end {
                vec![self.word(start, end, Glue::Space)]
            } else {
                Vec::new()
            };
            self.paragraph(words);
        }
    }

    /// Finds the code spans and hard breaks in `content`, and whether it holds something that
    /// can't be reflowed safely. Each hard break is its line, and whether it is written with
    /// spaces rather than a backslash.
    fn inlines(
        &mut self,
        content: &[Inline],
        code: &mut Vec<(u32, u32)>,
        breaks: &mut Vec<(usize, bool)>,
    ) {
        for inline in content {
            match inline {
                Inline::Code(text) => code.push((text.span.start, text.span.end)),
                Inline::HardBreak(span) => {
                    let spaces = self.text.as_bytes().get(span.start as usize) != Some(&b'\\');
                    breaks.push((self.line_at(span.start), spaces));
                }
                Inline::Emphasis { content, .. }
                | Inline::Strong { content, .. }
                | Inline::Image { content, .. } => self.inlines(content, code, breaks),
                Inline::Link {
                    span,
                    kind,
                    content,
                    ..
                } => {
                    let link = &self.text[span.start as usize..span.end as usize];
                    self.failed |= *kind == LinkKind::Inline && link.contains("](<");
                    self.inlines(content, code, breaks);
                }
                Inline::Tag(tag) => {
                    let written = &self.text[tag.span.start as usize..tag.span.end as usize];
                    let quoted = self
                        .containers
                        .iter()
                        .any(|c| matches!(c, Container::Quote));
                    self.failed |= matches!(tag.content, InlineTagContent::Snippet(_))
                        || quoted && written.contains(['\n', '\r']);
                    let description = match &tag.content {
                        InlineTagContent::Link { label, .. }
                        | InlineTagContent::LinkPlain { label, .. } => Some(label),
                        InlineTagContent::Index { description, .. }
                        | InlineTagContent::Return(description)
                        | InlineTagContent::Summary(description) => Some(description),
                        _ => None,
                    };
                    for block in description.into_iter().flatten() {
                        self.block_code(block, code);
                    }
                }
                _ => {}
            }
        }
    }

    /// Finds the code spans in an inline tag's descriptive content, which is Markdown, so that
    /// they aren't cut. Anything there but paragraphs falls back.
    fn block_code(&mut self, block: &Block, code: &mut Vec<(u32, u32)>) {
        match block {
            Block::Paragraph { content, .. } => {
                let mut breaks = Vec::new();
                self.inlines(content, code, &mut breaks);
                self.failed |= !breaks.is_empty();
            }
            _ => self.failed = true,
        }
    }

    /// Writes a paragraph or a heading, at `span`, holding `content`: its words, with a line
    /// break after each hard break. A heading's words never break apart.
    fn text(&mut self, span: Span, content: &[Inline], heading: bool) {
        let mut code = Vec::new();
        let mut breaks = Vec::new();
        self.inlines(content, &mut code, &mut breaks);
        if self.failed {
            return;
        }
        code.sort_unstable();
        let in_code = |at: u32| {
            let i = code.partition_point(|&(start, _)| start <= at);
            i > 0 && at < code[i - 1].1
        };
        let bytes = self.text.as_bytes();
        let first = self.line_at(span.start);
        let last = self.last_line(span);
        let mut words: Vec<Word> = Vec::new();
        // The last word's text, and whether a line ended after it.
        let mut previous: Option<(u32, u32)> = None;
        for line in first..=last {
            let mut new_line = line > first;
            if let Some(&(_, spaces)) = breaks.iter().find(|&&(at, _)| new_line && at == line - 1) {
                // Trailing spaces go, so a hard break written with them is written with a
                // backslash, which means the same.
                if spaces {
                    words.push(Word {
                        text: WordText::Static("\\"),
                        width: 1,
                        glue: Glue::Tight,
                    });
                }
                self.paragraph(std::mem::take(&mut words));
                previous = None;
                new_line = false;
            }
            let start = if line == first {
                span.start
            } else {
                self.continuation(line)
            };
            // A code span's content keeps the whitespace starting its later lines, as the parser
            // reads it, which the line's first word would leave out.
            let line_start = self.lines[line].start;
            let crossing = code
                .iter()
                .any(|&(s, e)| s < line_start && start < e && line_start <= start);
            if line > first && crossing && self.strip(line) != Some(start) {
                self.failed = true;
                return;
            }
            let raw_end = if line == last {
                span.end.min(self.lines[line].end)
            } else {
                self.lines[line].end
            };
            let end = self.trim_end(start, raw_end);
            if line < last && end < raw_end && in_code(end) {
                self.failed = true;
                return;
            }
            let mut at = start;
            while at < end {
                while at < end && is_blank(bytes[at as usize]) && !in_code(at) {
                    at += 1;
                }
                if at >= end {
                    break;
                }
                let word_start = at;
                while at < end && !(is_blank(bytes[at as usize]) && !in_code(at)) {
                    at += 1;
                }
                let word = &self.text[word_start as usize..at as usize];
                let glue = match previous {
                    _ if heading => Glue::Bound,
                    None => {
                        if starts_block(word) {
                            Glue::Bound
                        } else {
                            Glue::Space
                        }
                    }
                    Some((p_start, p_end)) => {
                        let before = &self.text[p_start as usize..p_end as usize];
                        if new_line && in_code(p_end.saturating_sub(1)) && in_code(word_start) {
                            // A code span written across lines: its line break is a space.
                            Glue::Bound
                        } else if new_line && ends_with_hyphen(before) {
                            // GJF joins `well-` and `known` (observed; the design's decision 11).
                            Glue::Tight
                        } else if ends_with_hyphen(before)
                            || before.ends_with('\\')
                            || word.starts_with('@')
                            || starts_block(word)
                        {
                            Glue::Bound
                        } else {
                            Glue::Space
                        }
                    }
                };
                words.push(self.word(word_start, at, glue));
                previous = Some((word_start, at));
                new_line = false;
            }
        }
        self.paragraph(words);
    }

    /// The words of `span`, written across lines, each line's past its whitespace.
    fn span_words(&mut self, span: Span, glue: Glue) -> Vec<Word> {
        let bytes = self.text.as_bytes();
        let mut words = Vec::new();
        let first = self.line_at(span.start);
        let last = self.line_at(span.end.saturating_sub(1).max(span.start));
        for line in first..=last {
            let start = if line == first {
                span.start
            } else {
                self.continuation(line)
            };
            let end = span.end.min(self.lines[line].end).max(start);
            let mut at = start;
            while at < end {
                while at < end && is_blank(bytes[at as usize]) {
                    at += 1;
                }
                let word_start = at;
                while at < end && !is_blank(bytes[at as usize]) {
                    at += 1;
                }
                if word_start < at {
                    words.push(self.word(word_start, at, glue));
                }
            }
        }
        words
    }

    /// Writes a block tag: its name and argument, then its body, the first paragraph on the tag's
    /// line and the rest of it 2 past the tag. A blank line goes before the first tag, and
    /// between two where the source has one or the one before ends with a block that gets one.
    fn tag(&mut self, tag: &BlockTag<Vec<Block>>, before: Option<&BlockTag<Vec<Block>>>) {
        if self.failed {
            return;
        }
        if !self.layout.paragraphs.is_empty() {
            let blank = before.is_none_or(|before| {
                before.body.last().is_some_and(spaced) || self.blank_between(before.span, tag.span)
            });
            self.gap(if blank { 2 } else { 1 });
        }
        let mut head = vec![self.word(tag.span.start, tag.name.span.end, Glue::Space)];
        let mut head_end = tag.name.span.end;
        if let Some(argument) = &tag.argument {
            let span = argument_span(argument);
            head.extend(self.span_words(span, Glue::Bound));
            head_end = span.end;
        }
        self.lead = Some(Lead {
            prefix: String::new(),
            words: head,
            glue: Glue::Space,
        });
        self.rest = "  ".to_owned();
        self.in_tag = true;
        match tag.body.first() {
            Some(Block::Paragraph { span, content }) => {
                self.text(*span, content, false);
                if tag.body.len() > 1 && !self.failed {
                    let blank =
                        spaced(&tag.body[1]) || self.blank_between(*span, tag.body[1].span());
                    self.gap(if blank { 2 } else { 1 });
                    self.blocks(&tag.body[1..]);
                }
            }
            // The parser reads a body from its first text, so the indentation of a fence there
            // wouldn't count, and its code would move.
            Some(Block::CodeBlock { .. }) => self.failed = true,
            Some(first) => {
                self.paragraph(Vec::new());
                let head = Span::new(tag.span.start, head_end);
                let blank = spaced(first) || self.blank_between(head, first.span());
                self.gap(if blank { 2 } else { 1 });
                self.blocks(&tag.body);
            }
            None => self.paragraph(Vec::new()),
        }
        self.rest.clear();
        self.in_tag = false;
    }
}

#[cfg(test)]
mod tests;
