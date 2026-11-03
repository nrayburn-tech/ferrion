//! Range formatting (docs/design/formatter-completeness.md, "Ranges"): the whole tree is
//! formatted, with its error units copied as written, and only the edits the range's units own
//! are kept.
//!
//! **Units** are what GJF's `--lines` formats as a whole (observed): a statement in a block or a
//! `switch` group, a `switch` label or rule, a member of a class or enum body, an enum constant,
//! a top-level declaration, an import, the package declaration, a module directive. The import
//! section is one unit when formatting rewrites it (sorts it or removes imports), so that a range
//! touching it applies the rewrite, which can't be done in part. A unit inside
//! an expression (a statement in a lambda body, a member of an anonymous class) is part of the
//! unit around the expression, so a range in a lambda body formats the whole statement the
//! lambda is in. A unit's own tokens, those not in a unit inside it, can come in several runs:
//! an `if` with blocks is `if (a) {`, `} else {`, and `}`, with the statements between them units
//! of their own. Each run is formatted on its own, as GJF formats a `}` alone, or `} else {`.
//!
//! **The expanded range** is the runs of the tokens the range touches, everything between them,
//! the comments before the first that don't trail the code before them, and the comments after
//! the last that trail it or are on its line (`a(); /* c */ b();` with `a();`, as GJF counts it).
//! A comment the range touches counts with the code it is placed with: a trailing comment with
//! the code before it, any other with the code after it.
//!
//! **The edits kept** are those inside the expanded range (gaps, re-indented comments, and
//! rewrites whose whole span is inside it), the edit of the gap before it, which gives the unit
//! its indentation and the blank lines before it, and the line breaks of the gap after it: GJF
//! normalizes the blank lines after a unit, and the trailing whitespace on its last line, but
//! leaves the indentation of the line after it alone (observed).

use std::cell::OnceCell;
use std::fmt;
use std::ops::Range;

use ferrion_diagnostics::SourceText;
use ferrion_syntax::comments::{CommentMap, Placement, is_comment};
use ferrion_syntax::{
    NodeId, SyntaxKind as K, SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree, TokenId,
};

use crate::edits::{self, Edit};
use crate::format::Formatter;
use crate::printer::trim_end;
use crate::regions::Region;
use crate::{imports, printer, printer_source, regions, rules};

/// Why [`format_range`] can't format a range.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RangeError {
    /// A syntax error affects the range: the error's unit, `error` (bytes of the text), overlaps
    /// the expanded range.
    Affected { error: Range<u32> },
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RangeError::Affected { error } => write!(
                f,
                "a syntax error at bytes {}..{} affects the range",
                error.start, error.end
            ),
        }
    }
}

impl std::error::Error for RangeError {}

/// Formats the part of `tree` in `range` (bytes of `source.text()`, cut at its end), for a tree
/// that may have syntax errors outside it, and returns the edits that do it, in order and not
/// overlapping, as [`crate::format_edits`] returns them.
///
/// The range is expanded to the units its tokens are in, as GJF's `--lines` expands it (see the
/// module's documentation): a statement, a member or the header of one with a body, an import.
/// The edits are those of whole-file formatting inside the expanded range, the gap before it,
/// and the line breaks after it; a rewrite (a reordered modifier run, the import section) is
/// applied only when it is all inside. A range that touches no token, only whitespace, gives no
/// edits.
///
/// Syntax errors don't stop it: each error's unit (`SyntaxTree::error_units`) is copied as
/// written, and the rest of the file is formatted as it would be without the error. If an error
/// unit overlaps the expanded range, it returns [`RangeError::Affected`]; so it does for any
/// range when an error unit holds the start or end of a disable region, or the error is outside
/// every unit. As with [`crate::format_tree`], the caller decides what a version error means:
/// the tree doesn't record them. In a file without errors, the edits are
/// [`crate::format_edits`]'s in the expanded range, with the one after it cut to its line
/// breaks.
pub fn format_range(
    tree: &SyntaxTree,
    source: &SourceText,
    range: Range<u32>,
) -> Result<Vec<Edit>, RangeError> {
    Ranges::new(tree, source).format(range)
}

