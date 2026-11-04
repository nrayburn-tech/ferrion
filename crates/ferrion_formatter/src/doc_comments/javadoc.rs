//! Traditional doc comments, `/** */` (docs/design/formatter-completeness.md, "Javadoc"): the
//! layout built from `ferrion_doc_comments`'s parse, and the comment's shape around the filled
//! lines.
//!
//! [`layout`] reads the comment in three steps:
//!
//! 1. **Lines:** each line's text after its decoration, as javadoc reads it (`content_lines`).
//!    Words are cut from these source ranges, so a word is always its source text, Unicode escapes
//!    and all.
//! 2. **Structure:** the parse's top-level nodes, in source order: block tags, the HTML elements
//!    the rules place (`<p>`, lists, `<pre>`, ...), comments, and snippets. `<pre>`, `<table>`,
//!    HTML comments, `{@snippet}`, and `<code>`'s content are copied runs. Inline tags and other
//!    elements are part of the words around them, so their whitespace is collapsed like text's.
//! 3. **Writing:** a scan of the lines feeds words and structure to a `Writer`, which applies the
//!    rules: the line breaks each element asks for before and after it, `<p>` between
//!    paragraphs, and the indentation of lists and block tags.
//!
//! A comment that can't be reflowed safely or as GJF does has no layout, and is re-indented as A1
//! does ("Fallback: re-indent only").

use ferrion_doc_comments::{
    DiagnosticKind, Inline, InlineTag, InlineTagContent, Span, StartElement, TagArgument,
    parse_javadoc,
};

use super::{Columns, Glue, Layout, Paragraph, Segment, Word, WordText, fill};
use crate::document::{LINE_WIDTH, width};
use crate::printer::trim_end;

/// Whether a doc comment written as `text` is one the rules reflow, rather than a block comment to
/// GJF: it starts with `/**` and a character other than `*`, and ends with `*/`. `/***/`,
/// `/*****/`, `/***  text */`, and `/*** … ***/` banners are left as A1 re-indents them, as GJF
/// leaves them (docs/design/formatter-completeness.md, "Which comments"). A comment whose `/**` or
/// `*/` is written with Unicode escapes is left too.
pub fn reflows(text: &str) -> bool {
    text.len() >= 5
        && text.starts_with("/**")
        && !text[3..].starts_with('*')
        && text.ends_with("*/")
}

/// The layout of the traditional doc comment at `start..end` of `text`, or `None` if it isn't
/// reflowed: see [`reflows`], and the fallback rules in the design, plus what the layout can't
/// keep safely (Unicode escapes for anything but letters and digits, `@` escapes, malformed
/// arguments and references, an element the rules place written across lines, and a word starting
/// with `@` that a rule would put at the start of a line, where it would become a block tag).
pub fn layout(text: &str, start: u32, end: u32) -> Option<Layout> {
    let comment = text.get(start as usize..end as usize)?;
    if !reflows(comment) || has_escape(comment) {
        return None;
    }
    let javadoc = parse_javadoc(text, Span::new(start, end));
    // An unclosed inline tag falls back (the design's rule). So do a block tag without a name, and
    // a malformed argument or a reference with whitespace in it, such as an unclosed `@see "...`:
    // joining its lines could close it, changing what it holds. A malformed reference of one word,
    // such as `@see https://example.com`, has no lines to join.
    if javadoc.diagnostics.iter().any(|d| match d.kind {
        DiagnosticKind::UnclosedInlineTag
        | DiagnosticKind::NestingTooDeep
        | DiagnosticKind::MissingTagName
        | DiagnosticKind::MalformedArgument => true,
        DiagnosticKind::MalformedReference => text[d.span.start as usize..d.span.end as usize]
            .contains(|c: char| c.is_whitespace() || c == '"'),
        _ => false,
    }) {
        return None;
    }
    let lines = content_lines(text, start, end);
    let mut marks = Vec::new();
    // Quoted strings, which end at the end of their line, so the fill never breaks in one.
    let mut quoted = Vec::new();
    collect(text, &javadoc.description, &mut marks, &mut quoted)?;
    for tag in &javadoc.tags {
        marks.push(Mark {
            start: tag.span.start,
            end: tag.name.span.end,
            kind: MarkKind::Tag,
        });
        if let Some(TagArgument::Quoted(text)) = &tag.argument {
            quoted.push((text.span.start, text.span.end));
        }
        collect(text, &tag.body, &mut marks, &mut quoted)?;
    }
    let structure = Structure::new(text, &lines, &marks)?;
    let mut scan = Scan {
        text,
        lines: &lines,
        structure: &structure,
        writer: Writer::new(text),
        quoted: Sweep::new(&quoted),
        space: false,
        hyphen: false,
        hyphen_break: false,
        breaks: 0,
        malformed: javadoc
            .diagnostics
            .iter()
            .find(|d| d.kind == DiagnosticKind::MalformedHtml)
            .map(|d| d.span.start),
    };
    scan.run()?;
    let writer = scan.writer;
    if writer.failed {
        return None;
    }
    Some(writer.layout)
}

