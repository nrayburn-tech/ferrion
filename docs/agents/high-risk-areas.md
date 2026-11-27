# High-risk areas

A change of a kind listed below is **Deep tier** (see [model-tiers.md](model-tiers.md)). It needs
the extra requirements listed for that area and a Deep-tier review before merge. The areas describe
kinds of change, not whole crates: most of Ferrion's crates hold both high-risk code and routine
code.

Correctness is Ferrion's first goal ([SCOPE.md](../SCOPE.md#goals)). Most of these areas are where a
mistake produces output that looks right and isn't.

The clean-room policy in [CONTRIBUTING.md](../../CONTRIBUTING.md#clean-room-policy) applies to every
change, in every area, and is not repeated below.

## 1. Formatter output (`ferrion_formatter`, and the doc comment model it reflows)

The reflow code is in `ferrion_formatter/src/doc_comments/`. It reads the model that
`ferrion_doc_comments` parses, so a change to that model's spans or text is in this area too.

**Why it's risky:** a formatter bug rewrites users' source files. A change in meaning, a lost or
duplicated comment, or output that doesn't settle is the worst bug Ferrion can have.

**High risk:** the rewrites beyond layout (import sorting and removal, modifier reordering, the
space after `//`), comment placement, doc comment reflow and the decision to re-indent only, range
formatting, disable regions, line endings and the byte-order mark, and the `verify` checks
themselves. Layout rules that snapshots and the safety properties cover are Standard.

**Extra requirements:**

- The four safety properties in [SCOPE.md](../SCOPE.md#formatter) hold on every snapshot, on the
  corpus (`cargo format-check verify`) and in the `format` fuzz target. Never loosen a property, or
  what `verify` accepts as equivalent, to make a case pass.
- A departure from Google Java Format has an entry in [style.md](../style.md) with its reason and a
  case in `tools/format-check/differences/`.
- Learn GJF's behavior by running it, never by reading its source.

## 2. Lexing and source positions (`ferrion_lexer`, `ferrion_diagnostics`)

**Why it's risky:** every later stage trusts the tokens and offsets. An off-by-one in a Unicode
escape or a UTF-16 width shifts every position after it.

**High risk:** Unicode escapes, line terminators and `\u001a`, text blocks, literals, the `>>`
splitting and contextual keywords, byte versus UTF-16 offsets, and line and column mapping.

**Extra requirements:**

- Compare with javac through `cargo oracle tokens` and `cargo oracle differences`. A new allowed
  difference is documented in
  [lexer-oracle-normalization.md](../design/lexer-oracle-normalization.md).
- Literal Unicode escapes written through an agent's tool input can arrive decoded. See "Pitfalls"
  in [AGENTS.md](../../AGENTS.md#pitfalls).

## 3. Parsing, the syntax tree and error recovery (`ferrion_parser`, `ferrion_syntax`)

**Why it's risky:** the tree must reproduce its source byte for byte. Recovery decides how much of a
broken file the formatter and linter still see.

**High risk:** the grammar and its language-version checks, error recovery, the tree's
representation and invariants, and `java.grammar`.

**Extra requirements:**

- `cargo parser-check round-trip` and `cargo oracle syntax` / `tree` agree with javac on the corpus.
  Recovery shares stay above `tools/parser-check/thresholds.toml`.
- Generated code changes only through `java.grammar` and `cargo codegen`.
- A pattern from the deep-nesting stress tests gets a test in each of the tree's consumers.

## 4. Semantic analysis (`ferrion_semantic`)

**Why it's risky:** the linter's findings rest on it. A wrong name, type or inference result
produces findings Ferrion can't justify, and a stale incremental answer is a bug that only shows up
in the language server.

**High risk:** name resolution, type relations, attribution and inference, the treatment of
incomplete analysis (error types, unresolved symbols, Lombok), and the incremental engine's
dependencies and invalidation.

**Extra requirements:**

- Compare with javac through `cargo oracle names`, `cargo oracle types` and `cargo oracle removed`.
  Known differences go under a category with an explanation, never as a bare entry.
- When unsure, report incomplete analysis rather than guess
  ([SCOPE.md](../SCOPE.md#incomplete-analysis)).
- Incremental changes keep the `semantic_edits` fuzz target's comparison with a fresh engine
  passing.
- Cite the JLS section a rule comes from.

## 5. Untrusted binary input (`ferrion_classfile`)

**Why it's risky:** class files, jars, module images and `ct.sym` come from the user's classpath and
JDK. A panic, a hang or unbounded allocation on a crafted file is a denial of service, in the
language server too.

**Extra requirements:**

- No panic, unbounded allocation or unbounded recursion on any input. Sizes and counts read from a
  file are checked against the bytes available before they are used.
- Compare with the JDK's own tools through `cargo oracle classfile`.
- Fuzz seeds are written by test code, never copied from a JDK.

## 6. Oracles, known differences and thresholds (`tools/`)

**Why it's risky:** these are how Ferrion measures its own correctness and speed. Weakening one
hides every regression behind it.

**High risk:** the oracles' normalization, the known-difference lists and snapshots under
`tools/oracle-check/` and `tools/format-check/differences/`, `tools/parser-check/thresholds.toml`,
`tools/bench/thresholds.toml`, `tools/format-check/gjf-diff.toml`, and the GJF pin.

**Extra requirements:**

- Never add a known difference, lower a threshold or raise a diff count to make a check pass without
  explaining why the old expectation was wrong. Thresholds rise when results improve, as each file
  says.
- `--bless` and `FERRION_BLESS=1` rewrite expectations: review every resulting diff by hand.
- Update the normalization document a difference belongs to in the same change.

## 7. Public contracts (`ferrion_cli`, `ferrion_config`, `ferrion_project`, `schemas/`)

**Why it's risky:** users, scripts, the Maven plugin and the IntelliJ extension depend on commands,
flags, exit codes, output formats, `ferrion.jsonc`, the schema, the project model and rule names.

**Extra requirements:**

- A change to any of these follows [docs/design/cli.md](../design/cli.md) and
  [SCOPE.md](../SCOPE.md#command-line-interface), and needs a human's yes before it starts.
- Messages that users or scripts read go through the snapshot catalog, and their diff is reviewed.
- Machine-readable output (`--json`, `github` format) is stable and deterministic.

## 8. Stack depth and resource limits (`ferrion_stack`, deep nesting)

**Why it's risky:** deeply nested input overflows the stack on one platform and not another, and a
missing limit turns a large file into a hang.

**Extra requirements:**

- Every recursive function whose depth follows the input calls `ferrion_stack::ensure_stack` on
  entry ([parser-deep-nesting.md](../design/parser-deep-nesting.md)). The deep-nesting tests and
  javac's deep-nesting stress tests pass.
- Think through Linux, macOS and Windows, whose main-thread stacks differ.

## 9. `unsafe`

The workspace denies `unsafe_code`. The only exception is `tools/bench`'s allocator, which is never
shipped. Any other exception needs approval, a `// SAFETY:` comment for each block, and tests that
run under Miri where feasible.

## 10. CI, fuzzing and supply-chain configuration

This covers `.github/`, `deny.toml`, `rust-toolchain.toml`, `fuzz/rust-toolchain.toml`, action
pins, `tools/scancode/` and the fuzz workflow.

- Third-party actions stay pinned to a full commit SHA with a version comment.
- Workflows grant the least `permissions:` they need. Never interpolate untrusted event fields (PR
  titles, branch names, comment bodies) into `run:` scripts with `${{ }}`; pass them through `env:`.
- Never weaken `deny.toml` or the ScanCode check: no new license allowances, advisory ignores or
  copyright-holder exceptions without a written reason and human approval.
- Don't shorten CI's fuzz or oracle runs to make a pull request pass.
