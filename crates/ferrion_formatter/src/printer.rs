//! The printer (docs/design/formatter.md, "The printer"): one loop over a [`Document`]'s elements
//! with a stack of open containers, deciding each group's mode as it starts, from its precomputed
//! width. Printing is linear in the number of elements plus the output, and never fails: a line
//! goes over the limit when nothing in the groups lets it break.

use std::ops::Range;

use ferrion_diagnostics::LineEnding;
use ferrion_syntax::SyntaxTree;

use crate::document::{Document, Element, LINE_WIDTH, VerbatimKind, width};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// What the printer reads tokens and copied text from.
pub struct Source<'a> {
    /// The tree whose tokens `Element::Token` names.
    pub tree: &'a SyntaxTree,
    pub line_ending: LineEnding,
    /// Whether the output starts with a byte-order mark.
    pub bom: bool,
}

/// A container open while printing.
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// Whether line elements directly in it print flat.
    flat: bool,
    /// Whether the groups in it are decided by fitting; otherwise they're flat, as in a flat
    /// group outside any hug.
    decide: bool,
    indent: u32,
    /// Whether it is a [`Element::Rewrite`].
    rewrite: bool,
}

/// A piece of the input written to the output, for the edit record (`crate::edits`): `input`
/// is its span in the tree's text, and `output` where it went in the output, byte-order mark
/// included. Pieces come in output order, and their inputs in input order, since only a rewrite
/// reorders, and a rewrite is one piece.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Piece {
    pub input: Range<u32>,
    pub output: Range<usize>,
}

/// A rewrite being printed: its input span, and where its output starts, once it has any.
struct OpenRewrite {
    input: Range<u32>,
    output: Option<usize>,
}

/// The text written last, for the check that two pieces of text written side by side don't run
/// together into other tokens.
#[derive(Clone, Copy, Debug)]
struct Last {
    /// The index of the token after it, when it was a token: a token written right after the
    /// token that followed it in the source needs no check.
    next_token: Option<usize>,
    last_char: char,
    number: bool,
}

struct Printer<'a> {
    source: &'a Source<'a>,
    out: Vec<u8>,
    column: u32,
    /// Line breaks waiting for the next text: 1 for a line break, 2 for a blank line.
    pending_breaks: u8,
    pending_space: bool,
    last: Option<Last>,
    /// The indentation of the line the pending line breaks start: the indent in effect at the
    /// last of them, wherever the text after them is.
    pending_indent: u32,
    /// Whether the last thing written was a region that runs to the end of the file.
    ended_by_region: bool,
    /// The edit record, when asked for.
    pieces: Option<Vec<Piece>>,
    rewrites: Vec<OpenRewrite>,
}

/// Prints `document` and returns the file's bytes: the byte-order mark if there is one, the text
/// with the source's line ending, and one line ending at the end if there is any text.
pub fn print(document: &Document, source: &Source<'_>) -> Vec<u8> {
    run(document, source, false).0
}

/// [`print`], with the record of where each piece of the input went.
pub fn print_recorded(document: &Document, source: &Source<'_>) -> (Vec<u8>, Vec<Piece>) {
    let (out, pieces) = run(document, source, true);
    (out, pieces.unwrap_or_default())
}

