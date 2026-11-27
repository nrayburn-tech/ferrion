# ferrion_diagnostics

Source files, spans and diagnostic rendering: `SourceText` (decoding, encoding, line endings, skip
reasons), `Span`, `LineIndex` and `LineCol` (byte offsets to lines and columns), and `Excerpt`
rendering with labels and severity. The `pretty`, JSON and GitHub Actions renderers are documented
here but not all written yet. It depends on no other Ferrion crate; the formatter and config crates
depend on it.

**Design:** [cli.md](../../docs/design/cli.md) ("Output"),
[formatter.md](../../docs/design/formatter.md) (line endings, byte-order mark)

**Risk level:** Deep for positions: byte versus UTF-16 offsets, line and column mapping, line
terminators and decoding (high-risk-areas.md §2). Deep for changes to rendered output formats
(§7). Standard for other internals.

## Invariants

- A span is a byte range in the source. Offsets and line and column mapping must be exact on
  non-ASCII text and every line terminator.
- Decoding reports why a file is skipped (`SkipReason`) rather than guessing.

## Where tests go

| Kind of change               | Add the test here                                  | Command |
| ---------------------------- | -------------------------------------------------- | ------- |
| Source decoding, line ends   | inline `mod tests` in `src/source.rs`              | `cargo test --package ferrion_diagnostics` |
| Spans, line index            | inline `mod tests` in `src/span.rs`, `line_index.rs` | `cargo test --package ferrion_diagnostics` |
| Excerpt rendering            | inline `mod tests` in `src/excerpt.rs`             | `cargo test --package ferrion_diagnostics` |

There is no `tests/` directory. Don't add new test frameworks without asking.

## Don't

- Don't change rendered output without checking the config error catalog
  (`crates/ferrion_config/tests/configs/errors.txt`) and the CLI's output design.
- Don't add dependencies on other Ferrion crates.
