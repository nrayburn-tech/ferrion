//! The import plan (docs/design/formatter-completeness.md, "Imports"): which imports of the
//! import section stay, in what order, with which comments. It is decided from the input before
//! layout, and only from what a second run sees the same way, so formatting the output plans the
//! same section (SCOPE.md's single pass).
//!
//! - **The section** runs from the first import to the last one and the comments trailing it. It
//!   is rewritten only when nothing pins it: no disable region overlaps it, and it holds no
//!   formatter marker or `ferrion-disable*` comment, since directives never move.
//! - **Comments** belong to an import as SCOPE.md says: the ones inside it, the ones trailing it on
//!   its line (as the comment map places them), and the ones directly above it with no blank line
//!   between. The comments above the first import are outside the section and stay where they
//!   are. Any other comment in the section floats to its top.
//! - **Order:** static imports, then `import module`, then the rest, with a blank line between
//!   groups; within a group by the whole name, Unicode escapes translated, compared by UTF-16 code
//!   unit as Java strings compare, as GJF orders them (observed).
//! - **Removal:** a single-type or single static import goes when its simple name has no use, a
//!   single-type import when it names a type of the file's own package, and any import when it
//!   repeats one kept before it, unless a comment belongs to it. Otherwise wildcard, module, and
//!   `java.lang` imports are never removed (SCOPE.md). Uses are the identifiers
//!   GJF counts and the references in attached doc comments ([`unused`]).

use std::collections::HashSet;
use std::ops::Range;

use ferrion_doc_comments::markdown::{Block, Inline as MarkdownInline, LinkTarget};
use ferrion_doc_comments::{
    Inline, InlineTagContent, Reference, TagArgument, parse_javadoc, parse_markdown,
};
use ferrion_lexer::TokenKind;
use ferrion_syntax::comments::{
    CommentMap, DocCommentKind, Element, Placement, can_have_doc_comment, doc_comment,
};
use ferrion_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, SyntaxTree, TokenId};

use crate::regions::{self, Region};

/// A rewritten import section: what [`crate::format::Formatter::rewritten_imports`] emits in place
/// of the input's.
#[derive(Debug)]
pub struct Rewrite<'t> {
    /// The first import's first token, where the rewrite starts.
    pub first: TokenId,
    /// The section's last token: the last import's last token, or the last comment trailing it.
    pub last: TokenId,
    /// The last import's last token, where the token cursor goes after the section.
    pub last_code: TokenId,
    /// The section's comments, by their index in the comment map: each is emitted once.
    pub comments: Range<usize>,
    /// The number of imports in the section, kept and removed.
    pub imports: usize,
    /// The comments that belong to no import, for the top of the section, in input order.
    pub floating: Vec<Placed>,
    /// The kept imports by group, in output order, with a blank line between groups.
    pub groups: Vec<Vec<Entry<'t>>>,
    pub removed: Vec<SyntaxNode<'t>>,
}

/// A comment of the section, by its index in the comment map, with the line breaks it gets in its
/// new place: 0 for a space, 1 for a line break, 2 for a blank line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub index: usize,
    pub before: u32,
    pub after: u32,
}

/// A kept import, with the comments directly above it. The comments inside it and trailing it are
/// placed by its own rule, as in place.
#[derive(Debug)]
pub struct Entry<'t> {
    pub import: SyntaxNode<'t>,
    pub above: Vec<Placed>,
}

/// The groups of the section, in output order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Group {
    Static,
    Module,
    Other,
}

/// What the plan reads of one import.
struct Import<'t> {
    node: SyntaxNode<'t>,
    group: Group,
    /// The imported name as compared: its dots, its member for a static import, and `.*` on
    /// demand, with Unicode escapes translated.
    name: String,
    /// Whether it may be removed when unused: a single-type or single static import, other than a
    /// `java.lang` one, which SCOPE.md never removes. Any import may go when it is repeated.
    single: bool,
    /// The comments before it, after the previous import's: the floating ones, then from
    /// `above` the ones directly above it. Empty for the first import, whose are outside.
    gap: Range<usize>,
    above: usize,
    /// Its comments, from the ones above it to the last one trailing it.
    owned: Range<usize>,
}

