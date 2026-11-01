//! The compilation unit, packages, imports, names, and `module-info.java`.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::annotations::{own_lines, with_type_use};
use super::{emit, first_token, parts, types};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::rules::space_between;

/// The rule for a compilation unit: a blank line after the package declaration and after the
/// imports, the imports in input order with the input's blank lines between them, and the
/// types and members with the blank lines of a class body between them.
pub(super) fn compilation_unit<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    with_type_use(node, || {
        let mut previous: Option<NodeOrToken<'t>> = None;
        let mut previous_spaced = false;
        for part in parts(node) {
            let kind = part.kind();
            if kind == K::Eof {
                emit(f, part);
                break;
            }
            let spaced = types::spaced_member(part);
            match previous.map(|p| p.kind()) {
                None => {}
                Some(K::PackageDecl) => f.blank_line(),
                Some(K::ImportDecl) if kind == K::ImportDecl => {
                    f.line_break_before(first_token(part));
                }
                Some(K::ImportDecl) => f.blank_line(),
                Some(_) => types::member_break(f, previous_spaced, part, spaced),
            }
            emit(f, part);
            previous = Some(part);
            previous_spaced = spaced;
        }
    });
}

/// The rule for a package declaration: its annotations on their own lines, then the declaration,
/// which never breaks.
pub(super) fn package<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut previous = None;
    for part in parts(node) {
        match part {
            NodeOrToken::Node(annotation) if annotation.kind() == K::Annotation => {
                own_lines(f, annotation);
                continue;
            }
            _ => {}
        }
        if let Some(previous) = previous
            && space_between(super::last_token(previous), first_token(part))
        {
            f.space();
        }
        emit(f, part);
        previous = Some(part);
    }
}

/// The rule for an import or a name: its tokens with a space only where the first rule puts one,
/// as in `import static a.B.c;`. Neither ever breaks.
pub(super) fn tokens<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut previous = None;
    for part in parts(node) {
        if let Some(previous) = previous
            && space_between(super::last_token(previous), first_token(part))
        {
            f.space();
        }
        emit(f, part);
        previous = Some(part);
    }
}

/// The rule for a module declaration: its annotations on their own lines, the header, and the
/// directives one per line, with a blank line between directives of different kinds and none
/// between directives of the same kind, whatever the input has.
pub(super) fn module<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    let directives = parts.iter().filter(|p| is_directive(p.kind())).count();
    let mut previous: Option<NodeOrToken<'t>> = None;
    let mut previous_directive: Option<K> = None;
    for part in parts {
        let kind = part.kind();
        match part {
            NodeOrToken::Node(annotation) if kind == K::Annotation => {
                own_lines(f, annotation);
                continue;
            }
            _ => {}
        }
        if is_directive(kind) {
            match previous_directive {
                None => f.hard_line(),
                Some(k) if k == kind => {
                    f.no_blank_line();
                    f.hard_line();
                }
                Some(_) => f.blank_line(),
            }
            previous_directive = Some(kind);
        } else if kind == K::RBrace {
            if directives > 0 || has_comments_before(first_token(part)) {
                f.comments_before(first_token(part).id());
                f.end();
                f.hard_line();
            } else {
                f.end();
            }
        } else if previous.is_some() {
            f.space();
        }
        emit(f, part);
        if kind == K::LBrace {
            f.start_indent(IndentKind::Block);
        }
        previous = Some(part);
    }
}

fn is_directive(kind: K) -> bool {
    matches!(
        kind,
        K::RequiresDirective
            | K::ExportsDirective
            | K::OpensDirective
            | K::UsesDirective
            | K::ProvidesDirective
    )
}

/// Whether the trivia before `token` holds a comment.
fn has_comments_before(token: ferrion_syntax::SyntaxToken<'_>) -> bool {
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

/// The rule for a module directive: its tokens on one line, except that the modules after `to`
/// and the classes after `with` go one per line at +4, always.
pub(super) fn directive<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut previous: Option<K> = None;
    let mut list = false;
    for part in parts(node) {
        let kind = part.kind();
        match previous {
            None => {}
            Some(K::ToKw | K::WithKw) => {
                f.start_indent(IndentKind::Continuation);
                list = true;
                f.hard_line();
            }
            Some(K::Comma) if list => f.hard_line(),
            Some(_) if matches!(kind, K::Semi | K::Comma) => {}
            Some(_) => f.space(),
        }
        if kind == K::Semi && list {
            f.end();
            list = false;
        }
        emit(f, part);
        previous = Some(kind);
    }
    if list {
        f.end();
    }
}
