//! The Markdown part of the doc comment check (docs/design/formatter-completeness.md, "Checking
//! it"), written from the design's list of what Markdown reflow may change, not from the reflow
//! code.
//!
//! A Markdown doc comment, a run of `///` lines that javac reads as a declaration's doc comment
//! from release 23, is compared by its parsed blocks (`ferrion_doc_comments::markdown`), which
//! already leave out much of what reflow may change: the space after `///` and the comment's
//! common indentation, lazy continuation lines, the indentation of list items' later lines and the
//! spaces after their markers, block quotes' `>` on later lines, and the blank lines between
//! blocks, at the start of a block quote, and that make a list loose. On top of that, what reflow
//! may change is left out:
//!
//! - whitespace in text, line breaks included, is collapsed, outside code spans, fenced code,
//!   tables, HTML blocks, and snippets, which are compared as written apart from their lines'
//!   trailing whitespace (as CommonMark reads a code span, a line break in one is a space);
//! - an unclosed fence's trailing blank lines, since reflow closes the fence;
//! - the line break after a word ending with a hyphen after a letter or digit, as GJF joins the
//!   two words;
//! - an ATX heading's closing `#`s, which the parse leaves out, and the whitespace in block tags'
//!   arguments, inline tags, and link titles.
//!
//! What's left must be the same, in order: the blocks and their kinds, list numbers, the words,
//! the inline elements with their targets, the tags with their names and arguments, and every
//! copied run.

use ferrion_doc_comments::markdown::{Block, Inline, LinkTarget, parse_markdown};
use ferrion_doc_comments::{InlineTagContent, Span};
use ferrion_syntax::comments::{
    DocCommentKind, MARKDOWN_DOC_COMMENT_RELEASE, can_have_doc_comment, doc_comment,
};
use ferrion_syntax::{SyntaxTree, TokenId};

use super::{argument_span, collapse, is_blank, is_space};

/// The first and last lines of each run of `///` lines in `tree` that javac reads as a
/// declaration's doc comment, in order: none below release 23.
pub(crate) fn runs(tree: &SyntaxTree) -> Vec<(TokenId, TokenId)> {
    let mut runs = Vec::new();
    if tree.release() < MARKDOWN_DOC_COMMENT_RELEASE {
        return runs;
    }
    for node in tree.nodes().filter(|n| can_have_doc_comment(n.kind())) {
        if let Some(doc) = doc_comment(node)
            && doc.kind() == DocCommentKind::Markdown
        {
            runs.push((doc.first_token().id(), doc.last_token().id()));
        }
    }
    runs.sort_unstable();
    runs.dedup();
    runs
}

/// The parsed content of the Markdown doc comment whose `///` lines are at `lines` in `text`:
/// each block and inline element a token between `\u{1}`s, text with its whitespace collapsed,
/// and each copied run between `\u{2}` and `\u{3}`.
pub(crate) fn content(text: &str, lines: &[Span]) -> String {
    let comment = parse_markdown(text, lines);
    let mut markdown = Markdown {
        text,
        out: String::new(),
    };
    markdown.blocks(&comment.description);
    for tag in &comment.tags {
        markdown.token(&format!("@{}", tag.name.text));
        if let Some(argument) = &tag.argument {
            let argument = markdown.source(argument_span(argument));
            markdown.token(&argument);
        }
        markdown.blocks(&tag.body);
    }
    markdown.out
}

/// The writer of a Markdown comment's content.
struct Markdown<'a> {
    text: &'a str,
    out: String,
}

