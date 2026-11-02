//! Delimited, comma-separated lists, in every area: arguments and record patterns' parts, array
//! initializers and annotations' arrays of values, parameters and record components,
//! annotations' arguments, and type arguments and parameters. [`delimited`] lays them out, and
//! [`Shape`] holds what differs between them, as observed in GJF.
//!
//! What they share:
//!
//! - **Outermost first:** an outer group breaks after the opening delimiter, and the items, at
//!   +4 (+2 in braces), are a group, fill, or line of their own that breaks between them.
//! - **Comments:** a comment on a line of its own before the first item goes before the items'
//!   group, so it doesn't break it; a comment on a separator's line stays after the separator,
//!   except one naming a parameter, `/* name= */`, which stays with the item after it; and an
//!   empty list keeps its comments inside, with a space before the closing delimiter after a
//!   block comment, as GJF prints `(/* none */ )`.
//! - **Filling:** arguments and initializers whose items are all at most [`FILL_WIDTH`] wide are
//!   filled, as many on a line as fit.
//! - **Rows:** initializers whose input has the items in rows, and arguments in pairs, keep the
//!   rows, one a line (see [`Rows`]).

use ferrion_syntax::comments::is_comment;
use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::comments::has_comments_before;
use super::expressions::{Indentation, lambdas};
use super::{Area, area, children, emit, first_token};
use crate::document::{IndentKind, width};
use crate::format::Formatter;

/// GJF fills a list whose items are all at most this wide: as many on a line as fit. Otherwise a
/// list that doesn't fit on a line puts each item on its own (observed: up to 9 fills, from 10
/// one per line, for arguments, array initializers, and annotations' arrays of values alike).
pub(crate) const FILL_WIDTH: u32 = 9;

/// What a delimited list is, which decides how it breaks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Shape {
    /// `(a, b)` of a call or a record pattern: all on the line; otherwise all on the next line
    /// at +4; otherwise one per line, or filled. The `)` follows the last item, in the list's
    /// group, so a line comment before it breaks the list.
    Arguments,
    /// `(x -> {`: one item, hugged (docs/design/formatter.md, "Hugging"): it stays on the line
    /// if the line up to its first forced break fits, and the item is indented +4 only if it
    /// doesn't. An item other than a lambda with a block body can also stay on the line and
    /// break inside when it doesn't fit on the next line either (see `Element::Alternative`):
    /// `foo(x ->` then the body at +4, or `when(a.b(` then `b`'s arguments.
    Hug,
    /// `{a, b}`, an array initializer, or an annotation's array of values when `values`: all on
    /// the line; otherwise the items on lines of their own at +2, all on one line, filled, or one
    /// per line, and the `}` on a line of its own. A trailing comma keeps the items off the
    /// braces' lines. In an array of values, a comment on a line of its own before the first
    /// value puts the values one per line (observed), and they aren't kept in rows.
    Initializer { values: bool },
    /// `(int a, int b)`, parameters or record components: as arguments, without filling, and
    /// with the `)` after the list's group, so a line comment before it puts it on a line of its
    /// own at +4 without breaking the list.
    Parameters,
    /// `@A(a = 1, b = 2)`: one per line at +4 when they don't fit, and always when `broken`. The
    /// `)` is in the list's group, as for arguments.
    Annotation { broken: bool },
    /// `<A, B>`, type arguments or parameters: as parameters, with the `>` after the indent too.
    /// In a group of its own when `own_group`, or else broken after the `<` whenever the group
    /// around it is.
    Angle { own_group: bool },
}

impl Shape {
    fn fills(self) -> bool {
        matches!(self, Shape::Arguments | Shape::Initializer { .. })
    }
}

