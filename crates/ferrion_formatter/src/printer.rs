//! The printer (docs/design/formatter.md, "The printer"): one loop over a [`Document`]'s elements
//! with a stack of open containers, deciding each group's mode as it starts, from its precomputed
//! width. Printing is linear in the number of elements plus the output, and never fails: a line
//! goes over the limit when nothing in the groups lets it break.

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
}

/// Prints `document` and returns the file's bytes: the byte-order mark if there is one, the text
/// with the source's line ending, and one line ending at the end if there is any text.
pub fn print(document: &Document, source: &Source<'_>) -> Vec<u8> {
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
    }];
    for (index, element) in elements.iter().enumerate() {
        let top = *stack.last().expect("the root frame is never popped");
        let indent = top.indent;
        match *element {
            Element::Token { token, len, width } => {
                let start = source.tree.token(token).span().start as usize;
                let text = &text[start..start + len as usize];
                printer.text(text, width, Some(token.index()));
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
                    !forced
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
                });
            }
            Element::Indent(kind) => stack.push(Frame {
                indent: top.indent + kind.width(),
                ..top
            }),
            Element::IndentIfBroken { group } => stack.push(Frame {
                indent: top.indent + if broken[group as usize] { 4 } else { 0 },
                ..top
            }),
            Element::FillBreak { soft, next } => {
                let separator = u32::from(!soft);
                if top.flat || printer.fits(separator + next) {
                    if !soft {
                        printer.space();
                    }
                } else {
                    printer.line_break(1, indent);
                }
            }
            Element::Alternative { .. } => {}
            Element::Hug => stack.push(Frame {
                decide: true,
                ..top
            }),
            Element::End => {
                stack.pop();
                assert!(!stack.is_empty(), "an End without a container");
            }
            Element::Verbatim(verbatim) => {
                let verbatim = document.verbatims[verbatim as usize];
                let text = &text[verbatim.start as usize..verbatim.end as usize];
                match verbatim.kind {
                    VerbatimKind::Copy { to_end } => {
                        printer.copy(text, verbatim.first_width);
                        printer.ended_by_region = to_end;
                    }
                    VerbatimKind::Comment => {
                        printer.comment(text);
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
    printer.out
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

    fn text(&mut self, text: &str, width: u32, token: Option<usize>) {
        if text.is_empty() {
            return;
        }
        self.start_text(text, token);
        self.out.extend_from_slice(text.as_bytes());
        self.column += width;
        self.end_text(text, token);
    }

    /// Copies `text` as written, with each line terminator replaced by the file's.
    fn copy(&mut self, text: &str, first_width: u32) {
        if text.is_empty() {
            return;
        }
        self.start_text(text, None);
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
    }

    /// Writes a block comment that spans lines, its later lines re-indented to the column it
    /// starts at (docs/design/formatter.md, "Copied text"). If every later line starts with `*`,
    /// the stars go one column after the `/`. Otherwise each later line moves by the amount that
    /// puts the least indented of them at the comment's column, keeping their indentation
    /// relative to each other. Trailing whitespace is removed, and lines of only whitespace are
    /// left empty.
    fn comment(&mut self, text: &str) {
        self.start_text(text, None);
        let column = self.column;
        let ending = self.source.line_ending.as_str().as_bytes();
        let mut lines = Lines::new(text);
        let first = trim_end(lines.next().unwrap_or_default());
        self.out.extend_from_slice(first.as_bytes());
        self.column += width(first);
        let later = lines;
        let starred = later.clone().all(|line| trim_start(line).starts_with('*'));
        // In characters of leading whitespace.
        let least = if starred {
            0
        } else {
            later
                .clone()
                .filter(|line| !trim_start(line).is_empty())
                .map(leading_whitespace)
                .min()
                .unwrap_or(0)
        };
        for line in later {
            self.out.extend_from_slice(ending);
            let rest = if starred {
                trim_start(line)
            } else {
                let skip: usize = line.chars().take(least).map(char::len_utf8).sum();
                &line[skip..]
            };
            let rest = trim_end(rest);
            if rest.is_empty() {
                self.column = 0;
                continue;
            }
            let pad = if starred { column + 1 } else { column };
            self.out.resize(self.out.len() + pad as usize, b' ');
            self.out.extend_from_slice(rest.as_bytes());
            self.column = pad + width(rest);
        }
        self.end_text(text, None);
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
