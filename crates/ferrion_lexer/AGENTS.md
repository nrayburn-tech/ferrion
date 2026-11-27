# ferrion_lexer

The Java lexer: it turns source text into tokens (trivia and comments included) and `LexError`s,
for Java 8 through the latest GA release, and the result doesn't depend on the Java version. It
translates Unicode escapes but keeps spans as written, and it holds the Unicode identifier tables.
It doesn't parse and doesn't check language versions. `ferrion_parser`, `ferrion_syntax` and
`ferrion_doc_comments` depend on it.

**Design:** [lexer-oracle-normalization.md](../../docs/design/lexer-oracle-normalization.md)

**Risk level:** Deep for any change to what is lexed or to a span: Unicode escapes, line
terminators and `\u001a`, text blocks, literals, `>>` splitting, contextual keywords, offsets
([high-risk-areas.md §2](../../docs/agents/high-risk-areas.md)). Standard for internal refactors
with identical output and for test-only changes.

## Invariants

- Tokens cover the text without gaps or overlaps, and end with one empty `Eof`: concatenating them
  reproduces the source.
- Any text produces tokens. The lexer never panics, and malformed input gives `LexError`s in source
  order beside the tokens.
- Spans cover Unicode escapes as written. No token boundary splits a character or an escape.
- The result is version independent; the parser reports constructs a release doesn't have.
- `ferrion_lexer::invariants` checks these promises for the fuzz targets and regression tests.

## Where tests go

| Kind of change              | Add the test here                                | Command                                                           |
| --------------------------- | ------------------------------------------------ | ----------------------------------------------------------------- |
| A lexing rule or error      | `src/tests.rs` (unit tests)                      | `cargo test --package ferrion_lexer --lib`                        |
| A fuzz failure              | `tests/fuzz-regressions/` (one file per input)   | `cargo test --package ferrion_lexer --test fuzz_regressions`      |
| A javac comparison          | `tools/oracle-check/differences/` snapshots      | `cargo oracle tokens`, `cargo oracle differences` (needs a JDK)   |

Fuzz targets: `lex` and `lex_fragments` (CONTRIBUTING.md "Fuzzing"). `FERRION_BLESS` isn't used
here; `cargo oracle differences --bless` rewrites the oracle snapshots, and every resulting diff
needs a hand review. Cite javac's behavior only as observed by running it.

Use this crate's existing test setup. Don't add new test frameworks, helper crates or test styles
without asking.

## Don't

- Don't hand-edit `src/unicode_tables.rs`: `tools/unicode-tables/UnicodeTables.java` generates it,
  with the JDK that `LATEST_RELEASE` names.
- Don't make lexing depend on the Java version.
- Don't add an allowed difference from javac without documenting it in
  `lexer-oracle-normalization.md` in the same change.
- Don't tidy the deliberately broken inputs in `tests/fuzz-regressions/`.
- Don't write literal Java Unicode escapes through tool input without checking. They can arrive
  decoded: build the backslash with a script, then grep the file
  ([AGENTS.md](../../AGENTS.md#pitfalls)).
