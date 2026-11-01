//! Rules for declarations: the compilation unit, packages and imports, modules and their
//! directives, modifiers and annotations, type declarations and their bodies, members, enum
//! constants, records, and parameters. [`super::area`] lists the kinds.

use ferrion_syntax::{SyntaxKind as K, SyntaxNode};

use super::generic;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    match node.kind() {
        K::Modifiers => crate::modifiers::format(f, node),
        _ => generic(f, node),
    }
}