/// Range formatting for one tree and as many ranges as wanted: what [`format_range`] finds for
/// any range (the comment map, the units, the error units) is found once, and the tree is
/// formatted once, the first time a range needs it. For the range test and the fuzz target.
pub struct Ranges<'t> {
    tree: &'t SyntaxTree,
    source: &'t SourceText,
    comments: CommentMap,
    owners: Owners,
    /// The error units' spans, in order.
    error_units: Vec<Range<u32>>,
    /// An error unit holding the start or end of a disable region, which affects every range.
    pins_region: Option<Range<u32>>,
    /// Whole-file formatting's edits, error units copied.
    edits: OnceCell<Vec<Edit>>,
    /// The imports a rewritten import section keeps, in output order.
    kept_imports: Option<Vec<&'t str>>,
}

impl<'t> Ranges<'t> {
    pub fn new(tree: &'t SyntaxTree, source: &'t SourceText) -> Ranges<'t> {
        debug_assert_eq!(tree.text(), source.text(), "the tree isn't the source's");
        let comments = CommentMap::new(tree);
        let units = tree.error_units();
        let regions = regions::find(tree, &comments);
        let pins_region = units
            .iter()
            .find(|unit| {
                let tokens = unit.token_range();
                regions.iter().any(|r| {
                    tokens.contains(&r.start) || r.end.is_some_and(|e| tokens.contains(&e))
                })
            })
            .map(|unit| span(*unit));
        let section = rewritten_section(tree, &comments, &regions);
        Ranges {
            tree,
            source,
            owners: Owners::new(tree, section.as_ref().map(|(first, _)| *first)),
            kept_imports: section.map(|(_, kept)| kept),
            comments,
            error_units: units.into_iter().map(span).collect(),
            pins_region,
            edits: OnceCell::new(),
        }
    }

    /// [`format_range`] for `range`.
    pub fn format(&self, range: Range<u32>) -> Result<Vec<Edit>, RangeError> {
        let Some(expanded) = self.expand(range) else {
            return Ok(Vec::new());
        };
        if let Some(error) = self.affected(&expanded) {
            return Err(RangeError::Affected { error });
        }
        Ok(keep(self.source.text(), self.edits(), &expanded))
    }

    /// The error unit that keeps `expanded` from being formatted, if any: one overlapping its
    /// units, or one holding a disable region's start or end.
    pub fn affected(&self, expanded: &Expanded) -> Option<Range<u32>> {
        let units = &expanded.units;
        // The error units are in order and don't overlap: the first ending after the units'
        // start is the only one that can overlap them.
        let next = self.error_units.partition_point(|u| u.end <= units.start);
        self.error_units
            .get(next)
            .filter(|u| u.start < units.end)
            .or(self.pins_region.as_ref())
            .cloned()
    }

    /// The text of the imports whole-file formatting keeps, in its order, when it rewrites the
    /// import section: which imports are removed depends on the whole file's code.
    pub fn kept_imports(&self) -> Option<&[&'t str]> {
        self.kept_imports.as_deref()
    }

    /// Whole-file formatting's edits for the tree, error units copied as written.
    pub fn edits(&self) -> &[Edit] {
        self.edits.get_or_init(|| {
            let (tree, source) = (self.tree, self.source);
            let document = Formatter::new(tree).format_root();
            let (output, pieces) =
                printer::print_recorded(&document, &printer_source(tree, source));
            let bom = if source.has_bom() { 3 } else { 0 };
            edits::from_pieces(source.text(), &output, bom, &pieces)
        })
    }

    /// `range` expanded to its units (see the module's documentation), or `None` if it touches
    /// no token.
    pub fn expand(&self, range: Range<u32>) -> Option<Expanded> {
        let (tree, comments) = (self.tree, &self.comments);
        let text_len = tree.text().len() as u32;
        let (start, end) = (range.start.min(text_len), range.end.min(text_len));
        if start >= end {
            return None;
        }
        let mut anchors: Option<(SyntaxToken<'_>, SyntaxToken<'_>)> = None;
        let mut token = tree.token_at_offset(start);
        while let Some(t) = token
            && t.span().start < end
        {
            if let Some(anchor) = anchor(comments, t) {
                let (first, last) = anchors.get_or_insert((anchor, anchor));
                if anchor.id() < first.id() {
                    *first = anchor;
                }
                if anchor.id() > last.id() {
                    *last = anchor;
                }
            }
            token = t.next_token();
        }
        let (first, last) = anchors?;
        let first = self.owners.run_start(first);
        let last = self.owners.run_end(last);
        let trailing = |token: SyntaxToken<'_>| {
            comments
                .get(token.id())
                .is_some_and(|c| c.placement == Placement::Trailing)
        };

        // The comments before the first token that lead it, and those trailing the last.
        let mut units_start = first.span().start;
        let mut t = first.prev_token();
        while let Some(token) = t
            && token.is_trivia()
        {
            if is_comment(token.kind()) {
                if trailing(token) {
                    break;
                }
                units_start = token.span().start;
            }
            t = token.prev_token();
        }
        // A comment on the last line of the units goes with them, as GJF counts it, even one
        // leading code after it on that line (`a(); /* c */ b();`).
        let mut units_end = last.span().end;
        let mut same_line = true;
        let mut t = last.next_token();
        while let Some(token) = t
            && token.is_trivia()
        {
            if token.kind() == K::LineTerminator {
                same_line = false;
            } else if is_comment(token.kind()) {
                if !same_line && !trailing(token) {
                    break;
                }
                units_end = piece_end(token);
                same_line &= !token.text().contains(['\n', '\r']);
            }
            t = token.next_token();
        }

        let before = tree
            .token_at_offset(units_start)
            .and_then(|t| t.prev_token())
            .and_then(piece_before)
            .map_or(0, piece_end);
        let next = tree
            .token_at_offset(units_end)
            .and_then(|t| piece_after(t, units_end));
        Some(Expanded {
            units: units_start..units_end,
            before,
            after: next.map_or(text_len, |t| t.span().start),
            comment_after: next.is_some_and(|t| is_comment(t.kind())),
        })
    }

    /// Every run of a unit's own tokens in the tree, in order (see the module's documentation).
    pub fn runs(&self) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        for token in self.tree.tokens() {
            if token.is_trivia() || token.kind() == K::Eof {
                continue;
            }
            let owner = self.owners.of(token);
            match runs.last_mut() {
                Some(run) if run.owner == owner => run.last = token.id(),
                _ => runs.push(Run {
                    first: token.id(),
                    last: token.id(),
                    owner,
                    kind: self.tree.node(owner).kind(),
                }),
            }
        }
        runs
    }
}

