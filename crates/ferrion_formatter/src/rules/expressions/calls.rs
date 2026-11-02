//! Method calls and field accesses, as chains; arguments; `new`; and method references.

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{Indentation, elements, emit, spaced};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::lists::{self, Shape};

/// A chain of method calls and field accesses, `a.b(x).c`, flattened into its root (`a`) and
/// its links (`.b(x)`, `.c`), innermost first, so that a chain of any length is formatted in a
/// loop. GJF breaks a chain in one of two ways:
///
/// - **With two or more calls,** or one call that isn't last, as one group: the links stay on
///   one line, or each link after the chain's prefix starts a line at +4, all at once. The
///   prefix is the root and the first link when the root is a short name (up to four
///   characters), `this`, or `super`, or a type's name, possibly qualified, and the link after
///   it: `list.stream()`, `this.field`, `ImmutableList.builder()`, `java.util.Objects.of(x)`.
///   Otherwise it is the root alone. A name is a type's if it starts with an uppercase letter
///   and has a lowercase one, so `FOO_BAR` is not. In a chain with a `.stream()` link, the
///   prefix is everything up to it, `map.entrySet().stream()`, if that fits on the line, and
///   otherwise breaks before each of its dots as well.
/// - **With one call, last,** the dots fill: each breaks only if what follows it up to the next
///   dot doesn't fit, and the call's arguments break before any dot does.
pub fn chain<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut links = Vec::new();
    let mut current = node;
    let root = loop {
        let receiver = match current.kind() {
            K::MethodCallExpr => ast::MethodCallExpr::cast(current)
                .and_then(|call| call.receiver())
                .map(|receiver| receiver.syntax()),
            K::FieldAccessExpr => ast::FieldAccessExpr::cast(current)
                .and_then(|access| access.target())
                .map(|target| target.syntax()),
            _ => None,
        };
        match receiver {
            Some(receiver) => {
                links.push(current);
                current = receiver;
            }
            None => break current,
        }
    };
    links.reverse();
    let root_is_call = root.kind() == K::MethodCallExpr;
    let calls = links
        .iter()
        .filter(|l| l.kind() == K::MethodCallExpr)
        .count()
        + usize::from(root_is_call);
    let last_is_call = links
        .last()
        .map_or(root_is_call, |l| l.kind() == K::MethodCallExpr);
    // An anonymous class, `new Foo() {...}`, keeps the first link on its `}`, and is outside
    // the chain's groups and indent, so its body is indented from the line it starts on
    // (observed).
    let body_root = root.kind() == K::NewExpr && root.children().any(|c| c.kind() == K::ClassBody);
    // `new Foo().bar(x)` and `"a".concat(x)` break like longer chains (observed).
    let dotted_root = body_root
        || !matches!(
            root.kind(),
            K::NewExpr | K::LiteralExpr | K::ParenExpr | K::CastExpr
        );
    if body_root {
        f.comments_before(root.first_token().id());
        new_with(f, root, Hugs::None);
    }
    if calls <= 1 && last_is_call && dotted_root {
        filled_dots(f, root, &links, body_root);
        return;
    }
    // Everything up to a `.stream()` is a group of its own, kept on the first line if it fits
    // there, and otherwise broken before each of its dots (observed).
    let stream = links.iter().position(|&l| is_stream(l));
    let prefix = match stream {
        Some(_) => usize::from(body_root),
        None if body_root => 1,
        None => prefix_len(root, &links, calls),
    };
    f.start_group();
    let indentation = Indentation::start(f, IndentKind::Continuation);
    if stream.is_some() {
        f.start_group();
    }
    if body_root {
        // Emitted above.
    } else if root_is_call {
        // A short call's only argument is hugged: `when(a.b(` (observed).
        let short = ast::MethodCallExpr::cast(root)
            .and_then(|call| call.name())
            .is_some_and(|name| name.text().chars().count() <= 4);
        call_link(f, root, if short { Hugs::Sole } else { Hugs::None });
    } else if root.kind() == K::NewExpr {
        f.comments_before(root.first_token().id());
        new_with(f, root, Hugs::None);
    } else {
        f.node(root);
    }
    for (i, &link) in links.iter().enumerate() {
        if i >= prefix {
            f.soft_line();
        }
        let in_prefix = stream.map_or(i < prefix, |s| i <= s);
        link_after_receiver(f, link, if in_prefix { Hugs::None } else { Hugs::Lambdas });
        if stream == Some(i) {
            f.end();
        }
    }
    indentation.end(f);
    f.end();
}

