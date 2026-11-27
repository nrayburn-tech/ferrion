# ferrion_syntax

The flat, lossless syntax tree that `ferrion_parser` produces and the formatter, linter and
semantic analysis read: `SyntaxTree`, the generated `SyntaxKind` and typed views in `ast`, error
units, and `comments` (comment placement and doc comment lookup). It holds no parsing logic.
Depends on `ferrion_lexer`; `ferrion_parser` depends on it, and its tests use the parser and
`ferrion_doc_comments`.

**Design:** [parser-syntax-tree.md](../../docs/design/parser-syntax-tree.md),
[parser-comments.md](../../docs/design/parser-comments.md),
[parser-error-recovery.md](../../docs/design/parser-error-recovery.md) ("What a syntax error
affects")

**Risk level:** Deep for the tree's representation and invariants, `java.grammar`, comment
placement and error units ([high-risk-areas.md §3](../../docs/agents/high-risk-areas.md)).
Standard for accessors and tests that leave the representation alone.

## Invariants

- The tree reproduces its text byte for byte, with the lexer's tokens; nodes are in consistent
  pre-order with correct parent and `subtree_end` links.
- Token ranges nest, aren't empty, and (except the root's) start and end on non-trivia tokens.
- `CONTAINS_ERROR` is set on every `Error` node and on its ancestors.
- Token kinds differ from the lexer's only for contextual keywords.
- The comment map and doc comment lookup keep what `comments` documents.
- `ferrion_syntax::invariants` checks these.

## Where tests go

| Kind of change            | Add the test here                         | Command                                                    |
| ------------------------- | ----------------------------------------- | ---------------------------------------------------------- |
| Comment placement, docs   | `tests/comments.rs`                       | `cargo test --package ferrion_syntax --test comments`      |
| Error units               | `tests/error_units.rs`                    | `cargo test --package ferrion_syntax --test error_units`   |
| Tree internals            | unit tests in `src/`                      | `cargo test --package ferrion_syntax --lib`                |

Parse-based checks of the tree are in `ferrion_parser` (`tests/snapshots`, `nesting_stress`); its
`parse` fuzz target also runs the tree's invariants. There is no fuzz-regression directory here. A
grammar change needs `cargo codegen`, which CI checks. Use the existing test setup; don't add
frameworks without asking.

## Don't

- Don't hand-edit `src/ast/generated.rs` or `src/syntax_kind/generated.rs`: change
  `java.grammar` and run `cargo codegen`.
- Don't add parsing logic here; it belongs in `ferrion_parser`.
- Don't make a tree walk recursive without `ferrion_stack::ensure_stack` and a deep-nesting test.
- Don't loosen `invariants` to make a tree pass.