/// Formats `node`'s list, from its opening delimiter to its closing one, as `shape` says.
/// Children before the opening delimiter, such as a record pattern's type, are emitted as they
/// are, with no space. Returns the group of the items, if they have one, which breaks when they
/// go one per line.
pub(crate) fn delimited<'t>(
    f: &mut Formatter<'t>,
    node: SyntaxNode<'t>,
    shape: Shape,
) -> Option<u32> {
    f.comments_before(node.first_token().id());
    let mut items = 0;
    let mut narrow = true;
    let mut trailing_comma = false;
    let mut opened = false;
    let mut rows = Rows::new(shape);
    for element in children(node) {
        match element {
            NodeOrToken::Token(token) if is_open(token) => opened = true,
            NodeOrToken::Node(child) if opened => {
                items += 1;
                // A comment before an item on its line counts toward its width, as GJF
                // measures it.
                narrow = narrow
                    && shape.fills()
                    && !comment_on_line_before(child.first_token())
                    && fits_in(child, FILL_WIDTH);
                trailing_comma = false;
                rows.item(child);
            }
            NodeOrToken::Token(token) if token.kind() == K::Comma => trailing_comma = true,
            _ => {}
        }
    }
    let rows = rows.finish(node);
    let mut elements = children(node).peekable();
    // Whatever comes before the opening delimiter.
    while let Some(&element) = elements.peek() {
        if matches!(element, NodeOrToken::Token(t) if is_open(t)) {
            break;
        }
        emit(f, element);
        elements.next();
    }
    let Some(NodeOrToken::Token(open)) = elements.next() else {
        return None;
    };
    f.token(open);
    // An expression's list indents with the expression rules' capped indents.
    let capped = area(node.kind()) == Some(Area::Expressions);
    let indentation = |f: &mut Formatter<'t>, kind| {
        if capped {
            Indentation::start(f, kind)
        } else {
            Indentation::uncapped(f, kind)
        }
    };
    let close = node.last_token();
    if items == 0 {
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
        } else if matches!(shape, Shape::Initializer { .. }) {
            // Dangling comments in braces go on lines of their own.
            let indent = indentation(f, IndentKind::Block);
            f.hard_line();
            f.comments_before(close.id());
            indent.end(f);
            f.hard_line();
            f.token(close);
        } else {
            // In parentheses they stay on the line, with a space before the `)` (GJF's), unless
            // one is on a line of its own, which puts the `)` on one too, at +4.
            let indent = indentation(f, IndentKind::Continuation);
            f.comments_before(close.id());
            f.space();
            f.token(close);
            indent.end(f);
        }
        return None;
    }
    let fill = narrow && items > 1 && rows.is_none();
    let items =
        elements.take_while(|e| !matches!(e, NodeOrToken::Token(t) if t.id() == close.id()));
    let alternative = shape == Shape::Hug
        && node
            .children()
            .next()
            .is_some_and(|item| item.kind() != K::LambdaExpr || !lambdas::has_block_body(item));
    let broken = match shape {
        Shape::Annotation { broken } => broken,
        Shape::Initializer { .. } => trailing_comma,
        _ => false,
    };
    let one_per_line = matches!(shape, Shape::Annotation { .. });
    let mut items_group = None;
    // The items, from the first, whose comments on lines of their own come before their group,
    // then the comments before the closing delimiter.
    let body = |f: &mut Formatter<'t>, outer: u32| {
        if broken {
            f.hard_line();
        } else {
            f.soft_line();
        }
        let mut items = items.peekable();
        if let Some(&first) = items.peek()
            && shape != (Shape::Initializer { values: true })
        {
            f.comments_before(first_token(first).id());
        }
        // The items, or those left, with a `Line` or `FillBreak` (when `fill`) after each comma.
        let separated = |f: &mut Formatter<'t>,
                         items: &mut dyn Iterator<Item = NodeOrToken<'t>>,
                         fill: bool| {
            for element in items {
                match element {
                    NodeOrToken::Token(comma) if comma.kind() == K::Comma => {
                        f.token(comma);
                        if let Some(next) = next_item(comma, close) {
                            if comment_on_line_after(comma) {
                                f.comments_before(next.id());
                            }
                            if broken && one_per_line {
                                f.hard_line();
                            } else if fill {
                                f.fill_break(false);
                            } else {
                                f.line();
                            }
                        }
                    }
                    NodeOrToken::Node(item) if shape == Shape::Hug => {
                        if alternative {
                            f.alternative_mark(outer);
                        }
                        f.hug_in_group(|f| f.node(item));
                    }
                    _ => emit(f, element),
                }
            }
        };
        let list = |f: &mut Formatter<'t>| {
            let mut items = items;
            if let Some(n) = rows {
                in_rows(f, items, n, close, capped);
            } else {
                separated(f, &mut items, fill);
            }
        };
        match shape {
            Shape::Hug | Shape::Annotation { .. } => list(f),
            _ if fill => f.fill(list),
            _ => items_group = Some(f.group(list)),
        }
        f.comments_before(close.id());
    };
    match shape {
        Shape::Arguments | Shape::Annotation { .. } => {
            // Around the group and the `)`, so that a `)` after a line break is at +4 too.
            let indent = indentation(f, IndentKind::Continuation);
            let outer = f.start_group();
            body(f, outer);
            f.token(close);
            f.end();
            indent.end(f);
        }
        Shape::Hug => {
            let outer = if alternative {
                f.start_group_with_alternative()
            } else {
                f.start_group()
            };
            Indentation::if_broken(f, outer, |f| body(f, outer));
            f.token(close);
            f.end();
        }
        Shape::Initializer { .. } => {
            let outer = f.start_group();
            let indent = indentation(f, IndentKind::Block);
            body(f, outer);
            indent.end(f);
            if trailing_comma {
                f.hard_line();
            } else {
                f.soft_line();
            }
            f.token(close);
            f.end();
        }
        Shape::Parameters => {
            let indent = indentation(f, IndentKind::Continuation);
            let outer = f.start_group();
            body(f, outer);
            f.end();
            // After the group, so that a line comment before it doesn't break the list.
            f.token(close);
            indent.end(f);
        }
        Shape::Angle { own_group } => {
            let outer = own_group.then(|| f.start_group());
            let indent = indentation(f, IndentKind::Continuation);
            body(f, outer.unwrap_or(0));
            indent.end(f);
            if outer.is_some() {
                f.end();
            }
            // After the groups, so that a line comment after it doesn't break the list.
            f.token(close);
        }
    }
    items_group
}

