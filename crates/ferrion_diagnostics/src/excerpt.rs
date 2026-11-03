//! Rendering a diagnostic as text with an excerpt of the source it is about, for the `pretty`
//! output format:
//!
//! ```text
//! error: unknown key `ignore` in configs[1]
//!  --> ferrion.jsonc:4:7
//!   |
//! 4 |       "ignore": ["**/legacy/**"],
//!   |       ^^^^^^^^ did you mean `ignores`?
//! ```
//!
//! The position after `-->` is 1-based, and its column counts UTF-16 code units, as the JSON
//! output and the Language Server Protocol do. In the excerpt itself, each character takes one
//! column and a tab four, so that the marks line up under the text they mark.

use std::fmt::Write;

use crate::{LineIndex, Span};

/// How many columns a tab takes in an excerpt.
const TAB_WIDTH: usize = 4;

/// The width a label is wrapped to, counting the gutter, unless that leaves it less than
/// [`MIN_LABEL_WIDTH`].
const MAX_WIDTH: usize = 100;
const MIN_LABEL_WIDTH: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Note,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Note => "note",
        }
    }
}

/// Whether a label marks what the diagnostic is about (`^`), or something related to it (`-`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelStyle {
    Primary,
    Secondary,
}

/// A span of the source, marked in the excerpt, with an optional text after the marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub style: LabelStyle,
    pub text: Option<String>,
}

impl Label {
    pub fn primary(span: Span, text: Option<String>) -> Label {
        Label {
            span,
            style: LabelStyle::Primary,
            text,
        }
    }

    pub fn secondary(span: Span, text: Option<String>) -> Label {
        Label {
            span,
            style: LabelStyle::Secondary,
            text,
        }
    }
}

/// A diagnostic to render with [`Excerpt::render`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Excerpt {
    pub severity: Severity,
    pub message: String,
    /// The spans to mark. The position after `-->` is the first primary label's, or the first
    /// label's if none is primary. A label whose span covers several lines is marked on its first.
    pub labels: Vec<Label>,
    pub help: Option<String>,
}

impl Excerpt {
    /// Renders the diagnostic, for a file at `path` with the text `text`, which `index` indexes.
    /// The result ends with a line break.
    pub fn render(&self, path: &str, text: &str, index: &LineIndex) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{}: {}", self.severity.as_str(), self.message);
        let lines = self.marks(text, index);
        let gutter = lines
            .last()
            .map_or(1, |(line, _)| (line + 1).to_string().len());
        let pad = " ".repeat(gutter);
        if let Some(anchor) = self
            .labels
            .iter()
            .find(|l| l.style == LabelStyle::Primary)
            .or(self.labels.first())
        {
            let at = index.line_col(anchor.span.start);
            let _ = writeln!(out, "{pad}--> {path}:{}:{}", at.line + 1, at.col + 1);
        } else {
            let _ = writeln!(out, "{pad}--> {path}");
        }
        if !lines.is_empty() {
            let _ = writeln!(out, "{pad} |");
        }
        let mut previous: Option<u32> = None;
        for (line, marks) in &lines {
            match previous {
                Some(p) if line - p == 2 => source_line(&mut out, text, index, p + 1, gutter),
                Some(p) if line - p > 2 => {
                    let _ = writeln!(out, "...");
                }
                _ => {}
            }
            source_line(&mut out, text, index, *line, gutter);
            for mark in marks {
                let symbol = match mark.style {
                    LabelStyle::Primary => "^",
                    LabelStyle::Secondary => "-",
                };
                let mut head = format!(
                    "{pad} | {}{}",
                    " ".repeat(mark.start),
                    symbol.repeat(mark.width)
                );
                match &mark.text {
                    Some(label) => {
                        let indent = head.chars().count() + 1;
                        let width = MAX_WIDTH.saturating_sub(indent).max(MIN_LABEL_WIDTH);
                        let mut wrapped = wrap(label, width).into_iter();
                        head.push(' ');
                        head.push_str(&wrapped.next().unwrap_or_default());
                        let _ = writeln!(out, "{head}");
                        let continuation = format!("{pad} |{}", " ".repeat(indent - gutter - 2));
                        for rest in wrapped {
                            let _ = writeln!(out, "{continuation}{rest}");
                        }
                    }
                    None => {
                        let _ = writeln!(out, "{head}");
                    }
                }
            }
            previous = Some(*line);
        }
        if let Some(help) = &self.help {
            let _ = writeln!(out, "{pad} = help: {help}");
        }
        out
    }

    /// The labels' marks, grouped by line, in line order, and in column order on each line.
    fn marks(&self, text: &str, index: &LineIndex) -> Vec<(u32, Vec<Mark>)> {
        let mut marks: Vec<(u32, Mark)> = self
            .labels
            .iter()
            .map(|label| {
                let line = index.line_col(label.span.start).line;
                let (line_start, content) = line_text(text, index, line);
                let start = (label.span.start - line_start) as usize;
                let end = (label.span.end.max(label.span.start) - line_start) as usize;
                let end = end.min(content.len());
                let from = display_width(&content[..start.min(content.len())]);
                let to = display_width(&content[..end.max(start.min(content.len()))]);
                (
                    line,
                    Mark {
                        start: from,
                        width: (to - from).max(1),
                        style: label.style,
                        text: label.text.clone(),
                    },
                )
            })
            .collect();
        marks.sort_by_key(|(line, mark)| (*line, mark.start));
        let mut lines: Vec<(u32, Vec<Mark>)> = Vec::new();
        for (line, mark) in marks {
            match lines.last_mut() {
                Some((last, group)) if *last == line => group.push(mark),
                _ => lines.push((line, vec![mark])),
            }
        }
        lines
    }
}

