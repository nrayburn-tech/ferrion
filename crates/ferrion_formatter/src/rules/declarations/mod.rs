//! Rules for declarations: the compilation unit, packages and imports, modules and their
//! directives, modifiers and annotations, type declarations and their bodies, members, enum
//! constants, records, and parameters. [`super::area`] lists the kinds, and docs/style.md
//! ("Declarations") describes the style.
//!
//! - [`unit`]: the compilation unit, packages, imports, names, and modules;
//! - [`annotations`]: modifiers, annotations, and their arguments;
//! - [`types`]: type declarations, their clauses and bodies, and enum constants;
//! - [`members`]: fields and variables, methods and constructors, initializers, and parameters.

mod annotations;
mod members;
mod types;
mod unit;

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use crate::document::{IndentKind, LINE_WIDTH, width};
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    match node.kind() {
        K::CompilationUnit => unit::compilation_unit(f, node),
        K::PackageDecl => unit::package(f, node),
        K::ImportDecl | K::Name => unit::tokens(f, node),
        K::ModuleDecl => unit::module(f, node),
        K::RequiresDirective
        | K::ExportsDirective
        | K::OpensDirective
        | K::UsesDirective
        | K::ProvidesDirective => unit::directive(f, node),
        K::Modifiers => annotations::modifiers_node(f, node),
        K::Annotation => annotations::annotation(f, node),
        K::AnnotationArguments => annotations::arguments(f, node),
        K::ElementValuePair => annotations::pair(f, node),
        K::ElementValueArray => annotations::array(f, node),
        K::ClassDecl | K::InterfaceDecl | K::EnumDecl | K::RecordDecl | K::AnnotationTypeDecl => {
            types::type_decl(f, node)
        }
        K::ExtendsClause | K::ImplementsClause | K::PermitsClause => types::clause(f, node),
        K::ClassBody => types::class_body(f, node),
        K::EnumBody => types::enum_body(f, node),
        K::EnumConstant => types::enum_constant(f, node),
        K::FieldDecl => members::field(f, node),
        K::VariableDeclarator => members::declarator(f, node),
        K::MethodDecl | K::ConstructorDecl => members::method(f, node),
        K::CompactConstructorDecl | K::Initializer => members::spaced(f, node),
        K::FormalParameters | K::RecordHeader => delimited_list(f, node, ListLayout::Parameters),
        K::FormalParameter | K::ReceiverParameter | K::RecordComponent => {
            members::parameter(f, node)
        }
        K::DefaultValue => members::default_value(f, node),
        _ => super::generic(f, node),
    }
}

/// The children of `node` that aren't trivia, in order.
fn parts(node: SyntaxNode<'_>) -> Vec<NodeOrToken<'_>> {
    node.children_with_tokens()
        .filter(|part| !matches!(part, NodeOrToken::Token(token) if token.is_trivia()))
        .collect()
}

/// Emits a child: a token, or a node with its kind's rule.
fn emit<'t>(f: &mut Formatter<'t>, part: NodeOrToken<'t>) {
    match part {
        NodeOrToken::Token(token) => f.token(token),
        NodeOrToken::Node(node) => f.node(node),
    }
}

fn first_token(part: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match part {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.first_token(),
    }
}

fn last_token(part: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match part {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.last_token(),
    }
}

/// How a delimited, comma-separated list breaks when it doesn't fit on the line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ListLayout {
    /// All on the next line at +4, or, if they don't fit there either, one per line: a method's
    /// parameters and a record's components.
    Parameters,
    /// One per line at +4: an annotation's arguments.
    OnePerLine,
    /// One per line at +4, even if they fit on the line.
    Broken,
}

/// The rule for a list between delimiters, such as `(a, b)`: the opening delimiter, the items
/// with `, ` between them, and the closing delimiter, all on one line if they fit, otherwise
/// broken by `layout`. An empty list stays together, with its dangling comments inside and a
/// space before the closing delimiter after a block comment, as GJF prints `(/* none */ )`.
///
/// A block comment on a separator's line that leads the next item stays on that line when the
/// list breaks (docs/design/formatter.md, "Comments").
pub(crate) fn delimited_list<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, layout: ListLayout) {
    f.comments_before(node.first_token().id());
    let parts = parts(node);
    let [open, items @ .., close] = parts.as_slice() else {
        parts.into_iter().for_each(|part| emit(f, part));
        return;
    };
    if items.is_empty() {
        emit(f, *open);
        dangling(f, first_token(*close));
        emit(f, *close);
        return;
    }
    f.group(|f| {
        emit(f, *open);
        f.indent(IndentKind::Continuation, |f| {
            let hard = layout == ListLayout::Broken;
            if hard {
                f.hard_line();
            } else {
                f.soft_line();
            }
            let items = |f: &mut Formatter<'t>| separated(f, items, hard);
            match layout {
                ListLayout::Parameters => {
                    f.group(items);
                }
                ListLayout::OnePerLine | ListLayout::Broken => items(f),
            }
        });
    });
    // After the group, so that a line comment after it doesn't break the list.
    emit(f, *close);
}