impl Import<'_> {
    /// Its simple name: the type's, or the static member's.
    fn simple_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or_default()
    }

    /// Its name without the simple name.
    fn qualifier(&self) -> &str {
        self.name.rsplit_once('.').map_or("", |(q, _)| q)
    }

    fn has_comments(&self) -> bool {
        !self.owned.is_empty()
    }

    /// Its place in the output relative to `other`'s: by group, then by name, compared by UTF-16
    /// code unit as Java strings compare (observed: GJF puts `a.b\u{1D538}` before `a.b\u{FF5A}`).
    fn order(&self, other: &Import<'_>) -> std::cmp::Ordering {
        self.group
            .cmp(&other.group)
            .then_with(|| self.name.encode_utf16().cmp(other.name.encode_utf16()))
    }
}

/// Plans the import section `imports`, the compilation unit's imports in input order, or returns
/// `None` when it is emitted as written: when nothing would change, or when it's pinned.
pub fn plan<'t>(
    tree: &'t SyntaxTree,
    comments: &CommentMap,
    regions: &[Region],
    imports: &[SyntaxNode<'t>],
) -> Option<Rewrite<'t>> {
    let (first, last) = (imports.first()?, *imports.last()?);
    // A section with a syntax error is emitted as written, as the error's unit is copied, so a
    // range format of a broken file never rewrites it.
    if imports.iter().any(|n| n.contains_error()) {
        return None;
    }
    // Imports split by an empty declaration (`;`), which older releases allow, aren't one
    // section, and sorting across the `;` would move it.
    let unit = first.parent()?;
    if unit
        .children()
        .filter(|n| n.kind() == K::ImportDecl)
        .count()
        != imports.len()
    {
        return None;
    }
    let list = comments.comments();
    let index = |token: TokenId| list.partition_point(|c| c.token < token);
    let mut planned: Vec<Import<'t>> = Vec::with_capacity(imports.len());
    let mut previous_end = 0;
    for (k, &node) in imports.iter().enumerate() {
        let last_token = node.last_token().id();
        let start = index(node.first_token().id());
        let mut end = index(last_token);
        while list.get(end).is_some_and(|c| {
            c.placement == Placement::Trailing && ends_at(tree, c.element, last_token)
        }) {
            end += 1;
        }
        let gap = if k == 0 {
            start..start
        } else {
            previous_end..start
        };
        let mut above = gap.end;
        while above > gap.start && list[above - 1].line_breaks_after < 2 {
            above -= 1;
        }
        let (group, name, single) = name(node);
        planned.push(Import {
            node,
            group,
            name,
            single,
            gap,
            above,
            owned: above..end,
        });
        previous_end = end;
    }
    let section_comments = planned[0].owned.start..previous_end;
    let first_token = first.first_token().id();
    let last_code = last.last_token().id();
    let last_token = if previous_end > index(last_code) {
        list[previous_end - 1].token
    } else {
        last_code
    };
    if pinned(
        tree,
        comments,
        regions,
        first_token,
        last_token,
        &section_comments,
    ) {
        return None;
    }
    // The comment map has a comment after the last import trail it, on a later line, when the
    // file ends there. The import's rule emits it with the import, but it isn't on the import's
    // line, so a second run would see it above the next import: such a section is left as is.
    if planned.iter().any(|i| {
        let last = i.node.last_token();
        list[index(last.id())..i.owned.end]
            .iter()
            .any(|c| !same_line(last, tree.token(c.token)))
    }) {
        return None;
    }

    let kept = kept(tree, comments, &planned);
    let mut order: Vec<usize> = (0..planned.len()).filter(|&k| kept[k]).collect();
    order.sort_by(|&a, &b| planned[a].order(&planned[b]).then(a.cmp(&b)));
    let floating: Vec<usize> = planned.iter().flat_map(|i| i.gap.start..i.above).collect();
    let unchanged = order.len() == planned.len()
        && order.iter().enumerate().all(|(i, &k)| i == k)
        && floating.is_empty()
        && (1..planned.len()).all(|k| {
            blank_before(comments, &planned, k) == (planned[k].group != planned[k - 1].group)
        });
    if unchanged {
        return None;
    }

    // Each floating comment on a line of its own, keeping a blank line only between two that had
    // one, with a blank line after the last. A doc comment that code follows in the output gets
    // one line break after it, as the comment rules give it in place (docs/style.md, "General").
    let next_is_comment = match order.first() {
        Some(&k) => planned[k].above < planned[k].gap.end,
        None => list.get(section_comments.end).is_some_and(|c| {
            last.last_token()
                .next_non_trivia()
                .is_none_or(|t| c.token < t.id())
        }),
    };
    let mut placed_floating: Vec<Placed> = Vec::with_capacity(floating.len());
    for i in &planned {
        for index in i.gap.clone().take(i.above - i.gap.start) {
            let before = if index == i.gap.start {
                1
            } else {
                list[index].line_breaks_before.clamp(1, 2)
            };
            if let Some(previous) = placed_floating.last_mut() {
                previous.after = before;
            }
            placed_floating.push(Placed {
                index,
                before,
                after: 2,
            });
        }
    }
    if let Some(last) = placed_floating.last_mut()
        && !next_is_comment
        && tree.token(list[last.index].token).kind() == K::DocComment
    {
        last.after = 1;
    }
    let mut groups: Vec<Vec<Entry<'t>>> = Vec::new();
    let mut group = None;
    for &k in &order {
        let import = &planned[k];
        let above = (import.above..import.gap.end)
            .map(|index| Placed {
                index,
                before: if index == import.above {
                    1
                } else {
                    list[index].line_breaks_before
                },
                after: list[index].line_breaks_after,
            })
            .collect();
        let entry = Entry {
            import: import.node,
            above,
        };
        match groups.last_mut() {
            Some(last) if group == Some(import.group) => last.push(entry),
            _ => groups.push(vec![entry]),
        }
        group = Some(import.group);
    }
    Some(Rewrite {
        first: first_token,
        last: last_token,
        last_code,
        comments: section_comments,
        imports: planned.len(),
        floating: placed_floating,
        groups,
        removed: (0..planned.len())
            .filter(|&k| !kept[k])
            .map(|k| planned[k].node)
            .collect(),
    })
}

