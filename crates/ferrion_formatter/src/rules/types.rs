//! Rules for types: primitive, class, array, intersection, and union types, type parameters and
//! arguments, wildcards, varargs, and `throws`. [`super::area`] lists the kinds.
//!
//! The style is in docs/style.md, "Types". Lists break as GJF breaks them: a `<...>` list first
//! after its `<`, with its items on one line at +4, then one item per line; bounds and
//! intersections fill their lines, breaking before `&`; union types put one alternative per line,
//! breaking before `|`; and a `throws` list keeps its first type on the `throws` line and puts
//! the rest one per line.

use ferrion_syntax::ast::{AstNode, ClassType};
use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::variables::type_then_name;
use super::{children, emit, first_token, generic};
use crate::document::IndentKind;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    match node.kind() {
        K::TypeParameters | K::TypeArguments => angle_list(f, node),
        K::TypeParameter => type_parameter(f, node),
        K::ClassType => class_type(f, node),
        K::PrimitiveType | K::Varargs | K::Dim => annotated(f, node),
        K::ArrayType => array_type(f, node),
        K::Wildcard => wildcard(f, node),
        K::IntersectionType => intersection_type(f, node),
        K::UnionType => union_type(f, node),
        K::ThrowsClause => throws_clause(f, node),
        _ => generic(f, node),
    }
}

/// A comma-separated list in angle brackets, `node`'s `<`, items, and `>`: type parameters or
/// type arguments. Flat if it fits; otherwise broken after the `<`, with the items at +4 on one
/// line if they fit there, or one per line. The `>` follows the last item. An empty list, the
/// diamond, is `<>`.
///
/// Comments before the first item go before the items' group, so that a line comment there
/// doesn't put each item on its own line. Comments before the `>` go inside the indent, and the
/// `>` itself after the groups, so that a line comment trailing it doesn't break the list.
pub(crate) fn angle_list<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    angle_list_in(f, node, true);
}

/// [`angle_list`], in a group of its own when `own_group`, or else broken after the `<` whenever
/// the group it's in is broken.
fn angle_list_in<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, own_group: bool) {
    let close = node.last_token();
    let mut children = children(node)
        .filter(|c| c.as_token().is_none_or(|t| t.id() != close.id()))
        .peekable();
    if let Some(open) = children.next() {
        emit(f, open);
    }
    let Some(&first) = children.peek() else {
        f.token(close);
        return;
    };
    if own_group {
        f.start_group();
    }
    f.indent(IndentKind::Continuation, |f| {
        f.soft_line();
        f.comments_before(first_token(first).id());
        f.group(|f| {
            for child in children {
                emit(f, child);
                if child.kind() == K::Comma {
                    f.line();
                }
            }
        });
        f.comments_before(close.id());
    });
    if own_group {
        f.end();
    }
    f.token(close);
}

/// `@A T extends B & C`: the annotations, then the name, with a line break between them when the
/// parameter doesn't fit; then `extends` and the bounds, broken after `extends` with the bounds
/// at +4 when they don't fit after it, and filled, breaking before `&` at a further +4.
fn type_parameter<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut children = children(node).peekable();
    let annotated = children.peek().is_some_and(|c| c.kind() == K::Annotation);
    if annotated {
        f.start_group();
    }
    while let Some(&child) = children.peek()
        && child.kind() == K::Annotation
    {
        children.next();
        emit(f, child);
        if children.peek().is_some_and(|c| c.kind() == K::Annotation) {
            f.space();
        } else {
            f.line();
        }
    }
    if let Some(name) = children.next() {
        emit(f, name);
    }
    if let Some(extends) = children.next() {
        f.space();
        f.group(|f| {
            emit(f, extends);
            f.indent(IndentKind::Continuation, |f| {
                f.line();
                filled(f, children, K::Amp);
            });
        });
    }
    if annotated {
        f.end();
    }
}

/// Items separated by `separator` tokens, filled at +4: each separator starts a new line only if
/// it and its item don't fit on the current one.
fn filled<'t>(
    f: &mut Formatter<'t>,
    children: impl Iterator<Item = NodeOrToken<'t>>,
    separator: K,
) {
    f.indent(IndentKind::Continuation, |f| {
        f.fill(|f| {
            for child in children {
                if child.kind() == separator {
                    f.fill_break(false);
                    emit(f, child);
                    f.space();
                } else {
                    emit(f, child);
                }
            }
        });
    });
}

