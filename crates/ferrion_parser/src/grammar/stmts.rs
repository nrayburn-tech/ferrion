//! Blocks, statements, and switch blocks (`java.grammar`: "Statements", "Switches").
//!
//! Not written yet. Stubs keep the signatures other areas call.

// Stubs no other area calls yet.
#![allow(dead_code)]

use crate::grammar::todo;
use crate::parser::{CompletedMarker, Parser};

/// A `Block`, at `{`: a method, constructor, initializer, or lambda body, or a statement.
///
/// Stub.
pub(crate) fn block(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}

/// A `SwitchBlock`, at `{`, for switch statements and switch expressions.
///
/// Stub.
pub(crate) fn switch_block(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}
