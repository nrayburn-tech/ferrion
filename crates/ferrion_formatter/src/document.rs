//! The document the formatter builds from a tree and the printer lays out (docs/design/formatter.md,
//! "The document"): a flat array of elements, with containers as start and end markers, so that
//! building and dropping it never recurses.
//!
//! [`DocumentBuilder`] appends elements and computes each group's flat width as it goes, so that
//! the printer's fitting check is a lookup. Only [`Element::Token`] and [`Element::Verbatim`] print
//! text other than whitespace, and both take it from the source by token, so a document can't
//! invent or alter code.

use ferrion_syntax::TokenId;

/// The column limit, counted in UTF-16 code units (SCOPE.md).
pub const LINE_WIDTH: u32 = 100;

/// How much an [`Element::Indent`] indents the lines it contains.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IndentKind {
    /// +2: a block's or a body's contents.
    Block,
    /// +4: a line that continues the one before it.
    #[allow(dead_code, reason = "for the construct rules still to come")]
    Continuation,
}

impl IndentKind {
    pub fn width(self) -> u32 {
        match self {
            IndentKind::Block => 2,
            IndentKind::Continuation => 4,
        }
    }
}

/// One element of a [`Document`]. Containers (`Group`, `Indent`, `IndentIfBroken`, `Fill`, and
/// `Hug`) run to their matching [`Element::End`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Element {
    /// The first `len` bytes of a token's text, which hold no line terminator: all of it, or a
    /// line comment's without trailing whitespace. `width` is that text's width in UTF-16 code
    /// units.
    Token {
        token: TokenId,
        len: u32,
        width: u32,
    },
    /// One space. Spaces at the start of a line print nothing, so that indentation decides.
    Space,
    /// A space when its group is flat, a line break when it's broken.
    Line,
    /// Nothing when its group is flat, a line break when it's broken.
    SoftLine,
    /// A line break. The groups around it can't be flat, unless it's inside a [`Hug`] in them.
    ///
    /// [`Hug`]: Element::Hug
    HardLine,
    /// A line break and an empty line, which also forces its groups to break.
    BlankLine,
    /// Its contents, flat if they fit on the rest of the line, otherwise broken. `end` is the
    /// index of its `End`. `width` is what fitting needs: the contents' flat width plus the width
    /// from the `End` to the next line element of any kind, or, when a forced break inside a hug
    /// `stopped` the measure, the width up to that break. `forced` means a forced break outside
    /// any hug, which always breaks the group.
    Group {
        end: u32,
        width: u32,
        forced: bool,
        stopped: bool,
    },
    /// Its contents, with the line breaks in them indented by the kind's width.
    Indent(IndentKind),
    /// Its contents, indented by +4 only if the group starting at index `group` is broken.
    IndentIfBroken { group: u32 },
    /// Items and [`FillBreak`](Element::FillBreak)s: flat if it all fits, like a group, and
    /// otherwise each separator breaks only if the item after it doesn't fit on the line.
    Fill {
        end: u32,
        width: u32,
        forced: bool,
        stopped: bool,
    },
    /// A separator in a [`Fill`](Element::Fill): a space (or nothing, when `soft`) if the next
    /// item, `next` wide, fits on the line, otherwise a line break.
    FillBreak { soft: bool, next: u32 },
    /// Its contents. Forced breaks in them don't force the groups around the hug to break, and
    /// the groups in them are decided by fitting even when the group around the hug is flat.
    Hug,
    /// Ends the innermost container.
    End,
    /// Text copied rather than laid out, by its index in [`Document::verbatims`].
    Verbatim(u32),
}

/// Text that is placed rather than laid out (docs/design/formatter.md, "Copied text"): a text
/// block, a block comment that spans lines, or a disable region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Verbatim {
    /// Byte offsets into the tree's text.
    pub start: u32,
    pub end: u32,
    pub kind: VerbatimKind,
    /// The width of the text up to its first line break, or all of it if it has none.
    pub first_width: u32,
    /// Whether the text holds a line break.
    pub breaks: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerbatimKind {
    /// Copied as written, apart from line terminators, which become the file's: a text block, or
    /// a disable region. `to_end` marks a region that runs to the end of the file, whose text
    /// ends the output as written, with no line ending added.
    Copy { to_end: bool },
    /// A block comment, its later lines re-indented to the column it starts at.
    Comment,
}

