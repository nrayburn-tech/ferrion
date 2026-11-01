//! Rules for statements: blocks, local variable declarations, control flow, `switch`
//! statements and expressions and their blocks, `try` and its resources and clauses, and labels.
//! [`super::area`] lists the kinds.

use ferrion_syntax::SyntaxNode;

use super::generic;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    generic(f, node);
}
