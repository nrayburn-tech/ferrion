//! Side tables keyed by node, and pointers that find a node again in another tree.

use std::ops::{Index, IndexMut};

use ferrion_diagnostics::Span;

use super::{NodeId, SyntaxNode, SyntaxTree};
use crate::SyntaxKind;

/// A value per node of one tree, indexed by [`NodeId`]: a `Vec` with an entry for every node.
/// For data on a few nodes, a hash map keyed by `NodeId` is smaller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeMap<T> {
    values: Vec<T>,
}

impl<T> NodeMap<T> {
    /// A map with `value(node)` for each node of `tree`, computed in pre-order.
    pub fn from_fn(tree: &SyntaxTree, value: impl FnMut(SyntaxNode<'_>) -> T) -> NodeMap<T> {
        NodeMap {
            values: tree.nodes().map(value).collect(),
        }
    }

    /// A map with `value` for each node of `tree`.
    pub fn filled(tree: &SyntaxTree, value: T) -> NodeMap<T>
    where
        T: Clone,
    {
        NodeMap {
            values: vec![value; tree.node_count()],
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, id: NodeId) -> Option<&T> {
        self.values.get(id.index())
    }

    /// Each node's id and value, in pre-order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &T)> {
        self.values
            .iter()
            .enumerate()
            .map(|(i, value)| (NodeId::from_index(i), value))
    }
}

impl<T: Default> NodeMap<T> {
    /// A map with the default value for each node of `tree`.
    pub fn new(tree: &SyntaxTree) -> NodeMap<T> {
        NodeMap {
            values: std::iter::repeat_with(T::default)
                .take(tree.node_count())
                .collect(),
        }
    }
}

impl<T> Index<NodeId> for NodeMap<T> {
    type Output = T;

    fn index(&self, id: NodeId) -> &T {
        &self.values[id.index()]
    }
}

impl<T> IndexMut<NodeId> for NodeMap<T> {
    fn index_mut(&mut self, id: NodeId) -> &mut T {
        &mut self.values[id.index()]
    }
}

/// A node's kind and span, to find "the same node" in another tree of the same text, such as a
/// reparse of it. `NodeId`s are valid only in their own tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SyntaxPtr {
    pub kind: SyntaxKind,
    pub span: Span,
}

impl SyntaxPtr {
    pub fn new(node: SyntaxNode<'_>) -> SyntaxPtr {
        SyntaxPtr {
            kind: node.kind(),
            span: node.span(),
        }
    }

    /// The outermost node of `tree` with this kind and span, if any.
    pub fn resolve(self, tree: &SyntaxTree) -> Option<SyntaxNode<'_>> {
        // The root covers the leading trivia, so it's checked on its own.
        if self.kind == SyntaxKind::CompilationUnit {
            let root = tree.root();
            return (root.span() == self.span).then_some(root);
        }
        let nodes = tree.raw_nodes();
        let start = |id: usize| tree.token_span(nodes[id].first_token).start;
        // Nodes are sorted by start, and the ones starting at the same offset are consecutive.
        let (mut low, mut high) = (1, nodes.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if start(middle) < self.span.start {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut index = low;
        while index < nodes.len() && start(index) == self.span.start {
            let node = tree.node(NodeId::from_index(index));
            if node.kind() == self.kind && node.span() == self.span {
                return Some(node);
            }
            index += 1;
        }
        None
    }
}
