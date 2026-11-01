//! Delimited, comma-separated lists: arguments, array initializers, and record patterns' parts.

use ferrion_syntax::comments::is_comment;
use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::{Indentation, elements, emit, first_token, lambdas};
use crate::document::{IndentKind, width};
use crate::format::Formatter;

/// GJF fills a list whose items are all at most this wide: as many on a line as fit. Otherwise a
/// list that doesn't fit on a line puts each item on its own (observed: up to 9 fills, from 10
/// one per line, for arguments and array initializers alike).
const FILL_WIDTH: u32 = 9;

/// The shape of a delimited list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Delimited {
    /// `(a, b)`: all on the line; otherwise all on the next line at +4; otherwise one per line,
    /// or filled. The `)` follows the last item.
    Arguments,
    /// `(x -> {`: one item, hugged (docs/design/formatter.md, "Hugging"): it stays on the line
    /// if the line up to its first forced break fits, and the item is indented +4 only if it
    /// doesn't. An item other than a lambda with a block body can also stay on the line and
    /// break inside when it doesn't fit on the next line either (see `Element::Alternative`):
    /// `foo(x ->` then the body at +4, or `when(a.b(` then `b`'s arguments.
    Hug,
    /// `{a, b}`: all on the line; otherwise the items on lines of their own at +2, all on one
    /// line, filled, or one per line, and the `}` on a line of its own. A trailing comma keeps the
    /// items off the braces' lines.
    Initializer,
}

/// Formats `node`'s list, from its opening delimiter to its closing one. Children before the
/// opening delimiter, such as a record pattern's type, are emitted as they are, with no space.
///
/// The list is two nested groups, so that it breaks outermost first: the outer one around the
/// delimiters, which breaks after the opening one, and the inner one around the items, which
/// breaks between them. Comments on their own lines before the first item go before the inner
/// group, so they don't break it, and a comment on a separator's line stays on it, after the
/// separator (docs/design/formatter.md, "Comments").
pub(crate) fn delimited<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, shape: Delimited) {
    let mut items = 0;
    let mut narrow = true;
    let mut trailing_comma = false;
    let mut opened = false;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if is_open(token) => opened = true,
            NodeOrToken::Node(child) if opened => {
                items += 1;
                narrow = narrow && fits_in(child, FILL_WIDTH);
                trailing_comma = false;
            }
            NodeOrToken::Token(token) if token.kind() == K::Comma => trailing_comma = true,
            _ => {}
        }
    }
    let mut elements = elements(node).peekable();
    // Whatever comes before the opening delimiter.
    while let Some(&element) = elements.peek() {
        if matches!(element, NodeOrToken::Token(t) if is_open(t)) {
            break;
        }
        emit(f, element);
        elements.next();
    }
    let Some(NodeOrToken::Token(open)) = elements.next() else {
        return;
    };
    f.token(open);
    if items == 0 {
        let close = node.last_token();
        if !has_comments_before(close) {
            f.token(close);
        } else if shape == Delimited::Initializer {
            // Dangling comments in braces go on lines of their own.
            let indentation = Indentation::start(f, IndentKind::Block, false);
            f.hard_line();
            f.comments_before(close.id());
            indentation.end(f);
            f.hard_line();
            f.token(close);
        } else {
            // In parentheses they stay on the line, with a space before the `)` (GJF's), unless
            // one is on a line of its own, which puts the `)` on one too, at +4.
            let indentation = Indentation::start(f, IndentKind::Continuation, false);
            f.comments_before(close.id());
            f.space();
            f.token(close);
            indentation.end(f);
        }
        return;
    }
    let fill = narrow && items > 1 && shape != Delimited::Hug;
    let close = node.last_token();
    debug_assert!(
        is_close(close),
        "a list that doesn't end in its closing delimiter"
    );
    let items = elements.take_while(|e| !matches!(e, NodeOrToken::Token(t) if is_close(*t)));
    let alternative = shape == Delimited::Hug
        && node
            .children()
            .next()
            .is_some_and(|item| item.kind() != K::LambdaExpr || !lambdas::has_block_body(item));
    // For arguments, around the group and the `)`, so that a `)` after a line break is at +4
    // too.
    let arguments_indentation = (shape == Delimited::Arguments)
        .then(|| Indentation::start(f, IndentKind::Continuation, false));
    let outer = if alternative {
        f.start_group_with_alternative()
    } else {
        f.start_group()
    };
    // The items, from the first, whose comments on lines of their own come before the group.
    let body = |f: &mut Formatter<'t>| {
        if shape == Delimited::Initializer && trailing_comma {
            f.hard_line();
        } else {
            f.soft_line();
        }
        let mut items = items.peekable();
        if let Some(&first) = items.peek() {
            f.comments_before(first_token(first).id());
        }
        let list = |f: &mut Formatter<'t>| {
            for element in items {
                match element {
                    NodeOrToken::Token(comma) if comma.kind() == K::Comma => {
                        f.token(comma);
                        if let Some(next) = next_item(comma) {
                            if comment_on_line_after(comma) {
                                f.comments_before(next.id());
                            }
                            if fill {
                                f.fill_break(false);
                            } else {
                                f.line();
                            }
                        }
                    }
                    NodeOrToken::Node(item) if shape == Delimited::Hug => {
                        if alternative {
                            f.alternative_mark(outer);
                        }
                        f.hug_in_group(|f| f.node(item));
                    }
                    _ => emit(f, element),
                }
            }
        };
        match shape {
            Delimited::Hug => list(f),
            _ if fill => f.fill(list),
            _ => {
                f.group(list);
            }
        }
        f.comments_before(close.id());
    };
    match shape {
        Delimited::Hug => Indentation::if_broken(f, outer, body),
        Delimited::Arguments => body(f),
        Delimited::Initializer => {
            let indentation = Indentation::start(f, IndentKind::Block, false);
            body(f);
            indentation.end(f);
        }
    }
    if shape == Delimited::Initializer {
        if trailing_comma {
            f.hard_line();
        } else {
            f.soft_line();
        }
    }
    // In the group, so that a line comment before it breaks the group, and one after it,
    // whose line break comes after the group ends, doesn't.
    f.token(close);
    f.end();
    if let Some(indentation) = arguments_indentation {
        indentation.end(f);
    }
}

