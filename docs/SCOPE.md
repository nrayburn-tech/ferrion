# Ferrion Scope

Ferrion is a Java toolchain written in Rust: a formatter, a linter, and a language server, built on a
custom lexer, parser, and type checker. It aims to replace Google Java Format for formatting, and
Checkstyle, PMD, and Error Prone for static analysis.

This document defines **what** Ferrion is and is not. The order of implementation lives in
[ROADMAP.md](ROADMAP.md).

## Goals

Goals are strictly ordered. When two goals conflict, the higher one wins.

1. **Correctness.** Ferrion never changes the meaning of a program when formatting or fixing, never
   reports a finding it cannot justify, and never implies code is valid when it could not fully
   analyze it.
2. **User experience.** Clear diagnostics, predictable behavior, one config file, low-friction
   integration with the CLI, Maven, and IntelliJ.
3. **Performance.** Wall-clock time, system time, and total memory use are expected to be lower than
   the upstream alternatives for equivalent work. There are no fixed numeric targets.

## Principles

- **No network access.** Ferrion never makes network requests. That rules out downloads, update
  checks, and telemetry. Fetching artifacts (binaries, rule plugins) is the job of the user's build
  tool or package manager.
- **Not a compiler.** Ferrion does not replace javac's error checking and does not produce bytecode.
  Code that fails to compile must not cause false positives, and the absence of findings must not be
  presented as proof that code is valid (see [Incomplete analysis](#incomplete-analysis)).
- **Clean-room implementation.** Upstream tools' code is off-limits except where its use is
  compatible with Ferrion's MIT OR Apache-2.0 licensing. Documentation and observable behavior may be
  used as references (see [Licensing and provenance](#licensing-and-provenance)).
- **Environment stays out of config.** Machine- and user-specific inputs (JDK path, classpath, module
  path, Java version) are passed as CLI arguments, or in a machine-generated project model file, never
  stored in `ferrion.jsonc`.
- **Same input, same output.** Formatting output depends only on the source file, the Java version,
  and the config. It never depends on the JDK, the classpath, or which tool (CLI, Maven, IDE) runs it.

## Language support

- **Java versions:** Java 8 through the latest generally available (GA) release. A Java version
  outside this range is a usage error.
- **Preview features:** not supported. A feature is supported once it ships as final (non-preview)
  in a GA release. Code that uses preview syntax gets a specific "preview feature, not supported"
  message rather than a generic syntax error.
- **Version-aware:** the target Java version is required for both linting and formatting. It is given
  by `--java-version`, or per source set in the project model.
- **File kinds:** ordinary compilation units, `module-info.java`, and `package-info.java`.
- **Encodings:** UTF-8 and ASCII. Files in any other encoding are skipped with a clear message.
- **Byte-order mark:** a UTF-8 BOM is supported. If a file has one, the formatter preserves it. If it
  doesn't, none is added.
- **Other JVM languages:** Kotlin, Groovy, Scala, and other source languages are out of scope. Their
  compiled classes on the classpath are consumed like any other class file.
- **New Java releases:** there is no fixed deadline for supporting a new GA release.

## Components

### Lexer and parser (internal)

The lexer and parser are internal and not published as a public API or as crates on crates.io.

- Implements the Java Language Specification lexical and syntactic grammar for every supported
  version, including Unicode escape processing.
- **Lossless:** the syntax tree reproduces the source byte for byte, including comments and
  whitespace.
- **Every version's syntax, then a version check:** the parser accepts the syntax of all supported
  versions, then a separate check reports constructs the target version doesn't allow (for example,
  "records require Java 16+"). Consumers always get a complete node rather than an error node.
- **Never fails:** parsing always returns a tree plus diagnostics. It never panics, and holds no
  policy about what to do with errors. Each consumer decides.
- **Error recovery:** produces a usable partial tree for files with syntax errors, so the language
  server can still work on files that are mid-edit.
- **Deep nesting:** deeply nested input (long `+` chains, long `else if` chains, generated code) never
  overflows the stack.
- Parses into structured data:
    - `module-info.java` declarations;
    - `package-info.java` declarations and annotations;
    - traditional Javadoc comments (`/** ... */`): tags, inline tags, HTML, and `{@code}` and `<pre>`
      blocks;
    - Markdown documentation comments (`///`) when the target version is Java 23 or later. Below Java
      23, `///` comments are plain line comments.

### Semantic analysis and type checking (internal)

- **Full type attribution** for every supported version. This covers name resolution, scopes,
  inheritance, overload resolution, generic type inference, lambdas, method references, records,
  sealed types, and pattern matching.
- **javac wins over the JLS.** Where javac's observable behavior differs from a literal reading of the
  JLS, Ferrion follows javac. A literal reading fails on common code such as
  `stream.collect(Collectors.toList())`. Deliberate departures from javac (for example, where javac is
  unsound) are documented.
- **Analysis rules depend on:** constant folding, definite assignment, effective finality,
  reachability, and switch exhaustiveness.
- **Cross-file analysis:** project-wide symbol resolution across all supplied sources.
- **Source sets:** main and test code (and any other set) are analyzed as separate source sets, each
  with its own source roots, classpath, module path, Java version, and JDK. A source set can depend on
  another, whose code is read from source, so analysis works before anything is compiled.
- **Multi-module builds:** one run covers the whole build. Sibling modules are read from source rather
  than from `target/classes`, so linting works before `compile`.
- **Inputs:**
    - project sources, including generated sources;
    - the classpath, passed directly or through a file containing the classpath;
    - the module path, treated leniently like the classpath at first;
    - a user-supplied JDK, used for the platform API of the target Java version;
    - for multiple source sets, a machine-generated **project model** file describing each set. It is
      written by integrations (Maven plugin, IntelliJ), never hand-edited, and never committed.
- **Type inputs are needed only when used.** The JDK and classpath are required only when an enabled
  rule needs types.
- **Platform API:**
    - JDK 8: javac 8's default boot class path in `jre/lib` (`rt.jar`, `jce.jar`, `jsse.jar`,
      `charsets.jar`, and the others present), then the jars in `jre/lib/ext`. A JDK 8 can only serve
      release 8;
    - JDK 9+: the module image, and `ct.sym` for older releases. Each JDK's `ct.sym` covers a limited
      range of releases, and its format differs between JDK versions;
    - compressed module images (such as the JetBrains Runtime) are supported;
    - JDK internals that aren't exported (such as `jdk.internal.misc`) don't resolve, as with javac.
- **Class files:** multi-release jars are read for the target release. Annotations that aren't visible
  at runtime, and type annotations, are read, because rules need them (for example,
  `@CheckReturnValue`).
- **Input validation:**
    - usage errors (exit 2): a JDK that is older than the target version, can't serve it, or isn't a JDK
      at all; a missing classpath jar; type inputs missing when an enabled rule needs them;
    - warnings: a classpath directory that doesn't exist, since it may be build output that hasn't been
      produced yet;
    - an unset classpath is different from an explicitly empty one.

  Every message names where the value came from (flag, project model, or `JAVA_HOME`).
- **Shadowing:** source classes shadow classpath and platform classes the same way javac does.
- **Duplicate class names:** when the same fully-qualified name is declared in more than one source
  root (for example, Guava's GWT super-source), the name is treated as ambiguous and rules stay
  silent on it.
- **Generated code** is fully parsed and type-checked but never linted or formatted. It is identified
  by generated-source roots in the inputs (the Maven plugin supplies these automatically) or by
  `@Generated` annotations.
- **Lombok** receives dedicated support:
    - all stable Lombok features are modeled during type checking;
    - `lombok.config` files are honored;
    - experimental Lombok features are out of scope for now.
- **Termination:** analysis has per-file limits. When a limit is exceeded, the result is "unknown"
  (treated like an error type), never a guess, and analysis never hangs.

#### Incomplete analysis

When code does not fully resolve (missing classpath entries, code that doesn't compile), Ferrion
behaves as follows:

1. **Error-type propagation.** Unresolved symbols get an internal error type. Rules stay silent on
   any code that involves an error type. This is what prevents false positives. Specifically:
    - a class whose supertypes don't all resolve has an incomplete hierarchy, and rules treat its
      members, overrides, and subtyping as unknown;
    - overload resolution with an error-typed argument produces an error type, never a confidently
      chosen method;
    - until Lombok support is complete, Lombok-annotated types are treated as incomplete.
2. **Per-file analysis status.** Every file is *complete*, *partial* (unresolved symbols or syntax
   errors limited the analysis), or *skipped* (unsupported encoding). Every run ends with a summary
   that includes these counts, even when there are no findings.
3. **Analysis-incomplete diagnostic.** Reported only for root causes, not for everything that follows
   from them. It is worded as a statement about Ferrion's coverage, not as a compile error. For
   example: "Cannot resolve `com.acme.Foo`; rules depending on it were not run here." Severity is
   configurable and defaults to `info`.
4. **Strict mode.** `--require-complete-analysis` turns any partial or skipped file into a failing
   exit code. This is recommended in the CI setup documentation.
5. **Classpath health check.** When a large share of imports fail to resolve, Ferrion reports one
   prominent "classpath appears incomplete" warning instead of a flood of individual diagnostics.
6. **Editor indication.** The language server shows partial files with a status indicator and
   low-severity hints at unresolved locations.
7. **Plain wording.** Output and documentation state that Ferrion is not a compiler, and that no
   findings does not mean the code compiles.
8. **Suppressions stay valid.** A suppression comment or baseline entry for a rule that didn't run (or
   didn't run on that code) is never reported as unused, and `--prune-suppressions` never removes it.

### Formatter

- Built on the **Wadler–Leijen** pretty-printing model.
- **One opinionated style**, expected to be close to Google Java Format but not matching it byte for
  byte. Intentional differences are documented. Options and configurability may come in later phases.
- **No type information:** the formatter needs no JDK or classpath (see "Same input, same output").
- **Layout:**
    - 2-space indentation;
    - 100-column limit for both code and documentation comments, measured as total line width
      including indentation and comment markers;
    - width is counted in UTF-16 code units, as Google Java Format counts it.
- **Rewrites beyond layout:**
    - modifiers are reordered into the standard JLS order;
    - long string literals are **not** split;
    - text blocks are **not** re-indented (for now);
    - `//` comments are **not** wrapped (for now).
- **Documentation comments:**
    - Javadoc and Markdown doc comments are reformatted;
    - comments that can't be reflowed safely (for example, unclosed `<pre>`, `<code>`, or `<table>`
      tags, unclosed inline tags, or Markdown setext headings) are re-indented only;
    - `///` comments below Java 23 are plain line comments and are not reflowed.
- **Imports:**
    - sorted into groups, with `import module` statements in their own group;
    - unused single-type imports are removed. Usage is detected from syntax, and Javadoc references
      count as usage;
    - single-type imports of the file's own package are removed;
    - module imports, wildcard imports, and `java.lang` imports are never added, removed, or expanded by
      the formatter, because doing so safely needs type information. Linter rules handle them (see the
      [Linter](#linter));
    - a comment belongs to an import when it is on the same line after the import, or directly above
      it with no blank line in between. It moves with that import when sorting, and an import with a
      comment is never removed, even if unused;
    - comments that belong to no import (such as a group header followed by a blank line) move to the
      top of the import section, keeping their order relative to each other.
- **Range formatting:** formatting a selected range is supported, for editors and changed-lines
  workflows. It works in files with syntax errors, as long as the range isn't affected by an error.
- **Disable comments:** only the Eclipse/IntelliJ markers are used. There are no Ferrion-specific
  formatter markers.
    - `@formatter:off` and `@formatter:on`, in either `//` or `/* */` comments, mark a region that is
    left exactly as written;
  - an `@formatter:off` with no matching `@formatter:on` stays in effect to the end of the file, so
    placing it before the `package` line leaves the whole file untouched;
  - modifiers are never reordered across a marker.
- **Directives are never wrapped:** formatter markers and `ferrion-disable*` suppression comments are
  never reflowed or moved.
- **Line endings:** the first line ending found in a file is used for the entire output file. If none
  can be found, LF is used.
- **Byte-order mark:** an existing UTF-8 BOM is preserved, and none is added to files without one.
- **Syntax and version errors:** whole-file formatting skips files with syntax errors or version
  errors, and the skip is reported to the user.
- **Single pass:** the library parses and formats each file once. Rewrites (import removal, modifier
  ordering, doc comment reflow) must be stable by construction, so formatting the output again changes
  nothing.
- **Safety properties,** checked in CI and fuzzing, and at runtime with the opt-in `--verify` flag:
  1. formatting twice changes nothing;
  2. the output is equivalent to the input, allowing only the formatter's intended rewrites (import
     sorting and removal, modifier reordering, doc comment reflow);
  3. every comment appears exactly once (modulo reflow);
  4. the output has no syntax errors the input didn't have.

  With `--verify`, a file that fails any property is not written, and the run exits 3.
- **Output stability:** formatting output is not covered by semantic versioning and may change in any
  release. Changes should be minimized and are called out in release notes.

### Linter

- **Rules** are individual units with no categories, groups, or presets. Each rule is enabled or
  disabled on its own, and **no rules are on by default**.
- **Rule metadata:**
  - a kebab-case name. Names should follow `no-*` (the rule flags something to avoid, such as
    `no-equals-without-hashcode`) or `use-*` (the rule flags a missing preferred construct, such as
    `use-try-with-resources`). This pattern is preferred but not required, for rules where neither
    form reads naturally;
  - a description and examples;
  - the analysis the rule needs: syntax only, name resolution, or full types. This decides whether a
    run needs type inputs at all;
  - the Java versions the rule applies to. Rules outside the target version's range don't run;
  - an options schema, with each option's type and default;
  - supported fix kinds, and why each fix is or isn't safe;
  - a "not reported" section listing the cases the rule deliberately ignores, written from the
    corpus false-positive review;
  - `conflicts`: rules that contradict this one (for example, one requiring `var` and one forbidding
    it). Enabling both for the same file is a config error;
  - `aliases`: former names, which keep working in config, suppression comments, and the baseline
    file after a rename;
  - an `experimental` flag;
  - references to equivalent upstream checks where applicable, such as
    `errorprone:EqualsHashCode` and `pmd:OverrideBothEqualsAndHashcode`. Each reference carries a
    parity note: equivalent, subset, superset, or different defaults.

  Rule documentation is generated from this metadata, and CI checks that the generated docs are up to
  date.
- **Rule focus:**
  1. bug-finding rules first;
  2. style rules second, including a `no-unformatted-code` rule driven by the formatter, import rules
     that need type information (unused module imports, wildcard imports, redundant `java.lang`
     imports), and additional style checks that the formatter does not cover.
- **No overlapping rules:** two rules never report the same code, and one rule's fix never undoes
  another's. Code produced by the formatter never triggers a style rule.
- **Syntax errors:** rules run on the parts of a file that aren't affected by a syntax error, both in
  the CLI and in the language server.
- **Fixes:**
  - every fix has a kind, initially `safe` or `dangerous`, and the set of kinds is open to
    extension;
  - `--fix` applies safe fixes, and `--fix=<kinds>` (for example `--fix=safe,dangerous`) applies the
    listed kinds;
  - a dry-run option prints the diff instead of writing files;
  - fixes are applied repeatedly until nothing changes, up to a fixed number of passes;
  - a pass whose fixes leave a syntax error is undone and reported as a Ferrion bug, never written;
  - fixes never drop comments, and are never applied to files with syntax errors;
  - fixed code is formatted (only the regions a fix changed), unless formatting is disabled for that
    file;
  - fixed code must still compile; this is checked across the corpus in CI.
- **Suppression comments** (the only way to suppress inline). Every suppression comment must include
  a reason after `--`:
  - `// ferrion-disable-next <rule> -- <reason>` applies to the next statement or declaration,
    excluding the contents of any nested block. For example, above a method it covers the signature
    but not the body. Several of these comments can be stacked so that each rule has its own reason;
  - `// ferrion-disable <rule> -- <reason>` and `// ferrion-enable <rule>` mark a region;
  - `// ferrion-disable-file <rule> -- <reason>` applies to the whole file;
  - suppression comments that suppress nothing are reported, with a configurable severity (subject to
    [Incomplete analysis](#incomplete-analysis) item 8);
  - a suppression comment with no reason or an unknown rule name is an `error`-level finding, but it
    still suppresses.
- **Baseline (bulk suppressions)**, modeled on ESLint's bulk suppressions:
  - a suppressions file records a violation count per file and per rule, so unrelated edits don't
    invalidate it. The file has a format version field;
  - the commands are `--suppress-all`, `--suppress-rule <rule>`, and `--prune-suppressions`;
  - violations beyond the recorded count are reported, and suppressions that are no longer needed are
    reported too.
- **User-defined rules** will eventually be supported. How they are written is undecided. Reserved
  now:
  - a `plugins` config key that maps a short name to a local plugin location;
  - plugin rules are referenced as `<plugin>/<rule>`, and built-in rules have no prefix;
  - plugins are resolved only from local paths. Ferrion never downloads them, and the user's build
    tool or package manager fetches them.
- **No rule-count target** for the linter release.

## Configuration

A single `ferrion.jsonc` file configures both the formatter and the linter. It is modeled on ESLint's
flat config, adapted to JSONC.

- **Location:** by default, `ferrion.jsonc` in the current working directory. `--config <path>` points
  to a different file. Configs do not cascade or merge across directories.
- **No config file:** `format` works without one. `lint` with no config, or with no rules enabled,
  exits 2 and suggests `ferrion init`, so an empty rule set never looks like clean code.
- **Root object:** the file is an object containing `$schema`, `plugins`, `linterOptions`, the
  baseline file location, and `configs`, an ordered flat-config array.
- **Config entries:**
  - **`files` and `ignores`:** globs that select which files an entry applies to;
  - **global ignores:** an entry that contains only `ignores` applies to all files;
  - **default scope:** an entry without `files` applies to all `**/*.java` files;
  - **`rules`:** a rule setting is either a level string or `{ "level": ..., "options": { ... } }`.
    The levels are `off`, `info`, `warn`, and `error`;
  - **`format`:** per-entry formatter settings. Initially only `{ "enabled": false }` to exclude
    files from formatting.
- **Precedence:** later entries win. The last entry that matches a file and mentions a rule supplies
  that rule's **entire** configuration. There is no merging of levels or options across entries.
- **Paths:**
  - all paths and globs are relative to the current working directory;
  - `.gitignore` is **not** respected by default;
  - integrations (Maven plugin, language server) run Ferrion from the project root.
- **Validation:**
  - unknown keys, rule names, and plugin names are config errors, with "did you mean" suggestions;
  - every entry is validated, even if it matches no file;
  - a config error stops the run before any file is changed.
- **Naming:** keys are camelCase, and rule names are kebab-case.
- **Not in config:** the JDK path, classpath, module path, and Java version.
- **Not supported:** `extends`, which covers both shared config files and plugin-provided presets.
- **Editor support:** a JSON Schema is published for editor completion and validation.

Example:

```jsonc
{
  "$schema": "https://…/ferrion.schema.json",
  "linterOptions": { "reportUnusedDisableDirectives": "warn" },
  "configs": [
    { "ignores": ["**/legacy/**"] },
    {
      "files": ["**/*.java"],
      "rules": {
        "no-equals-without-hashcode": "error",
        "no-unused-variable": { "level": "warn", "options": { "ignorePattern": "^_" } }
      }
    },
    {
      "files": ["src/test/**"],
      "rules": { "no-unused-variable": "off" },
      "format": { "enabled": false }
    }
  ]
}
```

## Command-line interface

**Commands:**

- `ferrion format`: format files. `--check` reports unformatted files without writing.
- `ferrion lint`: lint files, with optional fixes.
- `ferrion lsp`: start the language server.
- `ferrion rules`: list rules and explain individual rules.
- `ferrion init`: write a `ferrion.jsonc` listing every rule set to `off`, each with a one-line
  description comment.
- `ferrion config <file>`: print the effective config for a file, and which entry each setting came
  from.
- `ferrion version`: print the version. `--json` adds machine-readable details, including the CLI
  interface version that the Maven plugin checks.

**Key arguments** (exact names may change before 1.0):

| Argument | Purpose |
| --- | --- |
| `--java-version <N>` | Target Java version. Required for `format` and `lint` unless a project model gives it. |
| `--classpath <cp>` / `--classpath-file <path>` | Classpath, given directly or through a file. |
| `--module-path <mp>` | Module path. |
| `--jdk <path>` | JDK used for platform types. Falls back to `JAVA_HOME`. |
| `--project-model <path>` | Machine-generated description of multiple source sets. |
| `--config <path>` | Config file. Defaults to `./ferrion.jsonc`. |
| `--output-format <name>` | `pretty` (default), `json`, `github`. |
| `--fix[=<kinds>]` | Apply fixes of the listed kinds. Plain `--fix` means `--fix=safe`. |
| `--max-warnings <N>` | Fail the run when there are more than N `warn` findings. |
| `--require-complete-analysis` | Fail on partial or skipped files. |
| `--stdin` / `--stdin-filepath <path>` | Read one file from stdin. The path selects config entries. |
| `--lines <A:B>` | Format only the given line range. |
| `--verify` | Check the formatter's safety properties at runtime. |

**Which files a run covers:**

- with no paths given, all `**/*.java` files under the working directory;
- hidden folders are skipped, and symlinks aren't followed;
- an explicitly passed file that matches `ignores` is skipped with a note, so hooks such as
  pre-commit can pass every changed file.

**Output formats:**

- `pretty`: readable terminal output with code excerpts;
- `json`;
- `github`: GitHub Actions annotations.

Output formats are pluggable internally, so adding new ones is cheap.

**Exit codes:**

| Code | Meaning |
| --- | --- |
| 0 | Clean. `warn` and `info` findings alone don't fail a run. |
| 1 | `error` findings, more warnings than `--max-warnings`, unformatted files, files skipped because of syntax errors, or partial analysis under `--require-complete-analysis` |
| 2 | Usage or configuration error |
| 3 | Internal Ferrion error: a crash, a failed `--verify`, or a fix that broke code or didn't converge |

A crash in one file is reported as a Ferrion bug, and the rest of the files are still processed.

**Versioning:** semantic versioning covers:

- the CLI interface;
- the JSON output schema;
- rule names (a rename keeps the old name as an alias; removing a rule is a major release);
- rule options;
- the shape of `ferrion.jsonc`;
- the baseline file format.

It does not cover formatting output.

## Language server

- **Features:**
  - diagnostics;
  - formatting of whole files and ranges, returned as minimal edits so the cursor and selections stay
    put;
  - code actions for fixes, labeled by fix kind;
  - code actions to suppress a finding on the next statement or for the file, prompting for the
    reason;
  - hover documentation for rules.
- **Out of scope:** navigation, completion, and refactoring features. Ferrion is not a Java IDE
  backend.
- **Inputs:** the client supplies the classpath, module path, Java version, and JDK per source root,
  not one set for the whole workspace.
- **Reload:** changes to `ferrion.jsonc` and the baseline file are picked up without a restart.
- **Config errors** are shown as diagnostics in the config file.
- **Degraded mode:** when type inputs are missing, formatting and syntax-only rules keep working, and
  rules that need types are turned off with a visible notice.
- **No config:** the language server stays quiet about linting rather than reporting an error.
- **State:** analysis state is kept in memory between edits, and is designed for incremental
  updates.

## Integrations

### Maven plugin (`build-plugins/maven/`)

- Runs the native Ferrion binary as a subprocess. No JNI.
- **Requirements:** runs on Java 8+ and Maven 3.6.3+.
- **Getting the binary:**
  - binaries are published to Maven Central as one artifact per platform. Linux artifacts are static
    musl builds, so one binary works on every distribution;
  - the user declares the binary as a plugin dependency, and Maven downloads it;
  - Maven doesn't keep the executable bit, so the plugin copies the binary to a local per-user cache,
    marks it executable, and runs it from there;
  - nothing is bundled into the plugin.
- **Versioning:**
  - plugin and binary versions are independent;
  - a published compatibility matrix maps plugin versions to the supported CLI interface major
    versions;
  - at runtime the plugin checks the binary's reported interface version (`ferrion version --json`)
    and fails with a clear message, linking to the matrix, when they are incompatible.
- **One run per build:** Ferrion runs once for the whole multi-module build, from the root project
  directory, using a generated project model.
- **What it passes automatically,** per source set (main and test of each module):
  - the classpath;
  - generated-source roots;
  - the Java version (from `maven.compiler.release` or `source`, and `testRelease` for tests);
  - the JDK: the configured Maven toolchain if there is one, otherwise the JDK running Maven.
- **Lint goal ordering:** the lint goal runs after source generation, so generated sources exist.
- **Config:** the root project's `ferrion.jsonc` is used for all modules, and the plugin
  configuration can override the path.

### IntelliJ extension (`editor-plugins/intellij/`)

- Uses IntelliJ's split-mode architecture, targeting the 2026.3 EAP.
- Talks to `ferrion lsp` through IntelliJ's built-in LSP support.
- **Formatting:** the extension claims `.java` files for formatting (by default, IntelliJ keeps Java
  formatting for itself). Formatting done implicitly by refactorings and quick fixes still uses
  IntelliJ's own formatter. This is documented, since it can produce `format --check` failures in CI.
- **Finding the binary:**
  - for Maven projects, it uses the binary version configured in the project's Ferrion Maven plugin,
    from the local Maven repository (copied and marked executable, as the Maven plugin does);
  - otherwise, it uses a binary path set in the extension's settings. That setting is
    application-level only, so a cloned repository can't point the extension at a different binary;
  - it never downloads or bundles the binary. If the binary is missing, the extension says so.
- **Trusted projects:** no language server runs in an untrusted project. IntelliJ doesn't start
  language servers when a project becomes trusted, so the extension starts them itself.
- **Project settings:** it supplies the classpath, module path, Java version, and JDK per source root,
  from IntelliJ's project model.

### Later integrations

These are out of scope for the first releases:

- Gradle plugin (`build-plugins/gradle/`);
- VS Code extension (`editor-plugins/vscode/`).

## Distribution

| Artifact | Channel |
| --- | --- |
| CLI binary | GitHub Releases |
| CLI binary (per-platform artifacts for Maven) | Maven Central |
| Maven plugin | Maven Central |
| IntelliJ extension | JetBrains Marketplace |

- **Platforms:**
  - macOS: arm64, x64;
  - Linux: x64, arm64 (glibc and musl on GitHub Releases; static musl on Maven Central);
  - Windows: x64, arm64.
- **Maven Central releases can't be deleted,** so publishing to Central has a manual confirmation
  step.
- **Rule documentation** is generated from rule metadata. Where it is published is undecided.

## Performance

- Wall-clock time, system time, and total memory are expected to beat upstream alternatives doing
  equivalent work. There are no fixed numeric targets.
- **Comparison baselines:**
  - **formatter:** Google Java Format, on both the JVM and its native-image build;
  - **Checkstyle and PMD:** the same rules, run in each tool;
  - **Error Prone:** ferrion's total time compared with Error Prone's added cost (javac with Error
    Prone minus plain javac). Results are reported honestly, including where ferrion loses.
- **Caching:** an on-disk cache of class-file and JDK type information is planned for a later phase.
  The first linter release caches in memory only.

## Verification

- **javac as a reference,** compared through an oracle harness:
  - tokens, after normalization (how `>>` is split, contextual keywords, UTF-16 versus byte
    offsets);
  - syntax errors javac reports while parsing, per Java version. Errors javac reports in later phases
    are not compared, and syntax-shaped errors Ferrion reports earlier than javac are documented;
  - tree positions;
  - resolved names, chosen methods, attributed types, constant values, and variable finality.
- **Agreement with javac** is measured, not assumed to be perfect. Disagreements are allowed only as
  a documented, categorized list (javac error recovery, javac quirks, deliberate departures). Code
  where javac itself reports errors is excluded.
- **Real-world corpus:** CI runs against a pinned set of real open-source projects spanning Java 8
  through the latest GA release, including their test sources. For each project, the corpus records
  source roots, classpath, Java version, and JDK, and fetches its dependency jars.
- **Formatter properties:** the four safety properties are checked in CI across the corpus and
  javac's own test inputs.
- **Google Java Format comparison:** measured against a pinned Google Java Format version and JDK,
  compared with its fixed point (its own output isn't always stable), and excluding files it rejects
  and files with disable markers.
- **Fixes:** applying every fix across the corpus leaves files that parse, converge, and compile with
  javac with no new errors.
- **Formatter and linter agreement:** formatted corpus output produces no style-rule findings.
- **Fuzzing:**
  - the lexer, parser, doc comment parser, class-file reader, and zip/jar/module-image readers are
    fuzzed, and none of them may panic on any input;
  - the formatter fuzz target also checks the safety properties;
  - reduced fuzz inputs are kept as regression tests.
- **Stress tests:** deeply nested input (including javac's own deep-nesting tests) doesn't overflow
  the stack, and no file exceeds a per-file time limit.
- **Externally licensed test inputs:** test inputs under licenses incompatible with MIT OR Apache-2.0
  (for example, OpenJDK's GPL test suites) may be fetched at test time but are never committed.

## Licensing and provenance

- Ferrion is dual-licensed **MIT OR Apache-2.0**.
- **Dependency license checks run in CI:**
  - `cargo-deny` for Rust;
  - equivalent checks for the Maven plugin's and IntelliJ extension's dependencies.
- **Upstream code:** code from upstream tools (Google Java Format, Checkstyle, PMD, Error Prone,
  SpotBugs, OpenJDK, Lombok, and others) may not be copied or ported unless that use is compatible
  with MIT OR Apache-2.0. Their documentation and observable behavior may be used as references.
- **Provenance checks:**
  - ScanCode runs in CI to detect copied license notices and copyright statements;
  - a clean-room policy is documented in `CONTRIBUTING.md`;
  - a pull-request checklist item confirms nothing was copied or ported.

  Copied code without a notice, and a port from Java to Rust, cannot be reliably detected
  automatically, so the policy and checklist matter as much as the tooling.

## Non-goals

- Replacing javac: no compile errors, bytecode generation, or annotation processing.
- Matching Google Java Format output byte for byte.
- Reading Checkstyle, PMD, or Error Prone configuration files or suppressions.
- A public lexer or parser API.
- Preview language features, and Java versions before 8.
- Source encodings other than UTF-8 and ASCII.
- Any network access.
- Rule groups, presets, or rules enabled by default.
- `extends` in configuration.
- Respecting `.gitignore` by default.
- Ferrion-specific formatter markers.
- Splitting long string literals.
- IDE navigation, completion, or refactoring through the language server.
- Experimental Lombok features (for now).
- Fixed targets for rule counts, performance, or support of new Java releases.

## Open questions

- How user-defined rules are written, and how plugin locations that are specific to a user (such as
  jars in a local Maven repository) are passed to the CLI. One option is a `--plugin <name>=<path>`
  argument.
- Where rule documentation is published.
- The nullness annotation model (for example, JSpecify, with other annotation families mapped onto
  it), to be decided before any nullness rules are written.
- How the IntelliJ extension finds the binary for remote, WSL, and container projects, where the
  local Maven repository lives on a different host.
- When the Gradle plugin and VS Code extension are scheduled.
- Which formatter options, if any, are added in later phases.
