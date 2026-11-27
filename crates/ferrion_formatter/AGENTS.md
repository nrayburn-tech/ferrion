# ferrion_formatter

The Java formatter. `format` parses a file and formats it, `format_tree` formats a tree that is
already parsed, `format_edits` and `format_range` give the result as edits, and `verify`,
`verify_parsed` and `verify_range` check the four safety properties. The style is fixed: there are
no style options. Every piece of text in the output is a token of the input, and the formatter
writes only whitespace of its own, apart from reordering modifiers. It depends on the lexer, parser,
syntax tree, doc comment and diagnostics crates, and never reads a JDK or classpath.

**Design:** [formatter.md](../../docs/design/formatter.md),
[formatter-completeness.md](../../docs/design/formatter-completeness.md),
[doc-comments.md](../../docs/design/doc-comments.md), style in [style.md](../../docs/style.md)

**Risk level:** Deep for rewrites beyond layout (imports, modifier order, the space after `//`),
comment placement, doc comment reflow, range formatting, disable regions, line endings and the byte
order mark, and `verify` (high-risk-areas.md §1). Standard for layout rules that snapshots and the
safety properties cover.

## Invariants

- The four safety properties ([SCOPE.md](../../docs/SCOPE.md#formatter)): formatting twice changes
  nothing; the output is equivalent to the input apart from the intended rewrites; every comment
  appears once; the output has no syntax errors the input didn't have.
- A file with syntax or version errors is skipped, never partly formatted. A range may have errors
  elsewhere in the file, outside the units they affect.
- Output depends only on the source, the Java release and the config.
- Deeply nested input doesn't overflow the stack, and formatting time stays linear.

## Where tests go

| Kind of change                    | Add the test here                              | Command |
| --------------------------------- | ---------------------------------------------- | ------- |
| A construct or layout rule        | `tests/snapshots/<area>/X.java` and `X.formatted.java` | `cargo test --package ferrion_formatter --test snapshots` |
| Range formatting                  | `tests/ranges/` (range between `«` and `»`)    | `cargo test --package ferrion_formatter --test ranges` |
| A `format` fuzz failure           | `tests/fuzz-regressions/`                      | `cargo test --package ferrion_formatter --test fuzz_regressions` |
| A `javadoc` or `markdown` failure | `tests/javadoc-regressions/`, `tests/markdown-regressions/` | same `fuzz_regressions` target |
| A new deep-nesting pattern        | `tests/deep_nesting.rs` (also in the parser)   | `cargo test --package ferrion_formatter --test deep_nesting` |
| Internals (printer, imports, doc comments, document) | inline `tests.rs` beside the module | `cargo test --package ferrion_formatter --lib` |
| Corpus, GJF and snapshot checks   | `tools/format-check/` (`differences/` for departures from GJF) | `cargo format-check verify`, `ranges`, `gjf`, `differences`, `snapshots` |

Snapshots and range tests rewrite their expected files with `FERRION_BLESS=1` before the test
command. Review every diff by hand. A first line `// release: N` sets the release. Fuzz failures
are reduced first (see CONTRIBUTING.md, "Fuzzing"). Don't add new test frameworks or styles.

## Don't

- Don't hand-edit a snapshot's expected output without checking it is Google Java Format's output
  (`cargo format-check snapshots`). A snapshot that isn't GJF's on purpose is listed in `NOT_GJF` in
  `tools/format-check/src/snapshots.rs` with its reason.
- Don't read Google Java Format's source. Learn its behavior by running it.
- Don't loosen a safety property, or what `verify` accepts as equivalent, to make a case pass.
- Don't depart from GJF without an entry in `docs/style.md` and a case in
  `tools/format-check/differences/`.
- Don't add recursion without `ensure_stack`, or access the network or a JDK.
