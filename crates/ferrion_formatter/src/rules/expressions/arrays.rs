//! Array access, array creation and its dimensions, and array initializers.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::lists::{self, Delimited};
use super::{Indentation, elements, emit, spaced};
use crate::document::IndentKind;
use crate::format::Formatter;

/// `a[i]`: on one line, or a line break after the `[` and the index at +4.
pub fn access<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut indentation = None;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::LBracket && indentation.is_none() => {
                f.start_group();
                f.token(token);
                indentation = Some(Indentation::start(f, IndentKind::Continuation, false));
                f.soft_line();
            }
            NodeOrToken::Token(token) if token.kind() == K::RBracket && indentation.is_some() => {
                f.token(token);
                if let Some(indentation) = indentation.take() {
                    indentation.end(f);
                }
                f.end();
            }
            _ => emit(f, element),
        }
    }
}

/// `new int[n][]`, `new int[] {1, 2}`: the dimensions follow the type, and an initializer
/// follows after a space.
pub fn creation<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut previous = None;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::NewKw => {
                f.token(token);
                f.space();
                previous = None;
                continue;
            }
            NodeOrToken::Node(child) if child.kind() == K::ArrayInitializer => {
                f.space();
                f.node(child);
            }
            _ => spaced(f, previous, element),
        }
        previous = Some(element);
    }
}

/// `[n]`, with any annotations before it followed by a space: `@A [n]`.
pub fn dim_expr<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut annotated = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Node(child) if child.kind() == K::Annotation => {
                if annotated {
                    f.space();
                }
                f.node(child);
                annotated = true;
            }
            NodeOrToken::Token(token) if token.kind() == K::LBracket => {
                if annotated {
                    f.space();
                }
                f.token(token);
            }
            _ => emit(f, element),
        }
    }
}

/// `{a, b}` (see [`lists::delimited`]).
pub fn initializer<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    lists::delimited(f, node, Delimited::Initializer);
}