#[derive(Debug)]
struct Mark {
    start: usize,
    width: usize,
    style: LabelStyle,
    text: Option<String>,
}

/// Writes line `line` (0-based) of the text, after its number in the gutter.
fn source_line(out: &mut String, text: &str, index: &LineIndex, line: u32, gutter: usize) {
    let (_, content) = line_text(text, index, line);
    let expanded = expand_tabs(content);
    let number = line + 1;
    if expanded.is_empty() {
        let _ = writeln!(out, "{number:>gutter$} |");
    } else {
        let _ = writeln!(out, "{number:>gutter$} | {expanded}");
    }
}

/// The byte offset where line `line` starts, and its text without its line terminator.
fn line_text<'t>(text: &'t str, index: &LineIndex, line: u32) -> (u32, &'t str) {
    let start = index.line_start(line).unwrap_or(text.len() as u32);
    let end = index.line_start(line + 1).unwrap_or(text.len() as u32);
    let content = &text[start as usize..end as usize];
    let content = content
        .strip_suffix("\r\n")
        .or_else(|| content.strip_suffix('\n'))
        .or_else(|| content.strip_suffix('\r'))
        .unwrap_or(content);
    (start, content)
}

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

fn expand_tabs(text: &str) -> String {
    text.replace('\t', &" ".repeat(TAB_WIDTH))
}

