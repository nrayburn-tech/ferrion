# AGENTS.md: Ferrion

These are the main instructions for every coding agent working in this repository. Codex, Cursor,
Copilot, Gemini, Claude Code and others read this file. Humans should read
[CONTRIBUTING.md](CONTRIBUTING.md) too; it has the full commands and the clean-room policy.

The closest `AGENTS.md` to the file you are editing takes precedence. Keep this file short and put
details in `docs/`.

## What Ferrion is

Ferrion is a Java toolchain written in Rust: a formatter, a linter and a language server, built on
its own lexer, parser and type checker. [docs/SCOPE.md](docs/SCOPE.md) defines what it is and is
not, and its goals are strictly ordered: **correctness**, then user experience, then performance.
[docs/ROADMAP.md](docs/ROADMAP.md) sets the order of work. If the two disagree, the scope wins.

**Status: pre-release.** Nothing is published yet. The CLI, `ferrion.jsonc` and its schema, the
project model, rule names and machine-readable output become public contracts at the first release.
Treat them as if they already are.

Read the non-goals in [SCOPE.md](docs/SCOPE.md#non-goals) before you start, and don't build them
"while you're in there". Above all: **Ferrion never makes network requests**, and it does not
replace javac. If a task seems to need a non-goal, stop and ask.

## Repository map

```
Cargo.toml          workspace: shared dependencies and lints (members: crates/*, tools/*)
crates/             the Rust crates, each with its own AGENTS.md
tools/              corpus, javac and javap oracles, oracle-check, parser-check, format-check,
                    bench, codegen, scancode, unicode-tables: test and measurement tooling only
fuzz/               cargo-fuzz targets, seeds and dictionaries (own workspace, nightly toolchain)
schemas/            the JSON schema for ferrion.jsonc
docs/SCOPE.md       what Ferrion is and is not
docs/ROADMAP.md     phases, tracks and their exit criteria
docs/style.md       the formatter's style, construct by construct, and each departure from GJF
docs/design/        design documents: the decisions in force for each component
docs/agents/        agent guidance: model tiers, high-risk areas, verification
.github/workflows/  CI, fuzzing, the weekly oracle run, the daily advisory audit
justfile            `just ready` runs CI's required checks
```

## Commands

```bash
just ready                                            # the final checkpoint (needs a clean tree)
cargo fmt --all                                       # format
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked                       # hermetic: no JDK, network or corpus
cargo deny check                                      # licenses, advisories, bans, sources
cargo deny --manifest-path fuzz/Cargo.toml check      # the fuzz workspace, separately
cargo codegen                                         # after changing java.grammar
python3 tools/scancode/scan.py                        # copied notices and copyright holders
```

The corpus, oracle, parser-check, format-check and benchmark commands (`cargo corpus`, `cargo
oracle`, `cargo parser-check`, `cargo format-check`, `cargo benchmark`) need a JDK, Maven or a
fetched corpus. [CONTRIBUTING.md](CONTRIBUTING.md#development) explains each one.

**Choose checks based on what you changed. Don't run everything after every edit.** Follow
[docs/agents/verification.md](docs/agents/verification.md): scoped checks while iterating, then
`just ready` once at the end.

A new crate inherits every field set in `[workspace.package]` (`version`, `edition`, `license`,
`repository`, `publish`), each as `<field>.workspace = true`, and contains `[lints] workspace =
true`. Add a `crates/<name>/AGENTS.md` based on
[the template](docs/agents/crate-agents-template.md).

## Step 0: pick a model tier

Before you start, classify the task as **Light**, **Standard** or **Deep** using
[docs/agents/model-tiers.md](docs/agents/model-tiers.md). State the tier and the reason in one line
at the start of your work. If you are running below the tier the task needs, say so before you
change code.

Quick rule: if the task makes a change listed in
[docs/agents/high-risk-areas.md](docs/agents/high-risk-areas.md), it is **Deep**. That covers the
formatter's rewrites and comment handling, lexing and source positions, parsing and error recovery,
semantic analysis, reading class files and containers, the oracles and their known differences and
thresholds, public contracts, stack depth, `unsafe`, and CI.

## Guardrails

### Never

- Copy, port or work from the source of Google Java Format, javac or the rest of OpenJDK,
  Checkstyle, PMD, Error Prone, SpotBugs, Lombok or any other project whose license isn't
  compatible with MIT and Apache-2.0. That includes reading their source while you write the same
  thing, and translating it to Rust. Learn their behavior by running them on test inputs. If a
  question can only be answered by reading upstream source, stop and ask. The full rules are in
  [CONTRIBUTING.md](CONTRIBUTING.md#clean-room-policy).
- Commit files fetched at test time: corpus projects, javac's tests, upstream tools' test files.
- Publish, yank or tag releases. Never push, and never push to `main`, unless the human asks.
- Edit `Cargo.lock` or `fuzz/Cargo.lock` by hand, edit `LICENSE-*` or `NOTICE`, edit anything
  under `target/`, or hand-edit `crates/ferrion_syntax/src/**/generated.rs`.
- Add network access to Ferrion, or make a default test depend on the network, a JDK, Maven or the
  corpus.
- Change a snapshot, a recorded oracle output, a known difference, a threshold or a GJF diff count
  just to make a check pass, without explaining why the old expectation was wrong.
- Tidy test inputs. Recovery snapshots, fuzz regressions, fuzz seeds and oracle difference cases are
  often broken on purpose: unclosed comments, NUL bytes, trailing whitespace, mixed line endings.
- Silence a lint with a bare `#[allow]`. Prefer fixing the code. Otherwise use
  `#[expect(lint, reason = "...")]`, or `#[allow(lint, reason = "...")]` where the lint fires only
  in some builds.
- Pick a permanent identifier: the Maven groupId, the JetBrains plugin ID, or the crates.io name.
  The maintainer chooses them when the roadmap reaches A4 and A5.
- Build anything listed under non-goals.

### Ask first (stop and get a human yes)

- Adding a dependency, a new crate, or `unsafe` code.
- Any change to a public contract: commands, flags, exit codes, output formats, `ferrion.jsonc` and
  its schema, the project model, rule names, or message wording users and scripts read.
- Any change to [SCOPE.md](docs/SCOPE.md). A change in scope starts as an issue.
- Departing from a decision a design document in [docs/design/](docs/design/) records. Change the
  design document first, and get it reviewed, before code depends on the change.
- Editing CI workflows, `deny.toml`, either `rust-toolchain.toml`, workspace lints,
  `.cargo/config.toml`, `tools/scancode/`, `justfile`, or any `AGENTS.md`.
- Moving a pin: the Google Java Format version, a corpus project's commit, or the javac tests' tag.
- Running `cargo corpus lock`, which runs a corpus project's own build.
- Shortening or skipping CI's fuzz or oracle runs.

### Always

- Start from the scope, the roadmap stage you are working on, and the design documents for the
  area. Don't reopen decisions they already record.
- Keep each change small and about one thing. A change touching more than three crates or more than
  ~400 lines of non-test code needs a stated reason.
- Add a regression test that fails before the fix and passes after it. Reduced fuzz failures become
  regression tests (see [CONTRIBUTING.md](CONTRIBUTING.md#fuzzing)).
- Update docs in the same change: SCOPE.md for scope, [style.md](docs/style.md) for the formatter's
  style and its departures, the design document for a design change, and CONTRIBUTING.md for a new
  or changed command.
- When Java semantics are involved, cite the JLS, JVMS or JEP section in a comment rather than
  quoting it at length. When javac's or GJF's behavior is involved, find out by running the tool,
  and encode the answer in an oracle comparison, a difference case or a snapshot. Don't guess, and
  don't write expected output from memory.

## Rust conventions

- **Edition 2024**, toolchain pinned in `rust-toolchain.toml`; `fuzz/` uses the nightly pinned in
  `fuzz/rust-toolchain.toml`. Dependencies are declared only in `[workspace.dependencies]`, and
  crates use `dep.workspace = true`.
- **Hand-written front end.** The lexer and parser are hand-written; parser generators are not used.
  The syntax tree's kinds and typed views are generated from `crates/ferrion_syntax/java.grammar` by
  `cargo codegen`.
- **No panics on input.** The lexer, parser, doc comment parsers, class file reader and container
  readers never panic, on any input; the fuzz targets enforce it. Return a diagnostic or an error.
- **Deep input:** every recursive function whose depth follows the input calls
  `ferrion_stack::ensure_stack` on entry.
- **`unsafe` is denied** for the whole workspace. Only `tools/bench`'s allocator, which isn't
  shipped, opts out.
- **Determinism:** the same input gives the same output. Formatting depends only on the source, the
  Java version and the config, never on the JDK or classpath. Anything serialized, hashed or
  printed for machines uses ordered collections and stable sorting.
- **Positions:** line width is counted in UTF-16 code units, as GJF counts it. Be explicit about
  byte versus UTF-16 offsets.
- **Docs:** public items get doc comments. Module docs explain _why_, and link the design document
  they implement.

## Testing

- Unit tests sit next to the code. Integration tests go in `crates/<crate>/tests/`. Use the
  crate's existing test setup, and don't add test frameworks, helper crates or test styles without
  asking.
- Snapshot tests are rewritten with `FERRION_BLESS=1`, oracle snapshots with `--bless`. Review every
  resulting diff by hand. Never bulk-accept.
- `cargo test` is hermetic. The corpus and oracle checks are separate commands, and CI runs them in
  the `tools` job and the weekly oracle workflow.
- CI runs the tests on Linux, macOS and Windows. Anything involving paths, line endings or stack
  depth must pass on all three. Don't paper over a failure with `#[cfg(not(windows))]` without
  asking.

## Pitfalls

- **Unicode escapes in tool input.** A literal Java Unicode escape (a backslash, `u` and four hex
  digits) written through an agent's edit or write tool, or a heredoc, can arrive as the decoded
  character. The file is then wrong without breaking any build. Build the backslash in a script
  (Python `chr(92) + "u007d"`), then grep the file to confirm it landed. Rust string literals
  written as `"\\u0041"` survive; comments, Markdown and fuzz seeds don't.
- **One heavy job at a time.** Cargo commands that share a `target/` directory contend for its lock.
  Corpus, oracle, fuzz and benchmark runs use a lot of memory; don't run several at once, and don't
  run them in parallel with someone else's.
- **Local fuzz runs are short.** Run a target for 30 seconds (`-max_total_time=30`) locally. CI runs
  the long ones.

## Definition of done

1. `just ready` passes, or you list exactly which of its checks you ran and which you skipped.
2. New behavior has tests. Bug fixes have a regression test.
3. Docs are updated (see "Always").
4. For high-risk changes, a Deep-tier review was done and is noted in the PR.
5. The PR description discloses AI assistance: exact model and version, reasoning effort and tier.
   If you don't know the exact model or effort, say so; don't guess.

## Commits and PRs

- **Commit messages** have a short, imperative subject that states the behavior, not the file
  ("Keep the hyphen rules on bytes in a doc comment's reflow"), and a body wrapped at 72 columns
  that explains what changed and why.
- **Every commit stands alone.** Each one builds, passes the checks and does one thing. Nothing in
  it names a file, command or check that only a later commit adds. History on `main` is linear:
  rebase, never merge (see [CONTRIBUTING.md](CONTRIBUTING.md#rebase-dont-merge)).
- Agents do not commit, push or open PRs unless the human asks them to.
- Fill in [the PR template](.github/pull_request_template.md) completely, including the provenance
  items.
  Behavior changes show before and after output captured by actually running Ferrion.
