# Ferrion Roadmap

This document sets the order of implementation. What Ferrion is and is not is defined in
[SCOPE.md](SCOPE.md). If this roadmap and the scope disagree, the scope wins.

There is no timeline. Each phase ends with exit criteria. A phase is done when its criteria are met,
not when a date arrives.

## Strategy

The formatter needs only the lexer and parser. The linter needs full type checking, which is the
largest piece of the project. So:

- **Track A (formatter)** goes first and produces the **first release**: CLI, Maven plugin, and
  IntelliJ extension.
- **Track B (semantic analysis)** starts once the parser is stable and runs alongside the rest of
  Track A.
- **Track C (linter)** starts its framework early, ships rules that need only syntax and name
  resolution first, and adds rules that need types as Track B matures.

```
Phase 0 ── Phase 1 ── Phase 2 ──┬── A1 ── A2 ── A3 ── A4 ── A5 ──▶ First release (formatter)
foundations  lexer     parser   │
                                ├── B1 ── B2 ── B3 (a–d) ── B4 ── B5
                                │          │        │
                                └── C1 ────┴─ C2 ───┴── C3 ── C4 ── C5 ──▶ Linter release
                                                                      └── C6 (style rules)
```

## Phase 0: Foundations

- **Repository layout:**

  ```
  crates/                  Rust workspace (internal crates are `publish = false`)
    ferrion_cli/           `ferrion` binary
    ferrion_diagnostics/   spans, source files, rendering (pretty/json/github)
    ferrion_lexer/
    ferrion_parser/        AST, parser, error recovery, version checks
    ferrion_doc_comments/  Javadoc and Markdown doc comment parsing
    ferrion_formatter/
    ferrion_config/        ferrion.jsonc loading, flat-config resolution, JSON Schema
    ferrion_project/       inputs: source sets, project model, classpath and JDK validation
    ferrion_classfile/     class file, jar, jimage, ct.sym reading
    ferrion_semantic/      symbols, scopes, type attribution, flow analysis
    ferrion_linter/        rule framework, fixes, suppressions, baseline
    ferrion_rules/         built-in rules
    ferrion_lsp/
  build-plugins/maven/
  editor-plugins/intellij/
  tools/                   corpus fetcher, javac oracle harness, benchmarks
  docs/
  ```

- **Toolchain:** Rust 2024 edition, with a stable toolchain pinned in `rust-toolchain.toml`.
- **CI:** `rustfmt`, `clippy` (with warnings treated as errors), tests, and `cargo-deny` for licenses
  and advisories. Allowed licenses must be compatible with MIT OR Apache-2.0.
- **Licensing and provenance:**
  - `LICENSE-MIT` and `LICENSE-APACHE` files;
  - ScanCode in CI;
  - a clean-room policy in `CONTRIBUTING.md`;
  - a pull-request template with a provenance checklist item.
- **Corpus tooling:** `tools/corpus` fetches a pinned list of open-source projects, by commit hash,
  at test time. Nothing it fetches is committed.
  - the list spans Java 8 through the latest GA release, and includes projects that use Lombok and
    projects with `@formatter:off` markers;
  - test sources are included;
  - for each project and source set, it records source roots, classpath, Java version, and JDK, and
    fetches the dependency jars.
- **javac oracle:** `tools/javac-oracle` is a small Java harness, licensed under Ferrion's own license
  and used only in tests. It dumps javac's results:
  - tokens;
  - syntax errors from javac's parsing phase, per Java version;
  - tree positions;
  - later: resolved names, chosen methods, inferred and attributed types, constant values, variable
    finality, and switch errors.

  It analyzes one class at a time, so a javac crash only loses that class.

**Exit:** CI is green on an empty workspace with every check enforced, and the corpus and oracle
tools run.

## Phase 1: Lexer

- The full JLS lexical grammar for Java 8 through the latest GA release:
  - Unicode escapes;
  - text blocks;
  - all literal forms;
  - contextual keywords, which depend on the Java version.
