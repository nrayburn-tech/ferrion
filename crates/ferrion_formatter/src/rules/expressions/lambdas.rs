//! Lambdas.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{Indentation, elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;

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
                    let indentation = Indentation::start(f, IndentKind::Continuation);
                    f.line();
                    f.node(body);
                    indentation.end(f);
                }
            }
            _ => emit(f, element),
        }
    }
    if !block {
        f.end();
    }
}

/// Whether `lambda`'s body is a block.
pub fn has_block_body(lambda: SyntaxNode<'_>) -> bool {
    lambda
        .children()
        .last()
        .is_some_and(|b| b.kind() == K::Block)
}
