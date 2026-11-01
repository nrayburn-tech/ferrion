//! Types, type arguments, and type parameters (`java.grammar`: "Types").
//!
//! A type is parsed only where the syntax says one comes (decision 2 in `java.grammar`). Where a
//! type or an expression could come (a cast or a parenthesized expression, a local variable or an
//! expression statement), callers decide with [`scan_type`], which looks ahead without consuming
//! anything.
//!
//! The lexer splits `>>` and `>>>` into `>` tokens (decision 15), so nested type arguments close
//! one `>` at a time.

// Functions for areas not written yet.
#![allow(dead_code)]

#[cfg(test)]
pub(crate) mod tests;

use ferrion_diagnostics::Span;

use crate::SyntaxKind::*;
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::grammar::{IDENT, decls};
use crate::parser::{CompletedMarker, Marker, Parser, TokenSet};
use ferrion_syntax::token_sets;

/// `boolean`, `byte`, `char`, `short`, `int`, `long`, `float`, `double`.
pub(crate) const PRIMITIVE: TokenSet = TokenSet::new(token_sets::PRIMITIVE_TYPE_KEYWORD);

/// A type, with its leading annotations and any array dimensions: a `PrimitiveType`,
/// `ClassType`, `ArrayType`, or `VoidType`. `None`, consuming nothing, if no type starts here.
pub(crate) fn type_(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !at_annotation(p) && !p.at_set(PRIMITIVE) && !p.at(VoidKw) && !p.at_set(IDENT) {
        return None;
    }
    let m = p.start();
    annotations(p);
    let element = if p.at_set(PRIMITIVE) {
        p.bump();
        m.complete(p, PrimitiveType)
    } else if p.at(VoidKw) {
        p.bump();
        let void = m.complete(p, VoidType);
        if at_dim(p) {
            // `void[]`: javac takes `void` alone as a type, and expects a name after it.
            p.error_expected(Expected::Identifier);
        }
        void
    } else {
        class_type_rest(p, m, false)?
    };
    Some(array_type_rest(p, element))
}

/// A class or interface type, for `extends`, `implements`, `permits`, `throws`, and `new`: a
/// `ClassType` per segment, without dimensions.
pub(crate) fn class_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !at_annotation(p) && !p.at_set(IDENT) {
        return None;
    }
    let m = p.start();
    annotations(p);
    class_type_rest(p, m, false)
}

/// A type, or `A | B` as a `UnionType`, for a catch clause's parameter.
pub(crate) fn union_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let first = type_(p)?;
    Some(type_list_rest(p, first, Pipe, UnionType))
}

/// A type, or `A & B` as an `IntersectionType`, for a cast.
pub(crate) fn intersection_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let first = type_(p)?;
    Some(type_list_rest(p, first, Amp, IntersectionType))
}

/// After a type: `separator` and another type, any number of times, in a `kind` node.
fn type_list_rest(
    p: &mut Parser<'_>,
    first: CompletedMarker,
    separator: crate::SyntaxKind,
    kind: crate::SyntaxKind,
) -> CompletedMarker {
    if !p.at(separator) {
        return first;
    }
    let m = first.precede(p);
    let mut progress = p.progress();
    while p.at(separator) {
        p.bump();
        if type_(p).is_none() {
            type_expected(p);
        }
        if !progress.advanced(p) {
            break;
        }
    }
    m.complete(p, kind)
}

/// The rest of a class type, from its first name, with `m` started before the annotations
/// already consumed: a `ClassType` for each segment, each with its annotations and type
/// arguments. `diamond` allows `<>`, for `new`. `None`, consuming nothing, if there are no
/// annotations and no name.
pub(super) fn class_type_rest(
    p: &mut Parser<'_>,
    m: Marker,
    diamond: bool,
) -> Option<CompletedMarker> {
    if p.at_set(IDENT) {
        p.bump();
        if p.at(Lt) {
            type_arguments_with(p, diamond);
        }
    } else if m.consumed(p) {
        type_expected(p);
    } else {
        m.abandon(p);
        return None;
    }
    let mut segment = m.complete(p, ClassType);
    let mut progress = p.progress();
    // `.this` follows a receiver parameter's type; anything else after a `.` is a segment.
    while p.at(Dot) && p.nth(1) != ThisKw {
        let m = segment.precede(p);
        p.bump();
        annotations(p);
        if p.at_set(IDENT) {
            p.bump();
            if p.at(Lt) {
                type_arguments_with(p, diamond);
            }
        } else if p.at(Lt) {
            type_expected(p);
        } else {
            p.error_expected(Expected::Identifier);
        }
        segment = m.complete(p, ClassType);
        if !progress.advanced(p) {
            break;
        }
    }
    Some(segment)
}

fn complete_or_abandon(
    p: &mut Parser<'_>,
    m: Marker,
    kind: crate::SyntaxKind,
) -> Option<CompletedMarker> {
    if m.consumed(p) {
        Some(m.complete(p, kind))
    } else {
        m.abandon(p);
        None
    }
}