impl Markdown<'_> {
    fn token(&mut self, token: &str) {
        self.out.push('\u{1}');
        self.out.push_str(token);
        self.out.push('\u{1}');
    }

    fn copied(&mut self, run: &str) {
        self.out.push('\u{2}');
        self.out.push_str(run);
        self.out.push('\u{3}');
    }

    /// The source at `span`, on each line after the first without its whitespace and `///`, with
    /// its whitespace collapsed, and a line break after a word ending with a hyphen after a letter
    /// or digit left out.
    fn source(&self, span: Span) -> String {
        let source = &self.text[span.start as usize..span.end as usize];
        let mut joined = String::new();
        for (i, line) in source.split(['\n', '\r']).enumerate() {
            let trimmed = line.trim_start_matches(is_space);
            let line = match trimmed.strip_prefix("///") {
                Some(rest) if i > 0 => rest.trim_start_matches(is_space),
                _ => line,
            };
            let end = joined.trim_end_matches(is_space).len();
            joined.truncate(end);
            let hyphen = joined
                .strip_suffix('-')
                .and_then(|before| before.chars().next_back())
                .is_some_and(char::is_alphanumeric);
            if i > 0 && !hyphen {
                joined.push(' ');
            }
            joined.push_str(line);
        }
        collapse(&joined)
    }

    fn blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            self.block(block);
        }
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Paragraph { content, .. } => {
                self.token("p");
                self.inlines(content);
            }
            Block::Heading {
                level,
                setext,
                content,
                ..
            } => {
                self.token(&format!("h{level}{}", if *setext { "=" } else { "" }));
                self.inlines(content);
            }
            Block::BlockQuote { blocks, .. } => {
                self.token("quote");
                self.blocks(blocks);
                self.token("/quote");
            }
            Block::List { start, items, .. } => {
                self.token(&format!("list {start:?}"));
                for item in items {
                    self.token("item");
                    self.blocks(&item.blocks);
                    self.token("/item");
                }
                self.token("/list");
            }
            Block::CodeBlock { info, text, .. } => {
                self.token(&format!("code {info:?}"));
                let mut lines: Vec<&str> = text
                    .split('\n')
                    .map(|l| l.trim_end_matches(is_space))
                    .collect();
                while lines
                    .last()
                    .is_some_and(|l| l.trim_matches(is_space).is_empty())
                {
                    lines.pop();
                }
                self.copied(&lines.join("\n"));
            }
            Block::Html { content, .. } => {
                self.token("html");
                let mut run = String::new();
                for inline in content {
                    match inline {
                        // The parse has no node for an empty line, and a line of whitespace
                        // is one, once its trailing whitespace goes.
                        Inline::Html(line) => {
                            let line = line.text.trim_end_matches(is_blank);
                            if !line.is_empty() {
                                run.push_str(line);
                                run.push('\n');
                            }
                        }
                        other => run.push_str(&self.source(other.span())),
                    }
                }
                self.copied(&run);
            }
            Block::ThematicBreak { .. } => self.token("hr"),
            Block::Table {
                alignments,
                head,
                rows,
                ..
            } => {
                self.token(&format!("table {alignments:?}"));
                for row in std::iter::once(head).chain(rows) {
                    let cells: Vec<&str> = row
                        .cells
                        .iter()
                        .map(|cell| {
                            self.text[cell.span.start as usize..cell.span.end as usize]
                                .trim_matches(is_space)
                        })
                        .collect();
                    self.copied(&cells.join("|"));
                }
            }
            Block::LinkDefinition {
                label,
                destination,
                title,
                ..
            } => self.token(&format!("def {label} {destination} {title:?}")),
        }
    }

    /// Inline content: its text with whitespace collapsed, and its elements and copied runs.
    fn inlines(&mut self, content: &[Inline]) {
        let mut pieces = Vec::new();
        self.pieces(content, &mut pieces);
        // Whether a space is owed before what comes next, and whether anything came yet.
        let mut space = false;
        let mut started = false;
        for piece in pieces {
            let (open, text, close) = match &piece {
                Piece::Text(text) => {
                    for c in text.chars() {
                        if is_blank(c) {
                            space = started;
                            continue;
                        }
                        if std::mem::take(&mut space) {
                            self.out.push(' ');
                        }
                        self.out.push(c);
                        started = true;
                    }
                    continue;
                }
                Piece::Copied(text) => ('\u{2}', text, '\u{3}'),
                Piece::Token(text) => ('\u{1}', text, '\u{1}'),
            };
            if std::mem::take(&mut space) {
                self.out.push(' ');
            }
            self.out.push(open);
            self.out.push_str(text);
            self.out.push(close);
            started = true;
        }
        self.out.push('\u{1}');
    }

    fn pieces(&self, content: &[Inline], out: &mut Vec<Piece>) {
        let mut previous: Option<&Inline> = None;
        for inline in content {
            // Inline HTML written across lines is a node per line, with no line break between.
            if let (Some(Inline::Html(before)), Inline::Html(text)) = (previous, inline)
                && self
                    .text
                    .get(before.span.end as usize..text.span.start as usize)
                    .is_some_and(|gap| gap.contains(['\n', '\r']))
            {
                out.push(Piece::Text("\n".to_owned()));
            }
            previous = Some(inline);
            match inline {
                Inline::Text(text) | Inline::Html(text) => out.push(Piece::Text(text.text.clone())),
                Inline::Code(text) => out.push(Piece::Copied(text.text.clone())),
                Inline::SoftBreak(_) => {
                    let joined = match out.last_mut() {
                        Some(Piece::Text(text)) => {
                            let trimmed = text.trim_end_matches(is_space).len();
                            let hyphen = text[..trimmed]
                                .strip_suffix('-')
                                .and_then(|before| before.chars().next_back())
                                .is_some_and(char::is_alphanumeric);
                            if hyphen {
                                text.truncate(trimmed);
                            }
                            hyphen
                        }
                        _ => false,
                    };
                    if !joined {
                        out.push(Piece::Text("\n".to_owned()));
                    }
                }
                Inline::HardBreak(_) => out.push(Piece::Token("br".to_owned())),
                Inline::Emphasis { content, .. } => {
                    out.push(Piece::Token("em".to_owned()));
                    self.pieces(content, out);
                    out.push(Piece::Token("/em".to_owned()));
                }
                Inline::Strong { content, .. } => {
                    out.push(Piece::Token("strong".to_owned()));
                    self.pieces(content, out);
                    out.push(Piece::Token("/strong".to_owned()));
                }
                Inline::Link {
                    kind,
                    target,
                    title,
                    content,
                    ..
                } => {
                    let target = match target {
                        LinkTarget::Url(url) => url.clone(),
                        LinkTarget::Element(reference) => collapse(&reference.text),
                    };
                    let title = collapse(title);
                    out.push(Piece::Token(format!("a {kind:?} {target} {title}")));
                    self.pieces(content, out);
                    out.push(Piece::Token("/a".to_owned()));
                }
                Inline::Image {
                    kind,
                    destination,
                    title,
                    content,
                    ..
                } => {
                    let title = collapse(title);
                    out.push(Piece::Token(format!("img {kind:?} {destination} {title}")));
                    self.pieces(content, out);
                    out.push(Piece::Token("/img".to_owned()));
                }
                Inline::Tag(tag) => {
                    if matches!(tag.content, InlineTagContent::Snippet(_)) {
                        // Copied as written: its lines matter.
                        let snippet = &self.text[tag.span.start as usize..tag.span.end as usize];
                        out.push(Piece::Copied(snippet.to_owned()));
                    } else {
                        out.push(Piece::Token(self.source(tag.span)));
                    }
                }
            }
        }
    }
}