/// Emits `items`, a list's items and the commas between them, with a `Line` after each comma, or
/// a line break if `hard`. A block comment that leads the next item on the comma's line stays
/// after the comma.
fn separated<'t>(f: &mut Formatter<'t>, items: &[NodeOrToken<'t>], hard: bool) {
    let mut after_comma = false;
    for &part in items {
        if part.kind() == K::Comma {
            emit(f, part);
            after_comma = true;
            continue;
        }
        if after_comma {
            let first = first_token(part);
            if !f.in_region() && leads_on_separator_line(first) {
                f.comments_before(first.id());
            }
            if hard {
                f.hard_line();
            } else {
                f.line();
            }
        }
        emit(f, part);
        after_comma = false;
    }
}

/// Whether the comments before `token` start on the line of the code before them: a block
/// comment after a separator that leads the next item.
fn leads_on_separator_line(token: SyntaxToken<'_>) -> bool {
    let mut previous = token.prev_token();
    let mut comment = false;
    while let Some(t) = previous
        && t.is_trivia()
    {
        match t.kind() {
            K::LineTerminator => comment = false,
            K::BlockComment | K::DocComment => comment = true,
            K::Whitespace => {}
            _ => return false,
        }
        previous = t.prev_token();
    }
    comment
}

/// Places the comments before `close`, the closing delimiter of an empty list, with a space after
/// the last of them if it's a block comment.
fn dangling<'t>(f: &mut Formatter<'t>, close: SyntaxToken<'t>) {
    let Some(previous) = close.prev_token() else {
        return;
    };
    if matches!(previous.kind(), K::BlockComment | K::DocComment) && !f.in_region() {
        f.comments_before(close.id());
        f.space();
    }
}

/// Whether `node`, printed flat with a space where [`super::space_between`] puts one, fits in
/// `budget` columns together with the comments after it on its last line. It looks at no more
/// than `budget` columns of tokens, so it costs a constant, however large the node. Copied text
/// with a line break, or a body that isn't empty, makes it not fit, since they force a break, and
/// so does a comment that ends its line inside the node unless `comments` is false, when it
/// counts for nothing.
pub(crate) fn fits_flat(node: SyntaxNode<'_>, budget: u32, comments: bool) -> bool {
    let last = node.last_token().id();
    let mut used: u32 = 0;
    let mut previous: Option<SyntaxToken<'_>> = None;
    let mut token = Some(node.first_token());
    let mut past_end = false;
    while let Some(t) = token {
        past_end |= t.id() > last;
        match t.kind() {
            K::Whitespace => {}
            K::LineTerminator if past_end => break,
            K::LineTerminator => {}
            K::LineComment | K::DocLineComment if past_end => {
                used += 1 + width(t.text().trim_end());
                return used <= budget;
            }
            K::LineComment | K::DocLineComment | K::DocComment if !comments => {}
            K::LineComment | K::DocLineComment | K::DocComment => return false,
            K::BlockComment => {
                if t.text().contains(['\n', '\r']) {
                    return past_end && used < budget;
                }
                used += 1 + width(t.text());
            }
            _ if past_end => break,
            K::TextBlock => return false,
            K::LBrace
                if matches!(
                    t.parent().kind(),
                    K::Block | K::ClassBody | K::EnumBody | K::SwitchBlock
                ) && t.next_non_trivia().is_some_and(|n| n.kind() != K::RBrace) =>
            {
                return false;
            }
            _ => {
                if let Some(p) = previous
                    && super::space_between(p, t)
                    && !(p.kind() == K::LBrace || t.kind() == K::RBrace)
                {
                    used += 1;
                }
                used += width(t.text());
                previous = Some(t);
            }
        }
        if used > budget {
            return false;
        }
        token = t.next_token();
    }
    true
}

/// The column a declaration starts at, estimated from the bodies and blocks around it: +2 for
/// each, as they are when nothing around them breaks. It looks at a bounded number of ancestors,
/// since past that the line is full anyway.
fn estimated_indent(node: SyntaxNode<'_>) -> u32 {
    let mut indent = 0;
    for ancestor in node.ancestors().skip(1).take(64) {
        if matches!(
            ancestor.kind(),
            K::ClassBody | K::EnumBody | K::Block | K::SwitchBlock | K::SwitchGroup | K::ModuleDecl
        ) {
            indent += 2;
        }
    }
    indent
}

/// Whether a declaration starting at `node` fits on its line, as far as [`estimated_indent`]
/// and [`fits_flat`] can tell before printing, with `comments` as `fits_flat` takes it.
fn fits_on_line(node: SyntaxNode<'_>, comments: bool) -> bool {
    let indent = estimated_indent(node);
    indent < LINE_WIDTH && fits_flat(node, LINE_WIDTH - indent, comments)
}

/// Whether a doc comment is among the comments before `token`, back to the code before them.
fn doc_comment_before(token: SyntaxToken<'_>) -> bool {
    let mut previous = token.prev_token();
    while let Some(t) = previous
        && t.is_trivia()
    {
        if matches!(t.kind(), K::DocComment | K::DocLineComment) {
            return true;
        }
        previous = t.prev_token();
    }
    false
}
