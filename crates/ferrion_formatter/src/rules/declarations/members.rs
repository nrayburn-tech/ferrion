//! Members: fields and variable declarators, methods and constructors, compact constructors and
//! initializers, parameters, and annotation defaults.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::annotations::{self, After, Layout};
use super::{emit, first_token, parts, take};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::lists::{Shape, delimited};
use crate::rules::variables::{
    name_part, several_declarators, single_declarator, type_then_name, value_after_eq,
};

/// The rule for a field: its annotations as [`annotations::field_layout`] says, then the type
/// and the declarators. A single declarator's name moves to the next line at +4 only if
/// `Type name =` doesn't fit, and then its value is indented from the name; several declarators
/// go one per line at +4 when they don't fit on the line, after a break after the type if the
/// first doesn't fit on the type's line (see [`several_declarators`]).
pub(super) fn field<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.comments_before(node.first_token().id());
    let parts = parts(node);
    let mut rest = parts.as_slice();
    let mut group = false;
    let mut space = false;
    if let [NodeOrToken::Node(modifiers), tail @ ..] = rest
        && modifiers.kind() == K::Modifiers
    {
        let layout = annotations::field_layout(*modifiers);
        if layout == Layout::LineAfter {
            f.start_group();
            group = true;
        }
        space = annotations::modifiers(f, *modifiers, layout) == After::Space;
        rest = tail;
    }
    if space {
        f.space();
    }
    let (declaration, end) = match rest {
        [declaration @ .., semi] if semi.kind() == K::Semi => (declaration, Some(*semi)),
        _ => (rest, None),
    };
    variables(f, declaration);
    if group {
        f.end();
    }
    // After the group, so that a line comment after the field doesn't break it.
    if let Some(end) = end {
        emit(f, end);
    }
}

/// Emits a variable declaration's type and declarators: `parts`, after its modifiers.
fn variables<'t>(f: &mut Formatter<'t>, parts: &[NodeOrToken<'t>]) {
    let [ty, declarators @ ..] = parts else {
        return;
    };
    match declarators {
        [NodeOrToken::Node(declarator)] if declarator.kind() == K::VariableDeclarator => {
            single_declarator(f, *ty, *declarator);
        }
        _ => several_declarators(f, *ty, declarators),
    }
}

/// The rule for a variable declarator, `name[] = value`, breaking after the `=` unless the value
/// is an array initializer.
pub(super) fn declarator<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut after_eq = false;
    for part in parts(node) {
        if after_eq {
            value_after_eq(f, part);
            after_eq = false;
            continue;
        }
        if part.kind() == K::Eq {
            f.space();
            after_eq = true;
            emit(f, part);
        } else {
            name_part(f, part);
        }
    }
}

/// The rule for a method or a constructor.
///
/// Its annotations go on their own lines. Then three breaks, outermost first, each taken only if
/// what it ends doesn't fit on the line: after the type parameters, then between the return type
/// and the name (`Type name(` must fit), and then in the parameters (see
/// [`Shape::Parameters`]). `throws` goes on the next line at +4 when it doesn't fit
/// on the line of the `)`, or the parameters went one per line. The parameters are indented from
/// the name when a break before it is taken.
pub(super) fn method<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.comments_before(node.first_token().id());
    let parts = parts(node);
    let mut rest = parts.as_slice();
    // Whether a space goes before what follows.
    let mut space = false;
    if let [NodeOrToken::Node(modifiers), tail @ ..] = rest
        && modifiers.kind() == K::Modifiers
    {
        space = annotations::modifiers(f, *modifiers, method_layout(node)) == After::Space;
        rest = tail;
    }
    let type_parameters = take(&mut rest, |k| k == K::TypeParameters).map(|type_parameters| {
        f.comments_before(first_token(type_parameters).id());
        let group = f.start_group();
        if space {
            f.space();
        }
        f.start_indent(IndentKind::Continuation);
        emit(f, type_parameters);
        f.line();
        group
    });
    if type_parameters.is_some() {
        space = false;
    }
    // Annotations after the type parameters, the return type, and the name.
    let name_at = rest
        .iter()
        .position(|part| part.kind() == K::Identifier)
        .unwrap_or(0);
    let (before_name, after_name) = rest.split_at(name_at);
    let (annotations, ty) = match before_name {
        [annotations @ .., ty] => (annotations, Some(*ty)),
        [] => (before_name, None),
    };
    if space {
        f.space();
    }
    for &annotation in annotations {
        emit(f, annotation);
        f.space();
    }
    let Some((name, after_name)) = after_name.split_first() else {
        return;
    };
    let named = match ty {
        Some(ty) => {
            f.comments_before(first_token(ty).id());
            Some(f.group(|f| type_then_name(f, ty, |f| emit(f, *name))))
        }
        None => {
            emit(f, *name);
            None
        }
    };
    if type_parameters.is_some() {
        // The type parameters' indent and group.
        f.end();
        f.end();
    }
    let body_at = after_name
        .iter()
        .position(|part| matches!(part.kind(), K::Block | K::Semi | K::DefaultValue))
        .unwrap_or(after_name.len());
    let (header, body) = after_name.split_at(body_at);
    let tail = |f: &mut Formatter<'t>| method_tail(f, header);
    match (type_parameters, named) {
        (Some(outer), Some(inner)) => {
            f.indent_if_broken(outer, |f| f.indent_if_broken(inner, tail));
        }
        (Some(group), None) | (None, Some(group)) => f.indent_if_broken(group, tail),
        (None, None) => tail(f),
    }
    // The body is indented from the header's first line, outside the header's indents.
    for &part in body {
        if part.kind() != K::Semi {
            f.space();
        }
        emit(f, part);
    }
}

