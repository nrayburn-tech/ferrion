//! The import section's part of the equivalence check (docs/design/formatter-completeness.md,
//! "Imports", "Checking it"). The rest of the tree is compared without the imports, which the
//! formatter may sort and remove, and the sections are compared here.

use ferrion_syntax::{NodeId, SyntaxKind, SyntaxTree};

use super::{Property, Violation, violation};

/// The nodes of `tree` outside its imports, as the rest of the equivalence check sees them:
/// for each node, its index among them, or `None` for a node in an import.
pub(super) struct Kept {
    /// For each node index, the number of nodes before it outside the imports.
    before: Vec<usize>,
    in_import: Vec<bool>,
}

impl Kept {
    pub(super) fn new(tree: &SyntaxTree) -> Kept {
        let count = tree.node_count();
        let mut in_import = vec![false; count];
        for node in tree.nodes().filter(|n| n.kind() == SyntaxKind::ImportDecl) {
            let subtree = node.subtree();
            in_import[subtree.start.index()..subtree.end.index()].fill(true);
        }
        let mut before = Vec::with_capacity(count + 1);
        let mut kept = 0;
        for &excluded in &in_import {
            before.push(kept);
            kept += usize::from(!excluded);
        }
        before.push(kept);
        Kept { before, in_import }
    }

    /// The index of `node` among the nodes outside the imports, or `None` if it's in one.
    pub(super) fn index(&self, node: NodeId) -> Option<usize> {
        (!self.in_import[node.index()]).then(|| self.before[node.index()])
    }

    /// The number of nodes outside the imports in `nodes`.
    pub(super) fn count(&self, nodes: std::ops::Range<NodeId>) -> usize {
        self.before[nodes.end.index()] - self.before[nodes.start.index()]
    }
}

/// Compares the two trees' import sections: for now, the same imports, token for token, in the
/// same order, since nothing rewrites them yet.
pub(super) fn compare(
    before: &SyntaxTree,
    after: &SyntaxTree,
    bom: usize,
) -> Result<(), Violation> {
    let imports = |tree: &SyntaxTree| -> Vec<(Vec<(SyntaxKind, String)>, u32)> {
        tree.nodes()
            .filter(|n| n.kind() == SyntaxKind::ImportDecl)
            .map(|n| {
                let tokens = n
                    .tokens()
                    .filter(|t| !t.is_trivia())
                    .map(|t| (t.kind(), t.text().to_owned()))
                    .collect();
                (tokens, n.span().start)
            })
            .collect()
    };
    let (a, b) = (imports(before), imports(after));
    let strip = |list: &[(Vec<(SyntaxKind, String)>, u32)]| -> Vec<Vec<(SyntaxKind, String)>> {
        list.iter().map(|(tokens, _)| tokens.clone()).collect()
    };
    if let Some(i) = super::first_difference(&strip(&a), &strip(&b)) {
        let describe = |list: &[(Vec<(SyntaxKind, String)>, u32)]| {
            list.get(i).map_or("nothing".to_owned(), |(tokens, _)| {
                tokens.iter().map(|(_, t)| t.as_str()).collect::<String>()
            })
        };
        return Err(violation(
            Property::Equivalence,
            b.get(i).map(|&(_, start)| bom + start as usize),
            format!(
                "import {i} is {} in the input and {} in the output",
                describe(&a),
                describe(&b)
            ),
        ));
    }
    Ok(())
}
