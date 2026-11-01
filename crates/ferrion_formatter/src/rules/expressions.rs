//! Rules for expressions: operators, assignment, calls and their arguments, field and array
//! access, lambdas and method references, `?:`, object and array creation, array initializers,
//! casts, `instanceof`, and patterns. [`super::area`] lists the kinds.

use ferrion_syntax::SyntaxNode;

use super::generic;
use crate::format::Formatter;

pub fn format<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    generic(f, node);
}
