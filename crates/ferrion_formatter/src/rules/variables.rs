//! What declarations of variables share, in every area: `Type name`, which breaks before the
//! name, for fields, locals, parameters, methods, and `catch` parameters; and the value after a
//! declarator's `=`.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::expressions::Indentation;
use super::lists::separated;
use super::{children, emit, first_token};
use crate::document::IndentKind;
use crate::format::Formatter;

/// Emits `ty`, a line break at +4, and what `name` emits, in the caller's group. The type is in
/// the name's indent, so that its own breaks are indented from the name's.
pub(crate) fn type_then_name<'t>(
    f: &mut Formatter<'t>,
    ty: NodeOrToken<'t>,
    name: impl FnOnce(&mut Formatter<'t>),
) {
    f.indent(IndentKind::Continuation, |f| {
        emit(f, ty);
        f.line();
        name(f);
    });
}

/// Emits a type and its one declarator: `Type name` in a group that breaks before the name, then
/// `= value`, indented from the name if that group broke.
pub(crate) fn single_declarator<'t>(
    f: &mut Formatter<'t>,
    ty: NodeOrToken<'t>,
    declarator: SyntaxNode<'t>,
) {
    let parts: Vec<_> = children(declarator).collect();
    let eq = parts.iter().position(|part| part.kind() == K::Eq);
    let (name, value) = parts.split_at(eq.unwrap_or(parts.len()));
    // Comments before the type go before the group, so that a line comment doesn't break it.
    f.comments_before(first_token(ty).id());
    let group = f.group(|f| {
        type_then_name(f, ty, |f| {
            for &part in name {
                name_part(f, part);
            }
        });
    });
    if let [eq, value @ ..] = value {
        f.indent_if_broken(group, |f| {
            f.space();
            emit(f, *eq);
            match value {
                [value] => value_after_eq(f, *value),
                _ => value.iter().for_each(|&part| emit(f, part)),
            }
        });
    }
}

/// Emits a type and several declarators, `rest`, with the commas between them: one to a line at
/// +4 when they don't fit on one, and first a line break at +4 between the type and the first
/// declarator when the type and the whole first declarator up to its comma don't fit (observed
/// in GJF: `Method` then `A =` at +4 and its value at +8, then `B = ...` at +4).
pub(crate) fn several_declarators<'t>(
    f: &mut Formatter<'t>,
    ty: NodeOrToken<'t>,
    rest: &[NodeOrToken<'t>],
) {
    emit(f, ty);
    f.indent(IndentKind::Continuation, |f| {
        f.group(|f| {
            let comma = rest
                .iter()
                .position(|part| part.kind() == K::Comma)
                .unwrap_or(rest.len());
            let (first, rest) = rest.split_at(comma);
            // The break after the type measures the first declarator and its comma, which
            // stays out of the group so that a comment after it doesn't break the type.
            f.group(|f| {
                f.line();
                first.iter().for_each(|&part| emit(f, part));
            });
            separated(f, rest);
        });
    });
}

/// Emits a part of a declarator before its `=`: the name, or a dimension, which is spaced from
/// what comes before it when it has annotations, as in a type (`g @A []`).
pub(crate) fn name_part<'t>(f: &mut Formatter<'t>, part: NodeOrToken<'t>) {
    if let NodeOrToken::Node(dim) = part
        && dim.kind() == K::Dim
        && dim.first_child().is_some()
    {
        f.space();
    }
    emit(f, part);
}

/// Emits the value after an `=`, in a variable declarator, an element-value pair, or an
/// annotation element's default: on the line if it's an array initializer, whose braces break
/// instead, and otherwise in a group that breaks before it, at +4 (observed in GJF, as for an
/// assignment, whose group also holds what it assigns to; see [`assigned_value`]).
pub(crate) fn value_after_eq<'t>(f: &mut Formatter<'t>, value: NodeOrToken<'t>) {
    if matches!(value.kind(), K::ArrayInitializer | K::ElementValueArray) {
        f.space();
        emit(f, value);
    } else {
        f.group(|f| line_then_value(f, value, Indentation::uncapped));
    }
}

/// Emits an assignment's value, after its operator: a line break of the assignment's group,
/// which the caller has open around the target too, and the value at +4. GJF breaks after the
/// operator when the target doesn't fit on its line either. The indent is one of the expression
/// rules' capped ones, since assignments chain (docs/style.md, "Expressions").
pub(crate) fn assigned_value<'t>(f: &mut Formatter<'t>, value: NodeOrToken<'t>) {
    line_then_value(f, value, Indentation::start);
}

fn line_then_value<'t>(
    f: &mut Formatter<'t>,
    value: NodeOrToken<'t>,
    indent: fn(&mut Formatter<'_>, IndentKind) -> Indentation,
) {
    let indentation = indent(f, IndentKind::Continuation);
    f.line();
    emit(f, value);
    indentation.end(f);
}
