# ferrion_parser

The Java parser: `parse` lexes a file, builds a `SyntaxTree` (from `ferrion_syntax`) and reports
syntax errors, recovers from them, and runs the language version check (`version/`). It doesn't
own the tree's types or comment placement, which are in `ferrion_syntax`. It depends on
`ferrion_lexer`, `ferrion_syntax`, `ferrion_java_version` and `ferrion_stack`.

**Design:** [parser.md](../../docs/design/parser.md), with
[parser-syntax-tree.md](../../docs/design/parser-syntax-tree.md),
[parser-error-recovery.md](../../docs/design/parser-error-recovery.md),
[parser-language-versions.md](../../docs/design/parser-language-versions.md),
[parser-deep-nesting.md](../../docs/design/parser-deep-nesting.md),
[parser-oracle-normalization.md](../../docs/design/parser-oracle-normalization.md)

**Risk level:** Deep for grammar, language-version checks and error recovery
([high-risk-areas.md §3](../../docs/agents/high-risk-areas.md)), and for recursion depth
([§8](../../docs/agents/high-risk-areas.md)). Standard for test inputs, diagnostics wording that
snapshots cover, and refactors with identical trees.

## Invariants

- `parse` never fails and never panics: any text gives a tree that reproduces it byte for byte.
- Diagnostics are in source order, inside the text; the root is marked as containing an error
  exactly when a lexical or syntax error is reported.
- Every recursive function whose depth follows the input calls `ferrion_stack::ensure_stack`;
  `NESTING_LIMIT` is a resource limit with a `NestingTooDeep` diagnostic.
- `ferrion_parser::invariants` (and `ferrion_syntax::invariants`) check these.

## Where tests go

| Kind of change             | Add the test here                                      | Command                                                          |
| -------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------- |
| A construct                | `tests/snapshots/<area>/` (`.java` + `.tree`)          | `FERRION_BLESS=1 cargo test --package ferrion_parser --test snapshots` |
| Recovery                   | a `recovery` dir under `tests/snapshots/`; must error  | `cargo test --package ferrion_parser --test recovery_snapshots`  |
| Statements, types, exprs   | `tests/block-snapshots/`, `tests/snapshots/*/fragments/` | `FERRION_BLESS=1 cargo test --package ferrion_parser --lib`   |
| A version check            | `tests/version/` (`.java` + javac's `.expected`)       | `cargo test --package ferrion_parser --lib version`              |
| Deep nesting               | a pattern in `src/stress.rs`; `tests/deep_nesting*.rs` | `cargo test --package ferrion_parser --test nesting_stress`      |
| A fuzz failure             | `tests/fuzz-regressions/`                              | `cargo test --package ferrion_parser --test fuzz_regressions`    |

Fuzz target: `parse` (the `format` target reuses its seeds). Blessing rewrites expected trees:
review the diff by hand. A new stress pattern also needs a test in the formatter's
`tests/deep_nesting.rs`. The oracle comparisons are `cargo parser-check round-trip` and
`cargo oracle syntax` / `tree`. Use the existing test setup; don't add frameworks without asking.

## Don't

- Don't edit `.expected` files in `tests/version/` by hand: they are javac's output.
- Don't tidy deliberately broken inputs in `tests/snapshots/**/recovery/` or
  `tests/fuzz-regressions/`.
- Don't add recursion without `ensure_stack`, or a stress pattern test for a new nesting construct.
- Don't lower recovery thresholds in `tools/parser-check/thresholds.toml` to make a check pass.
