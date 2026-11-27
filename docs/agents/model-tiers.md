# Model tiers: matching the model to the task

This guide is vendor-neutral. Each agent tool maps these tiers to whatever models and reasoning
settings it offers. The aim is to use the most capable models where a mistake would be costly or
subtle, and fast, cheap models where the work is mechanical and easy to check.

## The three tiers

| Tier         | Use it when                                                                                                                                                       | Typical cost and speed                                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| **Light**    | The work is mechanical, fully specified, low in blast radius, and trivially verified by a command or a glance at the diff.                                        | Cheapest and fastest. Fine for loops and fan-out.                                                        |
| **Standard** | This is the default for normal feature and bug work inside one crate, following a design document that already exists.                                            | Moderate.                                                                                                |
| **Deep**     | The work involves design, ambiguity, correctness-critical algorithms, Java semantics, untrusted input, cross-platform edge cases, or anything in [high-risk-areas.md](high-risk-areas.md). | Most capable model with the highest reasoning effort. Slow and expensive, so spend it where it pays off. |

If your tool controls **reasoning effort** separately from the model, treat effort as part of the
tier: Light means low effort, Standard means medium, and Deep means high. On a Standard task, raise
the effort before you switch to a bigger model.

## How to classify a task

Rate each signal as low or high:

| Signal                      | High means...                                                                                         |
| --------------------------- | ----------------------------------------------------------------------------------------------------- |
| **Ambiguity**               | Requirements are unclear, several designs are plausible, or no design document covers it.             |
| **Blast radius**            | A mistake rewrites users' code wrongly, produces findings Ferrion can't justify, or affects many crates. |
| **Reversibility**           | A mistake ships in a public contract: CLI, config, schema, project model, rule names, output formats. |
| **Verification difficulty** | Tests can't easily prove it is correct: javac or GJF parity, incremental invalidation, platform behavior. |
| **Domain depth**            | It needs real knowledge of the JLS, the JVMS, javac's behavior, or GJF's observed style.              |

- **Any signal high** means **Deep**.
- **All signals low and the change is mechanical** means **Light**.
- **Everything else** is **Standard**.

When unsure, go one tier up for planning and one tier down for carrying out the plan (see the
patterns below).

## Routing table for Ferrion

### Light

- Search the codebase or summarize how something works (read-only exploration).
- Summarize a CI log, an oracle report or test failure output. Diagnosing the failure is a separate
  task with its own tier.
- Fix typos and doc wording, update links.
- Apply mechanical clippy suggestions, renames, moves, formatting.
- Add test cases that follow an existing pattern exactly, such as another snapshot input for a rule
  that already handles it.
- Make patch-version dependency bumps that `cargo deny check` and the tests already validate.

### Standard

- Implement a step that a reviewed design document specifies, inside one crate.
- Fix a bug that has a clear reproduction and is confined to one crate.
- Add a formatter layout rule that snapshots and the safety properties cover, matching GJF's
  observed output.
- Write new tests for new behavior, or reduce a fuzz failure into a regression test.
- Refactor within one crate with no public API or contract change.
- Improve error messages, apart from the wording of public messages, which goes through review.
- Write user-facing docs for implemented behavior.
- Improve the corpus, oracle and measurement tools without changing what they accept.

### Deep

- Write or change a design document, create a new crate, or change crate boundaries or public APIs
  between crates.
- Any change listed in [high-risk-areas.md](high-risk-areas.md): the formatter's rewrites, comment
  placement, doc comment reflow and range formatting; lexing and source positions; the grammar,
  error recovery and language-version checks; name resolution, types, inference and the
  incremental engine; reading class files and containers; the oracles' normalization, known
  differences and thresholds; public contracts; stack depth; `unsafe`; CI.
- Decide what javac or GJF does in a case nobody has recorded yet, and record it.
- Do performance work that needs profiling and interpretation.
- Debug a flaky, non-deterministic or platform-only failure, or a fuzz failure whose cause isn't
  obvious.
- Review any Standard-tier change that touches a high-risk area.

## Moving between tiers mid-task

### Go up a tier when

- The same failure has survived two fix attempts.
- The change turns out to touch a [high-risk area](high-risk-areas.md).
- You find you need a public contract or cross-crate API change.
- Tests or oracle checks start failing in a crate you did not mean to touch.
- You are about to add `unsafe`, a dependency or a crate. Those also require asking first.
- The design document doesn't settle a question about Java semantics or upstream behavior.
- The diff grows past ~400 lines of non-test code or more than three crates.

### Go down a tier when

- A Deep-tier plan exists with explicit steps and acceptance criteria, and the remaining steps are
  mechanical.
- The work is bulk read-only exploration, log triage, or repetitive edits across many files that
  follow one pattern.

## Patterns that combine tiers

1. **Plan, execute, verify.** Deep writes the design document or plan. Standard implements it. For
   high-risk areas, Deep reviews the diff again before it is called done.
2. **Fan-out exploration.** Light-tier helpers search and summarize in parallel. The task's own tier
   puts the results together and decides.
3. **Never let a lower tier be the only reviewer** of a change in a high-risk area.
4. **Don't use Deep for everything.** Running a Deep model over a mechanical rename wastes time and
   money, and it doesn't make the result safer than running the tests.

## What to do if you can't choose your own model

1. State the tier at the start: `Tier: Deep (changes import removal in the formatter)`.
2. If you are running below the required tier:
   - **High-risk area:** stop after planning. Tell the human the task needs a Deep-tier model and
     hand over the plan.
   - **Otherwise:** go ahead with a narrower scope, and flag the change in the PR as needing a
     higher-tier review.
3. If you are running above the required tier, go ahead. Where your tool allows it, hand mechanical
   sub-steps to lighter helpers.
4. Record the tier or tiers used in the PR description.