/// A record pattern, `Point(int x, int y)`: its type, then its parts as arguments.
pub fn record_pattern<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    delimited(f, node, Delimited::Arguments);
}

fn is_open(token: SyntaxToken<'_>) -> bool {
    matches!(token.kind(), K::LParen | K::LBrace)
}

fn is_close(token: SyntaxToken<'_>) -> bool {
    matches!(token.kind(), K::RParen | K::RBrace)
}

/// The first token of the item after `comma`, if there is one before the closing delimiter.
fn next_item(comma: SyntaxToken<'_>) -> Option<SyntaxToken<'_>> {
    comma
        .next_non_trivia()
        .filter(|t| !matches!(t.kind(), K::RParen | K::RBrace))
}

/// Whether a comment follows `token` on its line, after any that trail it, other than one
/// naming a parameter, `/* name= */`, which stays with the argument after it (observed).
fn comment_on_line_after(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    while let Some(t) = next
        && t.is_trivia()
    {
        match t.kind() {
            K::LineTerminator => return false,
            K::BlockComment if names_parameter(t.text()) => return false,
            kind if is_comment(kind) => return true,
            _ => {}
        }
        next = t.next_token();
    }
    false
}

/// Whether `comment` is `/* name= */`.
fn names_parameter(comment: &str) -> bool {
    comment
        .strip_prefix("/*")
        .and_then(|c| c.strip_suffix("*/"))
        .and_then(|c| c.trim().strip_suffix('='))
        .is_some_and(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        })
}

/// Whether there is a comment between `token` and the token before it.
fn has_comments_before(token: SyntaxToken<'_>) -> bool {
    let mut previous = token.prev_token();
    while let Some(t) = previous
        && t.is_trivia()
    {
        if is_comment(t.kind()) {
            return true;
        }
        previous = t.prev_token();
    }
    false
}

/// Whether `node` printed flat is at most `limit` wide, as near as the first rule's spacing
/// tells: its tokens' widths and the spaces between them. The scan stops once past the limit,
/// so it costs at most a few tokens, however large the node. A comment or a token with a line
/// break makes the node too wide.
pub(crate) fn fits_in(node: SyntaxNode<'_>, limit: u32) -> bool {
    let mut total = 0;
    let mut previous: Option<SyntaxToken<'_>> = None;
    for token in node.tokens() {
        if token.is_trivia() {
            if is_comment(token.kind()) {
                return false;
            }
            continue;
        }
        if token.kind() == K::TextBlock {
            return false;
        }
        if let Some(previous) = previous
            && flat_space(previous, token)
        {
            total += 1;
        }
        total += width(token.text());
        if total > limit {
            return false;
        }
        previous = Some(token);
    }
    true
}

/// Whether there is a space between two tokens printed flat: as in the first rule, except inside
/// an array initializer's braces.
fn flat_space(previous: SyntaxToken<'_>, next: SyntaxToken<'_>) -> bool {
    if previous.kind() == K::LBrace && previous.parent().kind() == K::ArrayInitializer
        || next.kind() == K::RBrace && next.parent().kind() == K::ArrayInitializer
    {
        return false;
    }
    super::super::space_between(previous, next)
}
