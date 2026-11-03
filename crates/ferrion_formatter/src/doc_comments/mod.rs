//! Doc comment reflow (docs/design/formatter-completeness.md, "Doc comments").
//!
//! A comment's lines depend on the column it starts at, which only the printer knows, so reflow
//! is split in two:
//!
//! - **Layout, at build time:** everything that doesn't depend on the column, once per comment.
//!   [`javadoc::layout`] parses a traditional comment with `ferrion_doc_comments` and cuts it into
//!   [`Paragraph`]s of [`Word`]s: which lines start where, at what indentation, with which line
//!   breaks between them, and what may go between each pair of words. A comment the rules don't
//!   reflow has no layout, and is re-indented as A1 does.
//! - **Fill, at print time:** [`fill`] breaks each paragraph into lines at the column, greedily,
//!   and [`javadoc::render`] puts the comment's decoration around them.
//!
//! The layout and the fill don't know about Javadoc's HTML, so a Markdown comment can be laid out
//! into the same paragraphs, with its own glue (a word that would start a block can't start a
//! line, as `@` can't) and its own decoration around the filled lines.

pub(crate) mod fill;
pub(crate) mod javadoc;

pub(crate) use fill::{Columns, fill};

/// A doc comment's layout: its paragraphs, each a run of words filled as one, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub paragraphs: Vec<Paragraph>,
    /// The words of every paragraph, in order; each paragraph has a range of them.
    pub words: Vec<Word>,
    /// The segments of the words copied across lines ([`WordText::Lines`]).
    pub lines: Vec<Segment>,
    /// Whether the comment has a block tag, which keeps it off one line (Javadoc's shape rule).
    pub tags: bool,
}

/// Lines filled from a run of words, starting on a line of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paragraph {
    /// The line breaks before it: 1 for a line break, 2 for a blank line. The first paragraph's
    /// is ignored.
    pub gap: u8,
    /// The indentation of its first line, past the comment's decoration.
    pub indent: u32,
    /// The indentation of its later lines: a block tag's continuation lines are 4 past its first.
    pub hanging: u32,
    /// Its words, in [`Layout::words`].
    pub words: (u32, u32),
}

/// A word of a paragraph: text that is never broken, with what goes between it and the word
/// before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: WordText,
    /// The width of its text up to its first line break, in UTF-16 code units, as written (a
    /// Unicode escape counts as the characters written).
    pub width: u32,
    /// What goes before it. Ignored for a paragraph's first word.
    pub glue: Glue,
}

/// A word's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordText {
    /// The source text from the first offset to the second, on one line.
    Source(u32, u32),
    /// Text the layout writes in place of the source's: a normalized `<p>` or `<br>`, or a `<p>`
    /// added before a paragraph.
    Static(&'static str),
    /// Text copied across lines (`<pre>` content, a table, a `<code>` that spans lines): the
    /// segments from the first index in [`Layout::lines`], the second many. The first line
    /// continues the line the word is on; each later line is a line of its own, copied after the
    /// decoration, and the words after it continue that line.
    Lines(u32, u32),
}

/// A part of a word copied across lines: the source from `start` to `end`, on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: u32,
    pub end: u32,
    /// Whether it continues the line of the segment before it, rather than starting a line: the
    /// line break between them is left out, as between `<pre>` and `{@code`.
    pub joins: bool,
}

/// What goes between two words, and whether a line may break there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glue {
    /// A space, or a line break if the next word doesn't fit.
    Space,
    /// A space, but never a line break: before a word that would mean something else at the start
    /// of a line, such as `@` (a block tag) in Javadoc.
    Bound,
    /// Nothing: the two are written together, as `<p>` and the word after it.
    Tight,
    /// Nothing, or a line break if the next word doesn't fit: after `<code>`.
    Soft,
}

impl Glue {
    /// Whether a line may break here.
    pub fn breaks(self) -> bool {
        matches!(self, Glue::Space | Glue::Soft)
    }

    /// The width it takes when the line doesn't break.
    pub fn width(self) -> u32 {
        match self {
            Glue::Space | Glue::Bound => 1,
            Glue::Tight | Glue::Soft => 0,
        }
    }
}

#[cfg(test)]
mod tests;
