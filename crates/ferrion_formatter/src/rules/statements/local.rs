//! Local variable declarations, as statements and in `for` headers and `try` resources.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::elements;
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::declarations::local_variable;
use crate::rules::variables::single_declarator;

/// A local variable declaration: modifiers, the type, the declarators, and the `;` of a
/// statement. What is inside a declarator, such as the break after its `=`, is the declarator's.
///
/// - **Modifiers:** [`local_variable`] puts the annotations on lines of their own or keeps them on
///   the line (docs/style.md, "Annotations on declarations").
/// - **One declarator,** in a statement: `Type name` breaks before the name at +4 when it doesn't
///   fit with the rest of its line, up to the `=` or the `;`, as a field's does.
/// - **Several declarators** go one to a line at +4, after their commas, when they don't fit. In
///   a statement, a line break at +4 comes first between the type and the first declarator, when
///   the type and the whole first declarator up to its comma don't fit.
///
/// The breaks after the type aren't made in `for` and `try` headers, where the first declarator
/// stays on the type's line, except before an enhanced `for` variable's name.
pub fn local_var_decl<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let statement = node.last_token().kind() == K::Semi;
    // A for-each variable breaks before its name, as a statement's does (observed in GJF).
    let for_each = node.parent().is_some_and(|p| p.kind() == K::ForEachStmt);
    let elements: Vec<_> = elements(node).collect();
    let (rest, semi) = match elements.as_slice() {
        [rest @ .., semi] if semi.kind() == K::Semi => (rest, Some(*semi)),
        rest => (rest, None),
    };
    let declaration = |f: &mut Formatter<'t>, declaration: &[NodeOrToken<'t>]| match declaration {
        [ty, NodeOrToken::Node(declarator)]
            if (statement || for_each) && declarator.kind() == K::VariableDeclarator =>
        {
            single_declarator(f, *ty, *declarator);
        }
        [ty, declarators @ ..] => {
            super::emit(f, *ty);
            declarators_after_type(f, declarators, statement);
        }
        [] => {}
    };
    match rest {
        [NodeOrToken::Node(modifiers), declarators @ ..] if modifiers.kind() == K::Modifiers => {
            local_variable(f, *modifiers, |f| declaration(f, declarators));
        }
        _ => declaration(f, rest),
    }
    // After the group the modifiers may start, so that a line comment after it doesn't break it.
    if let Some(semi) = semi {
        super::emit(f, semi);
    }
}

/// The declarators after the type, with the commas between them, when there are several or the
/// declaration is in a `for` or `try` header.
fn declarators_after_type<'t>(f: &mut Formatter<'t>, rest: &[NodeOrToken<'t>], statement: bool) {
    if rest.len() <= 1 {
        f.space();
        for &element in rest {
            super::emit(f, element);
        }
        return;
    }
    f.indent(IndentKind::Continuation, |f| {
        f.group(|f| {
            let mut rest = rest.iter().copied().peekable();
            if statement {
                // The break after the type measures the first declarator and its comma, which
                // stays out of the group so that a comment after it doesn't break the type.
                f.group(|f| {
                    f.line();
                    while let Some(element) = rest.next_if(|e| e.kind() != K::Comma) {
                        super::emit(f, element);
                    }
                });
            } else {
                // In a header, the first declarator stays on the type's line.
                f.space();
                if let Some(first) = rest.next() {
                    super::emit(f, first);
                }
            }
            for element in rest {
                if element.kind() == K::VariableDeclarator {
                    f.line();
                }
                super::emit(f, element);
            }
        });
    });
}
