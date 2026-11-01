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

use ferrion_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::{children as elements, emit, generic};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::comments::{self, leads_on_code_line};

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

/// A line break before `next`, which starts a line: a statement, or a rule or group of a switch.
/// With `keep_blank`, a blank line there in the input is kept.
///
/// A block comment that leads `next` but sits on the line of the code before it stays on that
/// line, as GJF does (`a(); /* why */` then `b();`), except after a `{`, where it goes on a line of
/// its own before `next`.
fn line_break<'t>(f: &mut Formatter<'t>, next: SyntaxToken<'t>, keep_blank: bool) {
    let comment = leads_on_code_line(next);
    let after_brace = comment
        && next
            .prev_non_trivia()
            .is_some_and(|t| t.kind() == K::LBrace);
    if comment && !after_brace {
        f.comments_before(next.id());
    }
    if keep_blank {
        f.line_break_before(next);
    } else {
        f.hard_line();
    }
    if after_brace {
        f.comments_before(next.id());
        f.hard_line();
    }
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
            line_break(f, body.first_token(), false);
            f.node(body);
        }),
        _ if comments::line_comment_after(before) => f.node(body),
        _ => {
            if leads_on_code_line(body.first_token()) {
                f.comments_before(body.first_token().id());
            }
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