/// A finished document.
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub elements: Vec<Element>,
    pub verbatims: Vec<Verbatim>,
}

/// The width of `text` in UTF-16 code units, as SCOPE.md counts columns. A Unicode escape counts
/// as the characters written.
pub fn width(text: &str) -> u32 {
    if text.is_ascii() {
        text.len() as u32
    } else {
        text.chars().map(|c| c.len_utf16() as u32).sum()
    }
}

/// A container open while building.
#[derive(Debug)]
struct Open {
    index: u32,
    /// The flat position at its start.
    start: u64,
    /// The flat position at the first forced break while it was open.
    stop: Option<u64>,
    /// Whether a forced break outside any hug in it was seen.
    forced: bool,
    hug: bool,
    /// For a fill, its last separator so far and the flat position after it.
    fill_break: Option<(u32, u64)>,
}

/// Builds a [`Document`], computing flat widths as it goes.
///
/// The flat position is the width of everything appended so far, as if printed on one line.
/// Each container records the position at its start and, at the first forced break inside it, the
/// position there, so its flat width is a subtraction. A forced break marks the open containers
/// from the innermost outward and stops at the first one already marked, since a mark on one
/// container means the containers around it were marked too: each is marked once, so building
/// is linear in the number of elements, however deep they nest.
#[derive(Debug, Default)]
pub struct DocumentBuilder {
    elements: Vec<Element>,
    verbatims: Vec<Verbatim>,
    open: Vec<Open>,
    position: u64,
}

impl DocumentBuilder {
    pub fn new() -> DocumentBuilder {
        DocumentBuilder::default()
    }

    pub fn token(&mut self, token: TokenId, len: u32, width: u32) {
        self.position += u64::from(width);
        self.elements.push(Element::Token { token, len, width });
    }

    pub fn space(&mut self) {
        self.position += 1;
        self.elements.push(Element::Space);
    }

    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn line(&mut self) {
        self.position += 1;
        self.elements.push(Element::Line);
    }

    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn soft_line(&mut self) {
        self.elements.push(Element::SoftLine);
    }

    pub fn hard_line(&mut self) {
        self.force_break();
        self.elements.push(Element::HardLine);
    }

    pub fn blank_line(&mut self) {
        self.force_break();
        self.elements.push(Element::BlankLine);
    }

    pub fn verbatim(&mut self, verbatim: Verbatim) {
        self.position += u64::from(verbatim.first_width);
        if verbatim.breaks {
            self.force_break();
        }
        let index = self.verbatims.len() as u32;
        self.verbatims.push(verbatim);
        self.elements.push(Element::Verbatim(index));
    }

    /// Starts a group, returning its index for [`DocumentBuilder::start_indent_if_broken`].
    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn start_group(&mut self) -> u32 {
        self.start(
            Element::Group {
                end: 0,
                width: 0,
                forced: false,
                stopped: false,
            },
            false,
        )
    }

    pub fn start_indent(&mut self, kind: IndentKind) {
        self.start(Element::Indent(kind), false);
    }

    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn start_indent_if_broken(&mut self, group: u32) {
        debug_assert!(
            matches!(self.elements[group as usize], Element::Group { .. }),
            "IndentIfBroken names element {group}, which isn't a group"
        );
        self.start(Element::IndentIfBroken { group }, false);
    }

    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn start_fill(&mut self) {
        self.start(
            Element::Fill {
                end: 0,
                width: 0,
                forced: false,
                stopped: false,
            },
            false,
        );
    }

