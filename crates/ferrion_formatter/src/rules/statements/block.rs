//! Blocks: every `{ … }` of statements, whether it is a statement, a body of a statement, or the
//! body of a method, constructor, initializer, or lambda.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::{elements, line_break};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::comments::{self, has_comments_before};

/// Whether the `}` of a block stays on the line of the block comment before it: unless the
/// statement goes on after it, it's an `if`'s without an `else`, or it ends a lambda in a call
/// (observed).
fn ends_on_comment_line(close: SyntaxToken<'_>) -> bool {
    let block = close.parent();
    let if_body = block.parent().is_some_and(|p| p.kind() == K::IfStmt)
        && block
            .first_token()
            .prev_non_trivia()
            .is_none_or(|t| t.kind() != K::ElseKw);
    comments::own_line_block_comment_on_line_of(close)
        && !continued(close)
        && !if_body
        && close
            .next_non_trivia()
            .is_none_or(|next| next.kind() != K::RParen)
}

/// A block: its statements on their own lines at +2, and the `}` on a line of its own.
///
/// Blank lines between statements are kept, at most one, and a blank line before the `}` is
/// removed, unless the statement goes on after the `}` (see [`continued`]) or a comment on a line
/// of its own comes before it, as GJF keeps it. Whether a blank line after the `{` is kept, and
/// whether an empty block prints as `{}` or as `{` and `}` on two lines, depend on what the block
/// belongs to (see [`Style`]); one on two lines keeps a blank line between them where a blank
/// line after the `{` is kept. Comments in an otherwise empty block go on their own lines inside
/// it, except a line comment on the `{`'s line, which stays there, and the `}` stays on the line of
/// a block comment that starts its line (`/* c */ }`).
pub fn block<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let style = Style::of(node);
    let mut first = true;
    let mut lbrace = None;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::LBrace => {
                f.token(token);
                lbrace = Some(token);
            }
            NodeOrToken::Token(token) if token.kind() == K::RBrace => {
                if !first {
                    f.comments_before(token.id());
                    f.end();
                    if continued(token) && !has_comments_before(token) {
                        f.line_break_before(token);
                    } else if ends_on_comment_line(token) {
                        f.space();
                    } else {
                        if !continued(token) && !comments::own_line_comment_before(token) {
                            f.no_blank_line();
                        }
                        f.hard_line();
                    }
                } else if style.keep_blank_at_start
                    && !style.keep_empty
                    && !has_comments_before(token)
                    && f.blank_line_before(token)
                {
                    f.blank_line();
                } else if has_comments_before(token) {
                    f.indent(IndentKind::Block, |f| {
                        if !lbrace.is_some_and(comments::line_comment_after) {
                            f.hard_line();
                        }
                        f.comments_before(token.id());
                    });
                    if ends_on_comment_line(token) {
                        f.space();
                    } else {
                        f.hard_line();
                    }
                } else if !style.keep_empty {
                    f.hard_line();
                }
                f.token(token);
            }
            NodeOrToken::Token(token) => f.token(token),
            NodeOrToken::Node(statement) => {
                if first {
                    f.start_indent(IndentKind::Block);
                    line_break(f, statement.first_token(), style.keep_blank_at_start);
                    first = false;
                } else {
                    line_break(f, statement.first_token(), true);
                }
                f.node(statement);
            }
        }
    }
}

/// Whether the statement goes on after `rbrace`, a block's `}`, on its line: `} catch`,
/// `} finally`, `} else`, or a `do`'s `} while`. GJF keeps a blank line before such a `}`.
fn continued(rbrace: SyntaxToken<'_>) -> bool {
    rbrace
        .next_non_trivia()
        .is_some_and(|next| match next.kind() {
            K::CatchKw | K::FinallyKw | K::ElseKw => true,
            K::WhileKw => next.parent().kind() == K::DoStmt,
            _ => false,
        })
}

/// How a block's ends print, by what it belongs to, as observed in GJF.
struct Style {
    /// An empty block prints as `{}`, rather than `{` and `}` on two lines.
    keep_empty: bool,
    /// A blank line after the `{` is kept, rather than removed.
    keep_blank_at_start: bool,
}

impl Style {
    fn of(block: SyntaxNode<'_>) -> Style {
        let (keep_empty, keep_blank_at_start) = match block.parent().map(|p| (p.kind(), p)) {
            Some((
                K::MethodDecl
                | K::ConstructorDecl
                | K::CompactConstructorDecl
                | K::WhileStmt
                | K::DoStmt
                | K::ForStmt
                | K::ForEachStmt,
                _,
            )) => (true, true),
            Some((K::LambdaExpr | K::SwitchRule, _)) => (true, false),
            Some((K::IfStmt, parent)) => (!in_else_chain(parent), true),
            Some((K::TryStmt, parent)) => (!has_clauses(parent), true),
            Some((K::CatchClause | K::FinallyClause, _)) => (false, true),
            _ => (false, false),
        };
        Style {
            keep_empty,
            keep_blank_at_start,
        }
    }
}

/// Whether an `if` statement has an `else`, or is the `else` of another: its empty blocks print
/// on two lines.
fn in_else_chain(if_stmt: SyntaxNode<'_>) -> bool {
    if if_stmt.direct_tokens().any(|t| t.kind() == K::ElseKw) {
        return true;
    }
    if_stmt.parent().is_some_and(|p| p.kind() == K::IfStmt)
        && if_stmt
            .first_token()
            .prev_non_trivia()
            .is_some_and(|t| t.kind() == K::ElseKw)
}

/// Whether a `try` statement has a `catch` or a `finally`.
fn has_clauses(try_stmt: SyntaxNode<'_>) -> bool {
    try_stmt
        .children()
        .any(|c| matches!(c.kind(), K::CatchClause | K::FinallyClause))
}