/// The comment's lines at `column`, the column its `/**` is at: the first without indentation,
/// the others with `column` spaces before their decoration. `/** text */` on one line exactly when
/// the filled text is one line that fits that way and the comment has no block tag; otherwise
/// `/**`, a ` * ` line for each filled line (` *` alone for a blank one, and ` *` and the text
/// for a copied one), and ` */`. An empty comment is `/** */`.
pub fn render(layout: &Layout, text: &str, column: u32) -> Vec<String> {
    let filled = fill(
        layout,
        text,
        Columns {
            text: column + 3,
            copied: column + 2,
            limit: LINE_WIDTH,
        },
    );
    match filled.as_slice() {
        [] => return vec!["/** */".to_owned()],
        [only]
            if !layout.tags
                && !only.copied
                && u64::from(column)
                    + 7
                    + u64::from(only.indent)
                    + u64::from(width(&only.text))
                    <= u64::from(LINE_WIDTH) =>
        {
            let pad = " ".repeat(only.indent as usize);
            return vec![format!("/** {pad}{} */", only.text)];
        }
        _ => {}
    }
    let margin = " ".repeat(column as usize);
    let mut out = Vec::with_capacity(filled.len() + 2);
    out.push("/**".to_owned());
    for line in &filled {
        out.push(if line.copied {
            format!("{margin} *{}", line.text)
        } else if line.text.is_empty() {
            format!("{margin} *")
        } else {
            let pad = " ".repeat(line.indent as usize);
            format!("{margin} * {pad}{}", line.text)
        });
    }
    out.push(format!("{margin} */"));
    out
}

/// Whether `comment` has a Unicode escape for a character other than a letter or digit, which
/// could stand for something the rules read (a line break, a star, a tag's `<` or `@`) where the
/// layout, which cuts words from the source, would see only text. Escapes are found without the
/// even-backslash rule, so `\\u0020`, which isn't one, counts too: that only re-indents a comment
/// that could have been reflowed.
pub(super) fn has_escape(comment: &str) -> bool {
    let bytes = comment.as_bytes();
    let mut i = 0;
    while let Some(at) = comment[i..].find('\\') {
        let mut j = i + at + 1;
        if bytes.get(j) != Some(&b'u') {
            i = j;
            continue;
        }
        while bytes.get(j) == Some(&b'u') {
            j += 1;
        }
        if let Some(hex) = comment.get(j..j + 4)
            && hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            let value = u32::from_str_radix(hex, 16).unwrap_or(0);
            if char::from_u32(value).is_none_or(|c| !c.is_alphanumeric()) {
                return true;
            }
        }
        i = j;
    }
    false
}

/// Java's whitespace within a line: space, tab, and form feed.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0c')
}

/// The source range of each line's content in the comment at `start..end`, as javadoc reads it:
/// between `/**` and `*/`; on lines after the first, without leading whitespace followed by stars
/// and the stars, but with whitespace not followed by a star, and with whitespace after the stars.
/// Stars right before `*/` go too, as GJF drops them (`/** a **/` is `/** a */`). Lines are split
/// at CR, LF, and CRLF; [`has_escape`] leaves out comments with escaped line breaks.
fn content_lines(text: &str, start: u32, end: u32) -> Vec<(u32, u32)> {
    let bytes = text.as_bytes();
    let body_end = end as usize - 2;
    let mut lines = Vec::new();
    let mut pos = start as usize + 3;
    let mut first = true;
    loop {
        let rest = &text[pos..body_end];
        let (line_end, next) = match rest.find(['\n', '\r']) {
            Some(k) => {
                let skip = if rest[k..].starts_with("\r\n") { 2 } else { 1 };
                (pos + k, Some(pos + k + skip))
            }
            None => (body_end, None),
        };
        let mut from = pos;
        if !first {
            let mut at = pos;
            while at < line_end && is_space(bytes[at]) {
                at += 1;
            }
            if at < line_end && bytes[at] == b'*' {
                while at < line_end && bytes[at] == b'*' {
                    at += 1;
                }
                from = at;
            }
        }
        lines.push((from as u32, line_end as u32));
        match next {
            Some(next) => pos = next,
            None => break,
        }
        first = false;
    }
    if let Some(last) = lines.last_mut() {
        while last.1 > last.0 && bytes[last.1 as usize - 1] == b'*' {
            last.1 -= 1;
        }
    }
    lines
}

/// An HTML element the rules place, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Html {
    P,
    Br,
    /// `<ul>`, `<ol>`, `<dl>`.
    List,
    /// `<li>`, `<dt>`, `<dd>`.
    Item,
    Blockquote,
    /// `<h1>` to `<h6>`.
    Heading,
    Pre,
    Table,
    Code,
}

impl Html {
    fn from_name(name: &str) -> Option<Html> {
        let lower = name.to_ascii_lowercase();
        Some(match lower.as_str() {
            "p" => Html::P,
            "br" => Html::Br,
            "ul" | "ol" | "dl" => Html::List,
            "li" | "dt" | "dd" => Html::Item,
            "blockquote" => Html::Blockquote,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Html::Heading,
            "pre" => Html::Pre,
            "table" => Html::Table,
            "code" => Html::Code,
            _ => return None,
        })
    }
}

