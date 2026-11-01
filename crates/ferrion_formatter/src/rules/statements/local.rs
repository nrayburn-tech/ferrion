//! Local variable declarations, as statements and in `for` headers and `try` resources.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::elements;
use crate::document::IndentKind;
use crate::format::Formatter;

/// A local variable declaration: modifiers, the type, the declarators, and the `;` of a
/// statement. What is inside a declarator, such as the break after its `=`, is the declarator's.
///
/// - **Annotations:** the modifiers are followed by a line break when they are only annotations
///   and [`annotations_break`] says so (`@SuppressWarnings("unchecked")` above the type), and by
///   a space otherwise.
/// - **Declarators:** when there are several and they don't fit, they go one to a line at +4,
///   after their commas.
/// - **After the type:** in a statement, a line break at +4 between the type and the first
///   declarator, taken when the type and the declarator up to its first possible break don't fit
///   on the line (`=`, or the `;` without an initializer), or, with several declarators, when the
///   type and the whole first declarator up to its comma don't. Not in `for` and `try` headers.
pub fn local_var_decl<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let statement = node.last_token().kind() == K::Semi;
    let declarators = node
        .children()
        .filter(|c| c.kind() == K::VariableDeclarator)
        .count();
    let mut elements = elements(node).peekable();
    let mut typed = false;
    while let Some(element) = elements.next() {
        match element {
            NodeOrToken::Node(modifiers) if modifiers.kind() == K::Modifiers => {
                f.node(modifiers);
                if only_annotations(modifiers) && annotations_break(modifiers) {
                    f.hard_line();
                } else {
                    f.space();
                }
            }
            NodeOrToken::Node(ty) if !typed => {
                typed = true;
                f.node(ty);
                let mut rest = Vec::new();
                while let Some(&element) = elements.peek()
                    && element.kind() != K::Semi
                {
                    elements.next();
                    rest.push(element);
                }
                declarators_after_type(f, &rest, statement, declarators);
            }
            _ => super::emit(f, element),
        }
    }
}

/// The declarators after the type, with the commas between them: `rest`.
fn declarators_after_type<'t>(
    f: &mut Formatter<'t>,
    rest: &[NodeOrToken<'t>],
    statement: bool,
    declarators: usize,
) {
    if declarators <= 1 {
        if statement {
            let after_type = f.group(|f| f.line());
            f.indent_if_broken(after_type, |f| {
                for &element in rest {
                    super::emit(f, element);
                }
            });
        } else {
            f.space();
            for &element in rest {
                super::emit(f, element);
            }
        }
        return;
    }
    f.indent(IndentKind::Continuation, |f| {
        f.group(|f| {
            let mut rest = rest.iter().copied();
            if statement {
                f.group(|f| {
                    f.line();
                    for element in rest.by_ref() {
                        super::emit(f, element);
                        if element.kind() == K::Comma {
                            break;
                        }
                    }
                });
            } else {
                f.space();
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

/// Whether `modifiers`, a `Modifiers` node, holds only annotations.
fn only_annotations(modifiers: SyntaxNode<'_>) -> bool {
    !modifiers.direct_tokens().any(|t| !t.is_trivia())
}

/// Whether the annotations at the start of a local declaration's modifiers go on lines of their
/// own, rather than staying on the line with the rest: when there are two or
/// more annotations in the modifiers, or one with arguments (observed in GJF: `@Nullable String
/// s` stays together, `@SuppressWarnings("unchecked")` goes above).
pub(crate) fn annotations_break(modifiers: SyntaxNode<'_>) -> bool {
    let mut annotations = 0;
    for child in modifiers.children() {
        if child.kind() == K::Annotation {
            annotations += 1;
            if annotations > 1 || child.children().any(|c| c.kind() == K::AnnotationArguments) {
                return true;
            }
        }
    }
    false
}
