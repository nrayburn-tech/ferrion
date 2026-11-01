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

pub(crate) use annotations::local_variable;
mod members;
mod types;
mod unit;

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::lists::{Shape, delimited};
use super::{children, emit, first_token, last_token};
use crate::document::{LINE_WIDTH, width};
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
        K::FormalParameters | K::RecordHeader => {
            delimited(f, node, Shape::Parameters);
        }
        K::FormalParameter | K::ReceiverParameter | K::RecordComponent => {
            members::parameter(f, node)
        }
        K::DefaultValue => members::default_value(f, node),
        _ => super::generic(f, node),
    }
}

/// The children of `node` that aren't trivia, in order.
fn parts(node: SyntaxNode<'_>) -> Vec<NodeOrToken<'_>> {
    children(node).collect()
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
