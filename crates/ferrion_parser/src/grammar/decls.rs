//! Modifiers, annotations, type declarations, and members (`java.grammar`: "Modifiers and
//! annotations", "Type declarations and members").
//!
//! Not written yet, apart from what package declarations need. Stubs keep the signatures other
//! areas call.

// Stubs no other area calls yet.
#![allow(dead_code)]

use crate::SyntaxKind::*;
use crate::diagnostic::Expected;
use crate::grammar::{name, todo};
use crate::parser::{CompletedMarker, Marker, Parser, TokenSet};
use ferrion_syntax::token_sets;

/// Everything after the imports: a module declaration (`modules::module_decl`), or type
/// declarations and, for an implicitly declared class, methods and fields. Consumes every token
/// up to `Eof`.
///
/// TEMPORARY: puts every token up to `Eof` in one `Unparsed` node, without a diagnostic.
pub(crate) fn compilation_unit_items(p: &mut Parser<'_>) {
    if p.at_eof() {
        return;
    }
    let m = p.start();
    while !p.at_eof() {
        p.bump();
    }
    m.complete(p, Unparsed);
}

/// Modifier keywords and annotations, as a `Modifiers` node, or `None` if there are none.
///
/// TODO: `sealed` and `non-sealed`, whose lookahead is settled with the declaration grammar.
pub(crate) fn modifiers(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    const KEYWORDS: TokenSet = TokenSet::new(token_sets::MODIFIER);
    let m = p.start();
    let mut progress = p.progress();
    loop {
        if p.at(At) && p.nth(1) != InterfaceKw {
            annotation(p);
        } else if p.at_set(KEYWORDS) && !p.at(SealedKw) && !p.at(NonSealedKw) {
            p.bump();
        } else {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    if m.consumed(p) {
        Some(m.complete(p, Modifiers))
    } else {
        m.abandon(p);
        None
    }
}

/// An annotation, at `@`: `@A`, `@a.B(1)`, `@A(x = 1, y = {2})`.
pub(crate) fn annotation(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.expect(At);
    if name(p).is_none() {
        p.error_expected(Expected::Identifier);
    }
    if p.at(LParen) {
        // Element values can be annotations, so this recurses.
        p.nested(|p| {
            annotation_arguments(p);
            Some(())
        });
    }
    m.complete(p, Annotation)
}

/// `(...)` after an annotation's name.
///
/// TEMPORARY: parses `(`, element values separated by commas through [`element_value`], and `)`,
/// without element-value pairs.
fn annotation_arguments(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    let mut progress = p.progress();
    while !p.at_eof() && !p.at(RParen) {
        if element_value(p).is_none() || !p.eat(Comma) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    p.expect(RParen);
    m.complete(p, AnnotationArguments);
}

/// An annotation's element value: an expression, an annotation, or `{...}`.
///
/// Stub.
pub(crate) fn element_value(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// A class body, at `{`, for anonymous classes and enum constants as well as declarations.
///
/// Stub.
pub(crate) fn class_body(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// Whether a local class, interface, enum, record, or annotation type starts at the current
/// token, after any modifiers (for block statements).
///
/// Stub.
pub(crate) fn at_type_decl(_p: &Parser<'_>) -> bool {
    false
}

/// A type declaration after its modifiers; `m` was started before them.
///
/// Stub.
pub(crate) fn type_decl(p: &mut Parser<'_>, m: Marker) -> Option<CompletedMarker> {
    m.abandon(p);
    todo(p)
}

/// `a = 1, b[] = {2}`: one or more `VariableDeclarator`s separated by commas, for fields and
/// local variables.
///
/// Stub.
pub(crate) fn variable_declarators(p: &mut Parser<'_>) {
    todo(p);
}
