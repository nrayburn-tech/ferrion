//! Type declarations: their headers, `extends`, `implements`, and `permits` clauses, class and
//! enum bodies, and enum constants.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken};

use super::annotations::{self, After, Layout};
use super::{
    doc_comment_before, emit, first_token, fits_on_line, leads_on_separator_line, parts, separated,
};
use crate::document::IndentKind;
use crate::format::Formatter;

/// The rule for a class, interface, enum, record, or annotation type declaration.
///
/// The header's annotations go on their own lines. Each clause (`extends`, `implements`,
/// `permits`) moves to the next line at +4 only if it doesn't fit on the line it would continue,
/// so `class A extends B` can stay together with `implements C` below it; a class's type
/// parameters break inside the first clause's indentation. A record's `implements` goes below
/// its components when they break. The body follows the header's last line.
pub(super) fn type_decl<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    let mut rest = parts.as_slice();
    let mut space = false;
    if let [NodeOrToken::Node(modifiers), tail @ ..] = rest
        && modifiers.kind() == K::Modifiers
    {
        space = annotations::modifiers(f, *modifiers, Layout::OwnLines) == After::Space;
        rest = tail;
    }
    // The keywords and the name; `@interface` has no space after the `@`.
    let mut previous: Option<SyntaxToken<'t>> = None;
    while let [NodeOrToken::Token(token), tail @ ..] = rest {
        if previous.map_or(space, |p| p.kind() != K::At) {
            f.space();
        }
        f.token(*token);
        previous = Some(*token);
        rest = tail;
    }
    let type_parameters = take(&mut rest, |k| k == K::TypeParameters);
    let header = take(&mut rest, |k| k == K::RecordHeader);
    let mut clauses = Vec::new();
    while let Some(clause) = take(&mut rest, |k| {
        matches!(k, K::ExtendsClause | K::ImplementsClause | K::PermitsClause)
    }) {
        clauses.push(clause);
    }
    match (node.kind(), header) {
        (K::RecordDecl, Some(header)) => {
            if let Some(type_parameters) = type_parameters {
                emit(f, type_parameters);
            }
            if clauses.is_empty() {
                emit(f, header);
            } else {
                f.group(|f| {
                    emit(f, header);
                    f.indent(IndentKind::Continuation, |f| {
                        for &clause in &clauses {
                            f.line();
                            emit(f, clause);
                        }
                    });
                });
            }
        }
        _ => {
            if let Some(header) = header {
                // A record header outside a record: not in a tree without errors.
                emit(f, header);
            }
            let mut clauses = clauses.into_iter();
            match (type_parameters, clauses.next()) {
                (type_parameters, Some(first)) => {
                    f.group(|f| {
                        f.indent(IndentKind::Continuation, |f| {
                            if let Some(type_parameters) = type_parameters {
                                emit(f, type_parameters);
                            }
                            f.line();
                            emit(f, first);
                        });
                    });
                }
                (Some(type_parameters), None) => emit(f, type_parameters),
                (None, None) => {}
            }
            for clause in clauses {
                f.group(|f| {
                    f.indent(IndentKind::Continuation, |f| {
                        f.line();
                        emit(f, clause);
                    });
                });
            }
        }
    }
    for &part in rest {
        f.space();
        emit(f, part);
    }
}

/// Takes the first of `rest` if its kind passes `kind`.
fn take<'t>(rest: &mut &[NodeOrToken<'t>], kind: impl Fn(K) -> bool) -> Option<NodeOrToken<'t>> {
    match rest {
        [first, tail @ ..] if kind(first.kind()) => {
            let first = *first;
            *rest = tail;
            Some(first)
        }
        _ => None,
    }
}

/// The rule for an `extends`, `implements`, or `permits` clause: the keyword, then the types on
/// its line, breaking one per line at +4 when they don't fit. An enum's `implements` breaks
/// differently (observed in GJF): the types move to the next line at +4 together, and fill the
/// lines from there.
pub(super) fn clause<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    let [keyword, items @ ..] = parts.as_slice() else {
        return;
    };
    emit(f, *keyword);
    if items.is_empty() {
        return;
    }
    let enum_clause = node.parent().is_some_and(|p| p.kind() == K::EnumDecl);
    if enum_clause {
        f.group(|f| {
            f.indent(IndentKind::Continuation, |f| {
                f.line();
                f.fill(|f| {
                    let mut after_comma = false;
                    for &part in items {
                        if part.kind() == K::Comma {
                            emit(f, part);
                            after_comma = true;
                            continue;
                        }
                        if after_comma {
                            f.fill_break(false);
                        }
                        emit(f, part);
                        after_comma = false;
                    }
                });
            });
        });
    } else if items.len() == 1 {
        f.space();
        emit(f, items[0]);
    } else {
        f.space();
        f.group(|f| f.indent(IndentKind::Continuation, |f| separated(f, items, false)));
    }
}