/// Splits `text` into lines of at most `width` characters at spaces, where it can.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut line));
            } else if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str, excerpt: &Excerpt) -> String {
        excerpt.render("ferrion.jsonc", text, &LineIndex::new(text))
    }

    fn span_of(text: &str, needle: &str) -> Span {
        let start = text.find(needle).unwrap() as u32;
        Span::new(start, start + needle.len() as u32)
    }

    #[test]
    fn one_label() {
        let text = "{\n  \"configs\": [\n    {},\n      \"ignore\": [\"**/legacy/**\"],\n  ]\n}\n";
        let excerpt = Excerpt {
            severity: Severity::Error,
            message: "unknown key `ignore` in configs[1]".into(),
            labels: vec![Label::primary(
                span_of(text, "\"ignore\""),
                Some("did you mean `ignores`?".into()),
            )],
            help: None,
        };
        assert_eq!(
            render(text, &excerpt),
            "\
error: unknown key `ignore` in configs[1]
 --> ferrion.jsonc:4:7
  |
4 |       \"ignore\": [\"**/legacy/**\"],
  |       ^^^^^^^^ did you mean `ignores`?
"
        );
    }

    #[test]
    fn two_lines_and_help() {
        let text = "{\n  \"a\": 1\n  \"b\": 2\n}";
        let one = span_of(text, "1").end;
        let excerpt = Excerpt {
            severity: Severity::Error,
            message: "expected `,` or `}` after this value".into(),
            labels: vec![
                Label::primary(
                    Span::empty(span_of(text, "\"b\"").start),
                    Some("next".into()),
                ),
                Label::secondary(Span::empty(one), Some("add `,` here".into())),
            ],
            help: Some("keys are separated by commas".into()),
        };
        assert_eq!(
            render(text, &excerpt),
            "\
error: expected `,` or `}` after this value
 --> ferrion.jsonc:3:3
  |
2 |   \"a\": 1
  |         - add `,` here
3 |   \"b\": 2
  |   ^ next
  = help: keys are separated by commas
"
        );
    }

    #[test]
    fn gaps_and_wide_gutters() {
        let mut text = String::new();
        for i in 1..=12 {
            let _ = writeln!(text, "line {i}");
        }
        let excerpt = Excerpt {
            severity: Severity::Warning,
            message: "m".into(),
            labels: vec![
                Label::secondary(span_of(&text, "line 7"), None),
                Label::secondary(span_of(&text, "line 9"), None),
                Label::primary(span_of(&text, "line 12"), Some("here".into())),
            ],
            help: None,
        };
        assert_eq!(
            render(&text, &excerpt),
            "\
warning: m
  --> ferrion.jsonc:12:1
   |
 7 | line 7
   | ------
 8 | line 8
 9 | line 9
   | ------
...
12 | line 12
   | ^^^^^^^ here
"
        );
    }

    #[test]
    fn tabs_and_utf16_columns() {
        // The `-->` column counts UTF-16 code units: the tab and `é` one each, `😀` two.
        let text = "\té😀 x";
        let excerpt = Excerpt {
            severity: Severity::Error,
            message: "m".into(),
            labels: vec![Label::primary(span_of(text, "x"), None)],
            help: None,
        };
        assert_eq!(
            render(text, &excerpt),
            "\
error: m
 --> ferrion.jsonc:1:6
  |
1 |     é😀 x
  |        ^
"
        );
    }

    #[test]
    fn spans_across_lines_and_at_the_end() {
        let text = "ab\r\ncd\r\n";
        let excerpt = Excerpt {
            severity: Severity::Error,
            message: "m".into(),
            labels: vec![
                Label::primary(Span::new(1, 6), Some("from here".into())),
                Label::secondary(Span::empty(text.len() as u32), Some("the end".into())),
            ],
            help: None,
        };
        assert_eq!(
            render(text, &excerpt),
            "\
error: m
 --> ferrion.jsonc:1:2
  |
1 | ab
  |  ^ from here
2 | cd
3 |
  | - the end
"
        );
    }

    #[test]
    fn long_labels_wrap_under_themselves() {
        let text = "    { \"files\": [\"../shared/**\"] },";
        let label = "globs are relative to the working directory, and can't match outside it, \
                     so this glob would never match any file at all";
        let excerpt = Excerpt {
            severity: Severity::Error,
            message: "a glob can't contain `..`".into(),
            labels: vec![Label::primary(
                span_of(text, "\"../shared/**\""),
                Some(label.into()),
            )],
            help: None,
        };
        assert_eq!(
            render(text, &excerpt),
            "\
error: a glob can't contain `..`
 --> ferrion.jsonc:1:17
  |
1 |     { \"files\": [\"../shared/**\"] },
  |                 ^^^^^^^^^^^^^^ globs are relative to the working directory, and can't match
  |                                outside it, so this glob would never match any file at all
"
        );
    }

    #[test]
    fn no_labels() {
        let excerpt = Excerpt {
            severity: Severity::Note,
            message: "m".into(),
            labels: vec![],
            help: Some("h".into()),
        };
        assert_eq!(
            render("", &excerpt),
            "note: m\n --> ferrion.jsonc\n  = help: h\n"
        );
    }
}