fn span(node: SyntaxNode<'_>) -> Range<u32> {
    let span = node.span();
    span.start..span.end
}

/// A range expanded to its units.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expanded {
    /// From the start of the units' first piece of text (a token or a comment) to the end of
    /// their last.
    pub units: Range<u32>,
    /// The end of the piece before the units, or 0: the gap before them starts here.
    pub before: u32,
    /// The start of the piece after the units, or the end of the text: the gap after them ends
    /// here.
    pub after: u32,
    /// Whether the piece after the units is a comment, which leads the code after it.
    pub comment_after: bool,
}

/// A run of a unit's own tokens, which a range touching any of them formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Run {
    pub first: TokenId,
    pub last: TokenId,
    /// The unit, and its kind: the root's for tokens in no unit.
    pub owner: NodeId,
    pub kind: SyntaxKind,
}

/// The code token a token the range touches stands for: itself for code, the code a comment is
/// placed with, and nothing for whitespace or the end of the file.
fn anchor<'t>(comments: &CommentMap, token: SyntaxToken<'t>) -> Option<SyntaxToken<'t>> {
    match token.kind() {
        K::Whitespace | K::LineTerminator | K::Eof => None,
        kind if is_comment(kind) => {
            let trailing = comments
                .get(token.id())
                .is_some_and(|c| c.placement == Placement::Trailing);
            let next = token.next_non_trivia().filter(|t| t.kind() != K::Eof);
            if trailing {
                token.prev_non_trivia().or(next)
            } else {
                next.or_else(|| token.prev_non_trivia())
            }
        }
        _ => Some(token),
    }
}