fn run(document: &Document, source: &Source<'_>, record: bool) -> (Vec<u8>, Option<Vec<Piece>>) {
    let text = source.tree.text();
    let mut printer = Printer {
        source,
        out: Vec::with_capacity(text.len() + text.len() / 4 + 64),
        column: 0,
        pending_breaks: 0,
        pending_space: false,
        last: None,
        pending_indent: 0,
        ended_by_region: false,
        pieces: record.then(Vec::new),
        rewrites: Vec::new(),
    };
    if source.bom {
        printer.out.extend_from_slice(UTF8_BOM);
    }
    let elements = &document.elements;
    // Whether each group was broken, for `IndentIfBroken`, which may name a group already ended.
    let mut broken = vec![false; elements.len()];
    let mut stack = vec![Frame {
        flat: false,
        decide: true,
        indent: 0,
        rewrite: false,
    }];
    for (index, element) in elements.iter().enumerate() {
        let top = *stack.last().expect("the root frame is never popped");
        let indent = top.indent;
        match *element {
            Element::Token { token, len, width } => {
                let start = source.tree.token(token).span().start;
                let input = start..start + len;
                printer.text(
                    &text[input.start as usize..input.end as usize],
                    width,
                    Some(token.index()),
                    input,
                );
                printer.ended_by_region = false;
            }
            Element::Space => {
                if printer.pending_breaks == 0 {
                    printer.pending_space = true;
                }
            }
            Element::Line => {
                if top.flat {
                    printer.space();
                } else {
                    printer.line_break(1, indent);
                }
            }
            Element::SoftLine => {
                if !top.flat {
                    printer.line_break(1, indent);
                }
            }
            Element::HardLine => printer.line_break(1, indent),
            Element::BlankLine => printer.line_break(2, indent),
            Element::Group { width, forced, .. } | Element::Fill { width, forced, .. } => {
                let flat = if top.flat && !top.decide {
                    true
                } else {
                    let with_broken = matches!(
                        elements.get(index + 1),
                        Some(&Element::BreaksWith { group }) if broken[group as usize]
                    );
                    !forced
                        && !with_broken
                        && (printer.fits(width)
                            || matches!(
                                elements.get(index + 1),
                                Some(&Element::Alternative { head, tail })
                                    if u64::from(indent) + 4 + u64::from(tail)
                                        > u64::from(LINE_WIDTH)
                                        && printer.fits(head)
                            ))
                };
                broken[index] = !flat;
                stack.push(Frame {
                    flat,
                    decide: !flat,
                    indent: top.indent,
                    rewrite: false,
                });
            }
            Element::Indent(kind) => stack.push(Frame {
                indent: top.indent + kind.width(),
                rewrite: false,
                ..top
            }),
            Element::IndentIfBroken { group } => stack.push(Frame {
                indent: top.indent + if broken[group as usize] { 4 } else { 0 },
                rewrite: false,
                ..top
            }),
            Element::FillBreak { soft, next } => {
                let separator = u32::from(!soft);
                if top.flat || printer.fits(separator.saturating_add(next)) {
                    if !soft {
                        printer.space();
                    }
                } else {
                    printer.line_break(1, indent);
                }
            }
            Element::Alternative { .. } | Element::BreaksWith { .. } => {}
            Element::Hug => stack.push(Frame {
                decide: true,
                rewrite: false,
                ..top
            }),
            Element::Rewrite { start, end } => {
                printer.rewrites.push(OpenRewrite {
                    input: start..end,
                    output: None,
                });
                stack.push(Frame {
                    rewrite: true,
                    ..top
                });
            }
            Element::End => {
                let frame = stack.pop();
                assert!(!stack.is_empty(), "an End without a container");
                if frame.is_some_and(|f| f.rewrite) {
                    printer.end_rewrite();
                }
            }
            Element::Verbatim(verbatim) => {
                let verbatim = document.verbatims[verbatim as usize];
                let input = verbatim.start..verbatim.end;
                let text = &text[input.start as usize..input.end as usize];
                match verbatim.kind {
                    VerbatimKind::Copy { to_end } => {
                        printer.copy(text, verbatim.first_width, input);
                        printer.ended_by_region = to_end;
                    }
                    VerbatimKind::Comment => {
                        printer.comment(text, input);
                        printer.ended_by_region = false;
                    }
                    VerbatimKind::Doc { token, layout } => {
                        let layout = layout.map(|i| &document.doc_comments[i as usize]);
                        printer.doc_comment(text, &verbatim, token, layout, input);
                        printer.ended_by_region = false;
                    }
                }
            }
        }
    }
    let wrote = printer.last.is_some();
    if wrote && !printer.ended_by_region {
        let ending = source.line_ending.as_str();
        printer.out.extend_from_slice(ending.as_bytes());
    }
    (printer.out, printer.pieces)
}

