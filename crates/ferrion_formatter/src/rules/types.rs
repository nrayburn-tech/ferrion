//! Rules for types: primitive, class, array, intersection, and union types, type parameters and
//! arguments, wildcards, varargs, and `throws`. [`super::area`] lists the kinds.

use ferrion_syntax::SyntaxNode;

use super::generic;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    generic(f, node);
}
