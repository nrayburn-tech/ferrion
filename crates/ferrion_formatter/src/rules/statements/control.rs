//! Control flow and the simple statements: `if`, loops and their headers, labels, `assert`,
//! `synchronized`, and the statements that are a keyword, an expression, or both, then `;`.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::{body, elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;

/// `if (…) …`, with an `else` after a space when the statement before it is a block, and on the
/// next line otherwise. `else if` stays together, so a chain lines up.
pub fn if_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut before: Option<SyntaxToken<'t>> = None;
    let mut closed = false;
    let mut then_block = false;
    let mut after_else = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) => {
                match token.kind() {
                    K::LParen => f.space(),
                    K::ElseKw if then_block => f.space(),
                    K::ElseKw => f.hard_line(),
                    _ => {}
                }
                f.token(token);
                closed |= token.kind() == K::RParen;
                after_else |= token.kind() == K::ElseKw;
                before = Some(token);
            }
            NodeOrToken::Node(child) if !closed => f.node(child),
            NodeOrToken::Node(child) => {
                let before = before.expect("a statement follows a token");
                if after_else && child.kind() == K::IfStmt {
                    f.space();
                    f.node(child);
                } else {
                    then_block = child.kind() == K::Block;
                    body(f, child, before);
                }
            }
        }
    }
}

/// `while (…) …`.
pub fn while_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut before: Option<SyntaxToken<'t>> = None;
    let mut closed = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) => {
                if token.kind() == K::LParen {
                    f.space();
                }
                f.token(token);
                closed |= token.kind() == K::RParen;
                before = Some(token);
            }
            NodeOrToken::Node(child) if !closed => f.node(child),
            NodeOrToken::Node(child) => body(f, child, before.expect("`)` comes first")),
        }
    }
}

/// `do … while (…);`, with the `while` after a space when the body is a block, and on the next
/// line otherwise.
pub fn do_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut before: Option<SyntaxToken<'t>> = None;
    let mut body_block = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) => {
                match token.kind() {
                    K::WhileKw if body_block => f.space(),
                    K::WhileKw => f.hard_line(),
                    K::LParen => f.space(),
                    _ => {}
                }
                f.token(token);
                before = Some(token);
            }
            NodeOrToken::Node(child) if before.is_some_and(|t| t.kind() == K::DoKw) => {
                body_block = child.kind() == K::Block;
                body(f, child, before.expect("checked"));
            }
            NodeOrToken::Node(child) => f.node(child),
        }
    }
}

/// `for (init; condition; update) …`. The header's three parts break together, each at +4 on a
/// line of its own, when they don't fit; a missing part leaves `; ` (`for (; ; )`).
pub fn for_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut elements = elements(node).peekable();
    let mut before: Option<SyntaxToken<'t>> = None;
    while let Some(element) = elements.next() {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::LParen => {
                f.space();
                f.token(token);
                f.group(|f| {
                    f.indent(IndentKind::Continuation, |f| {
                        let mut semis = 0;
                        while let Some(&element) = elements.peek()
                            && element.kind() != K::RParen
                        {
                            elements.next();
                            emit(f, element);
                            if element.kind() == K::Semi {
                                semis += 1;
                                match elements.peek().map(|e| e.kind()) {
                                    Some(K::RParen) if semis == 2 => f.space(),
                                    _ => f.line(),
                                }
                            }
                        }
                    });
                });
            }
            NodeOrToken::Token(token) => {
                f.token(token);
                before = Some(token);
            }
            NodeOrToken::Node(child) => body(f, child, before.expect("`)` comes first")),
        }
    }
}

