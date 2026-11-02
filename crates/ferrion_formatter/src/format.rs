//! The state of one formatting run, and the helpers every rule emits through
//! (docs/design/formatter.md, "Building the document").
//!
//! Rules emit tokens with [`Formatter::token`] and whitespace with the methods named after the
//! document's elements. The token helper is what keeps the safety properties by construction:
//!
//! - **Comments:** before a token, it places every comment the input has before it, and after
//!   the token, the comments that trail it. Comments are visited in source order, each once, so
//!   none is lost, doubled, or moved across code. The whitespace around each comes from the
//!   comment's own line breaks in the input, merged with what the rules ask for (see
//!   [`Formatter::comment`]).
//! - **Order:** debug builds assert that tokens are emitted in source order, each once, and that
//!   no token other than trivia is skipped. Modifier reordering is the one exception, through
//!   [`Formatter::reordered_run`].
//! - **Disable regions:** at a region's `off` comment it emits the region's text as written, and
//!   until the region's `on` comment, tokens, comments, and whitespace emit nothing.

use ferrion_syntax::comments::{Comment, CommentMap, Element as CommentElement, Placement};
use ferrion_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree, TokenId};

use crate::document::{Document, DocumentBuilder, IndentKind, Verbatim, VerbatimKind, width};
use crate::printer::{Lines, trim_end};
use crate::regions::{self, Region};
use crate::rules;

/// What a rule builds the document with.
pub struct Formatter<'t> {
    tree: &'t SyntaxTree,
    comments: CommentMap,
    regions: Vec<Region>,
    next_region: usize,
    doc: DocumentBuilder,
    /// The next comment to place, by its index in `comments.comments()`.
    next_comment: usize,
    /// The last token emitted, or skipped in a region.
    last_token: Option<TokenId>,
    /// The region being copied, if any.
    in_region: Option<Region>,
    /// The line breaks after the last comment that are still to come (see [`Formatter::doc`]):
    /// 0, 1 for a line break, or 2 for a blank line.
    line_owed: u8,
    /// Whether a comment's line break was paid since the last text, so that the next line
    /// element is a line break too (see [`Formatter::line`]).
    comment_break: bool,
    /// The capped indents open now (see [`Formatter::take_capped_indent`]).
    capped_indents: u32,
    /// The type-use annotations the file imports (see [`Formatter::type_use_imports`]).
    type_use_imports: u32,
}

