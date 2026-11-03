//! The import section's part of the equivalence and comments checks
//! (docs/design/formatter-completeness.md, "Imports", "Checking it"). The rest of the tree is
//! compared without the imports, which the formatter may sort and remove, and the rest of the
//! comments without the section's, which move with them; both are compared here, without the
//! import plan.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use ferrion_doc_comments::markdown::{Block, Inline as MarkdownInline, LinkTarget};
use ferrion_doc_comments::{
    Inline, InlineTagContent, Reference, TagArgument, parse_javadoc, parse_markdown,
};
use ferrion_lexer::TokenKind;
use ferrion_syntax::comments::{
    CommentMap, DocCommentKind, Element, Placement, can_have_doc_comment, doc_comment,
};
use ferrion_syntax::{NodeId, SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree, TokenId};

use super::{Property, Violation, violation};
use crate::imports::cooked;

/// The nodes of `tree` outside its imports, as the rest of the equivalence check sees them:
/// for each node, its index among them, or `None` for a node in an import.
pub(super) struct Kept {
    /// For each node index, the number of nodes before it outside the imports.
    before: Vec<usize>,
    in_import: Vec<bool>,
}

impl Kept {
    pub(super) fn new(tree: &SyntaxTree) -> Kept {
        let count = tree.node_count();
        let mut in_import = vec![false; count];
        for node in tree.nodes().filter(|n| n.kind() == SyntaxKind::ImportDecl) {
            let subtree = node.subtree();
            in_import[subtree.start.index()..subtree.end.index()].fill(true);
        }
        let mut before = Vec::with_capacity(count + 1);
        let mut kept = 0;
        for &excluded in &in_import {
            before.push(kept);
            kept += usize::from(!excluded);
        }
        before.push(kept);
        Kept { before, in_import }
    }

    /// The index of `node` among the nodes outside the imports, or `None` if it's in one.
    pub(super) fn index(&self, node: NodeId) -> Option<usize> {
        (!self.in_import[node.index()]).then(|| self.before[node.index()])
    }

    /// The number of nodes outside the imports in `nodes`.
    pub(super) fn count(&self, nodes: std::ops::Range<NodeId>) -> usize {
        self.before[nodes.end.index()] - self.before[nodes.start.index()]
    }
}

/// One import, as the checks compare it.
struct Import<'t> {
    node: SyntaxNode<'t>,
    /// Its tokens other than trivia, as written, which the multiset comparison matches.
    tokens: Vec<(SyntaxKind, &'t str)>,
    /// Its name with Unicode escapes translated, `static` or `module` before it if it has one.
    name: String,
    static_or_module: bool,
    /// Whether it is a single-type or single static import.
    single: bool,
}

impl Import<'_> {
    fn simple_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or_default()
    }

    /// The import as written, for messages: its tokens, with a space after each keyword.
    fn written(&self) -> String {
        let mut text = String::new();
        for (i, (_, t)) in self.tokens.iter().enumerate() {
            if i > 0
                && matches!(
                    self.tokens[i - 1].0,
                    SyntaxKind::ImportKw | SyntaxKind::StaticKw | SyntaxKind::ModuleKw
                )
            {
                text.push(' ');
            }
            text.push_str(t);
        }
        text
    }
}

fn imports(tree: &SyntaxTree) -> Vec<Import<'_>> {
    tree.root()
        .children()
        .filter(|n| n.kind() == SyntaxKind::ImportDecl)
        .map(|node| {
            let tokens: Vec<_> = node
                .tokens()
                .filter(|t| !t.is_trivia())
                .map(|t| (t.kind(), t.text()))
                .collect();
            let mut name = String::new();
            let mut static_or_module = false;
            let mut on_demand = false;
            let mut module = false;
            for token in node.tokens().filter(|t| !t.is_trivia()) {
                match token.kind() {
                    SyntaxKind::ImportKw | SyntaxKind::Semi => {}
                    SyntaxKind::StaticKw if name.is_empty() => static_or_module = true,
                    SyntaxKind::ModuleKw if name.is_empty() => {
                        static_or_module = true;
                        module = true;
                    }
                    kind => {
                        on_demand |= kind == SyntaxKind::Star;
                        name.push_str(&cooked(token));
                    }
                }
            }
            Import {
                node,
                tokens,
                name,
                static_or_module,
                single: !on_demand && !module,
            }
        })
        .collect()
}

