//! Builds a small hierarchy of generic classes, F-bounds, inner classes of generic classes, and
//! expansive inheritance, and types over it chosen by the fuzz input, and checks the type
//! relations' properties on them: subtyping reflexive and transitive, a subtype assignable, `lub`
//! above its operands, capture and erasure idempotent, every query within its fuel.

#![no_main]

use ferrion_semantic::invariants;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(message) = invariants::check_relations(bytes) {
        panic!("{message}");
    }
});
