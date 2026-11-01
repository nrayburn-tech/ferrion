//! Rules for statements: blocks, local variable declarations, control flow, `switch`
//! statements and expressions and their blocks, `try` and its resources and clauses, and labels.
//! [`super::area`] lists the kinds, and docs/style.md, "Statements", describes the style.
//!
//! Each rule walks its node's children once, in order, emitting each token and child node once.
//! A rule decides the whitespace between its children; what is inside a child is the child's.

mod block;
mod control;
mod local;
mod switch;
mod try_stmt;

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::generic;
use crate::document::IndentKind;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    match node.kind() {
        K::Block => block::block(f, node),
        K::LocalVarDecl => local::local_var_decl(f, node),
        K::IfStmt => control::if_stmt(f, node),
        K::WhileStmt => control::while_stmt(f, node),
        K::DoStmt => control::do_stmt(f, node),
        K::ForStmt => control::for_stmt(f, node),
        K::ForInit | K::ForUpdate => control::for_list(f, node),
        K::ForEachStmt => control::for_each_stmt(f, node),
        K::LabeledStmt => control::labeled_stmt(f, node),
        K::AssertStmt => control::assert_stmt(f, node),
        K::SynchronizedStmt => control::synchronized_stmt(f, node),
        K::EmptyStmt
        | K::ExprStmt
        | K::BreakStmt
        | K::ContinueStmt
        | K::ReturnStmt
        | K::ThrowStmt
        | K::YieldStmt => control::simple_stmt(f, node),
        K::TryStmt => try_stmt::try_stmt(f, node),
        K::ResourceSpecification => try_stmt::resources(f, node),
        K::CatchClause => try_stmt::catch_clause(f, node),
        K::CatchParameter => try_stmt::catch_parameter(f, node),
        K::FinallyClause => try_stmt::finally_clause(f, node),
        K::SwitchStmt | K::SwitchExpr => switch::switch(f, node),
        K::SwitchBlock => switch::switch_block(f, node),
        K::SwitchRule => switch::switch_rule(f, node),
        K::SwitchGroup => switch::switch_group(f, node),
        K::SwitchLabel => switch::switch_label(f, node),
        K::Guard => switch::guard(f, node),
        _ => generic(f, node),
    }
}

/// The children of `node` that aren't trivia, in order.
fn elements<'t>(node: SyntaxNode<'t>) -> impl Iterator<Item = NodeOrToken<'t>> {
    node.children_with_tokens()
        .filter(|element| !matches!(element, NodeOrToken::Token(token) if token.is_trivia()))
}

/// Emits a child: a token with [`Formatter::token`], a node with its rule.
fn emit<'t>(f: &mut Formatter<'t>, element: NodeOrToken<'t>) {
    match element {
        NodeOrToken::Token(token) => f.token(token),
        NodeOrToken::Node(node) => f.node(node),
    }
}

/// Whether a line comment follows `token` on its line, with only spaces between.
fn line_comment_after(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    while let Some(t) = next {
        match t.kind() {
            K::Whitespace => next = t.next_token(),
            K::LineComment | K::DocLineComment => return true,
            _ => return false,
        }
    }
    false
}

/// The body of an `if`, `else`, loop, or `do`, after `before`, the token ahead of it.
///
/// A block follows after a space, and an empty statement goes on the next line at +2. Any
/// other statement follows after a space if it fits, and otherwise starts the next line at +2.
/// When a line comment ends the line before it, GJF starts the statement on the next line at the
/// indentation of the line before, not +2, and so does this.
fn body<'t>(f: &mut Formatter<'t>, body: SyntaxNode<'t>, before: SyntaxToken<'t>) {
    match body.kind() {
        K::Block => {
            f.space();
            f.node(body);
        }
        K::EmptyStmt => f.indent(IndentKind::Block, |f| {
            f.hard_line();
            f.node(body);
        }),
        _ if line_comment_after(before) => f.node(body),
        _ => {
            f.group(|f| {
                f.indent(IndentKind::Block, |f| {
                    f.line();
                    trailed(f, body);
                });
            });
        }
    }
}

/// Emits `node`, a statement or expression ending a group the caller has open, so that comments
/// after it don't break the group: a line comment there ends the line, but the group is laid out
/// as if it weren't there, as GJF does (`if (a) b(); // why` stays on one line). Only statements
/// without bodies of their own are emitted that way, since a hug would also keep the group from
/// breaking around a block in them.
fn trailed<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let simple = matches!(
        node.kind(),
        K::ExprStmt
            | K::ReturnStmt
            | K::ThrowStmt
            | K::BreakStmt
            | K::ContinueStmt
            | K::YieldStmt
            | K::AssertStmt
    );
    if simple && comment_after(node.last_token()) {
        f.hug(|f| f.node(node));
    } else {
        f.node(node);
    }
}

/// Whether a comment follows `token`, on its line or the next.
fn comment_after(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    let mut lines = 0;
    while let Some(t) = next {
        match t.kind() {
            K::Whitespace => {}
            K::LineTerminator if lines == 0 => lines += 1,
            kind => return ferrion_syntax::comments::is_comment(kind),
        }
        next = t.next_token();
    }
    false
}