impl Printer<'_> {
    /// Whether `width` more fits on the line, after what is pending.
    fn fits(&self, width: u32) -> bool {
        let column = if self.pending_breaks > 0 && self.last.is_some() {
            self.pending_indent
        } else {
            self.column + u32::from(self.pending_space && self.last.is_some())
        };
        u64::from(column) + u64::from(width) <= u64::from(LINE_WIDTH)
    }

    fn space(&mut self) {
        if self.pending_breaks == 0 {
            self.pending_space = true;
        }
    }

    /// Asks for `breaks` line breaks before the next text, starting a line at `indent`, the
    /// indent in effect here. Adjacent requests merge into the largest, and the line starts at
    /// the last one's indent.
    fn line_break(&mut self, breaks: u8, indent: u32) {
        self.pending_breaks = self.pending_breaks.max(breaks);
        self.pending_indent = indent;
        self.pending_space = false;
    }

    /// Writes what is pending before text starting with `first`: line breaks and indentation, or
    /// a space. Nothing is written before the first text of the file. A space is added where two
    /// pieces of text side by side would run together into other tokens.
    fn start_text(&mut self, first: &str, token: Option<usize>) {
        let Some(last) = self.last else {
            self.pending_breaks = 0;
            self.pending_space = false;
            return;
        };
        if self.pending_breaks > 0 {
            let ending = self.source.line_ending.as_str().as_bytes();
            for _ in 0..self.pending_breaks {
                self.out.extend_from_slice(ending);
            }
            self.out
                .resize(self.out.len() + self.pending_indent as usize, b' ');
            self.column = self.pending_indent;
        } else if self.pending_space
            || (token.is_none() || last.next_token != token) && could_merge(last, first)
        {
            self.out.push(b' ');
            self.column += 1;
        }
        self.pending_breaks = 0;
        self.pending_space = false;
    }

    fn end_text(&mut self, text: &str, token: Option<usize>) {
        if let Some(last_char) = text.chars().next_back() {
            self.last = Some(Last {
                next_token: token.map(|t| t + 1),
                last_char,
                number: starts_number(text),
            });
        }
    }

    /// Records that the input's `input` was written to the output from `at` to its end: one
    /// piece, or part of the rewrite being printed.
    fn record(&mut self, input: Range<u32>, at: usize) {
        let Some(pieces) = &mut self.pieces else {
            return;
        };
        if self.rewrites.is_empty() {
            pieces.push(Piece {
                input,
                output: at..self.out.len(),
            });
        } else {
            for rewrite in &mut self.rewrites {
                rewrite.output.get_or_insert(at);
            }
        }
    }

    /// Ends the innermost rewrite. The outermost is recorded as one piece.
    fn end_rewrite(&mut self) {
        let rewrite = self
            .rewrites
            .pop()
            .expect("a rewrite's End without its start");
        if self.rewrites.is_empty()
            && let Some(pieces) = &mut self.pieces
        {
            let at = rewrite.output.unwrap_or(self.out.len());
            pieces.push(Piece {
                input: rewrite.input,
                output: at..self.out.len(),
            });
        }
    }

    fn text(&mut self, text: &str, width: u32, token: Option<usize>, input: Range<u32>) {
        if text.is_empty() {
            return;
        }
        self.start_text(text, token);
        let at = self.out.len();
        self.out.extend_from_slice(text.as_bytes());
        self.column += width;
        self.end_text(text, token);
        self.record(input, at);
    }

    /// Writes a doc comment (`VerbatimKind::Doc`): reflowed from its layout at the column it
    /// starts at (docs/design/formatter-completeness.md, "Where reflow happens"), or, without one,
    /// a one-line comment as written and a longer one as [`Printer::comment`] re-indents it. A
    /// Markdown comment, a run of `///` lines, always has one.
    fn doc_comment(
        &mut self,
        text: &str,
        verbatim: &crate::document::Verbatim,
        token: ferrion_syntax::TokenId,
        layout: Option<&crate::doc_comments::Layout>,
        input: Range<u32>,
    ) {
        let Some(layout) = layout else {
            if verbatim.breaks {
                self.comment(text, input);
            } else {
                self.text(text, verbatim.first_width, Some(token.index()), input);
            }
            return;
        };
        self.start_text(text, None);
        let at = self.out.len();
        let source = self.source.tree.text();
        let lines = if text.starts_with("///") {
            crate::doc_comments::markdown::render(layout, source, self.column)
        } else {
            crate::doc_comments::javadoc::render(layout, source, self.column)
        };
        let ending = self.source.line_ending.as_str().as_bytes();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                self.out.extend_from_slice(ending);
                self.column = 0;
            }
            self.out.extend_from_slice(line.as_bytes());
            self.column += width(line);
        }
        self.end_text(text, None);
        self.record(input, at);
    }

    /// Copies `text` as written, with each line terminator replaced by the file's.
    fn copy(&mut self, text: &str, first_width: u32, input: Range<u32>) {
        if text.is_empty() {
            return;
        }
        self.start_text(text, None);
        let at = self.out.len();
        let ending = self.source.line_ending.as_str().as_bytes();
        let mut lines = Lines::new(text);
        let first = lines.next().unwrap_or_default();
        self.out.extend_from_slice(first.as_bytes());
        self.column += first_width;
        for line in lines {
            self.out.extend_from_slice(ending);
            self.out.extend_from_slice(line.as_bytes());
            self.column = width(line);
        }
        self.end_text(text, None);
        self.record(input, at);
    }

    /// Writes a block comment that spans lines, its later lines re-indented to the column it
    /// starts at (docs/design/formatter.md, "Copied text"). If every later line starts with `*`,
    /// the stars go one column after the `/`. Otherwise each later line moves by the amount that
    /// puts the least indented of them at the comment's column, keeping their indentation
    /// relative to each other. A doc comment, which GJF rewrites with a star on every line, puts
    /// the stars it has one column after the `/` too, and moves its other lines by the amount that
    /// puts the least indented star there. Trailing whitespace is removed, and lines of only
    /// whitespace are left empty and don't count as the least indented.
    fn comment(&mut self, text: &str, input: Range<u32>) {
        self.start_text(text, None);
        let at = self.out.len();
        let column = self.column;
        let ending = self.source.line_ending.as_str().as_bytes();
        let mut lines = Lines::new(text);
        let first = trim_end(lines.next().unwrap_or_default());
        self.out.extend_from_slice(first.as_bytes());
        self.column += width(first);
        let later = lines;
        let is_starred = |line: &str| trim_start(line).starts_with('*');
        let is_blank = |line: &str| trim_start(line).is_empty();
        let doc = text.starts_with("/**");
        // Whether the lines that start with `*` have their stars aligned.
        let align_stars = later.clone().all(is_starred) || doc && later.clone().any(is_starred);
        // The other lines move from `from` characters of leading whitespace to `to` columns.
        let (from, to) = if align_stars {
            let stars = later.clone().filter(|&line| is_starred(line));
            (stars.map(leading_whitespace).min().unwrap_or(0), column + 1)
        } else {
            let least = later.clone().filter(|&line| !is_blank(line));
            (least.map(leading_whitespace).min().unwrap_or(0), column)
        };
        for line in later {
            self.out.extend_from_slice(ending);
            let (pad, rest) = if align_stars && is_starred(line) {
                (column + 1, trim_start(line))
            } else {
                // Whitespace past `from` stays as written, so tabs keep their place.
                let skip = leading_whitespace(line).min(from);
                let bytes: usize = line.chars().take(skip).map(char::len_utf8).sum();
                let pad = (u64::from(to) + skip as u64).saturating_sub(from as u64);
                (u32::try_from(pad).unwrap_or(u32::MAX), &line[bytes..])
            };
            let rest = trim_end(rest);
            if rest.is_empty() {
                self.column = 0;
                continue;
            }
            self.out.resize(self.out.len() + pad as usize, b' ');
            self.out.extend_from_slice(rest.as_bytes());
            self.column = pad + width(rest);
        }
        self.end_text(text, None);
        self.record(input, at);
    }
}

