//! Types, type arguments, and type parameters (`java.grammar`: "Types").
//!
//! Not written yet. Stubs keep the signatures other areas call.

// Stubs no other area calls yet.
#![allow(dead_code)]

use crate::grammar::todo;
use crate::parser::{CompletedMarker, Parser};

/// A type, with its leading annotations and any array dimensions: a `PrimitiveType`,
/// `ClassType`, `ArrayType`, or `VoidType`. `None`, consuming nothing, if no type starts here.
///
/// Stub.
pub(crate) fn type_(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// A class or interface type, for `extends`, `implements`, `permits`, `throws`, and `new`.
///
/// Stub.
pub(crate) fn class_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// `<A, B>` or `<>`, at `<`.
///
/// Stub.
pub(crate) fn type_arguments(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// `<T extends A & B, U>`, at `<`.
///
/// Stub.
pub(crate) fn type_parameters(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// Zero or more `Dim`s (`@A []`), as children of the node being parsed: C-style dimensions after a
/// declarator's or parameter's name or a method's parameters.
///
/// Stub.
pub(crate) fn dims(_p: &mut Parser<'_>) {}

/// Lookahead without consuming anything: if a type starts at the `n`th token ahead, the index of
/// the token after it. For the decisions that depend on whether a type comes next: local
/// variable or expression statement, cast or parenthesized expression, generic method reference.
///
/// Stub.
pub(crate) fn scan_type(_p: &Parser<'_>, _n: usize) -> Option<usize> {
    None
}