- **Encoding detection:** UTF-8 and ASCII are accepted, detected from content. Other encodings
  produce a clear "skipped" result. Whether the file has a UTF-8 BOM is recorded so the formatter can
  preserve it.
- Comments and whitespace are preserved, with exact source positions.
- **Line endings:** the first line ending found is recorded for the formatter.

**Exit:**

- tokens match the javac oracle across the corpus for every supported version, after a documented
  normalization (how `>>` is split, contextual keywords, UTF-16 versus byte offsets);
- concatenating the tokens reproduces every corpus file byte for byte;
- fuzzing finds no panics.

## Phase 2: Parser

- **Design decisions,** recorded in `docs/design/`:
  - memory layout (for example, arena allocation);
  - how comments are attached to nodes;
  - node identity;
  - stack strategy for deep nesting (iterative algorithms, large worker stacks, or nesting caps);
  - parsing every version's syntax, with a separate version check.
- **Full syntax** for every supported version, including `module-info.java` and `package-info.java`.
- **Version check** with specific messages (for example, "records require Java 16+"), and a specific
  message for preview syntax.
- **Error recovery,** with tests for the quality of the recovered trees.
- **Doc comment parsers** that produce structured data:
  - Javadoc (`/** */`);
  - Markdown doc comments (`///`), from Java 23 only.

**Exit:**

- syntax errors match javac's parsing-phase errors for every supported version, across the corpus
  and javac's own test inputs (fetched at test time). Syntax-shaped errors Ferrion reports earlier
  than javac are documented;