/// A top-level node of the parse that the layout looks at, by its source span.
#[derive(Clone, Copy, Debug)]
struct Mark {
    start: u32,
    end: u32,
    kind: MarkKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkKind {
    /// An inline tag, or an element the rules don't place: part of the words around it. `code`
    /// for a `{@code}` written across lines, and `anchor` for `<a href="...">` written just so,
    /// after which GJF writes the next word right after it or on the next line.
    Atom {
        inline_tag: bool,
        code: bool,
        anchor: bool,
    },
    /// An element the rules place. `close` for an end tag; `attributes` if it has any.
    Html {
        html: Html,
        close: bool,
        attributes: bool,
    },
    /// `<!-- ... -->`
    Comment,
    /// `{@snippet ...}`
    Snippet,
    /// A block tag's `@name`.
    Tag,
}

/// Adds the marks of `inlines`, a section's top-level content, and the spans of the quoted strings
/// in its inline tags (`{@index "..."}`, `{@value "..."}`), or fails for content the layout
/// doesn't keep (`@` escapes, which mean something only at the start of a line or after `{`).
fn collect(
    text: &str,
    inlines: &[Inline],
    marks: &mut Vec<Mark>,
    quoted: &mut Vec<(u32, u32)>,
) -> Option<()> {
    for node in inlines {
        let span = node.span();
        let kind = match node {
            Inline::Text(_) | Inline::LineBreak(_) | Inline::Entity(_) => continue,
            Inline::Escape(_) => return None,
            Inline::StartElement(element) => {
                match Html::from_name(&element.name.text) {
                    // A self-closing element is placed only as `<p/>` and `<br/>`.
                    Some(html) if !element.self_closing || matches!(html, Html::P | Html::Br) => {
                        MarkKind::Html {
                            html,
                            close: false,
                            attributes: !element.attributes.is_empty(),
                        }
                    }
                    _ => MarkKind::Atom {
                        inline_tag: false,
                        code: false,
                        anchor: is_anchor(text, element),
                    },
                }
            }
            Inline::EndElement(element) => match Html::from_name(&element.name.text) {
                Some(Html::Br) | None => MarkKind::Atom {
                    inline_tag: false,
                    code: false,
                    anchor: false,
                },
                Some(html) => MarkKind::Html {
                    html,
                    close: true,
                    attributes: false,
                },
            },
            Inline::HtmlComment(_) => MarkKind::Comment,
            Inline::Tag(tag) => match &tag.content {
                InlineTagContent::Snippet(_) => MarkKind::Snippet,
                _ => {
                    quoted_in_tag(tag, quoted)?;
                    MarkKind::Atom {
                        inline_tag: true,
                        code: matches!(tag.content, InlineTagContent::Code(_))
                            && spans_lines(text, span.start, span.end),
                        anchor: false,
                    }
                }
            },
        };
        marks.push(Mark {
            start: span.start,
            end: span.end,
            kind,
        });
    }
    Some(())
}

/// Whether `element` is written `<a href="...">`: lowercase, its only attribute a double-quoted
/// `href` without whitespace in its value, and `>` right after it. GJF writes the word after it
/// right after it, or on the next line if it doesn't fit, as after `<code>` (observed), and after
/// no other spelling.
fn is_anchor(text: &str, element: &StartElement) -> bool {
    let source = |span: Span| &text[span.start as usize..span.end as usize];
    let [attribute] = element.attributes.as_slice() else {
        return false;
    };
    source(element.span).starts_with("<a")
        && element.name.text == "a"
        && source(attribute.name.span) == "href"
        && attribute.value.as_ref().is_some_and(|value| {
            value.quote == Some('"') && !source(value.span).contains(char::is_whitespace)
        })
        && source(Span::new(attribute.span.end, element.span.end)) == ">"
}

/// Adds the spans of the quoted strings in `tag` and the tags nested in its descriptive content, in
/// order, or fails for a nested `{@snippet}`, whose body would be filled as text.
fn quoted_in_tag(tag: &InlineTag, quoted: &mut Vec<(u32, u32)>) -> Option<()> {
    let nested = match &tag.content {
        InlineTagContent::Index { term, description } => {
            if let Some(text) = term {
                quoted.push((text.span.start, text.span.end));
            }
            description
        }
        InlineTagContent::Value {
            format: Some(text), ..
        } => {
            quoted.push((text.span.start, text.span.end));
            return Some(());
        }
        InlineTagContent::Link { label, .. } | InlineTagContent::LinkPlain { label, .. } => label,
        InlineTagContent::Summary(description) | InlineTagContent::Return(description) => {
            description
        }
        _ => return Some(()),
    };
    // The parser nests tags at most `MAX_NESTING` deep, which bounds the recursion.
    for node in nested {
        if let Inline::Tag(inner) = node {
            if matches!(inner.content, InlineTagContent::Snippet(_)) {
                return None;
            }
            quoted_in_tag(inner, quoted)?;
        }
    }
    Some(())
}

/// What the scan meets in the content, in source order.
#[derive(Clone, Copy, Debug)]
struct Event {
    start: u32,
    end: u32,
    kind: EventKind,
}

#[derive(Clone, Copy, Debug)]
enum EventKind {
    /// An element the rules place, outside a copied run.
    Html {
        html: Html,
        close: bool,
        attributes: bool,
    },
    /// A copied run that is a block of its own: `<pre>` and `<table>` to their end tags, and
    /// `{@snippet}`. `open` and `close` are the whitespace left out between `<pre>` and a
    /// `{@code}` in it, and between its `}` and `</pre>`.
    Block {
        open: Option<(u32, u32)>,
        close: Option<(u32, u32)>,
    },
    /// A copied comment, `<!-- ... -->`.
    Comment,
    /// `<code>`, whose start tag ends at `open`, its content copied, and `</code>`, which starts
    /// at `close`.
    Code { open: u32, close: u32 },
    /// A block tag's `@name`.
    Tag,
}

/// The comment's structure: the events in order, the atoms whose inner line breaks are spaces,
/// and the checks that send a comment to the fallback.
struct Structure {
    events: Vec<Event>,
    /// Inline tags and other elements outside copied runs: a blank line in one isn't a paragraph
    /// break.
    atoms: Vec<(u32, u32)>,
    /// Where each `<a href="...">` outside copied runs ends.
    anchors: Vec<u32>,
}

impl Structure {
    /// Pairs the copied runs' start and end tags, and checks the fallback rules: an unclosed
    /// `<pre>`, `<code>`, or `<table>`; a line in `<pre>` or an inline tag starting with `@` and a
    /// lowercase letter, which javadoc reads as a block tag; an element the rules place written
    /// across lines; and a copied line starting with `/`, which would end the comment after its
    /// star.
    fn new(text: &str, lines: &[(u32, u32)], marks: &[Mark]) -> Option<Structure> {
        let mut events = Vec::new();
        let mut atoms = Vec::new();
        let mut anchors = Vec::new();
        // Spans where a line starting with `@` and a lowercase letter falls back.
        let mut tagged = Vec::new();
        // Copied runs, whose later lines are copied after the star.
        let mut copied = Vec::new();
        let mut i = 0;
        while i < marks.len() {
            let mark = marks[i];
            i += 1;
            let kind = match mark.kind {
                MarkKind::Atom {
                    inline_tag, anchor, ..
                } => {
                    atoms.push((mark.start, mark.end));
                    if anchor {
                        anchors.push(mark.end);
                    }
                    if inline_tag {
                        tagged.push((mark.start, mark.end));
                    }
                    continue;
                }
                MarkKind::Comment => EventKind::Comment,
                MarkKind::Snippet => {
                    tagged.push((mark.start, mark.end));
                    EventKind::Block {
                        open: None,
                        close: None,
                    }
                }
                MarkKind::Tag => EventKind::Tag,
                MarkKind::Html {
                    html: html @ (Html::Pre | Html::Table),
                    close: false,
                    ..
                } => {
                    // To the matching end tag, counting nested ones.
                    let inner = i;
                    let mut depth = 1;
                    let end_tag = loop {
                        let next = marks.get(i)?;
                        i += 1;
                        if let MarkKind::Html { html: h, close, .. } = next.kind
                            && h == html
                        {
                            if close {
                                depth -= 1;
                                if depth == 0 {
                                    break *next;
                                }
                            } else {
                                depth += 1;
                            }
                        }
                    };
                    let end = end_tag.end;
                    let (mut open, mut close) = (None, None);
                    if html == Html::Pre {
                        tagged.push((mark.start, end));
                        // `<pre>` and a `{@code}` written across lines, with only whitespace
                        // between, are joined, and so are its `}` and `</pre>` when only
                        // whitespace is between them, as GJF joins them (observed, taking the
                        // fixed point GJF reaches on a second run where blank lines are between).
                        let inside = &marks[inner..i - 1];
                        if let Some(code) = inside.first()
                            && matches!(code.kind, MarkKind::Atom { code: true, .. })
                            && blank(text, lines, mark.end, code.start)
                        {
                            open = Some((mark.end, code.start));
                            if inside.len() == 1 && blank(text, lines, code.end, end_tag.start) {
                                close = Some((code.end, end_tag.start));
                            }
                        }
                    }
                    events.push(Event {
                        start: mark.start,
                        end,
                        kind: EventKind::Block { open, close },
                    });
                    copied.push((mark.start, end));
                    continue;
                }
                MarkKind::Html {
                    html: Html::Code,
                    close: false,
                    ..
                } => {
                    let end_tag = loop {
                        let next = marks.get(i)?;
                        i += 1;
                        if let MarkKind::Html {
                            html: Html::Code,
                            close: true,
                            ..
                        } = next.kind
                        {
                            break *next;
                        }
                    };
                    let end = end_tag.end;
                    if spans_lines(text, mark.start, mark.end)
                        || spans_lines(text, end_tag.start, end_tag.end)
                    {
                        return None;
                    }
                    events.push(Event {
                        start: mark.start,
                        end,
                        kind: EventKind::Code {
                            open: mark.end,
                            close: end_tag.start,
                        },
                    });
                    copied.push((mark.end, end));
                    continue;
                }
                // A stray `</code>` is text.
                MarkKind::Html {
                    html: Html::Code,
                    close: true,
                    ..
                } => {
                    atoms.push((mark.start, mark.end));
                    continue;
                }
                MarkKind::Html {
                    html,
                    close,
                    attributes,
                } => {
                    if spans_lines(text, mark.start, mark.end) {
                        return None;
                    }
                    EventKind::Html {
                        html,
                        close,
                        attributes,
                    }
                }
            };
            if matches!(kind, EventKind::Comment | EventKind::Block { .. }) {
                copied.push((mark.start, mark.end));
            }
            events.push(Event {
                start: mark.start,
                end: mark.end,
                kind,
            });
        }
        // The lines and the spans are both in order, and the spans don't overlap, so one sweep
        // finds the span each line starts in.
        let bytes = text.as_bytes();
        let (mut in_copied, mut in_tagged) = (Sweep::new(&copied), Sweep::new(&tagged));
        for &(start, end) in &lines[1..] {
            let line = &bytes[start as usize..end as usize];
            if in_copied.inside(start) && line.first() == Some(&b'/') {
                return None;
            }
            if in_tagged.inside(start) {
                let rest = &line[line.iter().take_while(|&&b| is_space(b)).count()..];
                if rest.first() == Some(&b'@') && rest.get(1).is_some_and(u8::is_ascii_lowercase) {
                    return None;
                }
            }
        }
        // Nothing may reach past the content's last line, which drops stars before `*/`.
        let last = lines.last().map_or(0, |l| l.1);
        if events.iter().any(|e| e.end > last) || atoms.iter().any(|a| a.1 > last) {
            return None;
        }
        Some(Structure {
            events,
            atoms,
            anchors,
        })
    }
}

/// Whether `word` ends with a hyphen after a letter or digit, as `well-` does.
pub(super) fn ends_with_hyphen(word: &str) -> bool {
    word.strip_suffix('-')
        .and_then(|before| before.chars().next_back())
        .is_some_and(char::is_alphanumeric)
}

/// Spans in order, not overlapping, asked about positions in order.
struct Sweep<'a> {
    spans: &'a [(u32, u32)],
    next: usize,
}

