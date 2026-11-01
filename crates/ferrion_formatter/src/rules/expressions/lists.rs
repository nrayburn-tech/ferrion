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
    // Whether the input has the items in rows of two, `Map.of(` then `"a", A,` then `"b", B)`:
    // the first item starts a line, and each row starts with an item that starts a line at or
    // left of the first item's column. The second item of a row may be on a line of its own if
    // it is indented further, as a pair that doesn't fit on its line is printed.
    let mut pairs = true;
    let mut base = None;
    let mut row = 0;
    for element in elements(node) {
        match element {
            NodeOrToken::Token(token) if is_open(token) => opened = true,
            NodeOrToken::Node(child) if opened => {
                items += 1;
                // A comment before an item on its line counts toward its width, as GJF
                // measures it.
                narrow = narrow
                    && !comment_on_line_before(child.first_token())
                    && fits_in(child, FILL_WIDTH);
                trailing_comma = false;
                let column = line_start_column(child.first_token());
                let starts_row = match (base, column) {
                    (None, Some(column)) => {
                        base = Some(column);
                        true
                    }
                    (None, None) => {
                        pairs = false;
                        true
                    }
                    (Some(base), Some(column)) => column <= base,
                    (Some(_), None) => false,
                };
                if starts_row {
                    pairs = pairs && (row == 0 || row == 2);
                    row = 1;
                } else {
                    row += 1;
                }
            }
            NodeOrToken::Token(token) if token.kind() == K::Comma => trailing_comma = true,
            _ => {}
        }
    }
    // GJF keeps such pairs, one on each line, each breaking before its second item if it
    // doesn't fit (observed).
    let pairs = pairs && row == 2 && items >= 4 && !trailing_comma && shape != Delimited::Hug;
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
        // `{,}`: a comma without items.
        for element in elements {
            if let NodeOrToken::Token(comma) = element
                && comma.kind() == K::Comma
            {
                f.token(comma);
            }
        }
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
    let fill = narrow && items > 1 && shape != Delimited::Hug && !pairs;
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
            // How many items so far, and the indentation of the second item of a pair.
            let mut index = 0;
            let mut pair = None;
            for element in items {
                match element {
                    NodeOrToken::Token(comma) if comma.kind() == K::Comma => {
                        f.token(comma);
                        if let Some(next) = next_item(comma) {
                            if comment_on_line_after(comma) {
                                f.comments_before(next.id());
                            }
                            if pairs && index % 2 == 1 {
                                pair = Some(Indentation::start(f, IndentKind::Continuation, false));
                                f.line();
                            } else if pairs {
                                f.hard_line();
                            } else if fill {
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
                    NodeOrToken::Node(item) if pairs => {
                        if index % 2 == 0 {
                            f.start_group();
                        }
                        f.node(item);
                        if let Some(indentation) = pair.take() {
                            indentation.end(f);
                            f.end();
                        }
                        index += 1;
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

/// The column `token`'s line starts at in the input, its indentation, if only whitespace and
/// comments come before `token` on the line.
fn line_start_column(token: SyntaxToken<'_>) -> Option<u32> {
    // The width of the whitespace since the last comment, which is the indentation once the
    // line's start is reached.
    let mut column = 0;
    let mut previous = token.prev_token();
    while let Some(t) = previous {
        match t.kind() {
            K::LineTerminator => return Some(column),
            K::Whitespace => column += width(t.text()),
            kind if is_comment(kind) && !t.text().contains(['\n', '\r']) => column = 0,
            _ => return None,
        }
        previous = t.prev_token();
    }
    Some(column)
}

/// Whether a comment comes before `token` on its line, after the token before it.
fn comment_on_line_before(token: SyntaxToken<'_>) -> bool {
    let mut previous = token.prev_token();
    while let Some(t) = previous
        && t.is_trivia()
    {
        match t.kind() {
            K::LineTerminator => return false,
            kind if is_comment(kind) => return true,
            _ => {}
        }
        previous = t.prev_token();
    }
    false
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