/// `A & B` in a cast: filled, breaking before `&` at +4.
fn intersection_type<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.comments_before(node.first_token().id());
    filled(f, children(node), K::Amp);
}

/// `A | B` in a `catch`: on one line if it fits, otherwise one alternative per line at +4, each
/// starting with its `|`.
fn union_type<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    union_with(f, node, None::<fn(&mut Formatter<'t>)>);
}

/// `A | B e`, a `catch` parameter's union type, `node`, and what `name` emits: laid out as
/// [`union_type`], with the last alternative and the name a type and its name, which breaks
/// before the name (observed in GJF: the last alternative's breaks are at +4 from the others'
/// when the name breaks, and the name is at +8).
pub(crate) fn union_then_name<'t>(
    f: &mut Formatter<'t>,
    node: SyntaxNode<'t>,
    name: impl FnOnce(&mut Formatter<'t>),
) {
    union_with(f, node, Some(name));
}

fn union_with<'t>(
    f: &mut Formatter<'t>,
    node: SyntaxNode<'t>,
    name: Option<impl FnOnce(&mut Formatter<'t>)>,
) {
    f.comments_before(node.first_token().id());
    let mut children = children(node).peekable();
    f.group(|f| {
        f.indent(IndentKind::Continuation, |f| {
            while let Some(child) = children.next() {
                if child.kind() == K::Pipe {
                    f.line();
                    emit(f, child);
                    f.space();
                } else if children.peek().is_none()
                    && let Some(name) = name
                {
                    f.comments_before(first_token(child).id());
                    f.group(|f| type_then_name(f, child, name));
                    return;
                } else {
                    emit(f, child);
                }
            }
        });
    });
}

/// `throws A, B`: on one line if it fits; otherwise the types one per line at +4, with the first
/// on the `throws` line if it fits there.
fn throws_clause<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut children = children(node);
    f.group(|f| {
        if let Some(throws) = children.next() {
            emit(f, throws);
        }
        f.indent(IndentKind::Continuation, |f| {
            if let Some(first) = children.next() {
                f.group(|f| {
                    f.line();
                    emit(f, first);
                });
            }
            for child in children {
                emit(f, child);
                if child.kind() == K::Comma {
                    f.line();
                }
            }
        });
    });
}

/// A node of annotations followed by its own tokens: a primitive type (`@A int`), a dimension
/// (`@A []`), or varargs (`@A ...`). A space follows each annotation.
fn annotated<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for child in children(node) {
        emit(f, child);
        if child.kind() == K::Annotation {
            f.space();
        }
    }
}

/// The element type and its dimensions: `int[][]`, with a space before a dimension that has
/// annotations (`String @A []`).
fn array_type<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for child in children(node) {
        if let NodeOrToken::Node(dim) = child
            && dim.kind() == K::Dim
            && dim.first_child().is_some()
        {
            f.space();
        }
        emit(f, child);
    }
}

/// `? extends T`: broken after `extends` or `super`, with the bound at +4, when it doesn't fit.
fn wildcard<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut children = children(node);
    for child in children.by_ref() {
        emit(f, child);
        if child.kind() == K::Annotation {
            f.space();
        } else if child.kind() == K::Question {
            break;
        }
    }
    if let Some(keyword) = children.next() {
        f.space();
        f.group(|f| {
            emit(f, keyword);
            f.indent(IndentKind::Continuation, |f| {
                f.line();
                for child in children {
                    emit(f, child);
                }
            });
        });
    }
}