/// The layout of a method's or a constructor's modifiers: a constructor has no type for type-use
/// annotations to stay next to.
fn method_layout(node: SyntaxNode<'_>) -> Layout {
    if node.kind() == K::MethodDecl {
        Layout::BeforeType
    } else {
        Layout::OwnLines
    }
}

/// Emits a method header from its parameters on, before its body: `header`. `throws` goes on the
/// next line at +4 when the parameters went one per line, and otherwise when it doesn't fit on
/// the line of the `)`.
fn method_tail<'t>(f: &mut Formatter<'t>, header: &[NodeOrToken<'t>]) {
    let mut parameters = None;
    for &part in header {
        match part {
            NodeOrToken::Node(list) if list.kind() == K::FormalParameters => {
                parameters = delimited(f, list, Shape::Parameters);
            }
            NodeOrToken::Node(clause) if clause.kind() == K::ThrowsClause => {
                let throws = |f: &mut Formatter<'t>| {
                    f.indent(IndentKind::Continuation, |f| {
                        f.line();
                        emit(f, part);
                    });
                };
                match parameters {
                    Some(parameters) => {
                        f.group_breaking_with(parameters, throws);
                    }
                    None => {
                        f.group(throws);
                    }
                }
            }
            // The dimensions after the parameters of an old-style array method, with no spaces.
            _ => emit(f, part),
        }
    }
}

/// The rule for a compact constructor or an initializer: its annotations on their own lines, then
/// its tokens and body with a space between them.
pub(super) fn spaced<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut space = false;
    for part in parts(node) {
        match part {
            NodeOrToken::Node(modifiers) if modifiers.kind() == K::Modifiers => {
                space = annotations::modifiers(f, modifiers, Layout::OwnLines) == After::Space;
                continue;
            }
            _ => {}
        }
        if space {
            f.space();
        }
        emit(f, part);
        space = true;
    }
}

/// The rule for a formal parameter, a receiver parameter, or a record component: on one line if
/// it fits. Otherwise its leading annotations break first, as a group, one per line at +4 if they
/// don't fit together, and then the name, onto the next line at +4.
pub(super) fn parameter<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.comments_before(node.first_token().id());
    let parts = parts(node);
    let mut rest = parts.as_slice();
    let mut outer = false;
    if let [NodeOrToken::Node(modifiers), tail @ ..] = rest
        && modifiers.kind() == K::Modifiers
    {
        rest = tail;
        if annotations::leading_annotations(*modifiers).is_empty() {
            annotations::modifiers(f, *modifiers, Layout::Inline);
            f.space();
        } else {
            // The modifiers leave an indent open for the rest of the parameter.
            outer = true;
            f.start_group();
            if annotations::modifiers(f, *modifiers, Layout::Parameter) == After::Space {
                f.space();
            }
        }
    }
    if let [ty, rest @ ..] = rest {
        let mut rest = rest;
        let varargs = take(&mut rest, |k| k == K::Varargs);
        let ty_and_varargs = |f: &mut Formatter<'t>| {
            emit(f, *ty);
            if let Some(varargs) = varargs {
                if varargs
                    .as_node()
                    .is_some_and(|v| v.first_token().kind() != K::Ellipsis)
                {
                    f.space();
                }
                emit(f, varargs);
            }
        };
        f.comments_before(first_token(*ty).id());
        if rest.is_empty() {
            ty_and_varargs(f);
        } else {
            f.group(|f| {
                f.indent(IndentKind::Continuation, |f| {
                    ty_and_varargs(f);
                    f.line();
                    rest.iter().for_each(|&part| emit(f, part));
                });
            });
        }
    }
    if outer {
        f.end();
        f.end();
    }
}

/// The rule for an annotation element's default value: `default` and the value, which breaks
/// onto the next line at +4 as a variable's value does after its `=`.
pub(super) fn default_value<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    let [keyword, rest @ ..] = parts.as_slice() else {
        return;
    };
    emit(f, *keyword);
    match rest {
        [value] => value_after_eq(f, *value),
        _ => rest.iter().for_each(|&part| {
            f.space();
            emit(f, part);
        }),
    }
}
