# ferrion_doc_comments

Parsers for documentation comments: `parse_javadoc` for `/** */` and `parse_markdown` for runs of
`///` lines (JEP 467). They give structured data (description, block and inline tags, HTML,
references) with source spans, and diagnostics for malformed input. They don't depend on the Java
parser and don't decide whether `///` counts as documentation at a Java version. Reflow is not
here: `ferrion_formatter` reads this crate's output. Depends on `ferrion_lexer`.

**Design:** [doc-comments.md](../../docs/design/doc-comments.md)

**Risk level:** Deep for changes to spans, text values, escape handling or the model that the
formatter's doc comment reflow reads
([high-risk-areas.md §1](../../docs/agents/high-risk-areas.md)), and for Unicode escape handling ([§2](../../docs/agents/high-risk-areas.md)). Standard for new
diagnostics and for tests.

## Invariants

- Never fails and never panics; nesting beyond `MAX_NESTING` is read flat with a diagnostic.
- Every span is inside the comment, on character boundaries, and doesn't split a Unicode escape;
  children are inside parents and siblings are in order.
- Text is never empty; a traditional comment's text is its span with escapes translated.
- Diagnostics are inside the comment and in order; parsing is deterministic.
- `ferrion_doc_comments::invariants` checks these.

## Where tests go

| Kind of change          | Add the test here                                      | Command                                                         |
| ----------------------- | ------------------------------------------------------ | --------------------------------------------------------------- |
| A parsing rule          | unit tests in `src/` (note whether javadoc was run)    | `cargo test --package ferrion_doc_comments --lib`               |
| A fuzz failure          | `tests/fuzz-regressions/javadoc/` or `markdown/`       | `cargo test --package ferrion_doc_comments --test fuzz_regressions` |

Fuzz targets: `javadoc` and `markdown` (CONTRIBUTING.md "Fuzzing"). A reflow failure goes in the
formatter's `tests/javadoc-regressions/` or `markdown-regressions/`, not here. Where javadoc's
behavior is cited, it was observed by running javadoc 27. Use the existing test setup; don't add
frameworks without asking.

## Don't

- Don't read javadoc's source to learn its behavior; run it on a small comment.
- Don't change the parser without updating its tests and `doc-comments.md` together.
- Don't tidy the deliberately broken inputs in `tests/fuzz-regressions/`.
- Don't write literal Java Unicode escapes through tool input without checking. They can arrive
  decoded: build the backslash with a script, then grep the file
  ([AGENTS.md](../../AGENTS.md#pitfalls)).
