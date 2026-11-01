//! `try`, its resources, and its `catch` and `finally` clauses.

use ferrion_syntax::{SyntaxKind as K, SyntaxNode};

use super::{elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;

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
                match elements.peek().map(|e| e.kind()) {
                    Some(K::RParen) | None => f.space(),
                    Some(_) => f.hard_line(),
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

/// `catch (Type name) { … }`.
pub fn catch_clause<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for element in elements(node) {
        if matches!(element.kind(), K::LParen | K::Block) {
            f.space();
        }
        emit(f, element);
    }
}

/// A `catch` parameter: modifiers, the type, and the name, separated by spaces. The type's
/// alternatives (`A | B`) are the type's.
pub fn catch_parameter<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for (i, element) in elements(node).enumerate() {
        if i > 0 {
            f.space();
        }
        emit(f, element);
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