/// Where the piece of text `token` is ends: a line comment's trailing whitespace is the gap's
/// after it, which the printer doesn't write.
fn piece_end(token: SyntaxToken<'_>) -> u32 {
    match token.kind() {
        K::LineComment | K::DocLineComment => {
            token.span().start + trim_end(token.text()).len() as u32
        }
        _ => token.span().end,
    }
}

/// `token` or the nearest piece of text before it: a token other than whitespace and line
/// terminators.
fn piece_before(token: SyntaxToken<'_>) -> Option<SyntaxToken<'_>> {
    let mut t = Some(token);
    while let Some(token) = t {
        if !matches!(token.kind(), K::Whitespace | K::LineTerminator) {
            return Some(token);
        }
        t = token.prev_token();
    }
    None
}

/// The first piece of text starting at or after `offset`, from `token` on: a token other than
/// whitespace and line terminators, `Eof` included.
fn piece_after(token: SyntaxToken<'_>, offset: u32) -> Option<SyntaxToken<'_>> {
    let mut t = Some(token);
    while let Some(token) = t {
        if token.span().start >= offset
            && !matches!(token.kind(), K::Whitespace | K::LineTerminator)
        {
            return Some(token);
        }
        t = token.next_token();
    }
    None
}

/// The unit each node's tokens are in: the innermost unit around the node, or the node itself,
/// that isn't inside an expression.
struct Owners {
    owner: Vec<NodeId>,
}

impl Owners {
    /// The owners of `tree`'s nodes, with the imports of `section`, a rewritten import section's
    /// first import, all owned by it.
    fn new(tree: &SyntaxTree, section: Option<NodeId>) -> Owners {
        let count = tree.node_count();
        let mut owner = vec![NodeId::ROOT; count];
        let mut in_expression = vec![false; count];
        for node in tree.nodes() {
            let i = node.id().index();
            let (parent_owner, inside) = match node.parent() {
                Some(parent) => {
                    let p = parent.id().index();
                    (owner[p], in_expression[p] || is_expression(parent.kind()))
                }
                None => (NodeId::ROOT, false),
            };
            in_expression[i] = inside;
            owner[i] = match section {
                Some(first)
                    if node.kind() == K::ImportDecl && node.parent() == Some(tree.root()) =>
                {
                    first
                }
                _ if !inside && is_unit(node) => node.id(),
                _ => parent_owner,
            };
        }
        Owners { owner }
    }

    fn of(&self, token: SyntaxToken<'_>) -> NodeId {
        self.owner[token.parent().id().index()]
    }

    /// The first token of the run of `token`'s unit's own tokens that `token` is in.
    fn run_start<'t>(&self, token: SyntaxToken<'t>) -> SyntaxToken<'t> {
        let owner = self.of(token);
        let mut first = token;
        while let Some(previous) = first.prev_non_trivia()
            && self.of(previous) == owner
        {
            first = previous;
        }
        first
    }

    /// The last token of the run of `token`'s unit's own tokens that `token` is in.
    fn run_end<'t>(&self, token: SyntaxToken<'t>) -> SyntaxToken<'t> {
        let owner = self.of(token);
        let mut last = token;
        while let Some(next) = last.next_non_trivia()
            && next.kind() != K::Eof
            && self.of(next) == owner
        {
            last = next;
        }
        last
    }
}