/// Compares the two trees' import sections as multisets: every import of the output is one of the
/// input's, and every import of the input missing from the output is one the formatter may remove,
/// checked without the plan. A single-type or single static import may go when:
///
/// - no identifier in the output outside the imports that may refer to it, and no reference in
///   an attached doc comment, has its simple name ([`Names`]): that over-approximates its uses, so a
///   removal the plan got wrong can't pass;
/// - an import with the same name is still there (a duplicate);
/// - it is a single-type import of a type of the file's own package.
pub(super) fn compare(
    before: &SyntaxTree,
    after: &SyntaxTree,
    bom: usize,
) -> Result<(), Violation> {
    let (a, b) = (imports(before), imports(after));
    let mut left: HashMap<&[(SyntaxKind, &str)], usize> = HashMap::new();
    for import in &a {
        *left.entry(&import.tokens).or_default() += 1;
    }
    for import in &b {
        match left.get_mut(import.tokens.as_slice()) {
            Some(n) if *n > 0 => *n -= 1,
            _ => {
                return Err(violation(
                    Property::Equivalence,
                    Some(bom + import.node.span().start as usize),
                    format!("{} is in the output but not in the input", import.written()),
                ));
            }
        }
    }
    let package = package_name(after);
    let mut names: Option<Names> = None;
    for import in &a {
        let Some(n) = left.get_mut(import.tokens.as_slice()) else {
            continue;
        };
        if *n == 0 {
            continue;
        }
        *n -= 1;
        let mut reason = || -> Result<(), &'static str> {
            if !import.single {
                return Err("only single-type and single static imports are removed");
            }
            let duplicate = b
                .iter()
                .any(|i| i.static_or_module == import.static_or_module && i.name == import.name);
            let own_package = !import.static_or_module
                && package
                    .as_deref()
                    .is_some_and(|p| import.name.rsplit_once('.').is_some_and(|(q, _)| q == p));
            if duplicate || own_package {
                return Ok(());
            }
            let names = names.get_or_insert_with(|| Names::new(after));
            if names.identifiers.contains(import.simple_name()) {
                Err("an identifier in the output has its simple name")
            } else if names.doc_references.contains(import.simple_name()) {
                Err("a doc comment in the output names it")
            } else {
                Ok(())
            }
        };
        if let Err(why) = reason() {
            return Err(violation(
                Property::Equivalence,
                None,
                format!("{} was removed, but {why}", import.written()),
            ));
        }
    }
    Ok(())
}

/// The output's names that could be uses of an import, found without the plan's rules and
/// counting more than they do: its identifiers outside the imports other than declared names,
/// labels, and names after `.` or `::`, which never refer to an import; and the first segment of
/// every type in every reference of every doc comment javadoc reads: the one javac attaches to
/// each declaration (`ferrion_syntax::comments::doc_comment`).
struct Names {
    identifiers: HashSet<String>,
    doc_references: HashSet<String>,
}

impl Names {
    fn new(tree: &SyntaxTree) -> Names {
        let mut identifiers = HashSet::new();
        // The imports' tokens, which come in order, as the tokens do.
        let mut imports = tree
            .root()
            .children()
            .filter(|n| n.kind() == SyntaxKind::ImportDecl)
            .map(|n| n.token_range())
            .peekable();
        for token in tree.tokens() {
            while imports.next_if(|r| r.end <= token.id()).is_some() {}
            let in_import = imports.peek().is_some_and(|r| r.start <= token.id());
            if token.lexer_kind() == TokenKind::Identifier && !in_import && may_refer(token) {
                identifiers.insert(cooked(token).into_owned());
            }
        }
        let mut doc_references = HashSet::new();
        let mut found = |reference: &Reference| {
            for ty in reference.types() {
                if let Some(first) = ty.name.segments.first() {
                    doc_references.insert(first.text.clone());
                }
            }
        };
        let mut seen = HashSet::new();
        for node in tree.nodes().filter(|n| can_have_doc_comment(n.kind())) {
            let Some(doc) = doc_comment(node) else {
                continue;
            };
            if !seen.insert(doc.first_token().id()) {
                continue;
            }
            match doc.kind() {
                DocCommentKind::Javadoc => {
                    let javadoc = parse_javadoc(tree.text(), doc.span());
                    javadoc_inlines(&javadoc.description, &mut found);
                    for tag in &javadoc.tags {
                        tag_argument(tag.argument.as_ref(), &mut found);
                        javadoc_inlines(&tag.body, &mut found);
                    }
                }
                DocCommentKind::Markdown => {
                    let lines: Vec<_> = doc.tokens().map(|t| t.span()).collect();
                    let markdown = parse_markdown(tree.text(), &lines);
                    markdown_blocks(&markdown.description, &mut found);
                    for tag in &markdown.tags {
                        tag_argument(tag.argument.as_ref(), &mut found);
                        markdown_blocks(&tag.body, &mut found);
                    }
                }
            }
        }
        Names {
            identifiers,
            doc_references,
        }
    }
}

