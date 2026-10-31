//! A readable dump of a tree, for snapshot tests and debugging.

use std::fmt::Write as _;

use super::{NodeId, SyntaxTree, TokenId, flags};

impl SyntaxTree {
    /// Every node and token, one per line, indented by depth. A node shows its kind and span, and
    /// `[error]` if it contains an error; a token its kind, span, and text:
    ///
    /// ```text
    /// CompilationUnit 0..13
    ///   PackageDecl 0..12
    ///     PackageKw 0..7 "package"
    ///     Whitespace 7..8 " "
    ///     Name 8..11
    ///       Identifier 8..11 "foo"
    ///     Semi 11..12 ";"
    ///   LineTerminator 12..13 "\n"
    ///   Eof 13..13 ""
    /// ```
    pub fn debug_dump(&self) -> String {
        let mut out = String::new();
        let nodes = self.raw_nodes();
        let mut open: Vec<usize> = Vec::new();
        let mut next_node = 0;
        for token in 0..self.token_count() {
            let token = TokenId::from_index(token);
            while open
                .last()
                .is_some_and(|&node| nodes[node].token_end <= token)
            {
                open.pop();
            }
            while next_node < nodes.len() && nodes[next_node].first_token == token {
                let node = self.node(NodeId::from_index(next_node));
                let span = node.span();
                let error = if nodes[next_node].flags & flags::CONTAINS_ERROR != 0 {
                    " [error]"
                } else {
                    ""
                };
                let indent = "  ".repeat(open.len());
                writeln!(out, "{indent}{:?} {span:?}{error}", node.kind()).unwrap();
                open.push(next_node);
                next_node += 1;
            }
            let token = self.token(token);
            let indent = "  ".repeat(open.len());
            writeln!(
                out,
                "{indent}{:?} {:?} {:?}",
                token.kind(),
                token.span(),
                token.text()
            )
            .unwrap();
        }
        out
    }
}
