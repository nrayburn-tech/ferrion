# CLI and configuration

Stage A3 of the [roadmap](../ROADMAP.md) starts with this document, written before any A3 code. It
covers the `ferrion` command line for formatting (`format`, `version`, and `config`), how
`ferrion.jsonc` is read, checked, and applied to files, which files a run covers, the output
formats and their JSON schema, exit codes, and how a crash in one file is contained. What the CLI
and the config do is fixed in [SCOPE.md](../SCOPE.md#configuration) and
[its CLI section](../SCOPE.md#command-line-interface); this document is about how, and proposes
what SCOPE.md leaves open.

It builds on the [formatter](formatter.md), and on the edits and ranges of
[formatter completeness](formatter-completeness.md) (A2).

## Decisions made at the start

These were settled before this document was written:

- **The gate:** this document first, then a review, before any A3 code. The work runs on the local
  `a3/integration` branch, cut from `main` at 3273dd6.
- **Load:** after the review, one agent builds the config crate and file selection, which don't
  need A2, while A2 finishes. The rest of the CLI starts once A2 is integrated, so `--lines` is
  built on A2's final `format_range`.
- **The linter's keys** (`rules`, `plugins`, `linterOptions`, the baseline) are parsed and their
  structure is checked now. Rule and plugin *names* are checked from C1, when there are rules to
  check them against.
- **The JSON Schema** is generated and committed in A3 and tested against the loader. Its public
  `$schema` URL is chosen at A4, with the Maven groupId and the crate name.
- **Error messages:** this document shows their form with a few examples. Before A3 ends, a
  snapshot catalog of every config and usage error goes to review.

## The crates

- **`ferrion_config`**: the JSONC parser, the config model, validation, globs, and resolution
  (which entries apply to a path, and what they give). No file system access except reading the
  config file, so the language server (A5) can load a config from an editor buffer and get its
  errors as spans.
- **`ferrion_cli`**: argument parsing, file selection, the commands, the output formats, and crash
  containment. It builds the `ferrion` binary.

Config errors carry byte spans in the config file, so the CLI can print an excerpt and the
language server can show a diagnostic, from the same error.

### Dependencies

| Need | Choice | Why |
| --- | --- | --- |
| Arguments | `clap` (derive, no color feature) | Usage errors exit 2 by default, flag suggestions, help text. A hand-written parser would have to reproduce all of it. |
| Globs | `globset` | Matches a set of globs in one pass, with the options below. MIT OR Unlicense. |
| JSONC | hand-written | Every value needs a span, every error must be collected (not the first), and comments and trailing commas must be allowed. serde stops at the first error and keeps no spans. It is a few hundred lines. |
| Excerpts in `pretty` | hand-written | Columns are counted in UTF-16 code units and tabs must be expanded the same way everywhere; the linter will reuse it. |
| "Did you mean" | hand-written | An edit distance over a few candidate names. |
| Schema tests | `jsonschema` (dev-dependency only) | Checks the generated schemas against the test inputs and outputs. |

All pass `cargo deny check`.

## `ferrion format`

```
ferrion format [PATHS...] --java-version <N> [--check] [--config <path>]
               [--output-format pretty|json|github] [--stdin] [--stdin-filepath <path>]
               [--lines <A:B>]... [--verify]
```

| Flag | Meaning |
| --- | --- |
| `PATHS` | Files and folders to format. None means the working directory. |
| `--java-version <N>` | Required (SCOPE.md). An integer in `8..=27` (`ferrion_java_version::SUPPORTED_RELEASES`); `1.8` is accepted for 8, as javac's `-source` accepts it. Anything else is a usage error. |
| `--check` | Write nothing; report each file that isn't formatted. |
| `--config <path>` | The config file. Without it, `./ferrion.jsonc` if it exists, else no config. A `--config` file that doesn't exist is a usage error. |
| `--output-format` | `pretty` (default), `json`, or `github`. |
| `--stdin` | Read one file from stdin; write the result to stdout. |
| `--stdin-filepath <path>` | The path that the stdin file selects config entries with and is reported as. Implies `--stdin`. |
| `--lines <A:B>` | Format only lines A to B, 1-based and inclusive. Repeatable. Only with one file or stdin. |
| `--verify` | Check the safety properties on every file before writing it. |

`--java-version` stays required, as SCOPE.md has it, rather than defaulting to the latest release:
the release changes the output, not only the parse. A `///` comment is a Markdown doc comment from
Java 23 and is reflowed there, but below 23 it is a line comment, whose text is never changed.
Defaulting to the latest would reflow line comments in older projects. The Maven plugin and the
language server pass the version anyway. (Question 1.)

Usage errors exit 2 before any file is read: unknown flags, `--stdin` with `PATHS`, `--lines` with
anything but one file or stdin, a malformed or reversed range (`5:3`), `--java-version` out of
range, a `PATHS` entry that doesn't exist.

### Which files a run covers

- **No paths:** every `*.java` file under the working directory.
- **A folder:** every `*.java` file under it.
- **A file:** that file, whatever its extension, unless it is skipped below.
- **Walking** skips folders whose name starts with `.` (a folder passed explicitly is still walked,
  but hidden folders inside it are skipped), and never follows a symbolic link, to a file or a
  folder. Hidden *files* are included: SCOPE.md skips only hidden folders.
- **Skipped with a note,** for explicitly passed files, so that a hook can pass every changed file:
  - a file a global ignore matches (SCOPE.md);
  - a file whose effective `format.enabled` is `false`;
  - a file that doesn't end in `.java`;
  - a file outside the working directory, which no glob can match (all globs are relative to it).
  Walked files that are ignored or disabled are skipped silently. (Question 3.)
- **An explicitly passed symbolic link** is used: the user named it. It is matched by the path as
  given, and writing goes through it to its target.
- **Paths are normalized** lexically: `./`, `..`, and repeated separators are removed, and `\`
  becomes `/` on Windows, before matching and in reports. A file passed twice, or both directly and
  inside a passed folder, is formatted once.
- **Order:** files are reported sorted by their normalized path, byte by byte, whatever order they
  finish in.

Paths are matched as the walker spells them (or as the user typed them), case-sensitively, on every
platform, so a config means the same on macOS, Windows, and Linux. A walked folder is pruned when a
global ignore ending in `/**` matches it, which gives the same answer as matching each file under
it, without reading the folder.

### Reading and writing

- Each file is read whole and decoded with `SourceText::decode`. A file that isn't UTF-8 or ASCII,
  or is too large, is skipped and reported (exit 1).
- The file is formatted with `ferrion_formatter::format` at `--java-version`. A file with syntax
  or version errors is skipped and its errors are reported (exit 1).
- A file that `format` leaves `Unchanged` isn't written, so its modification time stays.
- A changed file is **written in place**: opened, truncated, and written. Writing a temporary file
  and renaming it would survive a crash mid-write, but replaces an explicitly passed symbolic link
  with a file, breaks hard links, and loses the file's owner and some permissions. (Question 5.)
- An I/O error on a file (it vanished, it can't be read, it can't be written) is reported for that
  file, and the run goes on (exit 1). (Question 6.)

### `--check`

Nothing is written. Each file whose output differs is reported as not formatted, with the line
and column of its first difference: the first byte where input and output differ, mapped to the
input's line. `pretty` and `github` show that position; neither prints a diff (a `--diff` flag can
come later without breaking anything). Any unformatted file exits 1.

### `--stdin`

- The input is read from stdin and decoded as a file is. The output, formatted or not, goes to
  stdout, and the report goes to **stderr**, so stdout is only the file.
- **The input comes back unchanged** when the path is ignored or disabled (with a note), and when
  it has syntax errors (with the errors, exit 1). A caller that pipes a buffer through Ferrion
  never gets an empty buffer back. (Question 4.)
- Without `--stdin-filepath`, no path selects entries: only entries without `files` and `ignores`
  apply, and the file is reported as `<stdin>`.
- With `--check`, nothing is written to stdout, and the exit code says whether the input was
  formatted.

### `--lines`

- Each range is turned into bytes with the file's `LineIndex`: from the start of line A to the end
  of line B, without B's line terminator. Lines end at CR, LF, or CRLF, as the formatter counts
  them. A range that ends past the last line ends at the last line; one that starts past it is a
  usage error.
- Ranges that overlap or touch are merged, then each is formatted with `format_range` (A2) on one
  parse of the file, and the edits of all ranges are applied together. A2 expands a range to whole
  units, so two ranges can still expand into the same unit and return the same edits: an edit
  returned twice is applied once. Two different edits that overlap would be a Ferrion bug, and are
  reported as one (exit 3).
- A range that a syntax error affects (`RangeError::Affected`) is reported with the error, and that
  range is left alone; the file's other ranges are still formatted. Errors outside every range
  don't stop anything, as SCOPE.md promises.
- With `--check`, the file is unformatted if any range has edits.

### `--verify`

Every changed file is checked with `verify` before it's written, and a file formatted with
`--lines` with `verify_range` (A2). A file that fails is not written, the violation is reported as a
Ferrion bug, with the property and position, and the run goes on and exits 3. With `--check`,
`--verify` checks the output that would have been written.

## `ferrion version`

`ferrion version` prints `ferrion 0.3.0`. `ferrion --version` prints the same. `--json` prints:

```json
{
  "schemaVersion": 1,
  "version": "0.3.0",
  "interfaceVersion": "1.0",
  "commit": "3273dd6",
  "target": "aarch64-apple-darwin",
  "javaVersions": { "oldest": 8, "latest": 27 }
}
```

- **`interfaceVersion`** is what the Maven plugin checks (A4). Its major part changes whenever the
  CLI, the JSON output, or the config's shape changes incompatibly; its minor part when something
  is added to them. A plugin needs the same major part and at least the minor part it was built for.
  A3 starts it at `1.0`. It is separate from `version` because formatting output can change in any
  release (SCOPE.md) without touching the interface.
- **`commit`** comes from `FERRION_COMMIT` at build time, which the release pipeline sets; `null`
  otherwise. A build script that runs git would make builds depend on the checkout.
- **`target`** is the Rust target triple the binary was built for.

## `ferrion config <file>`

Prints the effective config for a path, and where each setting came from. The path doesn't have to
exist; it is normalized as `format`'s paths are. It takes `--config` and `--output-format`.

```
src/test/java/app/FooTest.java
  entries: configs[1], configs[2] "tests"
  format.enabled: false                      from configs[2] "tests"
  rules:
    no-equals-without-hashcode: "error"      from configs[1]
    no-unused-variable: "off"                from configs[2] "tests"
```

```
src/legacy/Old.java
  ignored by configs[0], "**/legacy/**"
```

A setting no entry gives is shown as `(default)`. Rule settings are shown as written. With no
config file, it says so and shows the defaults. `--output-format json` gives the same as an
object, with each setting's `value` and `entry` index.

## Configuration

### The file

- **JSONC:** JSON with `//` and `/* */` comments and trailing commas in arrays and objects, as VS
  Code's JSONC has it. Nothing else of JSON5 (no single quotes, unquoted keys, or `NaN`).
- UTF-8, with or without a byte-order mark. A **duplicate key** in an object is an error, since one
  of them would be silently ignored.
- Values nest at most 64 deep (rule options included), so a hostile config can't overflow the
  stack; deeper is an error.
- The default location is `./ferrion.jsonc`. There is no search of parent folders, and no other
  file name (SCOPE.md: configs don't cascade).

### The root object

| Key | Value | Checked now |
| --- | --- | --- |
| `$schema` | a string | any string; the URL is chosen at A4 |
| `configs` | an array of entries | fully |
| `plugins` | an array of plugin names (strings) | structure only; names from C1 |
| `linterOptions` | an object: `reportUnusedDisableDirectives`, a level | fully |
| `baseline` | a path to the baseline file, relative to the working directory | that it's a string |

Every key is optional; an empty object is a valid config that changes nothing. SCOPE.md names
`plugins`, `linterOptions`, and "the baseline file location", but doesn't define their values; the
shapes above are proposed, and C1–C4 may still change them before 1.0. (Question 2.)

### Entries

| Key | Value |
| --- | --- |
| `name` | an optional name, shown in `ferrion config` and in errors (proposed: SCOPE.md doesn't have it, ESLint does) |
| `files` | a non-empty array of globs; without it, the entry applies to every `*.java` file |
| `ignores` | an array of globs; files they match are excluded from this entry |
| `rules` | an object of rule name to a level (`"off"`, `"info"`, `"warn"`, `"error"`) or `{ "level": ..., "options": { ... } }`; `options` must be an object, and is checked from C1 |
| `format` | an object: `enabled`, a boolean |

- An entry with only `ignores` (and optionally `name`) is a **global ignore**: files it matches are
  outside the run altogether.
- An entry **matches** a file when it has no `files` or one of its `files` globs matches, and none
  of its `ignores` globs does.
- **Precedence** (SCOPE.md): the last matching entry that mentions a rule gives that rule's whole
  setting. The same goes for `format`: the last matching entry with a `format` object gives all of
  it. Nothing is merged.

### Globs

Globs are matched against the path relative to the working directory, with `/` separators:

- `*` matches any run of characters except `/`; `?` one character except `/`; `**` as a whole path
  segment matches any number of segments; `[abc]`, `[a-z]`, `[!a]`, and `{a,b}` are as usual.
- **A glob is anchored to the working directory:** `*.java` matches only top-level files, and
  `**/*.java` matches them everywhere, as in ESLint's flat config. (Question 9.)
- A glob ending in `/` means that folder and everything under it: `build/` is `build/**`.
- A leading `./` is removed.
- Matching is case-sensitive everywhere.
- `\` escapes the next character, so `\*` is a literal star. A `\` before a character that needs no
  escaping is an error that suggests `/`, so a Windows-style path isn't silently misread.
- **Errors:** an absolute path, a `..` segment (no glob can match outside the working directory),
  a leading `!` (negation isn't supported), an empty glob, and a malformed one (`[a`, `{a`).

### Validation

Loading reads the whole file, then checks every entry, whether or not it matches any file
(SCOPE.md), and collects **every** error, in file order, up to 50 (then "and N more"). A JSONC
syntax error stops at that error, since the structure after it is unknown. Any config error stops
the run before any file is read, with exit 2.

An error has a message, the span of the value it's about, and optionally a label and a help line.
Unknown keys, levels, and `format` keys get a "did you mean" when a known name is within an edit
distance of 2 (or 1 for names of four letters or fewer), case differences counted as one. In
`pretty`:

```
error: unknown key `ignore` in configs[1]
 --> ferrion.jsonc:4:7
  |
4 |       "ignore": ["**/legacy/**"],
  |       ^^^^^^^^ did you mean `ignores`?
```

```
error: `format.enabled` must be true or false
 --> ferrion.jsonc:9:30
  |
9 |       "format": { "enabled": "no" }
  |                              ^^^^ found a string
```

```
error: a glob can't contain `..`
 --> ferrion.jsonc:3:17
  |
3 |     { "files": ["../shared/**"] },
  |                 ^^^^^^^^^^^^^^ globs are relative to the working directory, and can't
  |                                match outside it
```

```
error: expected `,` or `}` after this value
 --> ferrion.jsonc:6:7
  |
5 |       "rules": { "no-unused-variable": "warn" }
  |                                               - add `,` here
6 |       "format": { "enabled": false }
  |       ^ the next key starts here
```

The wording of every message goes into a snapshot catalog for review before A3 ends.

### The JSON Schema

`schemas/ferrion.schema.json` (draft 2020-12) is generated by `cargo codegen` from the same key
tables the validator uses, so the two can't disagree, and the codegen CI job checks that it is
current. A test checks every valid config test input against it, and every structurally invalid one
fails it. The schema can't express everything (a glob's syntax, duplicate keys), so the validator
stays the authority. `rules` accepts any name until C1 generates the list of rules into it.

## Output

### Where it goes

- The report goes to **stdout**, in the chosen format, except with `--stdin`, where stdout is the
  file and the report goes to stderr.
- **Usage errors** are always plain text on stderr, from clap: they can happen before the output
  format is known.
- **Config errors** are in the chosen format, so the Maven plugin gets them as JSON.
- Formats are one `Reporter` trait with an implementation each, so a new one is a new type, as
  SCOPE.md asks.

### `pretty`

One line per file that needs attention, then a summary:

```
src/main/java/app/Main.java:12:5: not formatted
error: syntax error: expected `;`
 --> src/main/java/app/Broken.java:3:14
  |
3 |     int x = 1
  |              ^ expected `;`
note: src/legacy/Old.java is ignored by configs[0], "**/legacy/**", so it was skipped
2 files not formatted, 1 skipped because of syntax errors, 1,204 already formatted
```

Without `--check`, the summary says "3 files formatted". Color is used when the stream is a
terminal and `NO_COLOR` isn't set; there's no color flag in A3.

### `json`

One document, written when the run ends:

```json
{
  "schemaVersion": 1,
  "command": "format",
  "summary": { "formatted": 0, "unformatted": 2, "unchanged": 1204, "skipped": 1, "ignored": 1,
               "failed": 0 },
  "files": [
    { "path": "src/main/java/app/Main.java", "status": "unformatted",
      "firstChange": { "line": 12, "column": 5, "offset": 301 } },
    { "path": "src/main/java/app/Broken.java", "status": "skipped",
      "diagnostics": [ { "severity": "error", "code": "syntax-error", "message": "expected `;`",
                         "range": { "start": { "line": 3, "column": 14, "offset": 52 },
                                    "end": { "line": 3, "column": 14, "offset": 52 } } } ] }
  ],
  "notes": [ { "path": "src/legacy/Old.java",
               "message": "ignored by configs[0], \"**/legacy/**\"" } ],
  "configErrors": []
}
```

- `files` lists only files with something to report; `summary` counts them all.
- `status` is one of `formatted`, `unformatted`, `skipped` (syntax errors, encoding, I/O), and
  `failed` (a crash or a failed `--verify`).
- Lines and columns are 1-based, and columns count UTF-16 code units, as the 100-column limit and
  LSP do; `offset` is the byte offset in the file, after any byte-order mark.
- **Versioning:** `schemaVersion` is an integer, raised on any incompatible change. Adding a field
  or a `status` value isn't one; consumers ignore fields they don't know. The schema is
  `schemas/output-v1.schema.json`, hand-written, and every JSON output in the end-to-end tests is
  checked against it. Diagnostics have the shape lint findings will use in C1.

### `github`

[Workflow commands](https://docs.github.com/actions/reference/workflow-commands-for-github-actions),
one per line, then the plain summary:

```
::error file=src/main/java/app/Main.java,line=12,col=5,title=Not formatted::Run ferrion format
::error file=src/main/java/app/Broken.java,line=3,col=14,title=Syntax error::expected `;`
::notice file=src/legacy/Old.java,title=Skipped::ignored by configs[0], "**/legacy/**"
```

Values are escaped as GitHub requires (`%`, CR, and LF everywhere; `:` and `,` in properties).

## Exit codes

| Code | `format` |
| --- | --- |
| 0 | Every file formatted or already formatted; notes don't count. |
| 1 | `--check` found an unformatted file, or a file was skipped (syntax or version errors, encoding, I/O). |
| 2 | A usage error or a config error. No source file was read or written. |
| 3 | A crash or a failed `--verify` in any file. |

The run's code is the highest that applies. `version` and `config` exit 0 or 2.

## Crashes

A panic in one file mustn't stop the others (SCOPE.md):

- Each file runs inside `catch_unwind`. A panic hook, installed once, records the message and
  location for the thread instead of printing them.
- The file is reported as `failed`: "Ferrion crashed while formatting this file. This is a bug in
  Ferrion; please report it at https://github.com/nrayburn-tech/ferrion/issues with the file and
  this message", then the message and location. Its output isn't written. The run goes on and
  exits 3.
- The release profile keeps `panic = "unwind"`, the default; a test guards it.
- What `catch_unwind` can't catch, an abort or running out of memory, ends the run. Deep nesting
  can't overflow the stack (`ferrion_stack`).

## Running files in parallel

Files are formatted on `std::thread::available_parallelism()` worker threads, through
`std::thread::scope`, taking the next file from a shared index; no thread-pool crate. Each worker
holds one file at a time. Results are put back in path order before they're reported: `pretty` and
`github` print each file once every file before it is done, and `json` writes at the end. The
output is the same whatever the thread count, and a test checks it with one thread and with many.

## Testing

- **`ferrion_config` unit tests:** JSONC (comments, trailing commas, every syntax error), every
  validation error, globs (each rule above, Windows-style input), and resolution (precedence, global
  ignores, `files` and `ignores` together). Error output is snapshot-tested in `pretty` form, with
  `FERRION_BLESS=1` as elsewhere.
- **End-to-end tests** in `crates/ferrion_cli/tests/` run the built binary
  (`CARGO_BIN_EXE_ferrion`) in a temporary folder each test creates: no paths, folders, explicit
  files, hidden folders, symbolic links (on Unix, and on Windows where the test can create them),
  ignored and disabled files, stdin with and without a path, `--lines` (one, several, overlapping,
  past the end, affected by an error), `--check`, `--verify`, all three output formats, and every
  exit code. Their outputs are snapshots, and JSON outputs are checked against the output schema.
- **Exit code 3** needs a crash and a `--verify` failure on demand: a debug build reads
  `FERRION_TEST_PANIC` (a path to panic on) and `FERRION_TEST_BREAK_OUTPUT` (a path whose output
  is corrupted before `verify`). Release builds don't read them.
- **The message catalog:** one snapshot file renders every config error and every usage error in
  `pretty`, for the review before A3 ends.
- **The corpus:** the weekly formatter job runs `ferrion format --check --output-format json` over
  each corpus project at its release, and checks that its results match `format-check`'s.

## Work order

At most five agents at once with A2's and B3's, heavy jobs through the shared slot lock.

1. **Config and files (one agent, after this review, alongside A2):** `ferrion_config` whole (JSONC,
   the model, validation, globs, resolution, schema generation through `cargo codegen`, its unit
   tests and snapshots), and file selection in `ferrion_cli` as a module with its own tests, not
   yet wired to a command.
2. **The CLI (after A2 is integrated into `a3/integration`):** clap, the three commands, reading and
   writing, `--check`, `--stdin`, `--verify`, `--lines` on `format_range`, the reporters and the
   output schema, crash containment, parallel running, and the end-to-end tests. One agent, or me
   if the load allows.
3. **The catalog:** generated and sent for review, a stop.
4. **Exit check:** the roadmap's A3 criteria, and the corpus run.

## What A3 leaves out

- `lint`, `lsp`, `rules`, and `init` aren't commands yet; `ferrion lint` is an unknown command
  until C1.
- Rule and plugin name checks (C1), the list of rules in the schema (C1), and the baseline's
  contents (C4).
- The `$schema` URL and publishing the schemas (A4).
- `--diff`, a color flag, and any option for the format itself.

## What was consulted

- SCOPE.md, the roadmap, and the A1 and A2 designs.
- From what their documentation describes, without reading their code: ESLint's flat config (the
  model SCOPE.md adapts), the CLIs of Prettier, Ruff, and Google Java Format (`--lines` and stdin
  handling), and GitHub's workflow command format.

## Questions for review

1. **`--java-version` stays required for `format`,** as SCOPE.md has it, because the release
   decides whether `///` comments are reflowed. `1.8` is accepted for 8. Recommended as written.
2. **The linter keys' shapes:** `plugins` as an array of names, `baseline` as a path string,
   `linterOptions.reportUnusedDisableDirectives` as a level, and an optional `name` on entries.
   Recommended as written, open to change before C1.
3. **Explicitly passed files that are skipped with a note:** besides ignored ones (SCOPE.md), also
   disabled ones, non-`.java` files, and files outside the working directory, rather than a usage
   error or formatting them anyway. Recommended as written: a hook can pass every changed file.
4. **`--stdin` echoes its input** when the file is ignored, disabled, or has syntax errors, rather
   than writing nothing. Recommended as written.
5. **Files are written in place,** not through a temporary file and a rename. Recommended as
   written: it keeps symbolic links, hard links, and ownership.
6. **An I/O error on one file exits 1** and the run goes on, rather than exit 2 (meant for usage
   and config errors) or stopping. Recommended as written.
7. **Dependencies:** `clap` and `globset`; hand-written JSONC, excerpts, and suggestions;
   `jsonschema` for tests only. Recommended as written.
8. **Work order:** the config agent now, the CLI after A2, a stop for the message catalog.
   Recommended as written.
9. **Globs are anchored** to the working directory, as ESLint's are: `*.java` matches only
   top-level files. The alternative is gitignore's rule, where a glob without `/` matches at any
   depth. Recommended: ESLint's, since SCOPE.md models the config on it.

## Decisions made in review

The user accepted all nine recommendations (2026-11-02):

1. **`--java-version` stays required** for `format`, with `1.8` accepted for 8.
2. **The linter keys** have the proposed shapes: `plugins` an array of names, `baseline` a path,
   `linterOptions.reportUnusedDisableDirectives` a level, and an optional `name` on entries.
3. **Explicitly passed files** that are disabled, don't end in `.java`, or are outside the working
   directory are skipped with a note, as ignored ones are.
4. **`--stdin` echoes its input** when the file is ignored, disabled, or has syntax errors.
5. **Files are written in place.**
6. **An I/O error on one file exits 1,** and the run goes on.
7. **Dependencies:** `clap` and `globset`; hand-written JSONC, excerpts, and suggestions;
   `jsonschema` for tests only.
8. **The work order** stands: the config agent now, the CLI after A2, a stop for the message
   catalog.
9. **Globs are anchored** to the working directory, as ESLint's are.
