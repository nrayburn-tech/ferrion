//! Rules for expressions: operators, assignment, calls and their arguments, field and array
//! access, lambdas and method references, `?:`, object and array creation, array initializers,
//! casts, `instanceof`, and patterns. [`super::area`] lists the kinds.
//!
//! The layouts are GJF 2.0.0's, as observed (docs/style.md, "Expressions"). Most are a group per
//! construct whose line breaks are indented +4, decided outermost first, so `x = a + b * c`
//! breaks after the `=`, then before every `+`, and only then inside `b * c`.

mod arrays;
mod calls;
pub(crate) mod lambdas;
pub(crate) mod lists;
mod operators;

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::generic;
use crate::document::IndentKind;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    // The comments before an expression go before the groups its rule starts, so that one on
    // a line of its own neither breaks them nor takes their indentation.
    f.comments_before(node.first_token().id());
    match node.kind() {
        K::BinaryExpr => operators::binary(f, node),
        K::ConditionalExpr => operators::conditional(f, node),
        K::AssignExpr => operators::assign(f, node),
        K::CastExpr => operators::cast(f, node),
        K::InstanceofExpr => operators::instanceof(f, node),
        K::MethodCallExpr | K::FieldAccessExpr => calls::chain(f, node),
        K::Arguments => calls::arguments(f, node, calls::Hugs::Lambdas),
        K::NewExpr => calls::new(f, node),
        K::MethodRefExpr => calls::method_ref(f, node),
        K::LambdaExpr => lambdas::lambda(f, node),
        K::ArrayAccessExpr => arrays::access(f, node),
        K::ArrayCreationExpr => arrays::creation(f, node),
        K::DimExpr => arrays::dim_expr(f, node),
        K::ArrayInitializer => arrays::initializer(f, node),
        K::RecordPattern => lists::record_pattern(f, node),
        _ => generic(f, node),
    }
}

/// The children of `node` that aren't trivia, in order.
fn elements<'t>(node: SyntaxNode<'t>) -> impl Iterator<Item = NodeOrToken<'t>> {
    node.children_with_tokens()
        .filter(|e| !matches!(e, NodeOrToken::Token(t) if t.is_trivia()))
}

/// Emits a child of a node: a token through the token helper, a node through its rule.
fn emit<'t>(f: &mut Formatter<'t>, element: NodeOrToken<'t>) {
    match element {
        NodeOrToken::Token(token) => f.token(token),
        NodeOrToken::Node(node) => f.node(node),
    }
}

fn first_token(element: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match element {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.first_token(),
    }
}

fn last_token(element: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match element {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.last_token(),
    }
}

/// Emits `element` after `previous`, with a space between them if the first rule would put one
/// there (see [`super::space_between`]).
fn spaced<'t>(f: &mut Formatter<'t>, previous: Option<NodeOrToken<'t>>, element: NodeOrToken<'t>) {
    if let Some(previous) = previous
        && super::space_between(last_token(previous), first_token(element))
    {
        f.space();
    }
    emit(f, element);
}

/// How many of the expression rules' indents may be open at once. Past it, the rules indent no
/// further. GJF indents each `?:` in an else branch, each cast, and each assignment +4 from the
/// one around it, and the parser takes chains of them at any length, so without a limit the
/// output would grow with the square of the input: 40 GB for an 800 KB chain of `?:`. Past 100
/// indents a line starts beyond column 400 anyway (docs/style.md, "Expressions").
const MAX_INDENTS: u32 = 100;

/// An indent started by an expression rule, or none past [`MAX_INDENTS`]. [`Indentation::end`]
/// ends it and gives its place back.
#[must_use]
struct Indentation {
    started: bool,
}

impl Indentation {
    /// Starts an indent of `kind`, hanging when `hanging` (see
    /// [`crate::document::Element::Hanging`]).
    fn start(f: &mut Formatter<'_>, kind: IndentKind, hanging: bool) -> Indentation {
        let started = f.take_capped_indent(MAX_INDENTS);
        if started {
            if hanging {
                f.start_hanging(kind);
            } else {
                f.start_indent(kind);
            }
        }
        Indentation { started }
    }

    /// Runs `body` indented by +4 if `group` breaks.
    fn if_broken<'t>(f: &mut Formatter<'t>, group: u32, body: impl FnOnce(&mut Formatter<'t>)) {
        if f.take_capped_indent(MAX_INDENTS) {
            f.indent_if_broken(group, body);
            f.give_back_capped_indent();
        } else {
            body(f);
        }
    }

    fn end(self, f: &mut Formatter<'_>) {
        if self.started {
            f.end();
            f.give_back_capped_indent();
        }
    }
}
