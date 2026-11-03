//! The import section (docs/design/formatter-completeness.md, "Imports"): the imports of a
//! compilation unit, from the first to the last, sorted and pruned as the import plan
//! (`crate::imports`) says.

use ferrion_syntax::{NodeOrToken, SyntaxNode};

use super::{emit, first_token};
use crate::format::Formatter;
use crate::imports::Rewrite;

/// The rule for the import section, `imports`, the compilation unit's imports in input order: the
/// planned section when the plan rewrites it, and otherwise the imports as written, one per line,
/// with the input's blank lines between them, at most one. A1's blank lines before and after the
/// section are the compilation unit's. Returns whether it emitted an import.
pub(super) fn section<'t>(f: &mut Formatter<'t>, imports: &[NodeOrToken<'t>]) -> bool {
    let nodes: Vec<SyntaxNode<'t>> = imports.iter().filter_map(|i| i.as_node()).collect();
    if !f.in_region()
        && let Some(plan) = f.plan_imports(&nodes)
    {
        rewritten(f, &plan);
        return !plan.groups.is_empty();
    }
    for (i, &import) in imports.iter().enumerate() {
        if i > 0 {
            f.line_break_before(first_token(import));
        }
        emit(f, import);
    }
    true
}

/// The section as planned: the comments above the first import where they are, then the floating
/// comments, then the groups, a blank line between groups and none inside one, each import with
/// the comments that belong to it.
fn rewritten<'t>(f: &mut Formatter<'t>, plan: &Rewrite<'t>) {
    f.comments_before(plan.first);
    f.rewritten_imports(plan, |w| {
        for comment in &plan.floating {
            w.comment(comment.index, comment.before, comment.after);
        }
        for (g, group) in plan.groups.iter().enumerate() {
            for (i, entry) in group.iter().enumerate() {
                match (g, i) {
                    (0, 0) => {}
                    (_, 0) => w.blank_line(),
                    _ => w.line_break(),
                }
                for comment in &entry.above {
                    w.comment(comment.index, comment.before, comment.after);
                }
                w.import(entry.import);
            }
        }
    });
}