/// A class type, `a.b.C<T>`, as one chain of segments, the qualifiers nested to the left in the
/// tree. GJF breaks a long chain by the case of its names (docs/style.md, "Qualified types"):
///
/// - a package and its types, `a.b.C.D` (lowercase names, then only uppercase ones), fill their
///   lines, breaking before a `.`;
/// - otherwise the chain's head stays on the first line: the names up to the end of the first
///   run of uppercase ones, filled, and the name after them; or with no uppercase name, the
///   first name, and the second too if the first is at most 4 columns wide. The rest goes one
///   name per line if it doesn't fit.
///
/// Breaks before a `.` are at +4, or +8 when the last segment has type arguments, which then
/// break at +4, and break whenever the chain does. The last segment's annotations, name, and
/// type arguments follow its `.`.
///
/// The qualifiers are formatted here, in a loop, rather than each by its own rule.
fn class_type<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let last = node;
    let Some(mut segment) = qualifier(node) else {
        segment_after_dot(f, node);
        return;
    };
    let mut segments = vec![node, segment];
    while let Some(qualifier) = qualifier(segment) {
        segments.push(qualifier);
        segment = qualifier;
    }
    segments.reverse();
    let uppercase = |segment: SyntaxNode<'_>| {
        name(segment)
            .and_then(|name| name.text().chars().next())
            .is_some_and(char::is_uppercase)
    };
    let (head, stay) = match segments.iter().position(|&s| uppercase(s)) {
        Some(first) => {
            let run = segments[first..]
                .iter()
                .take_while(|&&s| uppercase(s))
                .count();
            (first + run, first + run < segments.len())
        }
        None => {
            let short = name(segments[0]).is_some_and(|n| crate::document::width(n.text()) <= 4);
            (1, short)
        }
    };
    let has_arguments = ClassType::cast(last).is_some_and(|c| c.type_arguments().is_some());
    // The type's leading comments go before its group and indents.
    f.comments_before(node.first_token().id());
    if has_arguments {
        // The group the arguments break with.
        f.start_group();
    }
    f.start_indent(IndentKind::Continuation);
    if has_arguments {
        f.start_indent(IndentKind::Continuation);
    }
    segment_after_dot(f, segments[0]);
    let mut rest = segments[1..].iter().copied();
    let after_dot = |f: &mut Formatter<'t>, segment: SyntaxNode<'t>| {
        dot(f, segment);
        if segment != last {
            segment_after_dot(f, segment);
        }
    };
    f.fill(|f| {
        for segment in rest.by_ref().take(head - 1) {
            f.fill_break(true);
            after_dot(f, segment);
        }
    });
    if stay && let Some(segment) = rest.next() {
        after_dot(f, segment);
    }
    if rest.len() > 0 {
        f.group(|f| {
            for segment in rest {
                f.soft_line();
                after_dot(f, segment);
            }
        });
    }
    f.end();
    if has_arguments {
        f.end();
    }
    for child in children(last) {
        match child {
            NodeOrToken::Node(node) if node.kind() == K::TypeArguments => {
                angle_list_in(f, node, false);
            }
            _ => segment_child(f, child),
        }
    }
    if has_arguments {
        f.end();
    }
}

/// A class type segment's qualifier, its first child if that is a class type.
fn qualifier(segment: SyntaxNode<'_>) -> Option<SyntaxNode<'_>> {
    segment.first_child().filter(|c| c.kind() == K::ClassType)
}

fn name(segment: SyntaxNode<'_>) -> Option<SyntaxToken<'_>> {
    ClassType::cast(segment).and_then(|segment| segment.name())
}

/// The `.` of a qualified segment.
fn dot<'t>(f: &mut Formatter<'t>, segment: SyntaxNode<'t>) {
    if let Some(dot) = segment.direct_tokens().find(|t| t.kind() == K::Dot) {
        f.token(dot);
    }
}

/// A segment's annotations, name, and type arguments: everything after its qualifier and `.`.
fn segment_after_dot<'t>(f: &mut Formatter<'t>, segment: SyntaxNode<'t>) {
    for child in children(segment) {
        segment_child(f, child);
    }
}

/// A child of a class type segment other than its qualifier and `.`: an annotation, followed by
/// a space, the name, or the type arguments.
fn segment_child<'t>(f: &mut Formatter<'t>, child: NodeOrToken<'t>) {
    match child {
        NodeOrToken::Node(node) if node.kind() == K::ClassType => {}
        NodeOrToken::Token(token) if token.kind() == K::Dot => {}
        NodeOrToken::Node(node) if node.kind() == K::Annotation => {
            f.node(node);
            f.space();
        }
        _ => emit(f, child),
    }
}