impl<'a> Sweep<'a> {
    fn new(spans: &'a [(u32, u32)]) -> Sweep<'a> {
        Sweep { spans, next: 0 }
    }

    /// Whether `at` is strictly inside a span.
    fn inside(&mut self, at: u32) -> bool {
        while self.spans.get(self.next).is_some_and(|s| s.1 <= at) {
            self.next += 1;
        }
        self.spans.get(self.next).is_some_and(|s| s.0 < at)
    }
}

/// Whether the content from `start` to `end` is whitespace, line breaks and their decoration
/// aside.
fn blank(text: &str, lines: &[(u32, u32)], start: u32, end: u32) -> bool {
    let first = lines.partition_point(|l| l.1 < start);
    lines[first..]
        .iter()
        .take_while(|l| l.0 < end)
        .all(|&(line_start, line_end)| {
            let (from, to) = (line_start.max(start), line_end.min(end));
            from >= to
                || text.as_bytes()[from as usize..to as usize]
                    .iter()
                    .all(|&b| is_space(b))
        })
}

fn spans_lines(text: &str, start: u32, end: u32) -> bool {
    text[start as usize..end as usize].contains(['\n', '\r'])
}

/// Reads the content lines left to right, feeding the writer.
struct Scan<'a> {
    text: &'a str,
    lines: &'a [(u32, u32)],
    structure: &'a Structure,
    writer: Writer<'a>,
    /// The quoted strings, whose spaces don't break.
    quoted: Sweep<'a>,
    /// Whether whitespace or a line break came since the last word or element.
    space: bool,
    /// Whether the last word ends with a hyphen after a letter or digit (`well-`), and then whether its
    /// line ended right after it: GJF writes the next line's first word right after it, as
    /// `well-known` (observed).
    hyphen: bool,
    hyphen_break: bool,
    /// The line breaks since then, outside atoms.
    breaks: u32,
    /// Where the parse first found a `<` that isn't a tag. Whitespace after it where the source
    /// has none could make it one: `<b{@snippet ...}` is text, `<b {@snippet ...}>` a tag.
    malformed: Option<u32>,
}

