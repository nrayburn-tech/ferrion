//! Lambdas.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{Indentation, elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;

/// `params -> body`. A block body follows the arrow after a space, and its lines break as the
/// block's rule says. An expression body is on the arrow's line, or on the next line at +4.
pub fn lambda<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    lambda_in(f, node, None);
}

/// A lambda with an expression body hugged as a call's only argument: as [`lambda`], with the
/// head of the arguments' alternative ending at the arrow, so that `foo(x ->` can stay on the
/// call's line with the body on the next.
pub fn hugged<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, arguments: u32) {
    lambda_in(f, node, Some(arguments));
}

fn lambda_in<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, arguments: Option<u32>) {
    let block = node.children().last().is_some_and(|b| b.kind() == K::Block);
    if !block {
        f.start_group();
    }
    for element in elements(node) {
        match element {
            NodeOrToken::Token(arrow) if arrow.kind() == K::Arrow => {
                f.space();
                f.token(arrow);
                if let Some(arguments) = arguments {
                    f.alternative_head(arguments);
                }
            }
            NodeOrToken::Node(body) if body.kind() != K::LambdaParameters => {
                if block {
                    f.space();
                    f.node(body);
                } else {
                    let indentation = Indentation::start(f, IndentKind::Continuation, false);
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