impl<'t> Formatter<'t> {
    pub fn new(tree: &'t SyntaxTree) -> Formatter<'t> {
        let comments = CommentMap::new(tree);
        let regions = regions::find(tree, &comments);
        Formatter {
            tree,
            comments,
            regions,
            next_region: 0,
            doc: DocumentBuilder::new(),
            next_comment: 0,
            last_token: None,
            in_region: None,
            line_owed: 0,
            comment_break: false,
            capped_indents: 0,
            type_use_imports: 0,
        }
    }

    /// Builds the document for the whole tree.
    pub fn format_root(mut self) -> Document {
        self.node(self.tree.root());
        debug_assert_eq!(
            self.next_comment,
            self.comments.comments().len(),
            "a comment wasn't placed"
        );
        debug_assert_eq!(
            self.last_token.map(|t| self.tree.token(t).kind()),
            Some(SyntaxKind::Eof),
            "the rules stopped before the end of the file"
        );
        self.doc.finish()
    }

    /// Formats `node` with its kind's rule.
    pub fn node(&mut self, node: SyntaxNode<'t>) {
        rules::node(self, node);
    }

    /// Emits `token`, the next token of the input that isn't trivia, with the comments before it
    /// and the comments trailing it.
    pub fn token(&mut self, token: SyntaxToken<'t>) {
        let id = token.id();
        self.check_next(id, id);
        self.comments_before(id);
        if self.in_region.is_none() {
            self.text(token);
        }
        self.last_token = Some(id);
        self.trailing_comments(id);
    }

    /// Emits a run of adjacent keyword modifiers, from `first` to `last` in the input with only
    /// whitespace and line breaks between them, in the order of `units`: each unit is one modifier's tokens
    /// (`non-sealed` is three), with a space between units. Only modifier reordering emits out
    /// of order, and only outside disable regions.
    pub fn reordered_run(&mut self, first: TokenId, last: TokenId, units: &[&[SyntaxToken<'t>]]) {
        self.check_next(first, last);
        debug_assert!(
            (first.index()..=last.index())
                .map(|i| self.tree.token(TokenId::from_index(i)))
                .all(|t| matches!(
                    t.kind(),
                    SyntaxKind::Whitespace | SyntaxKind::LineTerminator
                ) || units.iter().any(|unit| unit.contains(&t))),
            "a run holds something other than its modifiers and whitespace"
        );
        self.comments_before(first);
        debug_assert!(self.in_region.is_none(), "a run reordered in a region");
        for (i, unit) in units.iter().enumerate() {
            if i > 0 {
                self.space();
            }
            for &token in *unit {
                self.text(token);
            }
        }
        self.last_token = Some(last);
        self.trailing_comments(last);
    }

    /// Places the comments before `token` that haven't been placed, without emitting the token:
    /// for a rule that wants them inside an indent, such as dangling comments before a `}`.
    pub fn comments_before(&mut self, token: TokenId) {
        while let Some(&comment) = self.comments.comments().get(self.next_comment)
            && comment.token < token
        {
            self.next_comment += 1;
            self.comment(comment);
        }
    }

    /// Takes a place for an indent that only `max` may have open at once, and returns whether
    /// there was one. The expression rules cap their indents this way (docs/style.md,
    /// "Expressions"); [`Formatter::give_back_capped_indent`] returns the place.
    pub fn take_capped_indent(&mut self, max: u32) -> bool {
        let free = self.capped_indents < max;
        if free {
            self.capped_indents += 1;
        }
        free
    }

    /// Returns a place taken with [`Formatter::take_capped_indent`].
    pub fn give_back_capped_indent(&mut self) {
        self.capped_indents -= 1;
    }

    /// Which of the type-use annotations that GJF recognizes by their import the file imports, one
    /// bit each, as the declarations rules record them with
    /// [`Formatter::set_type_use_imports`] at the compilation unit, so that the rules below it
    /// look them up without scanning the imports again.
    pub fn type_use_imports(&self) -> u32 {
        self.type_use_imports
    }

    /// Records the file's [`Formatter::type_use_imports`].
    pub fn set_type_use_imports(&mut self, imports: u32) {
        self.type_use_imports = imports;
    }

    /// Whether the formatter is inside a disable region, copying instead of laying out.
    pub fn in_region(&self) -> bool {
        self.in_region.is_some()
    }

    /// Whether any comment in `node` is a formatter marker.
    pub fn has_marker(&self, node: SyntaxNode<'t>) -> bool {
        node.tokens()
            .any(|t| ferrion_syntax::comments::is_comment(t.kind()) && regions::is_marker(t))
    }

    pub fn space(&mut self) {
        if self.in_region.is_none() {
            self.doc().space();
        }
    }

    /// A `Line`, or a line break if a comment's line break comes right before it. The groups
    /// a rule starts after a comment begin after the comment's line break; when one of them
    /// starts with a line element, that is where the comment's line break goes, as GJF puts it
    /// (`int x = // why` then `1;` at +4), so the line element breaks and its groups with it.
    pub fn line(&mut self) {
        if self.in_region.is_none() {
            self.doc();
            if self.comment_break {
                self.doc.hard_line();
            } else {
                self.doc.line();
            }
        }
    }

    /// A `SoftLine`, or a line break after a comment's, as for [`Formatter::line`].
    pub fn soft_line(&mut self) {
        if self.in_region.is_none() {
            self.doc();
            if self.comment_break {
                self.doc.hard_line();
            } else {
                self.doc.soft_line();
            }
        }
    }

    pub fn hard_line(&mut self) {
        if self.in_region.is_none() {
            self.doc().hard_line();
        }
    }

    pub fn blank_line(&mut self) {
        if self.in_region.is_none() {
            self.doc().blank_line();
        }
    }

    /// Removes a blank line just emitted, or still owed after a comment, as the line break after
    /// a trailing comment with a blank line after it in the input: for a rule that allows no
    /// blank line where the comment is, such as before a body's `}`.
    pub fn no_blank_line(&mut self) {
        if self.in_region.is_none() {
            self.line_owed = self.line_owed.min(1);
            self.doc.demote_blank_line();
        }
    }

    /// A line break before `token`, and a blank line if the input has one there (see
    /// [`Formatter::blank_line_before`]).
    pub fn line_break_before(&mut self, token: SyntaxToken<'t>) {
        if self.blank_line_before(token) {
            self.blank_line();
        } else {
            self.hard_line();
        }
    }

    /// Whether the input has a blank line right after the code before `token`: between the
    /// previous token that isn't trivia and the first comment, or `token` if there's none. Blank
    /// lines between comments, and between the last comment and `token`, are each comment's own
    /// (see [`Formatter::comment`]), so a blank line is kept where it was.
    pub fn blank_line_before(&self, token: SyntaxToken<'t>) -> bool {
        let start = token.prev_non_trivia().map_or(0, |t| t.id().index() + 1);
        let mut breaks = 0;
        for index in start..token.id().index() {
            match self.tree.token(TokenId::from_index(index)).kind() {
                SyntaxKind::Whitespace => {}
                SyntaxKind::LineTerminator => breaks += 1,
                _ => break,
            }
        }
        breaks >= 2
    }

    /// Runs `f` in a group and returns the group, for [`Formatter::indent_if_broken`].
    pub fn group(&mut self, f: impl FnOnce(&mut Self)) -> u32 {
        let group = self.doc().start_group();
        f(self);
        self.doc.end();
        group
    }

    /// Starts a group that the caller ends with [`Formatter::end`]; for an `IndentIfBroken` that
    /// names the group it's in.
    pub fn start_group(&mut self) -> u32 {
        self.doc().start_group()
    }

    /// Runs `f` in a group that also breaks if `group` broke (see
    /// [`crate::document::Element::BreaksWith`]), and returns the group.
    pub fn group_breaking_with(&mut self, group: u32, f: impl FnOnce(&mut Self)) -> u32 {
        let index = self.doc().start_group_breaking_with(group);
        f(self);
        self.doc.end();
        index
    }

    /// Starts a group with an alternative to breaking (see
    /// [`crate::document::Element::Alternative`]), which the caller ends with [`Formatter::end`].
    pub fn start_group_with_alternative(&mut self) -> u32 {
        self.doc().start_group_with_alternative()
    }

    /// Sets the mark of `group`'s alternative here, after its first line break.
    pub fn alternative_mark(&mut self, group: u32) {
        if self.in_region.is_none() {
            self.doc().alternative_mark(group);
        }
    }

    /// Ends the innermost container started with a `start_` method.
    pub fn end(&mut self) {
        self.doc.end();
    }

    pub fn indent(&mut self, kind: IndentKind, f: impl FnOnce(&mut Self)) {
        self.doc().start_indent(kind);
        f(self);
        self.doc.end();
    }

    pub fn start_indent(&mut self, kind: IndentKind) {
        self.doc().start_indent(kind);
    }

    pub fn indent_if_broken(&mut self, group: u32, f: impl FnOnce(&mut Self)) {
        self.doc().start_indent_if_broken(group);
        f(self);
        self.doc.end();
    }

    pub fn fill(&mut self, f: impl FnOnce(&mut Self)) {
        self.doc().start_fill();
        f(self);
        self.doc.end();
    }

    /// A separator between two items of the fill `f` is building.
    pub fn fill_break(&mut self, soft: bool) {
        if self.in_region.is_none() {
            self.doc().fill_break(soft);
        }
    }

    pub fn hug(&mut self, f: impl FnOnce(&mut Self)) {
        self.doc().start_hug();
        f(self);
        self.doc.end();
    }

    /// Runs `f` in a hug that keeps only the innermost group around it from breaking (see
    /// [`DocumentBuilder::start_hug_in_group`]).
    pub fn hug_in_group(&mut self, f: impl FnOnce(&mut Self)) {
        self.doc().start_hug_in_group();
        f(self);
        self.doc.end();
    }

    /// The document, after the line breaks owed to the last comment, if any. They come before
    /// whatever is emitted next, so they land after the containers that end at the comment:
    /// `a == b / 2 // c` then `|| d` breaks the `||` but not the `==` or the `/`, as in GJF, and
    /// a comment on its own line before an expression goes before the expression's groups.
    /// Only ending a container doesn't pay them.
    fn doc(&mut self) -> &mut DocumentBuilder {
        let owed = std::mem::take(&mut self.line_owed);
        if owed > 0 && self.in_region.is_none() {
            self.comment_break = true;
            if owed == 1 {
                self.doc.hard_line();
            } else {
                self.doc.blank_line();
            }
        }
        &mut self.doc
    }

    /// Asserts, in debug builds, that the tokens from `first` to `last` come next: after the last
    /// token emitted, with only trivia skipped between.
    fn check_next(&self, first: TokenId, last: TokenId) {
        if cfg!(debug_assertions) {
            let start = self.last_token.map_or(0, |t| t.index() + 1);
            assert!(
                first.index() >= start && last >= first,
                "{first:?} emitted out of order or twice, after {:?}",
                self.last_token
            );
            if let Some(skipped) = (start..first.index())
                .map(|i| self.tree.token(TokenId::from_index(i)))
                .find(|t| !t.is_trivia())
            {
                panic!("{skipped:?} was skipped");
            }
        }
    }

    /// Places the comments after `token` that trail it, or trail a node it ends.
    fn trailing_comments(&mut self, token: TokenId) {
        while let Some(&comment) = self.comments.comments().get(self.next_comment)
            && comment.placement == Placement::Trailing
            && self.ends_at(comment.element, token)
        {
            self.next_comment += 1;
            self.comment(comment);
        }
    }

    fn ends_at(&self, element: CommentElement, token: TokenId) -> bool {
        match element {
            CommentElement::Token(id) => id == token,
            CommentElement::Node(id) => self.tree.node(id).last_token().id() == token,
        }
    }

    /// Places a comment (docs/design/formatter.md, "Comments"). The comments around one gap
    /// between tokens are placed in order, and the line breaks around each come from the input,
    /// at most one blank line: none before it becomes a space if the input had one there, one a
    /// line break, and more a blank line, and the same after it, where a line comment always
    /// ends its line. They merge with the rules' whitespace, the larger winning, so formatting
    /// the output again asks for the same breaks.
    fn comment(&mut self, comment: Comment) {
        let token = self.tree.token(comment.token);
        if let Some(region) = self.in_region {
            if region.end == Some(comment.token) {
                self.in_region = None;
                self.after_comment(comment, token);
            }
            return;
        }
        self.before_comment(comment, token);
        if let Some(&region) = self.regions.get(self.next_region)
            && region.start == comment.token
        {
            self.next_region += 1;
            self.start_region(region);
            return;
        }
        self.text(token);
        self.after_comment(comment, token);
    }

    fn before_comment(&mut self, comment: Comment, token: SyntaxToken<'t>) {
        match comment.line_breaks_before {
            // A doc comment starts its line, as GJF prints it (observed).
            0 if token.kind() == SyntaxKind::DocComment => self.hard_line(),
            0 => {
                if token
                    .prev_token()
                    .is_some_and(|t| t.kind() == SyntaxKind::Whitespace)
                {
                    self.space();
                }
            }
            1 => self.hard_line(),
            _ => self.blank_line(),
        }
    }

    fn after_comment(&mut self, comment: Comment, token: SyntaxToken<'t>) {
        // A doc comment ends its line, as GJF prints it, and stays next to the code after it: a
        // blank line between them is removed, though not one before another comment (observed).
        let doc_comment = token.kind() == SyntaxKind::DocComment;
        let ends_line = doc_comment
            || matches!(
                token.kind(),
                SyntaxKind::LineComment | SyntaxKind::DocLineComment
            );
        let breaks = if doc_comment && !comment_follows(token) {
            comment.line_breaks_after.min(1)
        } else {
            comment.line_breaks_after
        };
        match breaks {
            0 if ends_line => self.line_owed = 1,
            0 => {
                if token
                    .next_token()
                    .is_some_and(|t| t.kind() == SyntaxKind::Whitespace)
                {
                    self.space();
                }
            }
            1 => self.line_owed = self.line_owed.max(1),
            _ => self.line_owed = 2,
        }
    }

    /// Emits a region's text as written, from its `off` comment to the end of its `on` comment
    /// or of the file, and copies until the `on` comment is reached.
    fn start_region(&mut self, region: Region) {
        let text = self.tree.text();
        let start = self.tree.token(region.start).span().start;
        let end = region
            .end
            .map_or(text.len() as u32, |e| self.tree.token(e).span().end);
        self.doc().verbatim(copied(
            text,
            start,
            end,
            VerbatimKind::Copy {
                to_end: region.end.is_none(),
            },
        ));
        self.comment_break = false;
        self.in_region = Some(region);
    }

    /// Emits a token's text: a text block or a block comment that spans lines as copied text,
    /// and a line comment without its trailing whitespace.
    fn text(&mut self, token: SyntaxToken<'t>) {
        let text = token.text();
        match token.kind() {
            SyntaxKind::TextBlock => {
                let span = token.span();
                let verbatim = copied(
                    self.tree.text(),
                    span.start,
                    span.end,
                    VerbatimKind::Copy { to_end: false },
                );
                self.doc().verbatim(verbatim);
            }
            SyntaxKind::BlockComment | SyntaxKind::DocComment if text.contains(['\n', '\r']) => {
                let span = token.span();
                let verbatim = copied(
                    self.tree.text(),
                    span.start,
                    span.end,
                    VerbatimKind::Comment,
                );
                self.doc().verbatim(verbatim);
            }
            SyntaxKind::LineComment | SyntaxKind::DocLineComment => {
                let trimmed = trim_end(text);
                if adds_space_after_slashes(trimmed) {
                    // The `//`, a space, then the rest of the comment, copied.
                    let start = token.span().start;
                    self.doc().token(token.id(), 2, 2);
                    self.doc.space();
                    self.doc.verbatim(copied(
                        self.tree.text(),
                        start + 2,
                        start + trimmed.len() as u32,
                        VerbatimKind::Copy { to_end: false },
                    ));
                } else {
                    self.doc()
                        .token(token.id(), trimmed.len() as u32, width(trimmed));
                }
            }
            _ if text.is_empty() => return,
            _ => self.doc().token(token.id(), text.len() as u32, width(text)),
        }
        self.comment_break = false;
    }
}

/// Whether a space goes after the `//` of `comment`, a line comment without its trailing
/// whitespace, as GJF puts one (docs/style.md, "General"): when the `//` is followed by text
/// that doesn't start with whitespace or another `/`. GJF leaves IntelliJ's `noinspection` and
/// Eclipse's `$NON-NLS-n$` without one, and Ferrion also leaves its own directives, formatter
/// markers and `ferrion-disable*` comments, as written.
pub(crate) fn adds_space_after_slashes(comment: &str) -> bool {
    let Some(rest) = comment.strip_prefix("//") else {
        return false;
    };
    let Some(first) = rest.chars().next() else {
        return false;
    };
    let nls = rest
        .strip_prefix("$NON-NLS-")
        .is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()));
    !matches!(first, ' ' | '\t' | '\x0c' | '/')
        && !rest.starts_with("noinspection")
        && !nls
        && !rest.starts_with("ferrion-disable")
        && !rest.starts_with("ferrion-enable")
        && !rest.contains(regions::OFF)
        && !rest.contains(regions::ON)
}

/// Whether the next token after `token` other than whitespace and line terminators is a comment.
fn comment_follows(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    while let Some(t) = next
        && matches!(
            t.kind(),
            SyntaxKind::Whitespace | SyntaxKind::LineTerminator
        )
    {
        next = t.next_token();
    }
    next.is_some_and(|t| ferrion_syntax::comments::is_comment(t.kind()))
}

/// Copied text from `start` to `end` in `text`.
fn copied(text: &str, start: u32, end: u32, kind: VerbatimKind) -> Verbatim {
    let copied = &text[start as usize..end as usize];
    let mut lines = Lines::new(copied);
    let first = lines.next().unwrap_or_default();
    let first = match kind {
        VerbatimKind::Comment => trim_end(first),
        VerbatimKind::Copy { .. } => first,
    };
    Verbatim {
        start,
        end,
        kind,
        first_width: width(first),
        breaks: lines.next().is_some(),
    }
}
