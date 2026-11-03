//! The doc comment part of the comments check (docs/design/formatter-completeness.md, "Doc
//! comments", "Checking it"): reflow changes a doc comment's text, so doc comments are compared
//! by what the reflow may not change.
//!
//! This is written from the design's list of what reflow may change, not from the reflow code, so
//! that the two can't share a mistake. A traditional doc comment's parsed content
//! (`ferrion_doc_comments`) is cut into tokens: words, HTML tags, block tags, and copied runs
//! (`<pre>`, `<code>`, `<table>`, HTML comments, `{@snippet}`), each run its source as javadoc
//! reads it. What reflow may change is left out:
//!
//! - whitespace outside the copied runs, including line breaks and blank lines between
//!   paragraphs and blocks, so only the words and tags around it count, not where it is;
//! - `<p>` (any spelling, without attributes), `</p>`, `</li>`, `</dt>`, and `</dd>`, and the
//!   spellings of `<br>`;
//! - trailing whitespace on a copied run's lines, whitespace at the start and end of `<code>`'s
//!   content, where a line may break, whitespace between `<pre>` and a `{@code}` written across lines and
//!   between its `}` and `</pre>`, which GJF joins, and stars right before `*/`;
//! - the line break after a word ending with a hyphen after a letter or digit, which GJF joins to
//!   the next word (`well-` then `known` is `well-known`).
//!
//! What's left must be the same, in order. A comment that fell back keeps its text apart from
//! re-indenting, so it also passes when its text compares as any block comment's does. A
//! Markdown doc comment is compared by its parsed blocks ([`markdown`]).

use ferrion_doc_comments::{BlockTag, Inline, InlineTagContent, Span, TagArgument, parse_javadoc};

pub(crate) mod markdown;

/// A Markdown doc comment's text as the comments check compares it: the comment whose `///` lines
/// are at `lines` in `text`, by its lines' texts as any line comment's are compared, or by its
/// parsed content.
pub(super) fn normalize_markdown(text: &str, lines: &[Span]) -> Compared {
    let joined: Vec<String> = lines
        .iter()
        .map(|line| super::normalize_comment(&text[line.start as usize..line.end as usize], false))
        .collect();
    Compared {
        text: joined.join("\n"),
        content: Some(markdown::content(text, lines)),
    }
}

/// A comment as the comments check compares it.
#[derive(Debug)]
pub(super) struct Compared {
    /// Its text, as any block comment's is compared (`super::normalize_comment`).
    pub(super) text: String,
    /// For a traditional doc comment, its content's tokens, joined.
    content: Option<String>,
}

impl Compared {
    /// A comment that isn't a traditional doc comment, compared by `text` alone.
    pub(super) fn text(text: String) -> Compared {
        Compared {
            text,
            content: None,
        }
    }

    /// The comment as one string, as the import section's comments and a range's comments are
    /// compared: its content for a traditional doc comment, so a reflowed one matches, or its
    /// text.
    pub(super) fn key(self) -> String {
        self.content.unwrap_or(self.text)
    }
}

/// Two comments match if their texts do, or, for doc comments, their contents do.
impl PartialEq for Compared {
    fn eq(&self, other: &Compared) -> bool {
        self.text == other.text || self.content.is_some() && self.content == other.content
    }
}

/// A traditional doc comment's text as the comments check compares it.
pub(super) fn normalize(text: &str) -> Compared {
    Compared {
        text: super::normalize_comment(text, false),
        content: content(text),
    }
}

/// The tokens of a doc comment's parsed content joined with `\u{1}`, a copied run between `\u{2}`
/// and `\u{3}`; `None` for a comment the design doesn't reflow (one starting `/***`).
pub(crate) fn content(text: &str) -> Option<String> {
    if text.len() < 5 || !text.starts_with("/**") || text[3..].starts_with('*') {
        return None;
    }
    // Stars right before `*/` go, as reflow drops them.
    let body = text[..text.len() - 2].trim_end_matches('*');
    let text = format!("{body} */");
    let javadoc = parse_javadoc(&text, Span::new(0, text.len() as u32));
    let mut items = Vec::new();
    items.extend(javadoc.description.iter().map(Item::Inline));
    for tag in &javadoc.tags {
        items.push(Item::Tag(tag));
        items.extend(tag.body.iter().map(Item::Inline));
    }
    let mut tokens = Tokens {
        text: &text,
        out: String::new(),
        words: String::new(),
    };
    tokens.run(&items);
    Some(tokens.out)
}