/// A part of inline content: text, whose whitespace is collapsed, a copied run, or an element.
enum Piece {
    Text(String),
    Copied(String),
    Token(String),
}

#[cfg(test)]
mod tests {
    use ferrion_doc_comments::Span;

    use super::content;

    /// The content of `comment`, `///` lines joined with LF.
    fn of(comment: &str) -> String {
        let mut spans = Vec::new();
        let mut at = 0;
        for line in comment.split('\n') {
            let start = at + (line.len() - line.trim_start().len());
            spans.push(Span::new(start as u32, (at + line.len()) as u32));
            at += line.len() + 1;
        }
        content(comment, &spans)
    }

    fn same(a: &str, b: &str) -> bool {
        of(a) == of(b)
    }

    /// A comment reflowed as the design lists.
    const INPUT: &str = "///Text   on\n///  lines, well-\n///known.\n/// - item\n///continues\n/// > quote\n/// more\n///\n/// `a   b` and `c\n///d`\n/// ```\n/// code   kept\n/// ```\n/// @param x the   x";

    const REFLOWED: &str = "/// Text on lines, well-known.\n/// - item continues\n///\n/// > quote more\n///\n/// `a   b` and `c d`\n///\n/// ```\n/// code   kept\n/// ```\n///\n/// @param x the x";

    #[test]
    fn reflow_compares_equal() {
        assert!(same(INPUT, REFLOWED), "{:?}\n{:?}", of(INPUT), of(REFLOWED));
    }

    #[test]
    fn an_unclosed_fence_compares_equal_closed() {
        assert!(same(
            "/// ```
/// code
///",
            "/// ```
/// code
/// ```"
        ));
    }

    #[test]
    fn a_dropped_word_fails() {
        assert!(!same(INPUT, &REFLOWED.replace("Text on", "Text")));
    }

    #[test]
    fn changed_copied_text_fails() {
        assert!(!same(INPUT, &REFLOWED.replace("code   kept", "code kept")));
        assert!(!same(INPUT, &REFLOWED.replace("`a   b`", "`a b`")));
    }

    #[test]
    fn a_new_block_fails() {
        assert!(!same(
            INPUT,
            &REFLOWED.replace("item continues", "item\n/// - continues")
        ));
        assert!(!same(INPUT, &REFLOWED.replace(" and `c", "\n/// # and `c")));
    }

    #[test]
    fn a_joined_block_tag_fails() {
        assert!(!same(
            INPUT,
            &REFLOWED.replace("///\n/// @param", "/// and @param")
        ));
    }
}