/// Whether `first` written right after `last`, with nothing between, could lex differently:
/// two words run together (`int` `x`), two operators make another (`-` `-`, `/` `*`), or a dot
/// and a number make a number (`1` `.`, `.` `5`). Conservative: a space where none is needed only
/// costs a column.
fn could_merge(last: Last, first: &str) -> bool {
    let Some(next) = first.chars().next() else {
        return false;
    };
    let a = last.last_char;
    (is_word(a) || a == '\\') && (is_word(next) || next == '\\')
        || is_operator(a) && is_operator(next)
        || a == '.' && (next == '.' || next.is_ascii_digit())
        || last.number && next == '.'
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$' || !c.is_ascii()
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '=' | '>' | '<' | '!' | '~' | '?' | ':' | '&' | '|' | '+' | '-' | '*' | '/' | '^' | '%'
    )
}

/// Whether `text` may be a number literal: it starts with a digit, a `.` and a digit, or a
/// Unicode escape, which could be either.
fn starts_number(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() || c == '\\' => true,
        Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

/// Java's whitespace within a line: space, tab, and form feed (JLS 3.6).
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\x0c')
}

pub(crate) fn trim_end(text: &str) -> &str {
    text.trim_end_matches(is_space)
}

fn trim_start(text: &str) -> &str {
    text.trim_start_matches(is_space)
}

fn leading_whitespace(line: &str) -> usize {
    line.chars().take_while(|&c| is_space(c)).count()
}

/// The lines of `text`, split at CR, LF, and CRLF, without their terminators.
#[derive(Clone)]
pub(crate) struct Lines<'a> {
    rest: Option<&'a str>,
}

impl<'a> Lines<'a> {
    pub(crate) fn new(text: &'a str) -> Lines<'a> {
        Lines { rest: Some(text) }
    }
}

impl<'a> Iterator for Lines<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = self.rest?;
        match rest.find(['\n', '\r']) {
            Some(i) => {
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                self.rest = Some(&rest[i + skip..]);
                Some(&rest[..i])
            }
            None => {
                self.rest = None;
                Some(rest)
            }
        }
    }
}

#[cfg(test)]
mod tests;
