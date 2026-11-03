//! The import section (docs/design/formatter-completeness.md, "Imports"): the imports of a
//! compilation unit, from the first to the last.

use ferrion_syntax::NodeOrToken;

use super::{emit, first_token};
use crate::format::Formatter;

/// The rule for the import section, `imports`, the compilation unit's imports in input order:
/// one per line, with the input's blank lines between them, at most one.
pub(super) fn section<'t>(f: &mut Formatter<'t>, imports: &[NodeOrToken<'t>]) {
    for (i, &import) in imports.iter().enumerate() {
        if i > 0 {
            f.line_break_before(first_token(import));
        }
        emit(f, import);
    }
}
