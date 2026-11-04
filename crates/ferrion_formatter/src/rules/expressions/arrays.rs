//! Array access, array creation and its dimensions, and array initializers.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{Indentation, elements, emit, spaced};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::lists::{self, Shape};

/// `a[i]`: on one line, or a line break after the `[` and the index at +4.
pub fn access<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut indentation = None;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::LBracket && indentation.is_none() => {
                f.start_group();
                f.token(token);
                indentation = Some(Indentation::start(f, IndentKind::Continuation));
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
/// follows after a space. A dimension with an expression that doesn't fit on the line goes on the
/// next at +4 (observed).
pub fn creation<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut previous = None;
    let mut elements = elements(node).peekable();
    while let Some(element) = elements.next() {
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
            NodeOrToken::Node(child) if child.kind() == K::DimExpr => {
                let indentation = Indentation::start(f, IndentKind::Continuation);
                // An annotated dimension has a space before it: `new int @A [n]`.
                let annotated = |n: SyntaxNode<'t>| n.children().any(|c| c.kind() == K::Annotation);
                f.fill(|f| {
                    f.fill_break(!annotated(child));
                    f.node(child);
                    while let Some(dim) =
                        elements.next_if(|e| e.as_node().is_some_and(|n| n.kind() == K::DimExpr))
                    {
                        f.fill_break(!dim.as_node().is_some_and(annotated));
                        emit(f, dim);
                    }
                });
                indentation.end(f);
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
    lists::delimited(f, node, Shape::Initializer { values: false });
}