    /// A separator between two items of the innermost container, which must be a fill.
    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn fill_break(&mut self, soft: bool) {
        let open = self.open.last_mut().expect("a fill break outside a fill");
        assert!(
            matches!(self.elements[open.index as usize], Element::Fill { .. }),
            "a fill break directly inside a container other than a fill"
        );
        if let Some((index, after)) = open.fill_break {
            set_fill_next(&mut self.elements, index, self.position - after);
        }
        if !soft {
            self.position += 1;
        }
        let index = self.elements.len() as u32;
        open.fill_break = Some((index, self.position));
        self.elements.push(Element::FillBreak { soft, next: 0 });
    }

    #[allow(dead_code, reason = "for the construct rules still to come")]
    pub fn start_hug(&mut self) {
        self.start(Element::Hug, true);
    }

    fn start(&mut self, element: Element, hug: bool) -> u32 {
        let index = self.elements.len() as u32;
        self.open.push(Open {
            index,
            start: self.position,
            stop: None,
            forced: false,
            hug,
            fill_break: None,
        });
        self.elements.push(element);
        index
    }

    /// Ends the innermost container.
    pub fn end(&mut self) {
        let open = self.open.pop().expect("an End without a container");
        let end = self.elements.len() as u32;
        let measured = open.stop.unwrap_or(self.position) - open.start;
        let measured = measured.min(u64::from(u32::MAX)) as u32;
        match &mut self.elements[open.index as usize] {
            Element::Group {
                end: e,
                width,
                forced,
                stopped,
            }
            | Element::Fill {
                end: e,
                width,
                forced,
                stopped,
            } => {
                *e = end;
                *width = measured;
                *forced = open.forced;
                *stopped = open.stop.is_some();
            }
            _ => {}
        }
        if let Some((index, after)) = open.fill_break {
            set_fill_next(&mut self.elements, index, self.position - after);
        }
        self.elements.push(Element::End);
    }

    /// Marks the open containers at a forced break: those inside the innermost hug as forced, and
    /// all of them as measured only up to here.
    fn force_break(&mut self) {
        for open in self.open.iter_mut().rev() {
            if open.hug || open.forced {
                break;
            }
            open.forced = true;
        }
        for open in self.open.iter_mut().rev() {
            if open.stop.is_some() {
                break;
            }
            open.stop = Some(self.position);
        }
    }

    /// Closes the document: adds to each group's width the width after it up to the next line
    /// element, in one backward pass. Panics if a container is still open.
    pub fn finish(mut self) -> Document {
        assert!(
            self.open.is_empty(),
            "{} containers still open",
            self.open.len()
        );
        // The width from the current element to the next line element, scanning backward, and
        // its value after each End not yet matched with its start.
        let mut rest: u64 = 0;
        let mut after_end: Vec<u64> = Vec::new();
        for element in self.elements.iter_mut().rev() {
            match element {
                Element::Token { width, .. } => rest += u64::from(*width),
                Element::Space => rest += 1,
                Element::Line
                | Element::SoftLine
                | Element::HardLine
                | Element::BlankLine
                | Element::FillBreak { .. } => rest = 0,
                Element::Verbatim(index) => {
                    let verbatim = &self.verbatims[*index as usize];
                    let first = u64::from(verbatim.first_width);
                    rest = if verbatim.breaks { first } else { rest + first };
                }
                Element::End => after_end.push(rest),
                Element::Group { width, stopped, .. } | Element::Fill { width, stopped, .. } => {
                    let after = after_end.pop().expect("a container without an End");
                    if !*stopped {
                        *width = (u64::from(*width) + after).min(u64::from(u32::MAX)) as u32;
                    }
                }
                Element::Indent(_) | Element::IndentIfBroken { .. } | Element::Hug => {
                    after_end.pop().expect("a container without an End");
                }
            }
        }
        Document {
            elements: self.elements,
            verbatims: self.verbatims,
        }
    }
}

fn set_fill_next(elements: &mut [Element], index: u32, next: u64) {
    if let Element::FillBreak { next: n, .. } = &mut elements[index as usize] {
        *n = next.min(u64::from(u32::MAX)) as u32;
    }
}

#[cfg(test)]
mod tests;