impl Scan<'_> {
    /// Feeds the whole comment, or fails if an event isn't where the lines say content is.
    fn run(&mut self) -> Option<()> {
        let events = &self.structure.events;
        let bytes = self.text.as_bytes();
        let mut next_event = 0;
        let mut next_atom = 0;
        let mut index = 0;
        let mut pos = self.lines[0].0;
        loop {
            // An event can end on a later line, so the line's end is read each time.
            while pos < self.lines[index].1 {
                let line_end = self.lines[index].1;
                if let Some(&event) = events.get(next_event)
                    && event.start == pos
                {
                    next_event += 1;
                    self.event(event, index);
                    pos = event.end;
                    while self.lines[index].1 < pos {
                        index += 1;
                    }
                    continue;
                }
                if is_space(bytes[pos as usize]) {
                    self.space = true;
                    pos += 1;
                    continue;
                }
                let stop = events
                    .get(next_event)
                    .map_or(line_end, |e| e.start.clamp(pos, line_end));
                let mut end = pos;
                // Spaces are ASCII, so this stops only at character boundaries.
                while end < stop && !is_space(bytes[end as usize]) {
                    end += 1;
                }
                if end == pos {
                    // An event inside a word that began before it: the lines and the parse
                    // disagree.
                    return None;
                }
                let joined = self.hyphen_break && self.breaks <= 1;
                self.before_token(pos);
                let bound = self.quoted.inside(pos);
                let space = self.space && !joined;
                self.writer.word(pos, end, space, bound);
                let word = &self.text[pos as usize..end as usize];
                self.hyphen = ends_with_hyphen(word);
                self.hyphen_break = false;
                if self.structure.anchors.binary_search(&end).is_ok() {
                    self.writer.soft_next = true;
                }
                self.space = false;
                pos = end;
            }
            let line_end = self.lines[index].1;
            if index + 1 == self.lines.len() {
                break;
            }
            let atoms = &self.structure.atoms;
            while next_atom < atoms.len() && atoms[next_atom].1 <= line_end {
                next_atom += 1;
            }
            if !atoms
                .get(next_atom)
                .is_some_and(|&(start, _)| start < line_end)
            {
                self.breaks += 1;
            }
            self.space = true;
            self.hyphen_break = std::mem::take(&mut self.hyphen);
            index += 1;
            pos = self.lines[index].0;
        }
        (next_event == events.len()).then_some(())
    }

    /// Tells the writer about the token about to be fed, at `start`: whether a blank line comes
    /// before it, and whether a line break must not start it.
    fn before_token(&mut self, start: u32) {
        self.writer.unbreakable = !self.space && self.malformed.is_some_and(|at| at < start);
        if self.breaks >= 2 {
            self.writer.blank();
        }
        self.breaks = 0;
    }

    /// Feeds `event`, which starts on line `line`.
    fn event(&mut self, event: Event, line: usize) {
        self.before_token(event.start);
        let space = std::mem::take(&mut self.space);
        self.hyphen = false;
        self.hyphen_break = false;
        let text = self.text;
        let source = |start: u32, end: u32| -> (WordText, u32) {
            (
                WordText::Source(start, end),
                width(&text[start as usize..end as usize]),
            )
        };
        match event.kind {
            EventKind::Tag => {
                let (word, w) = source(event.start, event.end);
                self.writer.tag(word, w);
            }
            EventKind::Html {
                html,
                close,
                attributes,
            } => {
                let (word, w) = source(event.start, event.end);
                let w_ = &mut self.writer;
                match (html, close) {
                    (Html::P, false) if !attributes => w_.paragraph(WordText::Static("<p>"), 3),
                    (Html::P, false) => w_.paragraph(word, w),
                    // Dropped, so the whitespace before it is still the next word's.
                    (Html::P | Html::Item, true) => self.space = space,
                    (Html::Br, _) if !attributes => w_.br(WordText::Static("<br>"), 4, space),
                    (Html::Br, _) => w_.br(word, w, space),
                    (Html::List, false) => w_.list_open(word, w),
                    (Html::List, true) => w_.list_close(word, w),
                    (Html::Item, false) => w_.item(word, w),
                    (Html::Blockquote, false) => w_.quote_open(word, w),
                    (Html::Blockquote, true) => w_.quote_close(word, w),
                    (Html::Heading, false) => w_.heading_open(word, w),
                    (Html::Heading, true) | (Html::Pre | Html::Table, true) => {
                        w_.block_close(word, w, space)
                    }
                    // Paired into copied runs, and a stray `</code>` is an atom.
                    (Html::Pre | Html::Table | Html::Code, _) => {}
                }
            }
            EventKind::Block { open, close } => {
                let skips: Vec<(u32, u32)> = open.into_iter().chain(close).collect();
                let (word, w) = self.copied(event.start, event.end, line, &skips);
                self.writer.block(word, w);
            }
            EventKind::Comment => {
                let (word, w) = self.copied(event.start, event.end, line, &[]);
                self.writer.comment(word, w, space);
            }
            EventKind::Code { open, close } => {
                let start_tag = source(event.start, open);
                let content = (open < close).then(|| self.copied(open, close, line, &[]));
                let end_tag = source(close, event.end);
                self.writer.code(start_tag, content, end_tag, space);
            }
        }
    }

    /// The copied run `start..end`, which starts on line `first`, as a word, leaving out the
    /// `skips` (whitespace, in order): its source if it's on one line, otherwise its segments, the
    /// first line without trailing whitespace.
    fn copied(
        &mut self,
        start: u32,
        end: u32,
        first: usize,
        skips: &[(u32, u32)],
    ) -> (WordText, u32) {
        let text = self.text;
        if skips.is_empty() && end <= self.lines[first].1 {
            return (
                WordText::Source(start, end),
                width(&text[start as usize..end as usize]),
            );
        }
        let layout = &mut self.writer.layout;
        let at = layout.lines.len() as u32;
        // Whether a line break that isn't left out came since the last segment.
        let mut broken = false;
        let mut push = |from: u32, to: u32, broken: &mut bool| {
            layout.lines.push(Segment {
                start: from,
                end: to,
                joins: !std::mem::take(broken),
            });
        };
        let mut index = first;
        loop {
            let (line_start, line_end) = self.lines[index];
            let (from, to) = (line_start.max(start), line_end.min(end));
            let mut pos = from;
            let mut touched = false;
            for &(skip_start, skip_end) in skips {
                if skip_start <= to && skip_end >= from {
                    touched = true;
                    if skip_start > pos {
                        push(pos, skip_start, &mut broken);
                    }
                    pos = pos.max(skip_end);
                }
            }
            if pos < to || !touched {
                push(pos.min(to), to, &mut broken);
            }
            if line_end >= end {
                break;
            }
            if !skips.iter().any(|&(s, e)| s <= line_end && e > line_end) {
                broken = true;
            }
            index += 1;
        }
        let count = layout.lines.len() as u32 - at;
        let segments = &mut layout.lines[at as usize..];
        // The first line's width, and no trailing whitespace on it.
        let on_first = segments[1..].iter().take_while(|s| s.joins).count() + 1;
        if let Some(last) = segments[..on_first].last_mut() {
            last.end =
                last.start + trim_end(&text[last.start as usize..last.end as usize]).len() as u32;
        }
        let first_width = segments[..on_first]
            .iter()
            .map(|s| width(&text[s.start as usize..s.end as usize]))
            .sum();
        (WordText::Lines(at, count), first_width)
    }
}

