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
    let name = |f: &mut Formatter<'t>| emit(f, *name);
    let union = ty.as_node().filter(|ty| ty.kind() == K::UnionType);
    // Before a union, annotations with arguments break onto a line of their own when the
    // alternatives do, in their group (observed in GJF).
    if let Some(union) = union
        && modifiers.iter().any(|&m| annotation_with_arguments(m))
    {
        let before = |f: &mut Formatter<'t>| {
            for &modifiers in modifiers {
                emit(f, modifiers);
            }
            f.line();
        };
        types::union_then_name(f, union, Some(before), name);
        return;
    }
    for &modifiers in modifiers {
        emit(f, modifiers);
        f.space();
    }
    match union {
        Some(union) => types::union_then_name(f, union, None::<fn(&mut Formatter<'t>)>, name),
        None => {
            f.comments_before(crate::rules::first_token(*ty).id());
            f.group(|f| type_then_name(f, *ty, name));
        }
    }
}

/// Whether `modifiers`, a `Modifiers` node, has an annotation with arguments before its first
/// keyword modifier: `@A(1)`, but not `@A` or `@A()`.
fn annotation_with_arguments(modifiers: NodeOrToken<'_>) -> bool {
    let Some(modifiers) = modifiers.as_node() else {
        return false;
    };
    modifiers
        .children_with_tokens()
        .filter(|part| !part.as_token().is_some_and(|t| t.is_trivia()))
        .map_while(|part| part.as_node().filter(|n| n.kind() == K::Annotation))
        .any(|annotation| {
            annotation
                .children()
                .filter(|c| c.kind() == K::AnnotationArguments)
                .any(|arguments| {
                    arguments
                        .children_with_tokens()
                        .filter(|part| !part.as_token().is_some_and(|t| t.is_trivia()))
                        .count()
                        > 2
                })
        })
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