/// After an element type: its dimensions, in an `ArrayType`, if it has any.
pub(super) fn array_type_rest(p: &mut Parser<'_>, element: CompletedMarker) -> CompletedMarker {
    let ty = if at_dim(p) {
        let m = element.precede(p);
        dims(p);
        m.complete(p, ArrayType)
    } else {
        element
    };
    // Annotations after a type belong to a dimension or to varargs (`String @A ... a`); javac
    // reports others where they start: `int @A x`.
    if at_annotation(p) && p.nth(annotations_end(p, 0)) != Ellipsis {
        type_expected(p);
    }
    ty
}

/// Whether a `Dim` starts here: `[`, or annotations followed by `[`.
fn at_dim(p: &Parser<'_>) -> bool {
    p.nth(annotations_end(p, 0)) == LBracket
}

/// Zero or more `Dim`s (`@A []`), as children of the node being parsed: C-style dimensions after a
/// declarator's or parameter's name or a method's parameters, and an array type's. A `[` must be
/// followed by `]`.
pub(crate) fn dims(p: &mut Parser<'_>) {
    let mut progress = p.progress();
    while at_dim(p) {
        let m = p.start();
        annotations(p);
        p.bump();
        if !p.eat(RBracket) {
            p.error_expected(Expected::Token(RBracket));
        }
        m.complete(p, Dim);
        if !progress.advanced(p) {
            break;
        }
    }
}

/// Annotations, as children of the node being parsed, while the current token is `@` (and not
/// `@interface`). Returns whether there were any.
pub(super) fn annotations(p: &mut Parser<'_>) -> bool {
    let mut any = false;
    let mut progress = p.progress();
    while at_annotation(p) {
        decls::annotation(p);
        any = true;
        if !progress.advanced(p) {
            break;
        }
    }
    any
}

pub(super) fn at_annotation(p: &Parser<'_>) -> bool {
    p.at(At) && p.nth(1) != InterfaceKw
}

/// `<A, B>`, at `<`. `<>` is reported: a diamond is allowed only after `new`.
pub(crate) fn type_arguments(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at(Lt) {
        return None;
    }
    Some(type_arguments_with(p, false))
}

/// `<A, B>`, or with `diamond`, also `<>`, at `<`.
pub(super) fn type_arguments_with(p: &mut Parser<'_>, diamond: bool) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if p.at(Gt) {
        if !diamond {
            type_expected(p);
        }
        p.bump();
        return m.complete(p, TypeArguments);
    }
    let mut progress = p.progress();
    loop {
        // Type arguments nest.
        if p.nested(type_argument).is_none() {
            type_expected(p);
        }
        if !p.eat(Comma) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    if !p.eat(Gt) {
        expected_at_next(p, Expected::OneOf(&[Gt, Comma]));
    }
    m.complete(p, TypeArguments)
}

/// A type or a `Wildcard`.
fn type_argument(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if p.nth(annotations_end(p, 0)) != Question {
        return type_(p);
    }
    let m = p.start();
    annotations(p);
    p.bump();
    if p.at(ExtendsKw) || p.at(SuperKw) {
        p.bump();
        if type_(p).is_none() {
            type_expected(p);
        }
    }
    Some(m.complete(p, Wildcard))
}

/// `<T extends A & B, U>`, at `<`.
pub(crate) fn type_parameters(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at(Lt) {
        return None;
    }
    let m = p.start();
    p.bump();
    let mut progress = p.progress();
    loop {
        let parameter = p.start();
        annotations(p);
        if p.at_set(IDENT) {
            p.bump();
        } else {
            p.error_expected(Expected::Identifier);
        }
        if p.eat(ExtendsKw) {
            let mut bounds = p.progress();
            loop {
                if p.nested(type_).is_none() {
                    type_expected(p);
                }
                if !p.eat(Amp) || !bounds.advanced(p) {
                    break;
                }
            }
        }
        complete_or_abandon(p, parameter, TypeParameter);
        if !p.eat(Comma) || !progress.advanced(p) {
            break;
        }
    }
    // A `>` directly followed by another closes nothing: for `<T extends A<B>>>`, javac reports
    // `>` expected at the second `>` (type-parameters-closed-by-a-shift.java in the oracle's
    // parser cases).
    if p.at(Gt) && !(p.nth_is_adjacent(1) && matches!(p.nth(1), Gt | GtEq)) {
        p.bump();
    } else {
        p.error_expected(Expected::Token(Gt));
    }
    Some(m.complete(p, TypeParameters))
}

/// Reports that `what` is missing, anchored at the start of the current token rather than after
/// the previous one, as javac does in a few places: after a type argument (`'>' or ','`), and
/// after `new X` (`'(' or '['`, or for `outer.new X`, `'('`).
pub(super) fn expected_at_next(p: &mut Parser<'_>, what: Expected) {
    let span = p.nth_span(0);
    p.error(
        ParseDiagnosticKind::Expected(what),
        Span::empty(span.start),
        span.start,
    );
}