/// What the writer wrote last, for the rules that look back: a blank line starts a paragraph,
/// with `<p>`, only after text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Last {
    Nothing,
    /// A word of text, an inline element, or a block tag's argument.
    Text,
    /// A block tag's name.
    Name,
    /// `<p>`, `<li>`, or a heading's start tag, which the next word follows directly.
    Open,
    /// `<br>` or a comment, after which a blank line is only a line break.
    Break,
    /// The end of a block, after which a blank line adds no `<p>`.
    Block,
}

/// A list, or a list item, open where the writer is.
#[derive(Clone, Copy, Debug)]
struct Level {
    /// Whether it is a list (`<ul>`, `<ol>`, `<dl>`), +2 for its items; otherwise a `<li>`
    /// outside any list.
    list: bool,
    /// The width of the start tag of the item open in it, if one is, which the item's later lines
    /// go past: +4 for `<li>`, and past the attributes of one that has them (observed).
    item: u32,
}

/// Applies the rules to what the scan feeds it, building the layout. Each element asks for line
/// breaks before and after it, which merge, the larger winning, before the next word; a blank
/// line becomes a line break in a list or a block tag.
struct Writer<'a> {
    text: &'a str,
    layout: Layout,
    /// The line breaks asked for before the next word: 1 or 2 (a blank line).
    pending: u8,
    /// Whether the next word follows the last directly, as after `<p>`.
    glue_next: bool,
    /// Whether the next word may not start a line, as in a quoted string.
    bound_next: bool,
    /// Whether the next word follows the last directly or on the next line, as after
    /// `<a href="...">`.
    soft_next: bool,
    last: Last,
    /// Whether the input has a blank line before the next token.
    blank: bool,
    in_tag: bool,
    levels: Vec<Level>,
    /// Whether the next token may not start a line, for a `<` that isn't a tag before it.
    unbreakable: bool,
    /// Whether a rule would put a word starting with `@` at the start of a line, or break a line
    /// where it may not.
    failed: bool,
}

