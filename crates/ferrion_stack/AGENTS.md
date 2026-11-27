# ferrion_stack

`ensure_stack`, which runs a function on a new heap-allocated stack segment when little stack is
left, so deeply nested input can't overflow the stack. It wraps `stacker` and keeps that
dependency's `unsafe` code in one place. The parser and every later phase that recurses on input
depth use it.

**Design:** [parser-deep-nesting.md](../../docs/design/parser-deep-nesting.md)

**Risk level:** Deep for changes to `RED_ZONE`, `SEGMENT_SIZE` or `ensure_stack`, and for adding
or removing calls in recursive code
([high-risk-areas.md §8](../../docs/agents/high-risk-areas.md)).
Standard for tests.

## Invariants

- `RED_ZONE` covers the deepest stack any recursive function uses between two `ensure_stack` calls.
- A million frames run on a 256 KiB thread without overflow, and `ensure_stack` returns `f`'s value.
- The check is a stack-pointer comparison, cheap enough for every call.

## Where tests go

| Kind of change   | Add the test here          | Command                                    |
| ---------------- | -------------------------- | ------------------------------------------ |
| Stack behavior   | unit tests in `src/lib.rs` | `cargo test --package ferrion_stack --lib` |

Tests of a consumer's depth go in that crate (`ferrion_parser`'s `tests/nesting_stress.rs`, the
formatter's and semantic crate's deep-nesting tests).

## Don't

- Don't add `unsafe` here; the workspace denies it
  ([high-risk-areas.md §9](../../docs/agents/high-risk-areas.md)).
- Don't raise `RED_ZONE` or `SEGMENT_SIZE` without thinking through Linux, macOS and Windows.