/// The rule for a class body: `{}` when empty, otherwise the members on their own lines at +2,
/// with the blank lines [`member_break`] gives between them. A blank line at the start of the
/// body is kept, and one at the end is removed.
pub(super) fn class_body<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    let [open, members @ .., close] = parts.as_slice() else {
        parts.into_iter().for_each(|part| emit(f, part));
        return;
    };
    let close_token = first_token(*close);
    emit(f, *open);
    if members.is_empty() && !has_comments_before(close_token) {
        emit(f, *close);
        return;
    }
    f.start_indent(IndentKind::Block);
    if members.is_empty() && block_comment_after(first_token(*open)) {
        // A block comment alone in the body goes on a line of its own; a line comment stays on
        // the line of the `{`.
        f.hard_line();
    }
    let mut previous_spaced = None;
    for &member in members {
        let spaced = spaced_member(member);
        match previous_spaced {
            None => {
                let first = first_token(member);
                f.line_break_before(first);
                // A block comment on the line of the `{` goes on a line of its own.
                if leads_on_separator_line(first) && !f.in_region() {
                    f.comments_before(first.id());
                    f.hard_line();
                }
            }
            Some(previous) => member_break(f, previous, member, spaced),
        }
        emit(f, member);
        previous_spaced = Some(spaced);
    }
    close_body(f, close_token);
    emit(f, *close);
}

/// Whether the first thing after `token` other than whitespace is a block comment on its line.
fn block_comment_after(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    while let Some(t) = next
        && t.kind() == K::Whitespace
    {
        next = t.next_token();
    }
    next.is_some_and(|t| matches!(t.kind(), K::BlockComment | K::DocComment))
}

/// Ends a body's indent before `close`, its `}`, with the comments before it inside.
fn close_body<'t>(f: &mut Formatter<'t>, close: SyntaxToken<'t>) {
    f.comments_before(close.id());
    f.end();
    f.hard_line();
}

/// Whether the trivia before `token` holds a comment.
fn has_comments_before(token: SyntaxToken<'_>) -> bool {
    let mut previous = token.prev_token();
    while let Some(t) = previous
        && t.is_trivia()
    {
        if ferrion_syntax::comments::is_comment(t.kind()) {
            return true;
        }
        previous = t.prev_token();
    }
    false
}

/// The line break before `next`, a member, after a member that [`spaced_member`] says is
/// `previous_spaced`. GJF puts a blank line before and after a member other than a plain field
/// (see [`spaced_member`]), but not before an empty declaration, `;`. Between two plain fields,
/// and next to `;`, the input's choice is kept.
pub(super) fn member_break<'t>(
    f: &mut Formatter<'t>,
    previous_spaced: bool,
    next: NodeOrToken<'t>,
    next_spaced: bool,
) {
    if next_spaced || previous_spaced && next.kind() != K::Semi {
        f.blank_line();
    } else {
        f.line_break_before(first_token(next));
    }
}

/// Whether GJF puts blank lines around a member: any member but a field and `;`, and a field with
/// a doc comment or with annotations on their own lines.
pub(super) fn spaced_member(member: NodeOrToken<'_>) -> bool {
    match member {
        NodeOrToken::Token(_) => false,
        NodeOrToken::Node(field) if field.kind() == K::FieldDecl => {
            doc_comment_before(field.first_token())
                || field.first_child().is_some_and(|modifiers| {
                    modifiers.kind() == K::Modifiers
                        && match annotations::field_layout(modifiers) {
                            Layout::OwnLines | Layout::BeforeType => true,
                            // A line comment that breaks the field doesn't make GJF space it (observed).
                            Layout::LineAfter => !fits_on_line(field, false),
                            _ => false,
                        }
                })
        }
        NodeOrToken::Node(_) => true,
    }
}

/// The rule for an enum body: the constants one per line, the `;` after the last of them unless
/// it follows a trailing comma, and then the members as in a class body. A blank line at the
/// start or the end is removed; blank lines between constants are kept.
pub(super) fn enum_body<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    let [open, items @ .., close] = parts.as_slice() else {
        parts.into_iter().for_each(|part| emit(f, part));
        return;
    };
    let close_token = first_token(*close);
    emit(f, *open);
    if items.is_empty() && !has_comments_before(close_token) {
        emit(f, *close);
        return;
    }
    f.start_indent(IndentKind::Block);
    f.hard_line();
    let mut constants = true;
    let mut previous: Option<NodeOrToken<'t>> = None;
    let mut previous_spaced = false;
    for &part in items {
        let kind = part.kind();
        match kind {
            K::EnumConstant if constants => {
                if previous.is_some() {
                    let first = first_token(part);
                    if leads_on_separator_line(first) && !f.in_region() {
                        f.comments_before(first.id());
                        f.hard_line();
                    } else {
                        f.line_break_before(first);
                    }
                }
                emit(f, part);
            }
            K::Comma if constants => emit(f, part),
            K::Semi if constants => {
                if previous.is_none_or(|p| p.kind() == K::Comma) {
                    f.hard_line();
                }
                emit(f, part);
                constants = false;
            }
            _ => {
                // The constants end with a `;` before any member, in a tree without errors.
                constants = false;
                let spaced = spaced_member(part);
                member_break(f, previous_spaced, part, spaced);
                emit(f, part);
                previous_spaced = spaced;
            }
        }
        previous = Some(part);
    }
    close_body(f, close_token);
    emit(f, *close);
}

/// The rule for an enum constant: its annotations on their own lines, its name, its arguments,
/// and its body.
pub(super) fn enum_constant<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut space = false;
    for part in parts(node) {
        match part {
            NodeOrToken::Node(modifiers) if modifiers.kind() == K::Modifiers => {
                space = annotations::modifiers(f, modifiers, Layout::OwnLines) == After::Space;
                continue;
            }
            _ => {}
        }
        if space || part.kind() == K::ClassBody {
            f.space();
        }
        emit(f, part);
        space = false;
    }
}
