//! Binary operators, `?:`, assignment, casts, and `instanceof`.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{Indentation, elements, emit};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::lists::{FILL_WIDTH, fits_in};
use crate::rules::variables::assigned_value;

/// A run of operators of one precedence, `a + b - c`, as one group: the operands stay on one
/// line, or each operator starts a line, all at once, +4 from the first operand. Operators of
/// higher precedence in an operand are a group of their own, decided after this one, so
/// `a + b * c` breaks before the `+` first. When every operand is at most [`FILL_WIDTH`] wide,
/// the run is a fill instead, each operator breaking only if what follows it doesn't fit.
///
/// The run is a left-nested spine of `BinaryExpr`s, walked in a loop rather than by recursion,
/// so a long run of `+` nests no deeper than one.
pub fn binary<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let level = precedence(node);
    let mut spine = vec![node];
    let mut leftmost = node.first_child();
    while let Some(lhs) = leftmost
        && lhs.kind() == K::BinaryExpr
        && precedence(lhs) == level
    {
        spine.push(lhs);
        leftmost = lhs.first_child();
    }
    // Operands all at most `FILL_WIDTH` wide fill, as many on a line as fit (observed).
    let fill = std::iter::once(leftmost)
        .chain(spine.iter().map(|b| b.children().nth(1)))
        .all(|operand| operand.is_some_and(|o| fits_in(o, FILL_WIDTH)));
    let body = |f: &mut Formatter<'t>| {
        let indentation = Indentation::start(f, IndentKind::Continuation);
        let mut first = true;
        for &binary in spine.iter().rev() {
            let mut seen_lhs = false;
            let mut seen_op = false;
            for element in elements(binary) {
                match element {
                    NodeOrToken::Node(child) if !seen_lhs => {
                        seen_lhs = true;
                        if first {
                            f.node(child);
                            first = false;
                        }
                    }
                    NodeOrToken::Token(op) => {
                        if !seen_op {
                            if fill {
                                f.fill_break(false);
                            } else {
                                f.line();
                            }
                            seen_op = true;
                        }
                        f.token(op);
                    }
                    NodeOrToken::Node(rhs) => {
                        f.space();
                        f.node(rhs);
                    }
                }
            }
        }
        indentation.end(f);
    };
    if fill {
        f.fill(body);
    } else {
        f.group(body);
    }
}

/// The precedence of a binary operator, higher binding tighter (JLS 15.7). `>>` and `>>>` are runs
/// of `>` tokens.
fn precedence(node: SyntaxNode<'_>) -> u8 {
    let mut ops = node.direct_tokens().filter(|t| !t.is_trivia());
    let Some(op) = ops.next() else { return 0 };
    match op.kind() {
        K::PipePipe => 1,
        K::AmpAmp => 2,
        K::Pipe => 3,
        K::Caret => 4,
        K::Amp => 5,
        K::EqEq | K::BangEq => 6,
        K::Gt if ops.next().is_some() => 8,
        K::Lt | K::Gt | K::LtEq | K::GtEq => 7,
        K::LtLt => 8,
        K::Plus | K::Minus => 9,
        K::Star | K::Slash | K::Percent => 10,
        _ => 0,
    }
}

/// `c ? a : b`: on one line, or the condition, then `? a` and `: b` on lines of their own at +4.
/// A `?:` in a branch is a group of its own.
pub fn conditional<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.start_group();
    let indentation = Indentation::start(f, IndentKind::Continuation);
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if matches!(token.kind(), K::Question | K::Colon) => {
                f.line();
                f.token(token);
                f.space();
            }
            _ => emit(f, element),
        }
    }
    indentation.end(f);
    f.end();
}

/// `x = value`: on one line, or a line break after the operator and the value at +4, as after a
/// declarator's `=` (see [`assigned_value`]).
pub fn assign<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.start_group();
    let mut target = true;
    let mut op = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Node(child) if target => {
                f.node(child);
                target = false;
            }
            // An operator the lexer splits, such as `>>=`, is several tokens.
            NodeOrToken::Token(token) => {
                if !op {
                    f.space();
                    op = true;
                }
                f.token(token);
            }
            NodeOrToken::Node(_) => assigned_value(f, element),
        }
    }
    f.end();
}

/// `(Type) value`: on one line, or a line break after the `)` and the value at +4. The type is
/// in the indent too, so its own breaks are at +4 from the `(`'s line and more.
pub fn cast<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.start_group();
    let indentation = Indentation::start(f, IndentKind::Continuation);
    let mut closed = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::RParen && !closed => {
                f.token(token);
                closed = true;
            }
            NodeOrToken::Node(value) if closed => {
                f.line();
                f.node(value);
            }
            _ => emit(f, element),
        }
    }
    indentation.end(f);
    f.end();
}

/// `x instanceof T`: on one line, or a line break before `instanceof` at +4. A record pattern
/// that doesn't fit after `instanceof` starts a line of its own there.
pub fn instanceof<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.start_group();
    let indentation = Indentation::start(f, IndentKind::Continuation);
    let mut after_keyword = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if token.kind() == K::InstanceofKw => {
                f.line();
                f.token(token);
                after_keyword = true;
            }
            NodeOrToken::Node(child) if after_keyword && child.kind() == K::RecordPattern => {
                f.start_group();
                f.line();
                f.node(child);
                f.end();
            }
            _ => {
                if after_keyword {
                    f.space();
                }
                emit(f, element);
            }
        }
    }
    indentation.end(f);
    f.end();
}