impl<'a> Writer<'a> {
    fn new(text: &'a str) -> Writer<'a> {
        Writer {
            text,
            layout: Layout::default(),
            pending: 0,
            glue_next: false,
            bound_next: false,
            soft_next: false,
            last: Last::Nothing,
            blank: false,
            in_tag: false,
            levels: Vec::new(),
            unbreakable: false,
            failed: false,
        }
    }

    fn started(&self) -> bool {
        self.last != Last::Nothing
    }

    /// The indentation of a new line: 4 in a block tag, and 2 for each list and 4 for each item
    /// open.
    fn indent(&self) -> u32 {
        let levels: u32 = self
            .levels
            .iter()
            .map(|l| if l.list { 2 } else { 0 } + l.item)
            .sum();
        levels + if self.in_tag { 4 } else { 0 }
    }

    fn in_list(&self) -> bool {
        self.levels.iter().any(|l| l.list)
    }

    /// Asks for `breaks` line breaks before the next word. Nothing comes before the first.
    fn request(&mut self, breaks: u8) {
        if self.started() {
            self.pending = self.pending.max(breaks);
        }
    }

    fn blank(&mut self) {
        if self.started() {
            self.blank = true;
        }
    }

    /// Writes a word, starting a paragraph if line breaks are pending. `space` is whether the
    /// input has whitespace before it.
    fn put(&mut self, text: WordText, width: u32, space: bool) {
        let at = self.layout.words.len() as u32;
        let glue = if !self.started() || self.pending > 0 {
            if self.started() && self.unbreakable {
                self.failed = true;
            }
            let gap = if self.pending == 2 && (self.in_tag || self.in_list()) {
                1
            } else {
                self.pending
            };
            let indent = self.indent();
            self.layout.paragraphs.push(Paragraph {
                gap,
                indent,
                hanging: indent,
                words: (at, at),
            });
            Glue::Space
        } else if self.soft_next {
            Glue::Soft
        } else if self.glue_next || !space {
            Glue::Tight
        } else if self.bound_next || self.starts_with_at(text) || self.after_hyphen() {
            Glue::Bound
        } else {
            Glue::Space
        };
        self.layout.words.push(Word { text, width, glue });
        if let Some(paragraph) = self.layout.paragraphs.last_mut() {
            paragraph.words.1 = at + 1;
        }
        self.pending = 0;
        self.glue_next = false;
        self.bound_next = false;
        self.soft_next = false;
        self.last = Last::Text;
    }

    fn starts_with_at(&self, text: WordText) -> bool {
        matches!(text, WordText::Source(start, _) if self.text.as_bytes()[start as usize] == b'@')
    }

    /// Whether the last word ends with a hyphen after a letter or digit, which the next run would
    /// join to the next line's first word if a line broke after it (GJF breaks there, and joins
    /// them on its next run).
    fn after_hyphen(&self) -> bool {
        matches!(self.layout.words.last(), Some(Word { text: WordText::Source(start, end), .. })
            if ends_with_hyphen(&self.text[*start as usize..*end as usize]))
    }

    /// Sets the hanging indentation of the paragraph just started to the current one.
    fn hang(&mut self) {
        let indent = self.indent();
        if let Some(paragraph) = self.layout.paragraphs.last_mut() {
            paragraph.hanging = indent;
        }
    }

    /// A word of text, `bound` if the line may not break before it. A blank line before it, after
    /// text, starts a paragraph with `<p>`.
    fn word(&mut self, start: u32, end: u32, space: bool, bound: bool) {
        self.implicit_paragraph();
        let text = WordText::Source(start, end);
        // A word starting with `@` that the parse didn't read as a block tag can't start a line,
        // the comment's first included (after a `<p>` that goes): javadoc would read one there.
        if (!self.started() || self.pending > 0) && self.starts_with_at(text) {
            self.failed = true;
        }
        let w = width(&self.text[start as usize..end as usize]);
        self.bound_next = bound;
        self.put(text, w, space);
    }

    fn implicit_paragraph(&mut self) {
        if self.blank && self.last == Last::Text && !self.glue_next {
            self.paragraph(WordText::Static("<p>"), 3);
        }
        self.blank = false;
    }

    /// `<p>`: a blank line before it, and the next word right after it. One without attributes
    /// before anything is written goes.
    fn paragraph(&mut self, text: WordText, width: u32) {
        self.blank = false;
        if !self.started() && text == WordText::Static("<p>") {
            return;
        }
        self.request(2);
        self.put(text, width, true);
        self.glue_next = true;
        self.last = Last::Open;
    }

    /// `<br>`, which ends its line.
    /// Written right after a word, it may still go on the next line, as GJF breaks it there when
    /// the line is full (observed).
    fn br(&mut self, text: WordText, width: u32, space: bool) {
        self.blank = false;
        self.soft_next = !space;
        self.put(text, width, space);
        self.request(1);
        self.last = Last::Break;
    }

    /// `<ul>`, `<ol>`, `<dl>`: a blank line before, its items at +2.
    fn list_open(&mut self, text: WordText, width: u32) {
        self.blank = false;
        self.request(2);
        self.put(text, width, true);
        self.levels.push(Level {
            list: true,
            item: 0,
        });
        self.request(1);
        self.last = Last::Block;
    }

    /// `<li>`, `<dt>`, `<dd>`: a line of its own, ending the item before it, its later lines past
    /// the tag (+4 for `<li>`), and the next word right after it.
    fn item(&mut self, text: WordText, width: u32) {
        self.blank = false;
        match self.levels.last_mut() {
            Some(level) if level.list => level.item = 0,
            Some(_) => {
                self.levels.pop();
            }
            None => {}
        }
        self.request(1);
        self.put(text, width, true);
        match self.levels.last_mut() {
            Some(level) if level.list => level.item = width,
            _ => self.levels.push(Level {
                list: false,
                item: width,
            }),
        }
        self.hang();
        self.glue_next = true;
        self.last = Last::Open;
    }

    /// `</ul>`, `</ol>`, `</dl>`: a line of its own at its list's indentation, then a blank line.
    fn list_close(&mut self, text: WordText, width: u32) {
        self.blank = false;
        if let Some(index) = self.levels.iter().rposition(|l| l.list) {
            self.levels.truncate(index);
            // Still in the list for the line break before it: a nested list's blank line after
            // it is a line break here too.
            self.pending = self.pending.min(1);
        }
        self.request(1);
        self.put(text, width, true);
        self.request(2);
        self.last = Last::Block;
    }

    fn quote_open(&mut self, text: WordText, width: u32) {
        self.blank = false;
        self.request(2);
        self.put(text, width, true);
        self.request(1);
        self.last = Last::Block;
    }

    fn quote_close(&mut self, text: WordText, width: u32) {
        self.blank = false;
        self.request(1);
        self.put(text, width, true);
        self.request(2);
        self.last = Last::Block;
    }

    fn heading_open(&mut self, text: WordText, width: u32) {
        self.blank = false;
        self.request(2);
        self.put(text, width, true);
        self.glue_next = true;
        self.last = Last::Open;
    }

    /// A heading's end tag, or a stray `</pre>` or `</table>`: a blank line after it.
    fn block_close(&mut self, text: WordText, width: u32, space: bool) {
        self.implicit_paragraph();
        self.put(text, width, space);
        self.request(2);
        self.last = Last::Block;
    }

    /// A copied block, `<pre>`, `<table>`, or `{@snippet}`, with a blank line before and after.
    fn block(&mut self, text: WordText, width: u32) {
        self.blank = false;
        self.request(2);
        self.put(text, width, true);
        self.request(2);
        self.last = Last::Block;
    }

    /// A copied HTML comment, on lines of its own.
    fn comment(&mut self, text: WordText, width: u32, space: bool) {
        self.blank = false;
        self.request(1);
        self.put(text, width, space);
        self.request(1);
        self.last = Last::Break;
    }

    /// `<code>`, its content, copied, and `</code>`, with a line break allowed after the start tag
    /// and before the end tag, as GJF breaks them (observed).
    fn code(
        &mut self,
        start_tag: (WordText, u32),
        content: Option<(WordText, u32)>,
        end_tag: (WordText, u32),
        space: bool,
    ) {
        self.implicit_paragraph();
        self.put(start_tag.0, start_tag.1, space);
        for (text, width) in content.into_iter().chain([end_tag]) {
            let at = self.layout.words.len() as u32;
            self.layout.words.push(Word {
                text,
                width,
                glue: Glue::Soft,
            });
            if let Some(paragraph) = self.layout.paragraphs.last_mut() {
                paragraph.words.1 = at + 1;
            }
        }
    }

    /// A block tag's `@name`: a line of its own, a blank line before the first, its later lines
    /// at +4. Lists don't continue into it.
    fn tag(&mut self, text: WordText, width: u32) {
        self.blank = false;
        self.glue_next = false;
        if self.started() {
            self.pending = if self.layout.tags {
                self.pending.max(1)
            } else {
                2
            };
        }
        self.levels.clear();
        self.in_tag = false;
        self.put(text, width, true);
        self.in_tag = true;
        self.hang();
        self.layout.tags = true;
        self.last = Last::Name;
    }
}