/// Whether the input has a list's items in rows that GJF keeps (observed), read one item at a
/// time:
///
/// - **Lines:** the first item starts a line, and each row starts with an item that starts a
///   line at the first item's column. An item on a line of its own further right continues the
///   row, as a row that doesn't fit is printed; one further left means there are no rows.
/// - **Counts:** every row has the same number of items, except that an initializer's last row
///   can have fewer, and at least two rows have them all. Arguments are kept only in pairs,
///   `Map.of(` then `"a", A,` then `"b", B)`, with no item left over.
/// - **Elements:** the first row has at least two elements other than arrays, counting an
///   array's elements instead of the array, so a row can be one array of two: `{0, 1},` then
///   `{1, 0}`, but not `{1},` then `{2}`.
/// - **Kinds:** the rows' first items are all of one kind of expression, and in each other
///   column more than half are (see [`ItemKind`]).
///
/// Rows are printed one a line, each filled at +4 (see [`in_rows`]).
struct Rows {
    /// False once the items can't be in rows.
    possible: bool,
    arguments: bool,
    /// The first item's column.
    base: Option<u32>,
    /// The number of items in the first row, once it's complete.
    count: Option<u32>,
    /// The number of items in the current row, and of rows with `count` items so far.
    current: u32,
    full: u32,
}

impl Rows {
    fn new(shape: Shape) -> Rows {
        Rows {
            possible: matches!(shape, Shape::Arguments | Shape::Initializer { .. }),
            arguments: shape == Shape::Arguments,
            base: None,
            count: None,
            current: 0,
            full: 0,
        }
    }

    fn item(&mut self, item: SyntaxNode<'_>) {
        if !self.possible {
            return;
        }
        let column = line_start_column(item.first_token());
        let starts_row = match (self.base, column) {
            (None, Some(column)) => {
                self.base = Some(column);
                false
            }
            (None, None) => {
                self.possible = false;
                return;
            }
            (Some(base), Some(column)) if column < base => {
                self.possible = false;
                return;
            }
            (Some(base), Some(column)) => column == base,
            (Some(_), None) => false,
        };
        if starts_row {
            // The row before it is complete, so it must have the first row's count.
            match self.count {
                None => self.count = Some(self.current),
                Some(count) if count == self.current => {}
                Some(_) => {
                    self.possible = false;
                    return;
                }
            }
            self.full += 1;
            self.current = 0;
        }
        self.current += 1;
    }

