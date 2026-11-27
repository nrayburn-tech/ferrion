# Template for `crates/<name>/AGENTS.md`

Copy this into every new crate. Keep it under about 60 lines. It adds to the root
[AGENTS.md](../../AGENTS.md) and never contradicts it. Link to the design documents rather than
repeating them.

```markdown
# <crate name>

<One paragraph: what this crate does and does not do, and which crates depend on it.>

**Design:** <docs/design/... documents that cover this crate>

**Risk level:** <Standard | Deep: see docs/agents/high-risk-areas.md §N>

## Invariants

- <Things that must always hold, e.g. "the tree reproduces its source byte for byte", "never
  panics on any input">

## Where tests go

| Kind of change          | Add the test here                       | Command                                          |
| ----------------------- | --------------------------------------- | ------------------------------------------------ |
| <e.g. a new construct>  | <e.g. `tests/snapshots/<area>/` input>  | `cargo test --package <crate> --test snapshots`  |
| <e.g. a fuzz failure>   | <e.g. `tests/fuzz-regressions/`>        | `cargo test --package <crate> --test fuzz_regressions` |

Use this crate's existing test setup. Don't add new test frameworks, helper crates or test styles
without asking.

## Don't

- <Crate-specific prohibitions, e.g. "don't hand-edit `generated.rs`; change the grammar and run
  `cargo codegen`">
```