/// A chain with one call, last: the root and the dots up to the call's name in a fill, then the
/// call's arguments.
///
/// When `attached`, the root is already emitted, and the first link follows it on its line.
///
/// The dot after a field that follows a type's name, `.get` in `Type.field.get(x)`, breaks if
/// the dot before the field does (observed), so the type and the field are a group, and the
/// links before and after them fills of their own.
fn filled_dots<'t>(
    f: &mut Formatter<'t>,
    root: SyntaxNode<'t>,
    links: &[SyntaxNode<'t>],
    attached: bool,
) {
    let Some(&last) = links.last() else {
        call_link(f, root, Hugs::Lambdas);
        return;
    };
    // Link `i`, the name only for the last.
    let link = |f: &mut Formatter<'t>, i: usize| {
        if i + 1 == links.len() {
            link_name(f, last);
        } else {
            link_after_receiver(f, links[i], Hugs::None);
        }
    };
    // The links in `range` filled, each after a break, except the first when `first` is false.
    let filled = |f: &mut Formatter<'t>, range: std::ops::Range<usize>, first: bool| {
        f.fill(|f| {
            for i in range.clone() {
                if first || i != range.start {
                    f.fill_break(true);
                }
                link(f, i);
            }
        });
    };
    let tied = if attached {
        None
    } else {
        last_type_name(root, links).filter(|&t| t + 1 < links.len())
    };
    let indentation = Indentation::start(f, IndentKind::Continuation);
    match tied {
        Some(t) => {
            f.fill(|f| {
                f.node(root);
                for i in 0..t {
                    f.fill_break(true);
                    link(f, i);
                }
            });
            let field = f.group(|f| {
                f.soft_line();
                link(f, t);
            });
            f.group_breaking_with(field, |f| f.soft_line());
            filled(f, t + 1..links.len(), false);
        }
        None if attached => filled(f, 0..links.len(), false),
        None => {
            f.fill(|f| {
                f.node(root);
                for i in 0..links.len() {
                    f.fill_break(true);
                    link(f, i);
                }
            });
        }
    }
    indentation.end(f);
    if let Some(list) = ast::MethodCallExpr::cast(last).and_then(|c| c.arguments()) {
        arguments(f, list.syntax(), Hugs::Lambdas);
    }
}

/// The leading field accesses of a name: 2 for `a.b.c` in `a.b.c.d()`, and 0 for a root that
/// isn't a name.
fn leading_names(root: SyntaxNode<'_>, links: &[SyntaxNode<'_>]) -> usize {
    if root.kind() != K::NameExpr {
        return 0;
    }
    links
        .iter()
        .take_while(|l| l.kind() == K::FieldAccessExpr && !has_annotations(**l))
        .count()
}

/// Of the root and its leading field accesses, the last that is a type's name (see
/// [`is_type_name`]): 0 for the root, `i` for `links[i - 1]`.
fn last_type_name(root: SyntaxNode<'_>, links: &[SyntaxNode<'_>]) -> Option<usize> {
    if root.kind() != K::NameExpr {
        return None;
    }
    let names = leading_names(root, links);
    std::iter::once(root)
        .chain(links[..names].iter().copied())
        .map(|n| n.last_token().text())
        .enumerate()
        .filter(|(_, name)| is_type_name(name))
        .map(|(i, _)| i)
        .last()
}

/// How many links stay with the root on the chain's first line (see [`chain`]).
fn prefix_len(root: SyntaxNode<'_>, links: &[SyntaxNode<'_>], calls: usize) -> usize {
    if calls == 1 {
        // One call, with links after it: the name before it is one unit.
        return leading_names(root, links);
    }
    if let Some(last_type) = last_type_name(root, links) {
        return (last_type + 1).min(links.len());
    }
    let short = match root.kind() {
        K::NameExpr => root.first_token().text().chars().count() <= 4,
        K::ThisExpr | K::SuperExpr => root.first_child().is_none(),
        _ => false,
    };
    usize::from(short).min(links.len())
}

/// Whether `link` is a call of `stream()` or `parallelStream()`, which GJF keeps with what comes
/// before it.
fn is_stream(link: SyntaxNode<'_>) -> bool {
    link.kind() == K::MethodCallExpr
        && ast::MethodCallExpr::cast(link)
            .and_then(|call| call.name())
            .is_some_and(|name| matches!(name.text(), "stream" | "parallelStream"))
}

