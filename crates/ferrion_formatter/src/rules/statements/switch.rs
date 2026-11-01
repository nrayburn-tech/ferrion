//! `switch` statements and expressions: the block, its rules (`case … ->`) and groups
//! (`case …:` and statements), their labels, and guards.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::{elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;

/// `switch (selector) { … }`.
pub fn switch<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for element in elements(node) {
        if matches!(element.kind(), K::LParen | K::SwitchBlock) {
            f.space();
        }
        emit(f, element);
    }
}

/// A switch block: its rules or groups on their own lines at +2, with blank lines between them
/// kept, at most one, and none after the `{` or before the `}`. Comments before the `}` go at +4,
/// as GJF puts them, and an empty block prints as `{` and `}` on two lines.
pub fn switch_block<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut first = true;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::RBrace => {
                f.indent(IndentKind::Block, |f| {
                    f.indent(IndentKind::Block, |f| f.comments_before(token.id()));
                });
                f.hard_line();
                f.token(token);
            }
            NodeOrToken::Node(item) => {
                f.indent(IndentKind::Block, |f| {
                    if first {
                        f.hard_line();
                    } else {
                        f.line_break_before(item.first_token());
                    }
                    f.node(item);
                });
                first = false;
            }
            NodeOrToken::Token(token) => f.token(token),
        }
    }
}

/// A rule, `case … -> …`. A block follows the `->` after a space. An expression or a `throw`
/// follows after a space if the rule fits on the line, and otherwise starts the next line at +4;
/// that break comes before any break in the label (see [`switch_label`]).
pub fn switch_rule<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    // Comments above the rule are placed first, so that they don't break it.
    f.comments_before(node.first_token().id());
    f.group(|f| {
        let mut arrow = false;
        let mut elements = elements(node).peekable();
        while let Some(element) = elements.next() {
            match element.kind() {
                K::Arrow => {
                    f.space();
                    emit(f, element);
                    arrow = true;
                }
                K::Block if arrow => {
                    f.space();
                    emit(f, element);
                }
                _ if arrow => {
                    let semi = elements.next_if(|e| e.kind() == K::Semi);
                    let last = semi.map_or_else(|| last_token(element), last_token);
                    let body = |f: &mut Formatter<'t>| {
                        f.indent(IndentKind::Continuation, |f| {
                            f.line();
                            emit(f, element);
                        });
                        // A comment after the `;` of the last rule goes at +4 from the
                        // `switch`, as for the last group's statements.
                        if let Some(semi) = semi {
                            f.indent(IndentKind::Block, |f| emit(f, semi));
                        }
                    };
                    // As for a statement's body (`super::trailed`), comments after the rule
                    // don't break it.
                    if super::comment_after(last) && element.kind() != K::SwitchExpr {
                        f.hug(body);
                    } else {
                        body(f);
                    }
                }
                _ => emit(f, element),
            }
        }
    });
}

/// A group, `case …:` then statements. Each label goes on a line of its own, and the statements
/// follow at +2 on their own lines, with blank lines between them kept, at most one, and none
/// after the last label.
pub fn switch_group<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut labels = true;
    let mut statements = false;
    for element in elements(node) {
        match element.kind() {
            K::SwitchLabel => {
                if !labels {
                    f.hard_line();
                }
                labels = false;
                emit(f, element);
            }
            K::Colon => emit(f, element),
            _ => {
                if !statements {
                    f.start_indent(IndentKind::Block);
                    f.hard_line();
                    statements = true;
                } else {
                    f.line_break_before(element.as_node().expect("a statement").first_token());
                }
                emit(f, element);
            }
        }
    }
    if statements {
        f.end();
    }
}

/// A label: `default`, or `case` and its constants or patterns, separated by commas, then any
/// guard. In a rule, a label that doesn't fit breaks after every comma and before the guard, at
/// +4. In a group, it breaks after every comma at the label's own indentation, and the guard
/// always starts a line there, as GJF does.
pub fn switch_label<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let in_rule = node.parent().is_some_and(|p| p.kind() == K::SwitchRule);
    let mut elements = elements(node).peekable();
    let Some(keyword) = elements.next() else {
        return;
    };
    emit(f, keyword);
    if elements.peek().is_none() {
        return;
    }
    f.space();
    let items = |f: &mut Formatter<'t>| {
        f.group(|f| {
            for element in elements {
                if element.kind() == K::Guard && in_rule {
                    f.line();
                } else if element.kind() == K::Guard {
                    f.hard_line();
                }
                emit(f, element);
                if element.kind() == K::Comma {
                    f.line();
                }
            }
        });
    };
    if in_rule {
        f.indent(IndentKind::Continuation, items);
    } else {
        items(f);
    }
}

fn last_token(element: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match element {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.last_token(),
    }
}

/// A guard, `when` and its condition.
pub fn guard<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for (i, element) in elements(node).enumerate() {
        if i > 0 {
            f.space();
        }
        emit(f, element);
    }
}
