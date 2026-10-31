//! Patterns (`java.grammar`: "Patterns").
//!
//! Not written yet. Stubs keep the signatures other areas call.

// Stubs no other area calls yet.
#![allow(dead_code)]

use crate::grammar::todo;
use crate::parser::{CompletedMarker, Parser};

/// A pattern: a `TypePattern`, `RecordPattern`, or, nested in a record pattern,
/// `UnnamedPattern`. For `instanceof` and case labels.
///
/// Stub.
pub(crate) fn pattern(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    todo(p)
}