- tree positions match the javac oracle;
- the tree reproduces every corpus file byte for byte;
- deep-nesting stress tests (including javac's own deep-nesting inputs) pass without stack overflow;
- fuzzing of the parser and doc comment parsers finds no panics.

## Track A: Formatter (first release)

### A1: Formatter core

- A Wadler–Leijen document representation and printer.
- Formatting rules for every syntactic construct: 2-space indentation, and a 100-column limit counted
  in UTF-16 code units.
- Modifiers reordered into JLS order (never across a formatter marker).
- Comments are placed correctly in the output.
- Line endings are preserved: the first one found is used, with LF as the fallback.
- An existing UTF-8 BOM is preserved, and none is added to files without one.
- `@formatter:off` / `@formatter:on`, in `//` and `/* */` comments. An unmatched `off` runs to the end
  of the file.
- Formatter markers and `ferrion-disable*` comments are never reflowed or moved.
- Files with syntax or version errors are skipped and reported.
- An implementation of the four safety properties, used by CI, fuzzing, and `--verify`.

**Exit:**

- the four safety properties hold across the entire corpus and every javac test input that parses;
- the formatter fuzz target checks the safety properties, and reduced failures are kept as regression
  tests;
- a style document, `docs/style.md`, lists every intentional difference from Google Java Format;
- the diff against Google Java Format is measured against a pinned version and JDK, compared with its
  fixed point, excluding files it rejects and files with disable markers, and tracked over time.

### A2: Formatter completeness

- **Imports:**
  - sorted into groups, with `import module` statements in their own group;
  - unused single-type imports removed, with usage detected from syntax and Javadoc references
    counting as usage;
  - imports of the file's own package removed;
  - module, wildcard, and `java.lang` imports left alone;
  - comments attached to an import move with it, and keep it from being removed;
  - comments attached to no import move to the top of the import section.
- **Javadoc reformatting,** with re-indent-only fallback for comments that can't be reflowed safely.
- **Markdown doc comment reformatting** (Java 23+), with the same fallback.
- **Range formatting,** including in files with syntax errors when the range isn't affected by an
  error.
- Rewrites are stable by construction (for example, imports are removed from the tree before layout),
  so a single pass is enough.

**Exit:**

- the A1 properties still hold across the corpus;
- dedicated test suites for imports, both comment styles, and ranges;
- a range-formatting test that breaks corpus files at random tokens and checks that unaffected ranges
  format the same as in the clean file.

### A3: CLI and configuration (formatting)

- `ferrion format`, with `--check`, `--java-version`, `--config`, `--output-format`
  (`pretty`, `json`, `github`), `--stdin` / `--stdin-filepath`, `--lines`, and `--verify`.
- `ferrion version [--json]` and `ferrion config <file>`.
- `ferrion.jsonc` loading:
  - flat-config resolution with `files`, `ignores`, global ignores, and `format.enabled`;
  - all paths relative to the working directory;
  - validation: unknown keys are errors with suggestions, every entry is validated, and a config
    error stops the run before any file changes;
  - `format` works without a config file;
  - a published JSON Schema.
- File selection: the default file set, hidden folders skipped, symlinks not followed, and explicitly
  passed ignored files skipped with a note.
- Input validation: a `--java-version` outside the supported range is a usage error.
- Exit codes 0–3. A crash in one file is reported as a Ferrion bug and the rest are still processed.
- The JSON output schema is versioned.

**Exit:** end-to-end CLI tests pass, including stdin, line ranges, and every exit code, and the config
error messages are reviewed for clarity.

### A4: Release pipeline and Maven plugin

- Cross-platform builds for every platform listed in the scope. Linux artifacts on Maven Central are
  static musl builds.
- Publishing to GitHub Releases, and per-platform binary artifacts to Maven Central with a manual
  confirmation step (Central releases can't be deleted).
- The Maven plugin (`build-plugins/maven/`):
  - runs on Java 8+ and Maven 3.6.3+;
  - format and check-format goals;
  - resolves the binary from its plugin dependency, copies it to a local per-user cache, and marks it
    executable;
  - the interface version check through `ferrion version --json`, with the compatibility matrix in the
    plugin's documentation;
  - one run for the whole build, from the root project, using the root project's config;
  - passes the Java version.
- The plugin's dependency license check runs in CI.

**Exit:** a sample multi-module Maven project formats and checks correctly on every platform in CI,
on both the minimum (Java 8, Maven 3.6.3) and the latest Java and Maven versions.

### A5: Language server and IntelliJ extension (formatting)

- `ferrion lsp`:
  - full-file and range formatting, returned as minimal edits;
  - reloads `ferrion.jsonc` on change, and shows config errors as diagnostics in the config file.
- The IntelliJ extension (`editor-plugins/intellij/`):
  - split mode, targeting 2026.3;
  - uses the built-in LSP support, and claims `.java` files for formatting;
  - finds the binary from the Maven plugin config (copied and marked executable), or from an
    application-level path setting;
  - shows a clear message when the binary is missing;
  - runs no language server in untrusted projects, and starts it when a project becomes trusted;
  - documents that refactorings and quick fixes still format with IntelliJ's own formatter.

**Exit:** formatting works in IntelliJ on the sample project, both from the Maven-resolved binary and
from a configured path, and nothing runs until the project is trusted.

### First release criteria

- Every phase from 0 through A5 is complete.
- No known bug in which formatting changes a program's meaning or drops a comment.
- Google Java Format users see a small, documented diff when switching.
- Benchmarks against Google Java Format (JVM and native image) are published. Wall-clock time, system
  time, and memory are expected to be lower. Any gap is tracked as a release-blocking issue unless
  it is explicitly waived.

## Track B: Semantic analysis

Until B4 lands, Lombok-annotated types are treated as incomplete, so rules stay silent on them.

### B1: Inputs and class file reading

- **Inputs** (`ferrion_project`):
  - single source set from `--java-version`, `--classpath` / `--classpath-file`, `--module-path`, and
    `--jdk` (falling back to `JAVA_HOME`);
  - multiple source sets from `--project-model`, each with its own source roots, generated-source
    roots, classpath, module path, Java version, JDK, and dependencies on other source sets;
  - validation: an unusable or too-old JDK, a missing jar, or missing type inputs that an enabled rule
    needs is a usage error; a missing classpath directory is a warning; unset and empty classpaths are
    distinguished; every message names where the value came from.
- **Class files** from jars and directories, including generic signatures, module descriptors,
  annotations (including those not visible at runtime), and type annotations.
- **Multi-release jars,** read for the target release.
- **The JDK platform API:**
  - JDK 8: the boot class path in `jre/lib` and the jars in `jre/lib/ext`;
  - JDK 9+: the module image (including compressed images such as the JetBrains Runtime) and `ct.sym`,
    handling its format differences between JDK versions;
  - only packages exported to everyone are visible.

**Exit:**

- every class in the JDK 8, 11, 17, 21, and latest GA platform APIs, from more than one JDK vendor
  and including a compressed image, and in the corpus's dependency jars, is read without loss compared
  with `javap`;
- fuzzing of the class-file, zip, jar, and module-image readers finds no panics.

### B2: Symbols and name resolution

- **Incremental design first:** a design spike for incremental computation, before the symbol tables
  are built. Conditions to meet:
  - cancellation in a few milliseconds;
  - scaling across threads;
  - a change inside a method body doesn't invalidate project-wide results;
  - memory: per-file summaries, rather than whole syntax trees, are kept for project-wide queries.
- Packages, modules, imports, scopes, and inheritance across all source sets. Dependent source sets
  and sibling modules are read from source.
- Source classes shadow classpath and platform classes as javac does.
- Duplicate fully-qualified names across source roots are ambiguous.
- Generated sources are included in the type information but marked so they are never linted or
  formatted.

**Exit:** names in the corpus resolve to the same symbols as in javac, except for a documented,
categorized list of disagreements. Code where javac reports errors is excluded.

### B3: Type attribution

B3 is split into milestones, each checked against the javac oracle:

- **B3a, type relations:** subtyping, capture, conversions, and least upper bounds.
- **B3b, attribution and overloads:** expression typing and overload resolution, without full
  inference. An error-typed argument gives an error-typed result, never a chosen overload.
- **B3c, inference:** generic type inference (JLS chapter 18), lambdas, and method references.
  Analysis limits resolve to "unknown", never a guess.
- **B3d, patterns and flow:** switch expressions and patterns, records and sealed types, switch
  exhaustiveness, constant folding, definite assignment, effective finality, and reachability.

Error types spread through any code that depends on an unresolved symbol or an incomplete hierarchy.

**Exit:**

- chosen methods, attributed types, constant values, variable finality, and switch errors match the
  javac oracle across the corpus, except for a documented, categorized list of disagreements. Code
  where javac reports errors is excluded;
- with symbols deliberately removed from the classpath, there are no crashes and no disagreements
  outside code that depends on those symbols;
- no file in the corpus or javac's test inputs exceeds the per-file time limit.

### B4: Lombok

- All stable Lombok features are modeled.
- `lombok.config` is honored.

**Exit:** the corpus projects that use Lombok reach complete analysis, with types matching javac plus
Lombok.

### B5: Incomplete-analysis reporting

- Per-file status: complete, partial, or skipped.
- An end-of-run summary.
- The analysis-incomplete diagnostic, reported only for root causes, with severity `info` by default.
- The classpath health check.
- `--require-complete-analysis`.

**Exit:** tests with deliberately broken classpaths and code that doesn't compile produce no false
positives, no cascades of follow-on diagnostics, and accurate summaries.

## Track C: Linter

### C1: Linter framework

- **Rule interface,** with metadata:
  - name, description, and examples;
  - the analysis it needs (syntax, resolution, or types) and the Java versions it applies to;
  - an options schema;
  - fix kinds, with the reason each is or isn't safe;
  - a "not reported" section;
  - `conflicts`, `aliases`, and an `experimental` flag;
  - upstream references, each with a parity note.
- **Running rules:**
  - type inputs are required, and attribution runs, only when an enabled rule needs types;
  - rules outside the target version's range don't run;
  - rules run on the parts of a file a syntax error doesn't affect.
- **Fix engine:**
  - fix kinds, initially `safe` and `dangerous`, and `--fix[=<kinds>]`;
  - conflict resolution when fixes overlap;
  - fixes applied repeatedly until nothing changes, with a pass limit;
  - a pass that leaves a syntax error is undone and reported as a Ferrion bug;
  - fixes never drop comments, and are never applied to files with syntax errors;
  - fixed regions are formatted, unless formatting is disabled for the file;
  - a dry-run diff.
- **Suppression comments:**
  - `ferrion-disable-next`, `ferrion-disable` / `ferrion-enable`, and `ferrion-disable-file`;
  - `ferrion-disable-next` covers the next statement or declaration, and can be stacked;
  - a missing reason or unknown rule name is an `error` finding, but the comment still suppresses;
  - unused suppression comments are reported, except for rules that didn't run on that code.
- **Config:** `rules`, where the last matching entry replaces the whole rule setting, and
  `linterOptions`. Unknown rule names are errors with suggestions, and enabling two conflicting rules
  for the same file is an error.
- The `plugins` key and `<plugin>/<rule>` names are reserved and validated, but plugins can't be
  loaded yet.
- **Commands:** `ferrion lint`, `ferrion rules`, and `ferrion init`. `lint` with no config, or no
  enabled rules, exits 2 and suggests `ferrion init`. `--max-warnings` is supported.
- Rule documentation is generated from metadata, and CI checks that it is up to date.

**Exit:** the framework is tested with sample rules, including fixes, fix convergence, rollback of a
broken fix, and every form of suppression.

### C2: Rules that need only syntax and name resolution

These are bug-finding rules that don't need type attribution, so the framework gets real use before B3
lands. Rules that need name resolution wait for B2. The candidate list below is a starting point, not
a commitment.

| Candidate rule | Upstream references |
| --- | --- |
| `no-self-assignment` | errorprone:SelfAssignment |
| `no-switch-fall-through` | errorprone:FallThrough, checkstyle:FallThrough, pmd:ImplicitSwitchFallThrough |
| `no-unused-variable` | errorprone:UnusedVariable, pmd:UnusedLocalVariable |
| `no-empty-catch` | pmd:EmptyCatchBlock, checkstyle:EmptyCatchBlock |
| `no-identical-operands` | errorprone:IdentityBinaryExpression |
| `no-ambiguous-precedence` | errorprone:OperatorPrecedence |
| `no-inner-assignment` | checkstyle:InnerAssignment, pmd:AssignmentInOperand |
| `no-unused-private-member` | pmd:UnusedPrivateField, pmd:UnusedPrivateMethod, errorprone:UnusedMethod |

**Exit:**

- each rule has documentation, tests, and a "not reported" section written from a false-positive
  review against the corpus;
- applying every fix across the corpus leaves files that parse, converge, and compile with javac with
  no new errors.

### C3: Rules that need types (after B3)

| Candidate rule | Upstream references |
| --- | --- |
| `no-equals-without-hashcode` | errorprone:EqualsHashCode, pmd:OverrideBothEqualsAndHashcode, checkstyle:EqualsHashCode |
| `use-equals-for-strings` | pmd:UseEqualsToCompareStrings |
| `no-boxed-primitive-equality` | errorprone:BoxedPrimitiveEquality |
| `no-array-to-string` | errorprone:ArrayToString |
| `no-array-equals` | errorprone:ArrayEquals |
| `no-incompatible-equals` | errorprone:EqualsIncompatibleType |
| `no-incompatible-collection-argument` | errorprone:CollectionIncompatibleType |
| `no-narrowing-compound-assignment` | errorprone:NarrowingCompoundAssignment |
| `no-ignored-return-value` | errorprone:ReturnValueIgnored, errorprone:CheckReturnValue, pmd:UselessOperationOnImmutable |
| `no-unthrown-exception` | errorprone:DeadException |
| `no-out-of-range-shift` | errorprone:BadShiftAmount |
| `no-format-string-mismatch` | errorprone:FormatString |
| `no-self-equals` | errorprone:SelfEquals |
| `no-infinite-recursion` | errorprone:InfiniteRecursion |

`no-equals-without-hashcode` needs types because an inherited `hashCode` counts, which requires
supertypes from the classpath.

**Exit:**

- each rule has documentation, tests, and a "not reported" section from a corpus review with no false
  positives, including on files with partial analysis and classes with incomplete hierarchies;
- applying every fix across the corpus leaves files that parse, converge, and compile with javac with
  no new errors.

### C4: Baseline (bulk suppressions)

- A suppressions file that records a count per file and per rule, with a format version field.
- `--suppress-all`, `--suppress-rule`, and `--prune-suppressions`.
- Suppressions that are no longer needed are reported, except for rules that didn't run on that code,
  which `--prune-suppressions` also leaves in place.

**Exit:** a corpus project can adopt Ferrion with a baseline, and only new violations are reported
afterwards, including on a run with a partial classpath.

### C5: Linter integrations

- **Maven plugin:**
  - a lint goal that runs after source generation;
  - one run for the whole build, with a generated project model covering the main and test source sets
    of each module: classpath, generated-source roots, Java version (`release`, `source`, or
    `testRelease`), and JDK (the configured Maven toolchain, otherwise the JDK running Maven).
- **Language server:**
  - diagnostics, with inputs per source root;
  - fix code actions, labeled by kind;
  - suppression code actions that prompt for the reason;
  - hover documentation for rules;
  - partial-analysis indication;
  - reloads the baseline file on change;
  - degraded mode when type inputs are missing: formatting and syntax-only rules keep working, and
    rules that need types are turned off with a visible notice;
  - stays quiet about linting when there is no config.
- **IntelliJ:** the extension passes the classpath, module path, Java version, and JDK per source root
  from the project model.

**Exit:**

- the sample multi-module Maven project lints correctly with one run, before `compile` has run;
- diagnostics appear in IntelliJ on the sample project, edits cancel stale analysis, and the degraded
  mode works when type inputs are removed.

### Linter release criteria

- Phases B1–B5 and C1–C5 are complete.
- No known false positives in shipped rules on the corpus.
- No known fix that breaks compilation.
- Benchmarks are published against PMD and Checkstyle (same rules) and Error Prone (its added cost
  over plain javac).

### C6: Style rules

| Candidate rule | Upstream references | Notes |
| --- | --- | --- |
| `no-unformatted-code` | | Powered by the formatter. Its fix formats the file, with kind `safe`. |
| `use-braces` | checkstyle:NeedBraces, pmd:ControlStatementBraces | |
| `use-naming-convention` | checkstyle:TypeName, checkstyle:MethodName, checkstyle:ConstantName | |
| `no-star-import` | checkstyle:AvoidStarImport | Its fix expands the wildcard into single-type imports when that is safe. |
| `no-unused-module-import` | | Needs the module's exported packages. |
| `no-redundant-import` | | Removes `java.lang` imports when no same-package type would be shadowed. |

**Exit:**

- formatted corpus output produces no findings from any style rule;
- no two rules report the same code, and no rule's fix undoes another's;
- applying every fix across the corpus leaves files that compile with javac with no new errors.

## Later

These are in scope but not scheduled:

- On-type formatting (formatting as `;` and `}` are typed).
- Modernization rules that depend on the target Java version (for example, `use-text-block`,
  `use-switch-expression`, `use-diamond`).
- Nullness rules, after the nullness annotation model is decided.
- An on-disk cache for class-file and JDK type information.
- User-defined rules: a design document first (how rules are written, the plugin package format, how
  user-specific plugin paths are passed), then the implementation.
- Gradle plugin (`build-plugins/gradle/`).
- VS Code extension (`editor-plugins/vscode/`).
- Formatter options.
- Experimental Lombok features.

## Cross-cutting work

These apply in every phase:

- **Corpus:** grows with each phase. Every bug fix adds a regression test.
- **Fuzzing:** every fuzz target runs in CI, and reduced failures are kept as regression tests.
- **Benchmarks:** start with A1. Performance regressions are flagged in CI.
- **Language updates:** each new GA Java release is a work item covering the lexer, parser, version
  check, formatter, type checker, and oracle updates. There is no fixed deadline. Language features
  that are still in preview are tracked but not implemented.
