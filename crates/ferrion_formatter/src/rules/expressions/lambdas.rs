//! Lambdas.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{Indentation, elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::{self, lists};

/// `params -> body`. A block body follows the arrow after a space, and its lines break as the
/// block's rule says. An expression body is on the arrow's line, or on the next line at +4.
pub fn lambda<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let block = has_block_body(node);
    if !block {
        f.start_group();
    }
    for element in elements(node) {
        match element {
            NodeOrToken::Token(arrow) if arrow.kind() == K::Arrow => {
                f.space();
                f.token(arrow);
            }
            NodeOrToken::Node(body) if body.kind() != K::LambdaParameters => {
                if block {
                    f.space();
                    f.node(body);
                } else {
                    // A group of its own, so that after parameters that went one per line, the
                    // body stays on the arrow's line if it fits there (observed).
                    f.group(|f| {
                        let indentation = Indentation::start(f, IndentKind::Continuation);
                        f.line();
                        f.node(body);
                        indentation.end(f);
                    });
                }
            }
            _ => emit(f, element),
        }
    }
    if !block {
        f.end();
    }
}

/// A lambda's parameters in parentheses: flat if `(params) ->` fits on its line, and otherwise
/// one per line at +4, the first after the `(` (observed). The lambda's own group breaks first,
/// so its body goes to the next line before the parameters break.
pub fn parameters<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts: Vec<_> = elements(node).collect();
    let [open, items @ .., close] = parts.as_slice() else {
        return rules::generic(f, node);
    };
    if open.kind() != K::LParen || close.kind() != K::RParen || items.is_empty() {
        return rules::generic(f, node);
    }
    emit(f, *open);
    f.group(|f| {
        let indentation = Indentation::start(f, IndentKind::Continuation);
        lists::separated(f, items);
        indentation.end(f);
    });
    emit(f, *close);
}

/// Whether `lambda`'s body is a block.
pub fn has_block_body(lambda: SyntaxNode<'_>) -> bool {
    lambda
        .children()
        .last()
        .is_some_and(|b| b.kind() == K::Block)
}