/// Whether `token`, an identifier, may refer to an imported type or member: not a declaration's
/// own name, a label, or a name after `.` or `::`.
fn may_refer(token: SyntaxToken<'_>) -> bool {
    let declared = matches!(
        token.parent().kind(),
        SyntaxKind::ClassDecl
            | SyntaxKind::InterfaceDecl
            | SyntaxKind::EnumDecl
            | SyntaxKind::RecordDecl
            | SyntaxKind::AnnotationTypeDecl
            | SyntaxKind::MethodDecl
            | SyntaxKind::ConstructorDecl
            | SyntaxKind::CompactConstructorDecl
            | SyntaxKind::VariableDeclarator
            | SyntaxKind::FormalParameter
            | SyntaxKind::CatchParameter
            | SyntaxKind::TypeParameter
            | SyntaxKind::LambdaParameter
            | SyntaxKind::EnumConstant
            | SyntaxKind::RecordComponent
            | SyntaxKind::TypePattern
            | SyntaxKind::LabeledStmt
            | SyntaxKind::BreakStmt
            | SyntaxKind::ContinueStmt
    );
    !declared
        && token
            .prev_non_trivia()
            .is_none_or(|t| !matches!(t.kind(), SyntaxKind::Dot | SyntaxKind::ColonColon))
}

type Found<'a> = dyn FnMut(&Reference) + 'a;

fn tag_argument(argument: Option<&TagArgument>, found: &mut Found<'_>) {
    match argument {
        Some(TagArgument::Reference(reference)) => found(reference),
        Some(TagArgument::SerialField { ty: Some(ty), .. }) => found(&Reference {
            span: ty.span,
            text: String::new(),
            module: None,
            qualifier: Some(ty.clone()),
            member: None,
            fragment: None,
        }),
        _ => {}
    }
}

/// Every reference of an inline tag, and of its descriptive content through `inner`.
fn tag<D>(content: &InlineTagContent<D>, found: &mut Found<'_>, inner: fn(&D, &mut Found<'_>)) {
    match content {
        InlineTagContent::Link { reference, label }
        | InlineTagContent::LinkPlain { reference, label } => {
            if let Some(reference) = reference {
                found(reference);
            }
            inner(label, found);
        }
        // `{@value X}` without a member names a field, not a type (the design, "Removal").
        InlineTagContent::Value {
            reference: Some(reference),
            ..
        } if reference.member.is_some() => found(reference),
        InlineTagContent::InheritDoc {
            supertype: Some(reference),
        } => found(reference),
        InlineTagContent::Index { description, .. }
        | InlineTagContent::Return(description)
        | InlineTagContent::Summary(description) => inner(description, found),
        _ => {}
    }
}

fn javadoc_inlines(inlines: &[Inline], found: &mut Found<'_>) {
    for inline in inlines {
        if let Inline::Tag(t) = inline {
            tag(&t.content, found, |d, found| javadoc_inlines(d, found));
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
                for cell in std::iter::once(head).chain(rows).flat_map(|r| &r.cells) {
                    markdown_inlines(&cell.content, found);
                }
            }
            _ => {}
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
            MarkdownInline::Tag(t) => tag(&t.content, found, |d, found| markdown_blocks(d, found)),
            _ => {}
        }
    }
}

/// The file's package name, with Unicode escapes translated.
fn package_name(tree: &SyntaxTree) -> Option<String> {
    let package = tree
        .root()
        .children()
        .find(|n| n.kind() == SyntaxKind::PackageDecl)?;
    let name = package.children().find(|n| n.kind() == SyntaxKind::Name)?;
    Some(
        name.tokens()
            .filter(|t| !t.is_trivia())
            .map(|t| cooked(t).into_owned())
            .collect(),
    )
}

/// The tokens, by index, whose comments the import section may move: from after the package
/// declaration (or the start of the file) to the first token of code after the imports. Only
/// for a file with imports ([`has_imports`]): one without has its comments compared in order.
pub(super) fn comment_region(tree: &SyntaxTree) -> Range<usize> {
    let root = tree.root();
    let package = root
        .children()
        .find(|n| n.kind() == SyntaxKind::PackageDecl);
    let start = package.map_or(0, |p| p.last_token().id().index() + 1);
    let after = root
        .children()
        .filter(|n| n.kind() == SyntaxKind::ImportDecl)
        .last()
        .or(package)
        .map(|n| n.last_token());
    let end = match after {
        Some(token) => token
            .next_non_trivia()
            .map_or(tree.token_count(), |t| t.id().index()),
        None => root
            .tokens()
            .find(|t| !t.is_trivia())
            .map_or(tree.token_count(), |t| t.id().index()),
    };
    start..end.max(start)
}

/// Whether `tree` has imports, so that its import section's comments are compared apart.
pub(super) fn has_imports(tree: &SyntaxTree) -> bool {
    tree.root()
        .children()
        .any(|n| n.kind() == SyntaxKind::ImportDecl)
}

