//! Filling a [`Layout`] at a column (docs/design/formatter-completeness.md, "Where reflow
//! happens"): each paragraph's words go on its lines greedily, a line breaking only where the next
//! word doesn't fit, which keeps the fill linear and stable. Filling filled text gives the same
//! lines, since its words, glue, and paragraphs are the same.

use super::{Layout, WordText};
use crate::document::width;
use crate::printer::trim_end;

/// Where a filled comment's lines go, in columns of the output.
#[derive(Clone, Copy, Debug)]
pub struct Columns {
    /// Where a line's text starts, before its indentation: past ` * ` in Javadoc.
    pub text: u32,
    /// Where a copied line's text starts: past ` *` in Javadoc, since a copied line keeps the
    /// whitespace after the star as written.
    pub copied: u32,
    /// The column limit: a line fits if it ends at or before it.
    pub limit: u32,
}

/// A filled line, without the comment's decoration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Its indentation past the decoration. Zero for a copied line.
    pub indent: u32,
    /// Its text, without trailing whitespace. Empty for a blank line.
    pub text: String,
    /// Whether it starts with a copied line of a [`WordText::Lines`] word, which goes right
    /// after the decoration's star, as written.
    pub copied: bool,
    /// The index of the paragraph it is a line of, or, for a blank line between paragraphs, of
    /// the paragraph after it.
    pub paragraph: u32,
}

/// Fills `layout`, whose source is `text`, at `columns`. The lines between paragraphs are blank
/// lines with empty text. No line is broken below a word's width, so at deep indentation a
/// paragraph has one word per line, and a word wider than the line overflows it.
pub fn fill(layout: &Layout, text: &str, columns: Columns) -> Vec<Line> {
    let mut lines = Vec::new();
    let limit = u64::from(columns.limit);
    for (index, paragraph) in layout.paragraphs.iter().enumerate() {
        let number = index as u32;
        if index > 0 {
            for _ in 1..paragraph.gap {
                lines.push(Line {
                    indent: 0,
                    text: String::new(),
                    copied: false,
                    paragraph: number,
                });
            }
        }
        let words = &layout.words[paragraph.words.0 as usize..paragraph.words.1 as usize];
        let mut line = Line {
            indent: paragraph.indent,
            text: String::new(),
            copied: false,
            paragraph: number,
        };
        let mut column = u64::from(columns.text) + u64::from(paragraph.indent);
        // Whether the line holds no text yet.
        let mut empty = true;
        let mut i = 0;
        while i < words.len() {
            // The unit is the words up to the next place a line may break.
            let mut j = i + 1;
            while j < words.len() && !words[j].glue.breaks() {
                j += 1;
            }
            let mut unit = 0u64;
            for (n, word) in words[i..j].iter().enumerate() {
                if n > 0 {
                    unit += u64::from(word.glue.width());
                }
                unit += u64::from(word.width);
                if matches!(word.text, WordText::Lines(..)) {
                    break;
                }
            }
            let mut separator = if empty { 0 } else { words[i].glue.width() };
            if !empty && column + u64::from(separator) + unit > limit {
                push(&mut lines, line);
                line = Line {
                    indent: paragraph.hanging,
                    text: String::new(),
                    copied: false,
                    paragraph: number,
                };
                column = u64::from(columns.text) + u64::from(paragraph.hanging);
                separator = 0;
            }
            for (n, word) in words[i..j].iter().enumerate() {
                let glue = if n == 0 { separator } else { word.glue.width() };
                if glue == 1 {
                    line.text.push(' ');
                }
                column += u64::from(glue);
                match word.text {
                    WordText::Source(start, end) => {
                        line.text.push_str(&text[start as usize..end as usize]);
                        column += u64::from(word.width);
                    }
                    WordText::Static(s) => {
                        line.text.push_str(s);
                        column += u64::from(word.width);
                    }
                    WordText::Lines(first, count) => {
                        let ranges = &layout.lines[first as usize..(first + count) as usize];
                        for (k, segment) in ranges.iter().enumerate() {
                            let part = &text[segment.start as usize..segment.end as usize];
                            if k == 0 || segment.joins {
                                line.text.push_str(part);
                                column += u64::from(width(part));
                            } else {
                                push(&mut lines, line);
                                line = Line {
                                    indent: 0,
                                    text: part.to_owned(),
                                    copied: true,
                                    paragraph: number,
                                };
                                column = u64::from(columns.copied) + u64::from(width(part));
                            }
                        }
                    }
                }
            }
            empty = false;
            i = j;
        }
        push(&mut lines, line);
    }
    lines
}

fn push(lines: &mut Vec<Line>, mut line: Line) {
    let trimmed = trim_end(&line.text).len();
    line.text.truncate(trimmed);
    lines.push(line);
}