/// The first import of `tree`'s import section, and the imports kept, when whole-file
/// formatting rewrites the section
/// (sorts it, or removes imports), which makes the section one unit: a range touching any of it
/// formats all of it, so that the rewrite applies (docs/design/formatter-completeness.md,
/// "Ranges"). `None` when the section is emitted as written, as the import rule decides: no
/// imports, a section the plan leaves alone (nothing to change, pinned, or with a syntax error),
/// or one starting in a disable region.
fn rewritten_section<'t>(
    tree: &'t SyntaxTree,
    comments: &CommentMap,
    regions: &[Region],
) -> Option<(NodeId, Vec<&'t str>)> {
    let imports: Vec<SyntaxNode<'t>> = tree
        .root()
        .children()
        .filter(|n| n.kind() == K::ImportDecl)
        .collect();
    let first = imports.first()?.first_token().id();
    if regions
        .iter()
        .any(|r| r.start < first && r.end.is_none_or(|end| end > first))
    {
        return None;
    }
    let plan = imports::plan(tree, comments, regions, &imports)?;
    let kept = plan
        .groups
        .iter()
        .flatten()
        .map(|e| e.import.text())
        .collect();
    Some((imports[0].id(), kept))
}

/// Whether a node of `kind` is an expression, whose statements and members are part of the unit
/// around it.
fn is_expression(kind: SyntaxKind) -> bool {
    kind == K::SwitchExpr || rules::area(kind) == Some(rules::Area::Expressions)
}

/// Whether `node`, outside any expression, is a unit (see the module's documentation).
fn is_unit(node: SyntaxNode<'_>) -> bool {
    let kind = node.kind();
    let Some(parent) = node.parent().map(|p| p.kind()) else {
        return false;
    };
    match kind {
        K::ImportDecl
        | K::PackageDecl
        | K::ModuleDecl
        | K::RequiresDirective
        | K::ExportsDirective
        | K::OpensDirective
        | K::UsesDirective
        | K::ProvidesDirective
        | K::EnumConstant
        | K::SwitchRule => true,
        K::SwitchLabel => parent == K::SwitchGroup,
        K::Error => matches!(
            parent,
            K::Block | K::SwitchGroup | K::ClassBody | K::EnumBody | K::CompilationUnit
        ),
        _ if rules::is_statement(kind) && matches!(parent, K::Block | K::SwitchGroup) => true,
        _ => {
            rules::is_member(kind)
                && matches!(parent, K::ClassBody | K::EnumBody | K::CompilationUnit)
        }
    }
}

/// The edits of `edits`, whole-file formatting's for `text`, that `expanded` keeps (see the
/// module's documentation).
fn keep(text: &str, edits: &[Edit], expanded: &Expanded) -> Vec<Edit> {
    let (units, before, after) = (&expanded.units, expanded.before, expanded.after);
    let first = edits.partition_point(|e| e.range.end < before);
    let mut kept = Vec::new();
    for edit in &edits[first..] {
        let range = &edit.range;
        if range.start > after {
            break;
        }
        // In the gap after the units, an insertion at their end included; inside them, or in
        // the gap before them.
        if range.start >= units.end && range.end <= after {
            kept.extend(line_breaks(
                text,
                units.end..after,
                edit,
                expanded.comment_after,
            ));
        } else if range.start >= before && range.end <= units.end {
            kept.push(edit.clone());
        }
    }
    kept
}

/// The part of `edit`, in the gap `gap` after the expanded range, that the range keeps: the
/// output's line breaks, and the whitespace before them, with the input's indentation of the
/// line after. When either has no line break there, all of the output's gap: code on the line
/// after the units moves up onto it, as GJF moves an enum's `;` up after its last constant, and
/// code on their last line moves to a line of its own (observed). A comment on the line after
/// them, `before` one, stays there, since it belongs to the code after it, whose layout it
/// decides.
fn line_breaks(text: &str, gap: Range<u32>, edit: &Edit, before: bool) -> Option<Edit> {
    let input = &text[gap.start as usize..gap.end as usize];
    let output = [
        &text[gap.start as usize..edit.range.start as usize],
        edit.text.as_str(),
        &text[edit.range.end as usize..gap.end as usize],
    ]
    .concat();
    let after_last_break = |s: &str| s.rfind(['\n', '\r']).map(|i| i + 1);
    let wanted = match (after_last_break(input), after_last_break(&output)) {
        (Some(i), Some(o)) => [&output[..o], &input[i..]].concat(),
        _ if before => return None,
        _ => output,
    };
    edits::gap_edit(input, wanted.as_bytes(), gap.start)
}

#[cfg(test)]
mod tests;