/// A comment of the import section's region, with its text as the comments check compares it.
pub(super) struct SectionComment {
    pub(super) token: TokenId,
    pub(super) kind: SyntaxKind,
    pub(super) text: String,
    pub(super) start: usize,
}

/// Compares the comments of the two import sections' regions ([`comment_region`]): each comment
/// is in the output once, and each comment that belongs to an import in the input (inside it,
/// trailing it, or directly above it with no blank line, except above the first import) is with
/// an import with the same tokens in the output.
pub(super) fn compare_comments(
    before: &SyntaxTree,
    after: &SyntaxTree,
    a: &[SectionComment],
    b: &[SectionComment],
    bom: usize,
) -> Result<(), Violation> {
    let mut left: HashMap<(SyntaxKind, &str), usize> = HashMap::new();
    for comment in a {
        *left
            .entry((comment.kind, comment.text.as_str()))
            .or_default() += 1;
    }
    for comment in b {
        match left.get_mut(&(comment.kind, comment.text.as_str())) {
            Some(n) if *n > 0 => *n -= 1,
            _ => {
                return Err(violation(
                    Property::Comments,
                    Some(bom + comment.start),
                    format!(
                        "the import section's comment {:?} isn't in the input, or is twice in \
                         the output",
                        comment.text
                    ),
                ));
            }
        }
    }
    if let Some(((_, text), _)) = left.iter().find(|(_, n)| **n > 0) {
        return Err(violation(
            Property::Comments,
            None,
            format!("the import section's comment {text:?} is missing from the output"),
        ));
    }
    if a.is_empty() {
        return Ok(());
    }
    let texts = |list: &[SectionComment]| -> HashMap<TokenId, String> {
        list.iter().map(|c| (c.token, c.text.clone())).collect()
    };
    let (texts_a, texts_b) = (texts(a), texts(b));
    let text = |texts: &HashMap<TokenId, String>, token: TokenId| -> String {
        texts.get(&token).cloned().unwrap_or_default()
    };
    let (map_a, map_b) = (CommentMap::new(before), CommentMap::new(after));
    let (imports_a, imports_b) = (imports(before), imports(after));
    let mut used = vec![false; imports_b.len()];
    for (k, import) in imports_a.iter().enumerate() {
        let mut owned: Vec<String> = belonging(&map_a, import.node, k > 0)
            .into_iter()
            .map(|t| text(&texts_a, t))
            .collect();
        if owned.is_empty() {
            continue;
        }
        owned.sort();
        let found = imports_b.iter().enumerate().position(|(j, other)| {
            !used[j] && other.tokens == import.tokens && {
                let mut with: Vec<String> = belonging(&map_b, other.node, true)
                    .into_iter()
                    .map(|t| text(&texts_b, t))
                    .collect();
                with.sort();
                contains(&with, &owned)
            }
        });
        match found {
            Some(j) => used[j] = true,
            None => {
                return Err(violation(
                    Property::Comments,
                    None,
                    format!(
                        "the comments of {} ({}) aren't with it in the output",
                        import.written(),
                        owned.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Whether sorted `outer` holds every element of sorted `inner`, as multisets.
fn contains(outer: &[String], inner: &[String]) -> bool {
    let mut outer = outer.iter().peekable();
    inner.iter().all(|x| {
        while outer.next_if(|y| *y < x).is_some() {}
        outer.next_if(|y| *y == x).is_some()
    })
}

/// The comments that belong to `import`: inside it, trailing it as the comment map places them,
/// and, with `above`, the ones directly above it with no blank line, back to the code or comment
/// trailing it before.
fn belonging(map: &CommentMap, import: SyntaxNode<'_>, above: bool) -> Vec<TokenId> {
    let list = map.comments();
    let tree = import.tree();
    let first = import.first_token().id();
    let last = import.last_token().id();
    let index = |token: TokenId| list.partition_point(|c| c.token < token);
    let mut owned = Vec::new();
    if above {
        let code = import.first_token().prev_non_trivia().map(|t| t.id());
        let mut i = index(first);
        while i > 0 && {
            let c = list[i - 1];
            code.is_none_or(|code| c.token > code)
                && c.placement != Placement::Trailing
                && c.line_breaks_after < 2
        } {
            i -= 1;
            owned.push(list[i].token);
        }
    }
    let mut i = index(first);
    while let Some(c) = list.get(i) {
        let trailing = c.placement == Placement::Trailing
            && match c.element {
                Element::Token(id) => id == last,
                Element::Node(id) => tree.node(id).last_token().id() == last,
            };
        if c.token > last && !trailing {
            break;
        }
        owned.push(c.token);
        i += 1;
    }
    owned
}
