//! Expressions (`java.grammar`: "Expressions").
//!
//! Not written yet. Stubs keep the signatures other areas call.

// Stubs no other area calls yet.
#![allow(dead_code)]

use crate::grammar::todo;
use crate::parser::{CompletedMarker, Parser};

/// Any expression, assignments and lambdas included. `None`, consuming nothing, if no expression
/// starts here.
///
/// Stub.
pub(crate) fn expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// Whether an expression can start at the current token.
///
/// Stub.
pub(crate) fn at_expr_start(_p: &Parser<'_>) -> bool {
    false
}

/// `(a, b)`, at `(`: a call's or an enum constant's `Arguments`.
///
/// Stub.
pub(crate) fn arguments(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// A variable's initializer: an expression, or an `ArrayInitializer` at `{`.
///
/// Stub.
pub(crate) fn variable_initializer(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}
