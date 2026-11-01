//! `try`, its resources, and its `catch` and `finally` clauses.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{elements, emit, line_break};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::types;
use crate::rules::variables::type_then_name;

/// `try (…) { … } catch (…) { … } finally { … }`, each part after a space.
pub fn try_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for (i, element) in elements(node).enumerate() {
        if i > 0 {
            f.space();
        }
        emit(f, element);
    }
}

/// A `try`'s resources: one stays on the `try`'s line, and two or more go one to a line at +4,
/// each after its `;`. A `;` after the last resource is followed by a space.
pub fn resources<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let several = node.children().nth(1).is_some();
    let resources = |f: &mut Formatter<'t>| {
        let mut elements = elements(node).peekable();
        while let Some(element) = elements.next() {
            emit(f, element);
            if element.kind() == K::Semi {
                match elements.peek() {
                    Some(NodeOrToken::Node(next)) => line_break(f, next.first_token(), false),
                    _ => f.space(),
                }
            }
        }
    };
    if several {
        f.indent(IndentKind::Continuation, resources);
    } else {
        resources(f);
    }
}

/// `catch (Type name) { … }`. A parameter whose type isn't a union breaks after the `(` first,
/// at +4, when it doesn't fit; a union stays on the `(`'s line (observed in GJF).
pub fn catch_clause<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::LParen => {
                f.space();
                f.token(token);
            }
            NodeOrToken::Node(parameter) if parameter.kind() == K::CatchParameter => {
                if parameter.children().any(|c| c.kind() == K::UnionType) {
                    f.node(parameter);
                } else {
                    f.comments_before(parameter.first_token().id());
                    f.group(|f| {
                        f.indent(IndentKind::Continuation, |f| {
                            f.soft_line();
                            f.node(parameter);
                        });
                    });
                }
            }
            _ => {
                if element.kind() == K::Block {
                    f.space();
                }
                emit(f, element);
            }
        }
    }
}

/// A `catch` parameter: modifiers, then the type and the name, which breaks before the name at
/// +4 as a variable's does. A union type's alternatives break first (see
/// [`types::union_then_name`]).
pub fn catch_parameter<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let elements: Vec<_> = elements(node).collect();
    let [modifiers @ .., ty, name] = elements.as_slice() else {
        elements.into_iter().for_each(|element| emit(f, element));
        return;
    };
    for &modifiers in modifiers {
        emit(f, modifiers);
        f.space();
    }
    let name = |f: &mut Formatter<'t>| emit(f, *name);
    match ty {
        NodeOrToken::Node(union) if union.kind() == K::UnionType => {
            types::union_then_name(f, *union, name);
        }
        _ => {
            f.comments_before(crate::rules::first_token(*ty).id());
            f.group(|f| type_then_name(f, *ty, name));
        }
    }
}

/// `finally { … }`.
pub fn finally_clause<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for (i, element) in elements(node).enumerate() {
        if i > 0 {
            f.space();
        }
        emit(f, element);
    }
}
