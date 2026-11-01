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
    /// Its contents, with the line breaks in them indented by the kind's width. A line starts
    /// at the indentation in effect at its line break, so a line break before the indent and
    /// text after its start leave the text's line unindented.
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
    /// A hug started with [`DocumentBuilder::start_hug_in_group`] spares only the innermost
    /// group around it; the printer treats both kinds alike.
    Hug,
    /// Ends the innermost container.
    End,
    /// Right after a group's start, an alternative to breaking the group, for an item hugged as
    /// a call's only argument (`foo(x ->` then the body at +4). The group's mark, after its
    /// first line break, splits it in two: `tail`, the width from the mark to the group's end
    /// and the rest of the line, and `head`, the width from the group's start to the first line
    /// element after the mark. When the group doesn't fit on the line, and the tail doesn't fit
    /// on a line of its own at +4 either, the group stays flat if its head fits, and the groups
    /// in the hug inside it break instead. Elsewhere it prints nothing.
    Alternative { head: u32, tail: u32 },
    /// Right after a group's start: the group breaks if the group starting at index `group`, which
    /// comes before it, broke, and otherwise as usual, if it doesn't fit. A method's `throws`
    /// stays on the line of its parameters when they all moved to the next line together, and
    /// goes on a line of its own when they went one per line (observed in GJF).
    BreaksWith { group: u32 },
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