    /// The number of items in each row, if `node`'s items are in rows.
    fn finish(self, node: SyntaxNode<'_>) -> Option<u32> {
        let count = self.count.filter(|_| self.possible)?;
        let full = self.full + u32::from(self.current == count);
        let counted = if self.arguments {
            count == 2 && self.current == count
        } else {
            self.current <= count
        };
        // An array of values with a comment before its first value has its values one per line
        // (observed).
        let commented = node.kind() == K::ElementValueArray
            && node
                .children()
                .next()
                .is_some_and(|item| has_comments_before(item.first_token()));
        let first_row = items(node).take(count as usize);
        (counted && full >= 2 && !commented && two_leaves(first_row) && same_kinds(node, count))
            .then_some(count)
    }
}

/// The items of `node`'s list, after its opening delimiter.
fn items<'t>(node: SyntaxNode<'t>) -> impl Iterator<Item = SyntaxNode<'t>> {
    children(node)
        .skip_while(|element| !matches!(element, NodeOrToken::Token(t) if is_open(*t)))
        .filter_map(|element| element.as_node())
}

/// Whether `items` have at least two elements other than arrays, counting an array's elements
/// instead of the array: a row of `{1, 2}` has two, and a row of `{}, {1}` one.
fn two_leaves<'t>(mut items: impl Iterator<Item = SyntaxNode<'t>>) -> bool {
    let mut arrays: Vec<SyntaxNode<'t>> = Vec::new();
    let mut leaves = 0;
    while let Some(node) = arrays.pop().or_else(|| items.next()) {
        let initializer = match node.kind() {
            K::ArrayInitializer | K::ElementValueArray => Some(node),
            K::ArrayCreationExpr => node
                .children()
                .find(|child| child.kind() == K::ArrayInitializer),
            _ => None,
        };
        match initializer {
            Some(initializer) => arrays.extend(initializer.children()),
            None => leaves += 1,
        }
        if leaves >= 2 {
            return true;
        }
    }
    false
}

/// Whether the rows of `count` items of `node` have items of one kind first, and in each other
/// column a kind that more than half of its items have.
fn same_kinds(node: SyntaxNode<'_>, count: u32) -> bool {
    let kinds: Vec<ItemKind> = items(node).map(ItemKind::of).collect();
    let count = count as usize;
    (0..count).all(|column| {
        let mut items = kinds.iter().skip(column).step_by(count);
        if column == 0 {
            let first = items.next();
            return items.all(|kind| Some(kind) == first);
        }
        // The kind with a majority, if there is one, is the one Boyer and Moore's vote finds.
        let mut candidate = None;
        let mut votes = 0;
        for kind in items.clone() {
            if votes == 0 {
                candidate = Some(kind);
            }
            if Some(kind) == candidate {
                votes += 1;
            } else {
                votes -= 1;
            }
        }
        let total = items.clone().count();
        items.filter(|&kind| Some(kind) == candidate).count() * 2 > total
    })
}

/// An item's kind of expression, as GJF compares them to keep rows (observed): javac's kind of
/// tree, after one unary operator. So `-1` and `1` are alike, as are `!a` and `b`, `a.b` and
/// `A.class`, and `"a"` and a text block, but `a + b` and `a - b` aren't, nor are `1` and `1L`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ItemKind(K, K);

impl ItemKind {
    fn of(item: SyntaxNode<'_>) -> ItemKind {
        let item = match item.kind() {
            K::PrefixExpr | K::PostfixExpr => item.children().next().unwrap_or(item),
            _ => item,
        };
        // The token that tells kinds of this node apart: an operator, or a literal.
        let token = || {
            children(item)
                .find_map(|child| child.as_token())
                .map_or(K::Error, |token| token.kind())
        };
        match item.kind() {
            K::LiteralExpr => ItemKind(
                K::LiteralExpr,
                match token() {
                    K::FalseKw => K::TrueKw,
                    K::TextBlock => K::StringLiteral,
                    kind => kind,
                },
            ),
            // javac reads `-1` as a literal.
            K::PrefixExpr
                if token() == K::Minus
                    && item.children().next().is_some_and(|operand| {
                        operand.kind() == K::LiteralExpr
                            && matches!(
                                operand.first_token().kind(),
                                K::IntLiteral | K::LongLiteral
                            )
                    }) =>
            {
                ItemKind::of(item.children().next().unwrap_or(item))
            }
            K::NameExpr => ItemKind(K::NameExpr, K::NameExpr),
            K::ThisExpr if item.first_token().kind() == K::ThisKw => {
                ItemKind(K::NameExpr, K::NameExpr)
            }
            K::FieldAccessExpr | K::ClassLiteralExpr | K::ThisExpr => {
                ItemKind(K::FieldAccessExpr, K::FieldAccessExpr)
            }
            K::ArrayInitializer | K::ArrayCreationExpr | K::ElementValueArray => {
                ItemKind(K::ArrayCreationExpr, K::ArrayCreationExpr)
            }
            K::BinaryExpr | K::AssignExpr | K::PrefixExpr => ItemKind(item.kind(), token()),
            K::PostfixExpr => ItemKind(
                K::PostfixExpr,
                children(item)
                    .filter_map(|child| child.as_token())
                    .last()
                    .map_or(K::Error, |token| token.kind()),
            ),
            kind => ItemKind(kind, kind),
        }
    }
}

