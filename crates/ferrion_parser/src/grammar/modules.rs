//! Module declarations (`java.grammar`: "Compilation units, packages, imports, and modules").
//!
//! Not written yet. Stubs keep the signatures other areas call.

// Stubs no other area calls yet.
#![allow(dead_code)]

use crate::grammar::todo;
use crate::parser::{CompletedMarker, Marker, Parser};

/// Whether a module declaration starts at the current token, after its annotations: `module` or
/// `open module`, followed by a name.
///
/// Stub.
pub(crate) fn at_module_decl(_p: &Parser<'_>) -> bool {
    false
}

/// A module declaration after its annotations; `m` was started before them.
///
/// Stub.
pub(crate) fn module_decl(p: &mut Parser<'_>, m: Marker) -> Option<CompletedMarker> {
    m.abandon(p);
    todo(p)
}