/// Which groups a hug keeps from being forced by the forced breaks in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HugKind {
    /// Every group around it.
    All,
    /// Only the innermost group or fill around it.
    InGroup,
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
    hug: Option<HugKind>,
    /// Whether it is a group or a fill.
    group: bool,
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
    /// The `Alternative` elements' indices and the indices of their marks, whose widths
    /// [`DocumentBuilder::finish`] sets.
    alternatives: Vec<(u32, u32)>,
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

    pub fn line(&mut self) {
        self.position += 1;
        self.elements.push(Element::Line);
    }

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

    /// Turns the last element into a line break if it's a blank line. Both force their groups to
    /// break, so no width changes.
    pub fn demote_blank_line(&mut self) {
        if let Some(last @ Element::BlankLine) = self.elements.last_mut() {
            *last = Element::HardLine;
        }
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
    pub fn start_group(&mut self) -> u32 {
        self.start(
            Element::Group {
                end: 0,
                width: 0,
                forced: false,
                stopped: false,
            },
            None,
        )
    }

    /// Starts a group with an [`Element::Alternative`], whose mark is set with
    /// [`DocumentBuilder::alternative_mark`].
    pub fn start_group_with_alternative(&mut self) -> u32 {
        let group = self.start_group();
        let index = self.elements.len() as u32;
        self.alternatives.push((index, index + 1));
        self.elements
            .push(Element::Alternative { head: 0, tail: 0 });
        group
    }

    /// Starts a group that also breaks if `group`, a group that started before it, broke (see
    /// [`Element::BreaksWith`]).
    pub fn start_group_breaking_with(&mut self, group: u32) -> u32 {
        debug_assert!(
            matches!(self.elements[group as usize], Element::Group { .. }),
            "BreaksWith names element {group}, which isn't a group"
        );
        let index = self.start_group();
        self.elements.push(Element::BreaksWith { group });
        index
    }

    /// Sets the mark of `group`'s alternative here. Until [`DocumentBuilder::finish`], the
    /// alternative's `tail` holds the mark's offset from the group's start.
    pub fn alternative_mark(&mut self, group: u32) {
        let offset = self.offset_in(group);
        let mark = self.elements.len() as u32;
        let index = group + 1;
        if let Element::Alternative { tail, .. } = &mut self.elements[index as usize] {
            *tail = offset;
        }
        if let Some(entry) = self
            .alternatives
            .iter_mut()
            .rev()
            .find(|(i, _)| *i == index)
        {
            entry.1 = mark;
        }
    }

    /// The flat width from the start of `group`, which is open, to here.
    fn offset_in(&self, group: u32) -> u32 {
        let open = self
            .open
            .iter()
            .rev()
            .find(|open| open.index == group)
            .expect("an alternative's group is open");
        (self.position - open.start).min(u64::from(u32::MAX)) as u32
    }

    pub fn start_indent(&mut self, kind: IndentKind) {
        self.start(Element::Indent(kind), None);
    }

    pub fn start_indent_if_broken(&mut self, group: u32) {
        debug_assert!(
            matches!(self.elements[group as usize], Element::Group { .. }),
            "IndentIfBroken names element {group}, which isn't a group"
        );
        self.start(Element::IndentIfBroken { group }, None);
    }

    pub fn start_fill(&mut self) {
        self.start(
            Element::Fill {
                end: 0,
                width: 0,
                forced: false,
                stopped: false,
            },
            None,
        );
    }

    /// A separator between two items of the innermost fill, which may be inside indents in the
    /// fill but no other container, so that an item can be indented from its separator on.
    pub fn fill_break(&mut self, soft: bool) {
        let elements = &self.elements;
        let open = self
            .open
            .iter_mut()
            .rev()
            .find(|open| !matches!(elements[open.index as usize], Element::Indent(_)))
            .expect("a fill break outside a fill");
        assert!(
            matches!(self.elements[open.index as usize], Element::Fill { .. }),
            "a fill break inside a container other than a fill or an indent"
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

    pub fn start_hug(&mut self) {
        self.start(Element::Hug, Some(HugKind::All));
    }

    /// Starts a hug whose forced breaks don't force the innermost group or fill around it, but
    /// do force the groups around that one. GJF hugs a lambda's block to the call's line this
    /// way: the call's arguments stay flat, but `x = foo(a -> {` still breaks after the `=`.
    pub fn start_hug_in_group(&mut self) {
        self.start(Element::Hug, Some(HugKind::InGroup));
    }

    fn start(&mut self, element: Element, hug: Option<HugKind>) -> u32 {
        let index = self.elements.len() as u32;
        self.open.push(Open {
            index,
            start: self.position,
            stop: None,
            forced: false,
            hug,
            group: matches!(element, Element::Group { .. } | Element::Fill { .. }),
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
    /// those outside the group around a hug started with [`DocumentBuilder::start_hug_in_group`],
    /// and all of them as measured only up to here. The marking stops at a container already
    /// marked, since the ones around it were marked with it, and the group a hug spares is
    /// skipped each time, so a break costs at most the containers between the hug and it.
    fn force_break(&mut self) {
        let mut sparing = false;
        for open in self.open.iter_mut().rev() {
            if sparing {
                sparing = !open.group;
                continue;
            }
            match open.hug {
                Some(HugKind::All) => break,
                Some(HugKind::InGroup) => {
                    sparing = true;
                    continue;
                }
                None => {}
            }
            if open.forced {
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
    /// element, and the same to the next item of each fill's last separator, so that the last
    /// item fits only if what follows the fill on its line does too. One backward pass. Panics if
    /// a container is still open.
    pub fn finish(mut self) -> Document {
        assert!(
            self.open.is_empty(),
            "{} containers still open",
            self.open.len()
        );
        // The width from the current element to the next line element, scanning backward, and,
        // for each End not yet matched with its start, the width after it and the last fill
        // separator inside its container, directly or in indents.
        let mut rest: u64 = 0;
        let mut after_end: Vec<(u64, Option<usize>)> = Vec::new();
        // The alternatives by their marks, last first, and the rest of the line at each mark.
        self.alternatives
            .sort_unstable_by_key(|&(_, mark)| std::cmp::Reverse(mark));
        let mut at_marks = vec![0u64; self.alternatives.len()];
        let mut next_mark = 0;
        for index in (0..self.elements.len()).rev() {
            match &mut self.elements[index] {
                Element::Token { width, .. } => rest += u64::from(*width),
                Element::Space => rest += 1,
                Element::Line | Element::SoftLine | Element::HardLine | Element::BlankLine => {
                    rest = 0;
                }
                Element::FillBreak { .. } => {
                    rest = 0;
                    if let Some((_, last @ None)) = after_end.last_mut() {
                        *last = Some(index);
                    }
                }
                Element::Verbatim(verbatim) => {
                    let verbatim = &self.verbatims[*verbatim as usize];
                    let first = u64::from(verbatim.first_width);
                    rest = if verbatim.breaks { first } else { rest + first };
                }
                Element::End => after_end.push((rest, None)),
                Element::Group { width, stopped, .. } => {
                    let (after, _) = after_end.pop().expect("a container without an End");
                    if !*stopped {
                        *width = (u64::from(*width) + after).min(u64::from(u32::MAX)) as u32;
                    }
                }
                Element::Fill { width, stopped, .. } => {
                    let (after, last) = after_end.pop().expect("a container without an End");
                    if !*stopped {
                        *width = (u64::from(*width) + after).min(u64::from(u32::MAX)) as u32;
                    }
                    if let Some(last) = last
                        && let Element::FillBreak { next, .. } = &mut self.elements[last]
                    {
                        *next = (u64::from(*next) + after).min(u64::from(u32::MAX)) as u32;
                    }
                }
                Element::Indent(_) => {
                    // A fill's last separator may be in an indent in it.
                    let (_, last) = after_end.pop().expect("a container without an End");
                    if let Some(last) = last
                        && let Some((_, outer @ None)) = after_end.last_mut()
                    {
                        *outer = Some(last);
                    }
                }
                Element::IndentIfBroken { .. } | Element::Hug => {
                    after_end.pop().expect("a container without an End");
                }
                Element::Alternative { .. } | Element::BreaksWith { .. } => {}
            }
            while let Some(&(_, mark)) = self.alternatives.get(next_mark)
                && mark as usize >= index
            {
                at_marks[next_mark] = rest;
                next_mark += 1;
            }
        }
        // The head runs from the group's start to the first line element after the mark, and
        // the tail from the mark to where the group's width ends.
        for (&(index, _), &at_mark) in self.alternatives.iter().zip(&at_marks) {
            let Element::Group { width, .. } = self.elements[index as usize - 1] else {
                continue;
            };
            if let Element::Alternative { head, tail } = &mut self.elements[index as usize] {
                let offset = *tail;
                *head = (u64::from(offset) + at_mark).min(u64::from(u32::MAX)) as u32;
                *tail = width.saturating_sub(offset);
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