/// An import's group, its name as compared, and whether it may be removed when unused
/// (`Import::single`).
fn name(node: SyntaxNode<'_>) -> (Group, String, bool) {
    let mut group = Group::Other;
    let mut name = String::new();
    let mut on_demand = false;
    for token in node.tokens().filter(|t| !t.is_trivia()) {
        match token.kind() {
            K::ImportKw | K::Semi => {}
            K::StaticKw if name.is_empty() => group = Group::Static,
            K::ModuleKw if name.is_empty() => group = Group::Module,
            kind => {
                on_demand |= kind == K::Star;
                name.push_str(&cooked(token));
            }
        }
    }
    // `java.lang.String`, but not `java.lang.Thread.State` or a static import.
    let java_lang =
        group == Group::Other && name.rsplit_once('.').is_some_and(|(q, _)| q == "java.lang");
    (
        group,
        name,
        !on_demand && group != Group::Module && !java_lang,
    )
}

/// A token's text with Unicode escapes translated, as names compare.
pub(crate) fn cooked(token: SyntaxToken<'_>) -> std::borrow::Cow<'_, str> {
    let tree = token.tree();
    ferrion_lexer::cooked_text(tree.text(), tree.lexer_tokens()[token.id().index()])
}

/// Whether `to` is on the line where `from` ends: no line terminator, and no comment that spans
/// lines, from one to the other.
fn same_line(from: SyntaxToken<'_>, to: SyntaxToken<'_>) -> bool {
    let tree = from.tree();
    (from.id().index()..to.id().index()).all(|i| {
        let token = tree.token(TokenId::from_index(i));
        token.kind() != K::LineTerminator && !token.text().contains(['\n', '\r'])
    })
}

/// Whether `element` ends at `token`.
fn ends_at(tree: &SyntaxTree, element: Element, token: TokenId) -> bool {
    match element {
        Element::Token(id) => id == token,
        Element::Node(id) => tree.node(id).last_token().id() == token,
    }
}

/// Whether the section from `first` to `last` is pinned: a disable region overlaps it, or one of
/// its comments is a formatter marker or a `ferrion-disable*` suppression comment, which never
/// move (SCOPE.md).
fn pinned(
    tree: &SyntaxTree,
    comments: &CommentMap,
    regions: &[Region],
    first: TokenId,
    last: TokenId,
    section: &Range<usize>,
) -> bool {
    regions
        .iter()
        .any(|r| r.start <= last && r.end.is_none_or(|end| end >= first))
        || comments.comments()[section.clone()].iter().any(|c| {
            let token = tree.token(c.token);
            regions::is_marker(token)
                || token.text().contains("ferrion-disable")
                || token.text().contains("ferrion-enable")
        })
}

/// Whether the input has a blank line between import `k - 1` (with the comments trailing it) and
/// import `k` (with the comments above it).
fn blank_before(comments: &CommentMap, planned: &[Import<'_>], k: usize) -> bool {
    let list = comments.comments();
    let (previous, import) = (&planned[k - 1], &planned[k]);
    let breaks = if !import.gap.is_empty() {
        list[import.gap.start].line_breaks_before
    } else if previous.owned.end > previous.above
        && list[previous.owned.end - 1].token > previous.node.last_token().id()
    {
        list[previous.owned.end - 1].line_breaks_after
    } else {
        let mut breaks = 0;
        let mut token = previous.node.last_token().next_token();
        while let Some(t) = token
            && matches!(t.kind(), K::Whitespace | K::LineTerminator)
        {
            breaks += u32::from(t.kind() == K::LineTerminator);
            token = t.next_token();
        }
        breaks
    };
    breaks >= 2
}

/// Which imports stay, by index among `planned`.
///
/// An import with a comment always stays. The first import's comments above it are outside the
/// section, but while they are directly above it and it stays first, nothing comes between them
/// and it in the output either, so they keep it as they would in the second run, which sees them
/// as its own: the first import stays when such comments are above it, no comment floats to the
/// top between them, and it sorts before every other import that stays.
fn kept(tree: &SyntaxTree, comments: &CommentMap, planned: &[Import<'_>]) -> Vec<bool> {
    let package = package_name(planned[0].node);
    // The simple names of the imports that go unless used.
    let mut wanted: HashSet<String> = planned
        .iter()
        .filter(|i| i.single && !i.has_comments())
        .map(|i| i.simple_name().to_owned())
        .collect();
    if !wanted.is_empty() {
        let section =
            planned[0].node.first_token().id()..planned.last().unwrap().node.last_token().id();
        unused(tree, section, &mut wanted);
    }
    let mut kept = vec![true; planned.len()];
    let mut seen: HashSet<(Group, &str)> = HashSet::new();
    for (k, import) in planned.iter().enumerate() {
        let duplicate = seen.contains(&(import.group, import.name.as_str()));
        let own_package = import.group == Group::Other
            && package.as_deref().is_some_and(|p| p == import.qualifier());
        if !import.has_comments()
            && (duplicate
                || import.single && (own_package || wanted.contains(import.simple_name())))
        {
            kept[k] = false;
        } else {
            seen.insert((import.group, import.name.as_str()));
        }
    }
    // The first import's comments above it.
    let first = &planned[0];
    if !kept[0] {
        let list = comments.comments();
        let start = first.owned.start;
        let code = first.node.first_token().prev_non_trivia().map(|t| t.id());
        let above = start > 0 && {
            let c = list[start - 1];
            code.is_none_or(|code| c.token > code)
                && c.placement != Placement::Trailing
                && c.line_breaks_after < 2
        };
        let floating = planned.iter().any(|i| i.above > i.gap.start);
        let sorts_first = planned
            .iter()
            .zip(&kept)
            .all(|(i, &k)| !k || first.order(i).is_le());
        kept[0] = above && !floating && sorts_first;
    }
    kept
}

/// The file's package name, with Unicode escapes translated, if it has one.
fn package_name(import: SyntaxNode<'_>) -> Option<String> {
    let unit = import.parent()?;
    let package = unit.children().find(|n| n.kind() == K::PackageDecl)?;
    let name = package.children().find(|n| n.kind() == K::Name)?;
    Some(
        name.tokens()
            .filter(|t| !t.is_trivia())
            .map(|t| cooked(t).into_owned())
            .collect(),
    )
}

/// Removes from `wanted` each name that has a use in the file outside the imports, the tokens in
/// `section`, as GJF counts uses (observed):
///
/// - an identifier in the code that isn't a declaration's own name, a label, the member after
///   `::`, or a later segment of a qualified name (`Pkg.a.b()` uses `Pkg`, `x.Sel.y()` doesn't use
///   `Sel`). Declarations around it don't matter, so a local variable `Local` keeps `import
///   u.Local`;
/// - in a doc comment attached to a declaration, the first segment of each type in a reference
///   ([`references`]).
///
/// Counting too much only keeps an import, so this is generous where GJF is; `verify` checks each
/// removal independently.
fn unused(tree: &SyntaxTree, section: Range<TokenId>, wanted: &mut HashSet<String>) {
    for token in tree.tokens() {
        if wanted.is_empty() {
            return;
        }
        if token.id() >= section.start && token.id() <= section.end {
            continue;
        }
        if token.lexer_kind() == TokenKind::Identifier && is_use(token) {
            wanted.remove(&*cooked(token));
        }
    }
    let mut seen = HashSet::new();
    for node in tree.nodes() {
        if wanted.is_empty() {
            return;
        }
        if !can_have_doc_comment(node.kind()) {
            continue;
        }
        let Some(doc) = doc_comment(node) else {
            continue;
        };
        if !seen.insert(doc.first_token().id()) {
            continue;
        }
        // A comment that can't name one of them isn't parsed.
        let text = &tree.text()[doc.span().range()];
        if !text.contains("\\u") && !wanted.iter().any(|name| text.contains(name.as_str())) {
            continue;
        }
        references(tree, doc, &mut |reference| {
            for ty in reference.types() {
                if let Some(first) = ty.name.segments.first() {
                    wanted.remove(&first.text);
                }
            }
        });
    }
}

/// Whether `token`, an identifier outside the imports, is a use of the name it spells.
fn is_use(token: SyntaxToken<'_>) -> bool {
    let parent = token.parent().kind();
    let declares = matches!(
        parent,
        K::ClassDecl
            | K::InterfaceDecl
            | K::EnumDecl
            | K::RecordDecl
            | K::AnnotationTypeDecl
            | K::MethodDecl
            | K::ConstructorDecl
            | K::CompactConstructorDecl
            | K::VariableDeclarator
            | K::FormalParameter
            | K::CatchParameter
            | K::TypeParameter
            | K::LambdaParameter
            | K::EnumConstant
            | K::RecordComponent
            | K::TypePattern
            | K::LabeledStmt
            | K::BreakStmt
            | K::ContinueStmt
    );
    if declares {
        return false;
    }
    match token.prev_non_trivia() {
        Some(previous) if matches!(previous.kind(), K::Dot | K::ColonColon) => false,
        // `a.<T>m()`: the method is a later segment too.
        Some(previous)
            if parent == K::MethodCallExpr
                && previous.kind() == K::Gt
                && previous.parent().kind() == K::TypeArguments =>
        {
            previous
                .parent()
                .first_token()
                .prev_non_trivia()
                .is_none_or(|t| t.kind() != K::Dot)
        }
        _ => true,
    }
}

/// Calls `found` with each reference in `doc` that names types an import may be for: `{@link}`,
/// `{@linkplain}`, `{@value}` with a member, and the references of `@see`, `@throws`,
/// `@exception`, `@uses`, and `@provides`, and in Markdown comments, links to program elements.
/// Labels, prose, `{@code}`, `{@literal}`, and snippets don't count.
fn references(
    tree: &SyntaxTree,
    doc: ferrion_syntax::comments::DocComment<'_>,
    found: &mut Found<'_>,
) {
    match doc.kind() {
        DocCommentKind::Javadoc => {
            let javadoc = parse_javadoc(tree.text(), doc.span());
            javadoc_inlines(&javadoc.description, found);
            for tag in &javadoc.tags {
                if let Some(TagArgument::Reference(reference)) = &tag.argument {
                    found(reference);
                }
                javadoc_inlines(&tag.body, found);
            }
        }
        DocCommentKind::Markdown => {
            let lines: Vec<_> = doc.tokens().map(|t| t.span()).collect();
            let markdown = parse_markdown(tree.text(), &lines);
            markdown_blocks(&markdown.description, found);
            for tag in &markdown.tags {
                if let Some(TagArgument::Reference(reference)) = &tag.argument {
                    found(reference);
                }
                markdown_blocks(&tag.body, found);
            }
        }
    }
}

/// What [`references`] calls with each reference it finds.
type Found<'a> = dyn FnMut(&Reference) + 'a;

/// The references of an inline tag's content, and of its descriptive content `D` through
/// `inner`.
fn tag_references<D>(
    content: &InlineTagContent<D>,
    found: &mut Found<'_>,
    inner: fn(&D, &mut Found<'_>),
) {
    match content {
        InlineTagContent::Link { reference, label }
        | InlineTagContent::LinkPlain { reference, label } => {
            if let Some(reference) = reference {
                found(reference);
            }
            inner(label, found);
        }
        InlineTagContent::Value {
            reference: Some(reference),
            ..
        } if reference.member.is_some() => found(reference),
        InlineTagContent::Index { description, .. }
        | InlineTagContent::Return(description)
        | InlineTagContent::Summary(description) => inner(description, found),
        _ => {}
    }
}

fn javadoc_inlines(inlines: &[Inline], found: &mut Found<'_>) {
    for inline in inlines {
        if let Inline::Tag(tag) = inline {
            tag_references(&tag.content, found, |d, found| javadoc_inlines(d, found));
        }
    }
}

fn markdown_blocks(blocks: &[Block], found: &mut Found<'_>) {
    for block in blocks {
        match block {
            Block::Paragraph { content, .. }
            | Block::Heading { content, .. }
            | Block::Html { content, .. } => markdown_inlines(content, found),
            Block::BlockQuote { blocks, .. } => markdown_blocks(blocks, found),
            Block::List { items, .. } => {
                for item in items {
                    markdown_blocks(&item.blocks, found);
                }
            }
            Block::Table { head, rows, .. } => {
                for row in std::iter::once(head).chain(rows) {
                    for cell in &row.cells {
                        markdown_inlines(&cell.content, found);
                    }
                }
            }
            Block::CodeBlock { .. }
            | Block::ThematicBreak { .. }
            | Block::LinkDefinition { .. } => {}
        }
    }
}

fn markdown_inlines(inlines: &[MarkdownInline], found: &mut Found<'_>) {
    for inline in inlines {
        match inline {
            MarkdownInline::Link {
                target, content, ..
            } => {
                if let LinkTarget::Element(reference) = target {
                    found(reference);
                }
                markdown_inlines(content, found);
            }
            MarkdownInline::Emphasis { content, .. }
            | MarkdownInline::Strong { content, .. }
            | MarkdownInline::Image { content, .. } => markdown_inlines(content, found),
            MarkdownInline::Tag(tag) => {
                tag_references(&tag.content, found, |d, found| markdown_blocks(d, found));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