/// A `for` header's initialization or update: a local declaration, or expressions separated by
/// commas. Expressions in the initialization go one to a line at +4 when they don't fit, like
/// declarators. Those in the update go as many to a line as fit, continuing at +4 from the
/// first.
pub fn for_list<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    if node
        .first_child()
        .is_some_and(|c| c.kind() == K::LocalVarDecl)
    {
        for element in elements(node) {
            emit(f, element);
        }
    } else if node.kind() == K::ForInit {
        f.indent(IndentKind::Continuation, |f| {
            f.group(|f| {
                for element in elements(node) {
                    emit(f, element);
                    if element.kind() == K::Comma {
                        f.line();
                    }
                }
            });
        });
    } else {
        // The first item continues the header's line, and the others are indented from it.
        f.fill(|f| {
            let mut first = true;
            for element in elements(node) {
                if first || element.kind() == K::Comma {
                    emit(f, element);
                } else {
                    f.indent(IndentKind::Continuation, |f| emit(f, element));
                }
                if element.kind() == K::Comma {
                    f.fill_break(false);
                }
                first = false;
            }
        });
    }
}

/// `for (variable : expression) …`, breaking after the `:` at +4 when the header doesn't fit.
pub fn for_each_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut elements = elements(node).peekable();
    let mut before: Option<SyntaxToken<'t>> = None;
    while let Some(element) = elements.next() {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::LParen => {
                f.space();
                f.token(token);
                f.group(|f| {
                    while let Some(&element) = elements.peek()
                        && element.kind() != K::RParen
                    {
                        elements.next();
                        if element.kind() == K::Colon {
                            f.space();
                            f.token(element.as_token().expect("a token"));
                            f.indent(IndentKind::Continuation, |f| {
                                f.line();
                                while let Some(&element) = elements.peek()
                                    && element.kind() != K::RParen
                                {
                                    elements.next();
                                    emit(f, element);
                                }
                            });
                        } else {
                            emit(f, element);
                        }
                    }
                });
            }
            NodeOrToken::Token(token) => {
                f.token(token);
                before = Some(token);
            }
            NodeOrToken::Node(child) => body(f, child, before.expect("`)` comes first")),
        }
    }
}

/// `label:`, then the statement on the next line, at the same indentation.
pub fn labeled_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for element in elements(node) {
        if element.as_node().is_some() {
            f.hard_line();
        }
        emit(f, element);
    }
}

/// `assert condition : message;`, breaking before the `:` at +4 first, and then inside the
/// condition and the message, from +4.
pub fn assert_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut elements = elements(node).peekable();
    while let Some(element) = elements.next() {
        match element.kind() {
            K::AssertKw => {
                emit(f, element);
                f.space();
                f.indent(IndentKind::Continuation, |f| {
                    f.group(|f| {
                        while let Some(&element) = elements.peek()
                            && element.kind() != K::Semi
                        {
                            elements.next();
                            if element.kind() == K::Colon {
                                f.line();
                                emit(f, element);
                                f.space();
                            } else {
                                emit(f, element);
                            }
                        }
                    });
                });
            }
            _ => emit(f, element),
        }
    }
}

/// `synchronized (lock) { … }`, breaking after the `(` at +4 when the lock doesn't fit.
pub fn synchronized_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut elements = elements(node).peekable();
    while let Some(element) = elements.next() {
        match element.kind() {
            K::LParen => {
                f.space();
                emit(f, element);
                f.group(|f| {
                    f.indent(IndentKind::Continuation, |f| {
                        f.soft_line();
                        while let Some(&element) = elements.peek()
                            && element.kind() != K::RParen
                        {
                            elements.next();
                            emit(f, element);
                        }
                    });
                });
            }
            K::Block => {
                f.space();
                emit(f, element);
            }
            _ => emit(f, element),
        }
    }
}

/// The statements that are a keyword, an expression, or both, then `;`: `return x;`,
/// `break label;`, `f();`, `;`, and the like, with a space after the keyword.
pub fn simple_stmt<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut keyword = false;
    for element in elements(node) {
        if keyword && element.kind() != K::Semi {
            f.space();
        }
        keyword = matches!(
            element.kind(),
            K::ReturnKw | K::ThrowKw | K::YieldKw | K::BreakKw | K::ContinueKw
        );
        emit(f, element);
    }
}