fn has_annotations(node: SyntaxNode<'_>) -> bool {
    node.children().any(|c| c.kind() == K::Annotation)
}

/// Whether `name` looks like a type's: an uppercase letter first, and a lowercase one somewhere.
fn is_type_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(char::is_uppercase) && chars.any(char::is_lowercase)
}

/// Emits a link of a chain without its receiver: `.name(arguments)` or `.field`, its arguments
/// hugging as `hugs` says.
fn link_after_receiver<'t>(f: &mut Formatter<'t>, link: SyntaxNode<'t>, hugs: Hugs) {
    link_name(f, link);
    if link.kind() == K::MethodCallExpr
        && let Some(list) = ast::MethodCallExpr::cast(link).and_then(|c| c.arguments())
    {
        arguments(f, list.syntax(), hugs);
    }
}

/// Emits a link's `.`, type arguments, annotations, and name: everything after the receiver and
/// before the arguments.
fn link_name<'t>(f: &mut Formatter<'t>, link: SyntaxNode<'t>) {
    let mut receiver = true;
    let mut previous = None;
    for element in elements(link) {
        match element {
            NodeOrToken::Node(child) if receiver && child.kind() != K::Annotation => {
                receiver = false;
            }
            NodeOrToken::Node(child) if child.kind() == K::Arguments => {}
            NodeOrToken::Token(token) if token.kind() == K::Dot => {
                receiver = false;
                f.token(token);
                previous = Some(element);
            }
            _ => {
                spaced(f, previous, element);
                previous = Some(element);
            }
        }
    }
}

/// A call without a receiver, `foo(x)`, the root of a chain or on its own.
fn call_link<'t>(f: &mut Formatter<'t>, call: SyntaxNode<'t>, hugs: Hugs) {
    for element in elements(call) {
        match element {
            NodeOrToken::Node(child) if child.kind() == K::Arguments => {
                arguments(f, child, hugs);
            }
            _ => emit(f, element),
        }
    }
}

/// Which only argument a call's arguments hug (observed).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hugs {
    /// None: the calls in the prefix of a chain that breaks before its other links,
    /// `foo(x -> {...}).bar()`.
    None,
    /// A lambda: other calls.
    Lambdas,
    /// Anything but a lambda: a call of a short name, up to four characters, at the root of
    /// such a chain, `when(a.b(...)).then(c)`.
    Sole,
}

/// A call's arguments (see [`lists::delimited`]). A lambda that is the only argument is hugged
/// when `hugs` allows it: `foo(x -> {` stays on the call's line if that fits, with the block
/// indented from the statement.
pub fn arguments<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, hugs: Hugs) {
    let mut children = node.children();
    let hug = match (children.next(), children.next()) {
        (Some(only), None) => match hugs {
            Hugs::None => false,
            Hugs::Lambdas => only.kind() == K::LambdaExpr,
            Hugs::Sole => only.kind() != K::LambdaExpr,
        },
        _ => false,
    };
    lists::delimited(f, node, if hug { Shape::Hug } else { Shape::Arguments });
}

/// `new Type(arguments)`, then any class body after a space.
pub fn new<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    new_with(f, node, Hugs::Lambdas);
}

fn new_with<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, hugs: Hugs) {
    let mut previous = None;
    for element in elements(node) {
        match element {
            NodeOrToken::Node(child) if child.kind() == K::Arguments => {
                arguments(f, child, hugs);
            }
            NodeOrToken::Node(child) if child.kind() == K::ClassBody => {
                f.space();
                f.node(child);
            }
            NodeOrToken::Token(token) if token.kind() == K::NewKw => {
                f.token(token);
                f.space();
                previous = None;
                continue;
            }
            // `new <T>Foo()`: no space after the constructor's type arguments.
            _ if previous.is_some_and(|p: NodeOrToken<'_>| p.kind() == K::TypeArguments) => {
                emit(f, element);
            }
            _ => spaced(f, previous, element),
        }
        previous = Some(element);
    }
}

/// `qualifier::name`: on one line, or a line break before the `::` at +4.
pub fn method_ref<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.start_group();
    let indentation = Indentation::start(f, IndentKind::Continuation);
    for element in elements(node) {
        if let NodeOrToken::Token(token) = element
            && token.kind() == K::ColonColon
        {
            f.soft_line();
        }
        emit(f, element);
    }
    indentation.end(f);
    f.end();
}