/// Emits `items`, a list's items and the commas between them, in rows of `count`: a line break
/// between rows, and each row filled, its items after the first breaking at +4 only if they
/// don't fit, as GJF prints rows (observed).
fn in_rows<'t>(
    f: &mut Formatter<'t>,
    items: impl Iterator<Item = NodeOrToken<'t>>,
    count: u32,
    close: SyntaxToken<'t>,
    capped: bool,
) {
    let elements: Vec<NodeOrToken<'t>> = items.collect();
    let mut rest = &elements[..];
    while let Some(&first) = rest.first() {
        // The row: its items and the commas between them, up to its last item.
        let mut seen = 0;
        let end = rest
            .iter()
            .position(|element| {
                seen += u32::from(element.as_node().is_some());
                seen == count && element.as_node().is_some()
            })
            .map_or(rest.len(), |last| last + 1);
        let (row, after) = rest.split_at(end);
        // A comment on a line of its own before the row goes before its fill.
        f.comments_before(first_token(first).id());
        f.fill(|f| {
            emit(f, first);
            let indentation = if capped {
                Indentation::start(f, IndentKind::Continuation)
            } else {
                Indentation::uncapped(f, IndentKind::Continuation)
            };
            for &element in &row[1..] {
                emit(f, element);
                if let NodeOrToken::Token(comma) = element
                    && comma.kind() == K::Comma
                    && let Some(next) = next_item(comma, close)
                {
                    if comment_on_line_after(comma) {
                        f.comments_before(next.id());
                    }
                    // The item after a text block starts a line (observed).
                    if comma
                        .prev_non_trivia()
                        .is_some_and(|t| t.kind() == K::TextBlock)
                    {
                        f.hard_line();
                    } else {
                        f.fill_break(false);
                    }
                }
            }
            indentation.end(f);
        });
        rest = after;
        // The comma after the row, and the comments on its line.
        if let Some(&NodeOrToken::Token(comma)) = rest.first()
            && comma.kind() == K::Comma
        {
            f.token(comma);
            rest = &rest[1..];
            if let Some(next) = next_item(comma, close) {
                if comment_on_line_after(comma) {
                    f.comments_before(next.id());
                }
                f.hard_line();
            }
        }
    }
}

/// Emits `items`, a list's items and the commas between them, without delimiters, with a `Line`
/// after each comma. A comment on a comma's line stays after it, as in [`delimited`].
pub(crate) fn separated<'t>(f: &mut Formatter<'t>, items: &[NodeOrToken<'t>]) {
    for (i, &part) in items.iter().enumerate() {
        emit(f, part);
        if let NodeOrToken::Token(comma) = part
            && comma.kind() == K::Comma
            && let Some(&next) = items.get(i + 1)
        {
            if comment_on_line_after(comma) {
                f.comments_before(first_token(next).id());
            }
            f.line();
        }
    }
}

fn is_open(token: SyntaxToken<'_>) -> bool {
    matches!(token.kind(), K::LParen | K::LBrace | K::Lt)
}

/// The first token of the item after `comma`, if there is one before `close`.
fn next_item<'t>(comma: SyntaxToken<'t>, close: SyntaxToken<'t>) -> Option<SyntaxToken<'t>> {
    comma.next_non_trivia().filter(|t| t.id() != close.id())
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
    let braces = |token: SyntaxToken<'_>| {
        matches!(
            token.parent().kind(),
            K::ArrayInitializer | K::ElementValueArray
        )
    };
    if previous.kind() == K::LBrace && braces(previous) || next.kind() == K::RBrace && braces(next)
    {
        return false;
    }
    super::space_between(previous, next)
}