/// A top-level part of the parse, in source order: a block tag's head, or a section's node.
#[derive(Clone, Copy)]
enum Item<'a> {
    Tag(&'a BlockTag<Vec<Inline>>),
    Inline(&'a Inline),
}

/// The tokenizer: words are cut from the text the nodes hold, where whitespace is; elements,
/// block tags, and copied runs are tokens of their own.
struct Tokens<'a> {
    text: &'a str,
    out: String,
    /// The text since the last token that isn't a word, cut into words when one comes.
    words: String,
}

impl Tokens<'_> {
    fn push(&mut self, token: &str) {
        self.flush();
        self.out.push_str(token);
        self.out.push('\u{1}');
    }

    /// A copied run, the source from `start` to `end` as javadoc reads it, compared exactly apart
    /// from its lines' trailing whitespace.
    fn push_run(&mut self, start: u32, end: u32) {
        self.push_runs(&[(start, end)]);
    }

    /// A copied run made of the parts of the source given, written one after the other.
    fn push_runs(&mut self, parts: &[(u32, u32)]) {
        self.flush();
        let mut run = String::new();
        for &(start, end) in parts {
            run.push_str(&javadoc_lines(&self.text[start as usize..end as usize]).join("\n"));
        }
        let lines: Vec<&str> = run
            .split('\n')
            .map(|l| l.trim_end_matches([' ', '\t', '\x0c']))
            .collect();
        self.out.push('\u{2}');
        self.out.push_str(&lines.join("\n"));
        self.out.push('\u{3}');
    }

    /// Whether the source from `start` to `end` is whitespace as javadoc reads it.
    fn blank(&self, start: u32, end: u32) -> bool {
        start <= end
            && javadoc_lines(&self.text[start as usize..end as usize])
                .iter()
                .all(|l| l.trim_matches(is_space).is_empty())
    }

    /// Cuts the text since the last token into words. A word ending with a hyphen after a letter or
    /// digit at the end of a line is one word with the next line's first, as GJF joins them.
    fn flush(&mut self) {
        let words = std::mem::take(&mut self.words);
        let mut pending = String::new();
        let mut rest = words.as_str();
        loop {
            let gap = rest.len() - rest.trim_start_matches(is_blank).len();
            let breaks = rest[..gap].matches('\n').count();
            rest = &rest[gap..];
            let end = rest.find(is_blank).unwrap_or(rest.len());
            let word = &rest[..end];
            rest = &rest[end..];
            let hyphen = pending
                .strip_suffix('-')
                .and_then(|before| before.chars().next_back())
                .is_some_and(char::is_alphanumeric);
            if !(hyphen && breaks == 1) && !pending.is_empty() {
                self.out.push_str(&pending);
                self.out.push('\u{1}');
                pending.clear();
            }
            if word.is_empty() {
                break;
            }
            pending.push_str(word);
        }
        if !pending.is_empty() {
            self.out.push_str(&pending);
            self.out.push('\u{1}');
        }
    }

    /// The source from `start` to `end` as javadoc reads it.
    fn source(&self, start: u32, end: u32) -> String {
        javadoc_lines(&self.text[start as usize..end as usize]).join("\n")
    }

    fn run(&mut self, items: &[Item<'_>]) {
        let mut i = 0;
        while i < items.len() {
            let item = items[i];
            i += 1;
            let node = match item {
                Item::Tag(tag) => {
                    self.push(&format!("\n@{}", tag.name.text));
                    if let Some(argument) = &tag.argument {
                        let span = argument_span(argument);
                        let argument = self.source(span.start, span.end);
                        self.words.push_str(&argument);
                        self.words.push(' ');
                    }
                    continue;
                }
                Item::Inline(node) => node,
            };
            let span = node.span();
            match node {
                Inline::Text(text) => self.words.push_str(&text.text),
                Inline::LineBreak(_) => self.words.push('\n'),
                Inline::Escape(escape) => self.words.push(escape.ch),
                Inline::Entity(_) => {
                    let entity = self.source(span.start, span.end);
                    self.words.push_str(&entity);
                }
                Inline::HtmlComment(_) => self.push_run(span.start, span.end),
                Inline::Tag(tag) => {
                    if matches!(tag.content, InlineTagContent::Snippet(_)) {
                        self.push_run(span.start, span.end);
                    } else {
                        // Literal or descriptive, its whitespace may be collapsed either way.
                        let tag = self.source(span.start, span.end);
                        self.words.push_str(&tag);
                    }
                }
                Inline::EndElement(element) => {
                    let name = element.name.text.to_ascii_lowercase();
                    if !matches!(name.as_str(), "p" | "li" | "dt" | "dd") {
                        let tag = collapse(&self.source(span.start, span.end));
                        self.push(&tag);
                    }
                }
                Inline::StartElement(element) => {
                    let name = element.name.text.to_ascii_lowercase();
                    let bare = element.attributes.is_empty();
                    match name.as_str() {
                        "pre" | "table" | "code" if !element.self_closing => {
                            // To the end tag that closes it (`<pre>` and `<table>` nest), or the
                            // end of the comment.
                            let inner = i;
                            let mut depth = 1;
                            let mut end = None;
                            while let Some(&next) = items.get(i) {
                                i += 1;
                                let Item::Inline(next) = next else { continue };
                                match next {
                                    Inline::StartElement(e)
                                        if name != "code"
                                            && e.name.text.eq_ignore_ascii_case(&name) =>
                                    {
                                        depth += 1;
                                    }
                                    Inline::EndElement(e)
                                        if e.name.text.eq_ignore_ascii_case(&name) =>
                                    {
                                        depth -= 1;
                                        if depth == 0 {
                                            end = Some((next.span().start, next.span().end));
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let comment_end = self.text.len() as u32 - 2;
                            let (end_tag, end) = end.unwrap_or((comment_end, comment_end));
                            if name == "code" {
                                // A line may break right after `<code>` and right before
                                // `</code>`, so the content is compared without whitespace at
                                // either end, and the tags apart.
                                let tag = collapse(&self.source(span.start, span.end));
                                self.push(&tag);
                                let content = self.source(span.end, end_tag);
                                self.flush();
                                self.out.push('\u{2}');
                                let content: Vec<&str> = content
                                    .trim_matches(is_blank)
                                    .split('\n')
                                    .map(|l| l.trim_end_matches(is_space))
                                    .collect();
                                self.out.push_str(&content.join("\n"));
                                self.out.push('\u{3}');
                                let tag = collapse(&self.source(end_tag, end));
                                self.push(&tag);
                            } else if name == "pre" {
                                // Whitespace between `<pre>` and a `{@code}` written across
                                // lines, and between its `}` and `</pre>`, may go.
                                let inside: Vec<_> = items[inner..i.saturating_sub(1).max(inner)]
                                    .iter()
                                    .filter(|item| match item {
                                        Item::Inline(Inline::LineBreak(_)) => false,
                                        Item::Inline(Inline::Text(text)) => {
                                            !text.text.trim_matches(is_space).is_empty()
                                        }
                                        _ => true,
                                    })
                                    .collect();
                                let mut parts = vec![(span.start, end)];
                                if let Some(&&Item::Inline(code @ Inline::Tag(tag))) =
                                    inside.first()
                                    && matches!(tag.content, InlineTagContent::Code(_))
                                    && self.text[tag.span.start as usize..tag.span.end as usize]
                                        .contains(['\n', '\r'])
                                    && self.blank(span.end, code.span().start)
                                {
                                    parts = vec![(span.start, span.end), (code.span().start, end)];
                                    if inside.len() == 1 && self.blank(code.span().end, end_tag) {
                                        parts = vec![
                                            (span.start, span.end),
                                            (code.span().start, code.span().end),
                                            (end_tag, end),
                                        ];
                                    }
                                }
                                self.push_runs(&parts);
                            } else {
                                self.push_run(span.start, end);
                            }
                        }
                        "p" if bare => self.flush(),
                        "br" if bare => self.push("<br>"),
                        _ => {
                            let tag = collapse(&self.source(span.start, span.end));
                            self.push(&tag);
                        }
                    }
                }
            }
        }
        self.flush();
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

/// The lines of `text`, a part of a comment, as javadoc reads them: on each line after the
/// first, leading whitespace followed by stars goes, with the stars.
fn javadoc_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    let mut first = true;
    loop {
        let (line, next) = match rest.find(['\n', '\r']) {
            Some(i) => {
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                (&rest[..i], Some(&rest[i + skip..]))
            }
            None => (rest, None),
        };
        let mut line = line;
        if !first {
            let trimmed = line.trim_start_matches([' ', '\t', '\x0c']);
            if trimmed.starts_with('*') {
                line = trimmed.trim_start_matches('*');
            }
        }
        first = false;
        lines.push(line);
        match next {
            Some(next) => rest = next,
            None => break,
        }
    }
    lines
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\x0c')
}

/// Whitespace, line breaks included.
fn is_blank(c: char) -> bool {
    is_space(c) || c == '\n'
}

/// `tag` with each run of whitespace made one space.
fn collapse(tag: &str) -> String {
    tag.split(is_blank)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::normalize;

    /// A comment reflowed as GJF does, with every change the design lists.
    const INPUT: &str = "/**\n   * Text   on\n   * lines.\n   *\n   * Next <p>para<br/>here.\n   * <ul><li>item</li></ul>\n   * <pre>\n   *   kept   as\n   *     written\n   * </pre>\n   * {@code a    b} and <code>x  y</code>\n   * @param x the   x\n   **/";

    const REFLOWED: &str = "/**\n * Text on lines.\n *\n * <p>Next\n *\n * <p>para<br>\n * here.\n *\n * <ul>\n *   <li>item\n * </ul>\n *\n * <pre>\n *   kept   as\n *     written\n * </pre>\n *\n * {@code a b} and <code>x  y</code>\n *\n * @param x the x\n */";

    fn same(a: &str, b: &str) -> bool {
        normalize(a) == normalize(b)
    }

    #[test]
    fn reflow_compares_equal() {
        assert!(same(INPUT, REFLOWED));
    }

    #[test]
    fn a_dropped_word_fails() {
        assert!(!same(INPUT, &REFLOWED.replace("Text on", "Text")));
    }

    #[test]
    fn a_changed_pre_line_fails() {
        assert!(!same(
            INPUT,
            &REFLOWED.replace(" *   kept   as", " * kept   as")
        ));
        assert!(!same(INPUT, &REFLOWED.replace("kept   as", "kept as")));
    }

    #[test]
    fn a_changed_code_text_fails() {
        assert!(!same(INPUT, &REFLOWED.replace("{@code a b}", "{@code a}")));
        assert!(!same(INPUT, &REFLOWED.replace("{@code a b}", "{@code ab}")));
        assert!(!same(
            INPUT,
            &REFLOWED.replace("<code>x  y</code>", "<code>x y</code>")
        ));
    }

    #[test]
    fn a_block_tag_joined_into_text_fails() {
        assert!(!same(
            INPUT,
            &REFLOWED.replace(" *\n * @param", " and @param")
        ));
    }

    #[test]
    fn a_comment_that_fell_back_compares_by_its_text() {
        let input = "/**\n     * <pre>\n         unclosed\n     */";
        let reindented = "/**\n * <pre>\n     unclosed\n */";
        assert!(same(input, reindented));
    }

    #[test]
    fn banners_compare_by_their_text() {
        assert!(!same("/*** a   b */", "/*** a b */"));
    }
}
