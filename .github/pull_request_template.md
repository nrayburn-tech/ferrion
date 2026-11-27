## Summary

<!-- What does this change do, and why? Link any related issue or design document. For a behavior
     change, show before and after output captured by running Ferrion (see the pr-writer skill). -->

## Risk areas touched

<!-- See docs/agents/high-risk-areas.md. Check every box that applies. Any checked box needs a
     Deep-tier review. -->

- [ ] None
- [ ] Formatter output: rewrites, comments, doc comment reflow, ranges, `verify`
- [ ] Lexing and source positions
- [ ] Parsing, the syntax tree or error recovery
- [ ] Semantic analysis: names, types, inference, the incremental engine
- [ ] Reading class files, jars, module images or `ct.sym`
- [ ] Oracles, known differences, thresholds or the GJF pin
- [ ] Public contracts: CLI, `ferrion.jsonc` and its schema, the project model, rule names, output
      formats, message wording
- [ ] Stack depth or resource limits
- [ ] `unsafe`
- [ ] CI, fuzzing or supply-chain config

## Checklist

- [ ] **Provenance:** nothing in this change was copied or ported from another project, except
      code whose license allows distribution under both MIT and Apache-2.0, which keeps its notice
      and is identified below. See the
      [clean-room policy](https://github.com/nrayburn-tech/ferrion/blob/main/CONTRIBUTING.md#clean-room-policy).
- [ ] I have not studied an upstream implementation of what this change implements, or I have
      described below what I studied.
- [ ] Tests cover the change, and every bug fix includes a regression test.
- [ ] Every changed snapshot, known difference or threshold is explained in the summary.
- [ ] Documentation is updated (docs/SCOPE.md for any change in scope, docs/style.md for the
      formatter's style, the design document for a design change, and generated rule docs for any
      change in rule metadata).
- [ ] `just ready` passes, or the checks I ran and skipped are listed below, with any corpus or
      oracle checks.

## Provenance notes

<!-- Copied third-party code (source and license), or upstream implementations you have studied.
     Write "None" if there are none. -->

## Verification

<!-- The commands you ran, and the checks you skipped and why. -->

## AI assistance

<!-- Required. See docs/agents/model-tiers.md. -->

- AI-assisted: yes / no
- Model(s) and version(s): <!-- exact names as your tool reports them, or "unknown" -->
- Reasoning effort: <!-- e.g. low / medium / high, or "unknown" -->
- Tier(s) used: Light / Standard / Deep (and for which parts)
- Deep-tier review of risk areas done: yes / not applicable
- A human has read and understands every line of this diff: yes