/// Reports that a type is missing at the current token, without consuming it.
pub(super) fn type_expected(p: &mut Parser<'_>) {
    let span = p.nth_span(0);
    p.error(
        ParseDiagnosticKind::TypeExpected(p.current()),
        span,
        span.start,
    );
}

// ----- Lookahead -----

/// What [`scan_type_info`] found.
#[derive(Clone, Copy, Debug)]
pub(super) struct TypeScan {
    /// The index of the token after the type.
    pub end: usize,
    /// A name, qualified or not, with no annotations, type arguments, or dimensions, which could
    /// as well be an expression.
    pub name_only: bool,
    /// A primitive type without dimensions.
    pub primitive: bool,
    /// It has dimensions.
    pub dims: bool,
    /// Its last token is the `>` of type arguments, so it could also be the start of a
    /// comparison: `a < b > c`.
    pub ends_with_type_arguments: bool,
}

/// Lookahead without consuming anything: if a type starts at the `n`th token ahead, the index of
/// the token after it. For the decisions that depend on whether a type comes next: local
/// variable or expression statement, cast or parenthesized expression, generic method reference.
///
/// The type has annotations, a primitive type or a qualified name with type arguments, and
/// dimensions (`[]`). `void` isn't one: callers that take it check for it.
pub(crate) fn scan_type(p: &Parser<'_>, n: usize) -> Option<usize> {
    scan_type_info(p, n, usize::MAX).map(|scan| scan.end)
}

/// [`scan_type`], with what the type is made of. Type arguments longer than `budget` tokens
/// aren't recognized: the type then ends before their `<`.
pub(super) fn scan_type_info(p: &Parser<'_>, start: usize, budget: usize) -> Option<TypeScan> {
    let mut n = annotations_end(p, start);
    let mut scan = TypeScan {
        end: n,
        name_only: n == start,
        primitive: false,
        dims: false,
        ends_with_type_arguments: false,
    };
    if p.nth_at_set(n, PRIMITIVE) {
        n += 1;
        scan.name_only = false;
        scan.primitive = true;
    } else if p.nth_at_set(n, IDENT) {
        n += 1;
        loop {
            if p.nth(n) == Lt {
                let Some(end) = type_arguments_end(p, n, budget) else {
                    break;
                };
                n = end;
                scan.name_only = false;
                scan.ends_with_type_arguments = true;
            }
            if p.nth(n) != Dot {
                break;
            }
            let name = annotations_end(p, n + 1);
            if !p.nth_at_set(name, IDENT) {
                break;
            }
            if name != n + 1 {
                scan.name_only = false;
            }
            n = name + 1;
            scan.ends_with_type_arguments = false;
        }
    } else {
        return None;
    }
    loop {
        let bracket = annotations_end(p, n);
        if p.nth(bracket) != LBracket || p.nth(bracket + 1) != RBracket {
            break;
        }
        n = bracket + 2;
        scan.dims = true;
        scan.name_only = false;
        scan.primitive = false;
        scan.ends_with_type_arguments = false;
    }
    scan.end = n;
    Some(scan)
}

/// At the `<` at `n`: the index after the `>` that closes it, if the tokens up to it can be type
/// arguments, within `budget` tokens. Iterative, so deep nesting costs no stack.
fn type_arguments_end(p: &Parser<'_>, mut n: usize, budget: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut scanned = 0usize;
    loop {
        scanned += 1;
        if scanned > budget {
            return None;
        }
        match p.nth(n) {
            Lt => depth += 1,
            Gt => {
                depth -= 1;
                if depth == 0 {
                    return Some(n + 1);
                }
            }
            At => {
                let end = annotation_end(p, n)?;
                n = end;
                continue;
            }
            Identifier | Underscore | Dot | Comma | Question | ExtendsKw | SuperKw | LBracket
            | RBracket | VoidKw => {}
            kind if PRIMITIVE.contains(kind) => {}
            _ => return None,
        }
        n += 1;
    }
}

/// The index after the annotations starting at the `n`th token ahead, or `n` if there are none.
pub(super) fn annotations_end(p: &Parser<'_>, mut n: usize) -> usize {
    while let Some(end) = annotation_end(p, n) {
        n = end;
    }
    n
}

/// If an annotation starts at the `n`th token ahead, the index after it: `@`, a qualified name,
/// and a parenthesized group, skipped whole.
fn annotation_end(p: &Parser<'_>, mut n: usize) -> Option<usize> {
    if p.nth(n) != At || !p.nth_at_set(n + 1, IDENT) {
        return None;
    }
    n += 2;
    while p.nth(n) == Dot && p.nth_at_set(n + 1, IDENT) {
        n += 2;
    }
    if p.nth(n) == LParen {
        let mut depth = 0usize;
        loop {
            match p.nth(n) {
                LParen => depth += 1,
                RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(n + 1);
                    }
                }
                Eof => return None,
                _ => {}
            }
            n += 1;
        }
    }
    Some(n)
}
