# Formatter design

Stage A1 of the [roadmap](../ROADMAP.md) starts with this document, written before any formatter
code. It covers the document model and its printer, how the document is built from the syntax tree,
where comments go, the parts of a file the formatter copies rather than lays out, modifier
reordering, how the four safety properties are checked, the Google Java Format comparison, and the
order of the work. What the formatter does is fixed in [SCOPE.md](../SCOPE.md#formatter); this
document is about how.

The formatter builds on the [syntax tree](parser-syntax-tree.md), the
[comment classification](parser-comments.md), the [deep-nesting rules](parser-deep-nesting.md), and
the [error units](parser-error-recovery.md) of Phase 2.

## Decision

- **A flat Wadler–Leijen document.** The formatter turns the tree into a document of text, line
  breaks, groups, and indentation, then a printer chooses where to break. The document is a flat
  array of elements with start and end markers, not a tree, so building and dropping it never
  recurses, and the printer is one loop (see [deep nesting](parser-deep-nesting.md)).
- **Output tokens are input tokens.** Every piece of text in the document is a token of the input,
  copied from the source. The formatter writes only whitespace of its own. The tokens of the output,
  comments included, are the tokens of the input in the same order, except where a rewrite
  (modifier reordering in A1, imports in A2) reorders or removes them. That makes safety
  properties 2 and 3 a token comparison, and gives the language server minimal edits for free.
- **Comments never move across code.** A comment is printed where the
  [classification](parser-comments.md#classification) places it: before its element, after it, or
  inside its delimiters. Because tokens keep their order, no comment changes places with a token.
- **Linear printing.** Every group's flat width is computed while the document is built, so the
  printer's fitting check is a lookup, not a scan. Formatting is linear in the size of the input
  plus the output (see [Deep nesting](#deep-nesting)).
- **Google Java Format 2.0.0 is the reference.** Ferrion matches its observed output, except where
  that output is unstable, changes meaning, or goes against SCOPE.md. Every departure is listed in
  `docs/style.md` and checked against a pinned GJF run. GJF's source is never consulted (see
  [CONTRIBUTING.md](../../CONTRIBUTING.md#clean-room-policy)); what it does is learned by running it,
  and the observations are logged here and in `docs/style.md`.

## What the formatter serves

- **`ferrion format`** (A3): whole files, `--check`, and `--verify`.
- **Range formatting** (A2) and the **language server** (A5): edits for part of a file, possibly
  one with syntax errors outside the range, returned as minimal edits.
- **The fuzz target and CI:** the safety properties on any input that parses.
- **`no-unformatted-code`** (C6): whether a file is formatted, and a fix that formats it.

All of them see the same output for the same source, release, and config ("same input, same
output", SCOPE.md).

## The crate

`ferrion_formatter` depends on `ferrion_diagnostics` (`SourceText`, line endings, the byte-order
mark), `ferrion_syntax`, `ferrion_stack`, `ferrion_parser`, and later `ferrion_doc_comments` (A2).
It is internal (`publish = false`).

The core works on a tree that's already parsed, so the language server and range formatting
format the tree they hold without parsing again. `format` and `verify` wrap it and parse, and
`verify` has to parse the output in any case. That's why the crate depends on `ferrion_parser`.
The [parser design](parser.md) says consumers of the tree depend on `ferrion_syntax` alone; the
formatter is the exception, for these two functions.

```rust
pub struct FormatOptions {
    pub release: u32,      // the Java release to parse for
}

pub enum Formatted {
    Unchanged,             // the file is already formatted
    Changed(Vec<u8>),      // the new file, byte-order mark and line endings included
    Skipped(Vec<ParseDiagnostic>), // syntax or version errors; nothing was formatted
}

/// Formats a tree without errors, parsed from `source` at the tree's release.
pub fn format_tree(tree: &SyntaxTree, source: &SourceText) -> Formatted;

/// Parses `source` at `options.release`, then formats it with `format_tree` unless the parse
/// has errors.
pub fn format(source: &SourceText, options: &FormatOptions) -> Formatted;

/// Checks the four safety properties for an input and the formatter's output for it.
pub fn verify(input: &SourceText, output: &[u8], options: &FormatOptions) -> Result<(), Violation>;
```

`format_tree` takes the `SourceText` as well as the tree for the file's line ending and
byte-order mark. The tree doesn't hold them: its text is `SourceText::text`, after any byte-order
mark. The caller checks the parse for errors; `format_tree` can't tell that a version error was
reported. There are no style options. A1 has no range API; A2 adds one (see
[Ranges](#ranges-and-edits)).

## Pipeline

1. **Parse** once, at the given release (`format` only; `format_tree` starts at step 2). Any
   diagnostic with `Severity::Error` (lexical, syntax, or version) skips the file, and the
   diagnostics are returned for the caller to report. Warnings don't skip.
2. **Classify comments** with `CommentMap::new` (from `ferrion_syntax`).
3. **Find disable regions:** one scan of the comment tokens (see
   [Disable regions](#formatter-disable-regions)).
4. **Build the document:** a recursive walk from the root. A function per node kind emits the
   node's elements from its children and tokens.
5. **Print:** one loop over the document, writing the output with the file's line ending, then the
   byte-order mark if the input had one.
6. **Compare:** if the output equals the input, the result is `Unchanged`.

`verify` is separate, so `format` costs one parse. `--verify`, CI, and the fuzz target call both.

## The document

### Elements

The document is a `Vec<Element>`, appended to in order. Containers are pairs of start and end
elements, and each start records the index of its end.

| Element | Prints as |
| --- | --- |
| `Token(TokenId)` | the token's text from the source. Its width is the text's length in UTF-16 code units. |
| `Space` | one space |
| `Line` | a space when its group is flat, a line break when it's broken |
| `SoftLine` | nothing when flat, a line break when broken |
| `HardLine` | always a line break; its enclosing groups can't be flat (see [Hugging](#hugging)) |
| `BlankLine` | a line break and an empty line; also forces its groups to break |
| `Group { end, flat_width }` … `End` | its contents, flat if they fit on the rest of the line, otherwise broken |
| `Indent(kind)` … `End` | its contents, with line breaks inside indented by +2 (`Block`) or +4 (`Continuation`) |
| `IndentIfBroken { group }` … `End` | indented by +4 only if the given group is broken |
| `Fill` … `End` | items and separators, each separator breaking only if the next item doesn't fit |
| `Hug` … `End` | its contents; their forced breaks don't force the groups around the hug to break |
| `Verbatim { tokens }` | a run of source text copied as written, line breaks included (see [Copied text](#copied-text)) |

Only `Token` and `Verbatim` produce non-whitespace text, and both take it from the source by token
index, so the document can't invent or alter code.

Consecutive line elements merge when they're printed: two `HardLine`s print one line break, and a
`HardLine` next to a `BlankLine` prints the blank line. Rules can then each ask for "a line break
here" without counting what the rules around them asked for, and comments can add their own breaks
without doubling them.

Indentation is written lazily, at the first text after a line break, so no line ends with
whitespace. Indentation is always spaces.

### Why Wadler–Leijen

GJF's observed layout is what a Wadler–Leijen printer produces from nested groups, with one
exception (hugging):

- **Outermost first.** `int total = a + b * c - d;` that doesn't fit breaks after `=` first, then
  between the operands of the lowest-precedence operators, all at once, and keeps `b * c` together:

  ```java
  int total =
      firstOperandValue
          + secondOperandValue * thirdOperandValue
          - fourthOperandValue
          + fifthOperand;
  ```

  That is a group for the assignment, holding a group per precedence level, each breaking all its
  lines together.
- **Forced breaks propagate.** `Runnable r = () -> { run(); };` breaks after the `=` because the
  lambda's block can't be flat. That is a hard line forcing every group around it to break.
- **The rest of the line counts.** A trailing `// comment` makes the code before it break, even to
  `int x =` on its own line with `1; // ...` below. Fitting a group checks the group plus everything
  after it up to the next possible line break, comments included.
- **Arguments:** all on the call's line; otherwise all on the next line at +4; otherwise one per
  line. That is two nested groups.

### Hugging

GJF 2.0.0 keeps a lambda that is a call's only argument on the call's line, and falls back to the
normal layout when the call's line up to the `{` doesn't fit:

```java
executor.submit(() -> {
  doSomething();
});
someVeryLongReceiverName.someVeryLongMethodNameThatTakesALambda(
    (firstParameter, secondParameter) -> {
      doSomething();
    });
```

Plain Wadler–Leijen can't express that: the block's hard lines force the argument group to break.
Two elements cover it:

- `Hug` stops the block's forced breaks from propagating out, and a group whose forced breaks are
  all inside hugs is measured only up to its first forced break. So the argument group fits if
  `executor.submit(() -> {` fits, and is printed flat.
- `IndentIfBroken` indents the arguments only when the argument group breaks, so the hugged block
  is indented from the statement (+2), and the unhugged one from the arguments (+4, then +2).

Inside a hug, groups are decided by fitting as usual, even when the group around the hug is flat,
so long statements in a hugged lambda still break.

The same two elements are the plan for any other construct the work finds to hug. If one needs a
choice that fitting can't make, the fallback is a `BestFit` element with alternatives tried in
order, as some other pretty printers have, added then rather than now.

### Fill

GJF fills array initializers of short elements, as many per line as fit:

```java
int[] a = {
  1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
  27
};
```

`Fill` holds items and separators, and breaks a separator only when the item after it doesn't fit
on the line. Which initializers fill and which put one element per line is a construct rule, to be
observed.

## The printer

The printer is a loop over the elements with a stack of open containers. Each entry holds the
container's mode (flat or broken) and indentation. Starting a group decides its mode:

- **Broken** if it contains a forced break outside any hug.
- Otherwise **flat** if `column + flat_width + rest ≤ 100`, where `rest` is the width from the
  group's end to the next line element of any kind. Otherwise **broken**.
- Inside a flat group, nested groups are flat without a check, except inside a `Hug`.

`flat_width` is computed while building: the builder keeps a stack of open groups, adds each
token's width and each `Line`'s space to every open group's running total in one step (by adding
to the innermost and folding into the parent at `End`), and marks open groups at a forced break.
`rest` is one backward pass over the elements before printing. So deciding a group is constant
time, and printing is linear in the number of elements.

Width is counted in UTF-16 code units, as SCOPE.md and GJF count it: a character outside the Basic
Multilingual Plane counts 2 (observed: a line of emoji breaks where the same number of CJK
characters doesn't). Each token's width is computed once, with a fast path for ASCII. A Unicode
escape counts as the characters written in the source.

Output is written to one `Vec<u8>`, reserved at the input's size plus a margin. Line breaks are
written as the file's line ending (`SourceText::line_ending`, LF if it has none), and the output
starts with a byte-order mark if `SourceText::has_bom`.

A line can still go over 100 columns: a long token, a long string, or text the formatter copies.
The printer never fails; it does the best the groups allow.

## Building the document

- **One function per node kind,** in modules by area (`declarations`, `statements`,
  `expressions`, `types`), reading children through the typed views. Each function emits its
  node's tokens, its children (by calling the dispatch function), and whitespace elements around
  them.
- **Every token is emitted through one helper** that also emits the token's leading and trailing
  comments, checks for disable regions, and asserts in debug builds that tokens are emitted in
  source order, each once. Modifier reordering is the one place that emits out of order, through
  its own helper.
- **Recursion** is handled by `ensure_stack` at each node entry, as for every other consumer of
  the tree (see [Deep nesting](#deep-nesting)).
- **Syntactic heuristics are allowed** where the observed style needs them. For example, GJF keeps
  `ImmutableList.<String>builder()` together at the start of a chain but breaks after `builder` in
  `builder.append(...)...`, apparently treating names starting with an uppercase letter as types.
  The formatter has no type information, so any such rule is syntactic, and each is written down in
  `docs/style.md`.

## Deep nesting

The parser accepts up to `NESTING_LIMIT` (10,000) nested constructs. The formatter has to format
any file the parser accepts without overflowing its stack, and does that with `ensure_stack` and a
printer that is a loop.

Indentation grows with depth, so the output can be much larger than the input: 10,000 nested
blocks print about 20,000 lines with an average of 10,000 spaces of indentation, about 200 MB. The
formatter indents as GJF does anyway, with no cap. A cap would be a departure, and code nested this
deep isn't written by hand. Time and memory are linear in the size of the input plus the output.

Testing:

- The parser's generated patterns move from `crates/ferrion_parser/tests/nesting_stress.rs` into
  a module the parser exports for tests, `#[doc(hidden)]` like `invariants`, so the formatter's
  tests use the same inputs.
- At full depth, each pattern is formatted on a thread with a small stack, for stack safety only.
- Linear time is checked at lower depths, where the output stays small.

## Comments

Comments are placed by the [classification](parser-comments.md#classification), from the input's
line breaks:

- **Leading,** on its own line in the input: the comment, a line break, then the element. A blank
  line between the comment and the element is kept.
- **Leading,** on the element's line (`/* block */ int b = 2;`): the comment, a space, then the
  element. In a list, a block comment that leads an item and sits on the line of the separator
  before it stays on that line when the list breaks, as GJF does: `a, /* c */` then `b`. The
  classification is unchanged; the line break goes after the comment instead of before it.
- **Trailing:** a space, then the comment. A trailing `//` comment ends its line: it forces its
  groups to break, and the next text starts on a new line. If the next element is already a line
  break, the two merge; if it's text (`if // c` then `(x)`), a line break is inserted. A line break
  is legal between any two Java tokens, so this never changes the program.
- **Dangling,** inside an element's delimiters: printed inside them, with line breaks if any of
  them is a `//` comment or was on its own line (`if (x) {` then `// only a comment` then `}`).
  GJF puts a space after a dangling block comment before a closing `)` (`foo(/* none */ );`), and
  Ferrion matches it.
- **Blank lines** before a comment are kept, at most one, as for code.

The width of a trailing comment counts toward its line, as observed in GJF.

`//` comments are never wrapped and their text never changes, apart from trailing whitespace,
which is removed. Block comments that span lines are re-indented (see [Copied
text](#copied-text)). Formatter markers and `ferrion-disable*` comments, found by their text, are
copied exactly.

Safety property 3 holds by construction: the token helper emits each comment once. `verify`
checks it anyway.

## Blank lines

Line breaks between tokens are not stored in the tree; the formatter counts `LineTerminator` tokens
in the trivia between two tokens, as the [comments design](parser-comments.md#blank-lines)
describes. Where the style keeps the input's blank lines (between statements, between members,
between imports in A1), one or more blank lines in the input become one. Where the style requires
one (GJF puts one before every method and type, and after the package and import sections), the
rule emits `BlankLine` regardless. Which places keep, require, or remove blank lines is a construct
rule. Observed so far: GJF keeps a blank line at the start of a block, removes one at its end, and
keeps the input's choice between two fields.

## Copied text

Some text isn't laid out, only placed. It is emitted as `Verbatim`, whose first line counts toward
the line it starts on and whose last line sets the column after it. A `Verbatim` with line breaks
forces its groups to break, unless it is hugged.

- **Block comments that span lines** are re-indented to the comment's new column. If every line
  after the first starts with `*`, the stars are aligned one column after the `/`, as GJF does.
  Otherwise each later line moves by the amount that puts the least indented of them at the
  comment's column, so their indentation relative to each other is kept (observed in GJF). Doc
  comments get the starred rule in A1, and are reflowed in A2.
- **Text blocks** are not re-indented (SCOPE.md). Their content and closing delimiter are copied as
  written, so moving the opening `"""` doesn't change the string's value: the value is
  determined by the content lines and the closing delimiter's indentation (JLS 3.10.6). The opening
  delimiter is placed as GJF 2.0.0 places it, on a line of its own: a text block's line breaks
  force its groups to break like any other copied text, giving `String s =` then `"""` at +4, and
  `foo(` then `"""` at +4 for an argument (observed). Only the re-indenting of the content is a
  departure.
- **Disable regions** (below).

Line terminators inside copied text are converted to the file's line ending, since SCOPE.md gives
the whole output one line ending. That never changes a program: text blocks normalize their line
terminators (JLS 3.10.6), and other copied text is comments and whitespace between tokens.

## Formatter disable regions

As the [comments design](parser-comments.md#formatter-disable-regions) describes, a region runs
from the start of an `@formatter:off` comment to the end of the matching `@formatter:on` comment,
or to the end of the file. The token helper sees the region's first token and emits the region as
one `Verbatim`; tokens inside it then emit nothing. The `off` comment itself is placed by the usual
comment rules, so the whitespace before it is the formatter's and everything from it to the end of
the `on` comment is the input's. The line break after a `// @formatter:on` comes from the `//`
comment rule.

Inside a region, nothing is rewritten: no reordering, and in A2 no import removal or doc comment
reflow. A modifier list that contains a marker isn't reordered at all.

GJF doesn't recognize the markers (observed: it reformats the code between them), so files with
markers are left out of the GJF comparison, as SCOPE.md says.

## Modifier reordering

Keyword modifiers are reordered into this order, merged from the JLS productions for class,
field, method, constructor, and interface member modifiers (JLS 8.1.1, 8.3.1, 8.4.3, 8.8.3, 9.1.1,
9.4):

`public protected private abstract default static final sealed non-sealed transient volatile
synchronized native strictfp`

Pairs the JLS never allows together (`final` and `sealed`, `default` and `static`) have no
meaningful order; the list fixes one anyway, so the result is deterministic. `non-sealed` is three
tokens, emitted together.

- **Runs:** only adjacent keyword modifiers are reordered among themselves. An annotation or a
  comment between two modifiers ends a run, and stays where it is. GJF leaves annotations in place
  (observed: `static @Foo public void a()` is unchanged), so this matches it for annotations.
- **Comments:** GJF reorders modifiers across a comment and leaves the comment at the same position
  in the list, so it ends up next to different modifiers: `final /* c */ public static int b`
  becomes `public /* c */ static final int b`. The comments design has a comment pin the modifiers
  around it, and comments never move across code, so Ferrion gives
  `final /* c */ public static int b`. This is a departure.
- **Markers:** a list containing a formatter marker isn't reordered (SCOPE.md).

Declaration annotations go on their own lines for types, methods, and fields, and stay inline for
parameters and locals, and after a keyword modifier (observed). That is a construct rule, not part
of reordering.

## Safety properties

`verify(input, output)` checks the four properties of SCOPE.md:

1. **Idempotence:** formatting `output` gives `output`.
2. **Equivalence:** the output parses to the same tree. It compares the two trees' node kinds in
   pre-order and their non-trivia tokens (kind and text) in order. Comparing node kinds catches
   what tokens alone can't, such as `>` `>` no longer being adjacent and so not a shift. Within each
   modifier run, the tokens are compared as a sorted list. A2 adds its rewrites: removed imports
   must be ones the import analysis reports as removable, and sorted imports are compared as a
   set.
3. **Comments:** the input's and output's comment tokens are the same, in the same order, with the
   same text after normalizing what the formatter may change: trailing whitespace, the indentation
   of a re-indented block comment's later lines, and in A2 reflowed doc comments by their parsed
   content.
4. **No new syntax errors:** the output's parse has no errors. Whole-file formatting only runs on
   files without errors, so any error is new.

Text block contents are compared exactly, apart from line terminators.

The properties hold by construction (only source tokens are printed, in order, each once), so
`verify` is a check on the construction, not a second implementation of the style. A `Violation`
names the property and the first place the two sides differ.

## Ranges and edits

Because output tokens are input tokens in order, the difference between input and output is a list
of whitespace changes between adjacent tokens, plus re-indented comments and reordered modifier
runs. That list is the language server's minimal edits (A5), and range formatting (A2) applies the
changes that fall in the range. A1 only has to keep that true; the range API itself is A2's.

## Testing

- **Snapshots:** `crates/ferrion_formatter/tests/snapshots/<area>/`, each input with its expected
  output, written with `FERRION_BLESS=1` as the parser's are. Every snapshot also runs `verify`.
  Expected outputs that match GJF are checked against it by the comparison below, so they don't
  drift from the reference.
- **Corpus and javac tests:** a new tool, `tools/format-check` (`cargo format-check`), formats every
  corpus file at the releases it's meant for, and every javac test input that parses, runs `verify`
  on each, and reports throughput. It runs on one corpus project in CI for each pull request, as
  `parser-check` does, and on the whole corpus and the javac tests in the weekly job.
- **Fuzzing:** a `format` target in `fuzz/` decodes the input, parses it at a few releases, and for
  each parse without errors formats and runs `verify`, panicking on any violation. Seeds are the
  parser's, and reduced failures become snapshots.
- **Deep nesting:** the parser's generated stress inputs, as [above](#deep-nesting).

## Google Java Format comparison

### The pin

- **GJF 2.0.0,** the `all-deps` jar from Maven Central, released on 2026-10-30, with the SHA-256
  `c78feb48c549fa7114b7ea11db3b456808f7b371ffe84e6643a44dd2f9588639`. It is recorded in
  `tools/format-check/gjf.toml` and fetched at test time into the corpus's cache directory, like the
  corpus jars. The jar is Apache-2.0 and is never committed.
- **JDK 27 (Temurin).** GJF parses with the JDK it runs on: under JDK 21 it rejects Java 25 syntax
  such as `import module java.base;` (observed), so files that use it would drop out. JDK 27 is
  `LATEST_RELEASE`, the newest release the corpus targets (Helidon), and the JDK CI already installs
  for its other JDK jobs. The tool reads the JDK from `JAVA_HOME` and checks its feature version, as
  `oracle-check` does.

### The run

- **Flags:** `--skip-reflowing-long-strings` (SCOPE.md: long strings are not split),
  `--cleanup=false` (2.0.0's default cleanup pass removes redundant semicolons, empty annotation
  parentheses, and parentheses around a lambda's single parameter, which Ferrion doesn't), and for
  A1, `--skip-javadoc-formatting`, `--skip-sorting-imports`, and `--skip-removing-unused-imports`,
  since A1 doesn't reflow doc comments or touch imports. A2 drops the last three.
- **Fixed point:** GJF runs on its own output until it stops changing, at most three times. Files
  that don't settle are reported and left out. Over the corpus, 27 files change again on a second
  pass, and all settle on it (observed): mostly a `//` comment aligned under a trailing `//`
  comment on the line before, which moves to the code's indentation on the second pass. The
  observations are logged in `tools/format-check/gjf.toml`.
- **Left out:** files GJF rejects, files with `@formatter:off` or `@formatter:on`, and files
  Ferrion skips.
- **Batching:** GJF is given many files per JVM start, on copies in a temporary directory, so the
  comparison doesn't pay a JVM start per file.

### The measure

For each corpus project: files compared, files identical, and lines that differ (a line diff's
insertions plus deletions), between Ferrion's output and GJF's fixed point, for the same input.
`cargo format-check gjf` prints them, with the largest differing files and a sample of hunks.

**Tracking:** the totals are committed in `tools/format-check/gjf-diff.toml`. The weekly job fails
if a project's differing lines go above the committed number, and a change that lowers them updates
the file. The file's history is the record over time, as `thresholds.toml` is for recovery
quality.

### Departures, checked

Each departure in `docs/style.md` has a case in `tools/format-check/differences/`: an input, GJF's
output, and Ferrion's. `cargo format-check differences` checks that both tools still produce them.
When the GJF pin moves, a departure GJF no longer makes shows up there.

## The style document

`docs/style.md` describes Ferrion's style construct by construct, with an example of each, and marks
every place it differs from GJF 2.0.0, with the reason: unstable in GJF, changes meaning, or
SCOPE.md. The construct work writes it as it goes.

Departures found so far:

| Input | GJF 2.0.0 | Ferrion | Reason |
| --- | --- | --- | --- |
| a text block | re-indented, `"""` on its own line | content copied; `"""` on its own line, as in GJF | SCOPE.md |
| `final /* c */ public static int b;` | `public /* c */ static final int b;` | unchanged | SCOPE.md and comments design: comments never move across code |
| code between `@formatter:off` and `on` | formatted | copied | SCOPE.md |

Observed, considered as departures, and matched (decided in review):

| Input | GJF 2.0.0, which Ferrion matches | The departure considered |
| --- | --- | --- |
| `foo(/* none */);` | `foo(/* none */ );` | no space before `)` |
| `{}` as a statement | `{` and `}` on two lines | `{}`, as GJF prints an empty method or loop body |
| `f(a, /* c */ b)` broken | `a, /* c */` then `b` | `a,` then `/* c */ b`, since the comment leads `b` |
| `int x = 1; // long comment` over the limit | `int x =` then `1; // ...` | keep `int x = 1;` and let the comment overflow |

## Benchmarks

Benchmarks start with A1 (roadmap, cross-cutting work):

- **Throughput:** `cargo format-check` reports formatting throughput and peak memory over the
  corpus, as `parser-check round-trip` does for parsing. These are the numbers to compare with GJF
  later.
- **Regressions in CI:** a `formatter` module in the [benchmark harness](benchmarks.md)
  (`tools/bench`), with `formatter/corpus`: format every source file of the corpus projects at its
  source set's release, from trees parsed in setup, so the iteration measures `format_tree` alone.
  The `bench` job in CI then gates its instruction and heap counts against the base, by
  `tools/bench/thresholds.toml`, like the other components.
- **GJF benchmarks** (JVM and native image) are a first-release criterion, not A1's. They can
  reuse the comparison's GJF fetch.

## Work order

Phase 2 is complete, so A1 runs on the local `a1/integration` branch, with at most four agents at
once and heavy jobs going through the shared slot lock.

1. **Core (one workstream):** the crate's API, the document and printer with unit tests, the token
   and comment helpers, copied text and disable regions, modifier reordering, `verify`, the
   deep-nesting tests, the fuzz target, `tools/format-check` with the property check and the GJF
   comparison, the `formatter/corpus` benchmark, and a first rule for every node kind that prints
   its tokens with single spaces and hard lines. That last part means every later step is measured
   from a formatter that already passes the safety properties on the whole corpus. The core then
   stops for review before the construct work starts.
2. **Constructs, in parallel, one module each:**
   - declarations: compilation unit, package and imports (printed in input order), types, bodies,
     members, annotations, enums, records, `module-info.java`;
   - statements: blocks, control flow, switch statements and expressions, `try`, labels, local
     declarations;
   - expressions: operators, assignment, calls and chains, lambdas and method references,
     `?:`, object creation, array initializers, casts, `instanceof` and patterns;
   - types: type parameters and arguments, arrays, wildcards, type annotations, `throws`.

   Each starts by observing GJF on its constructs, records the observations as snapshots, and adds
   its section to `docs/style.md`.
3. **The GJF diff:** work down the largest differences on the corpus, and finish `docs/style.md`.
4. **Exit check** against the roadmap's A1 criteria.

## What was consulted

- Philip Wadler, "A prettier printer" (2003), and Daan Leijen's description of the `wl-pprint`
  library, for the document model.
- Published documentation of other pretty printers' document commands, for hugging and fill.
- The Google Java Style Guide, section 4 (formatting).
- Google Java Format 2.0.0's release notes and `--help`, and its output on inputs written for this
  document, run on Corretto 21, 25, and 27 (logged above). Its source was not consulted.
- The JLS sections cited above.

## Decisions made in review

- **The GJF pin is 2.0.0,** not 1.37.0, the last 1.x release. 2.0.0 was released the day before
  this document was written and replaces GJF's layout engine. Most GJF users were still on 1.x
  then, but the first release is some way off, and GJF recommends reformatting in one commit when
  upgrading.
- **The comparison runs on Temurin 27.** The first plan was Corretto 21, then Corretto 25, the
  newest release the corpus targeted when this document was drafted. Helidon has since been added
  at Java 27, and CI installs Temurin 27 (see [The pin](#the-pin)).
- **The formatter's core takes a parsed tree** (`format_tree`), and `format` and `verify` parse
  around it. The crate depends on `ferrion_parser` for those two, as an exception to the parser
  design's rule that consumers of the tree depend on `ferrion_syntax` alone.
- **Indentation isn't capped at deep nesting,** even though the output grows with the square of
  the depth (see [Deep nesting](#deep-nesting)).
- **The core workstream stops for review** before the construct agents start.
- **Text blocks break before the opening `"""`, as in GJF,** rather than being hugged so that
  `String s = """` stays on one line. Their content is still copied as written.
- **The four small differences considered in [the style document](#the-style-document) are not
  departures:** Ferrion matches GJF on each, following the rule that it departs only where GJF's
  output is unstable, changes meaning, or goes against SCOPE.md.
- **Benchmark regressions are flagged by instruction counts,** with the callgrind-based check in
  [Benchmarks](#benchmarks), rather than by wall-clock time.

## Choices made while building the core

The core (work order step 1) settled some details the design left open:

- **Whitespace around comments** comes from each comment's own line breaks in the input, at most
  one blank line: none before or after a comment becomes a space if the input had whitespace
  there, and nothing if it didn't (`/*a*/b` stays together); one becomes a line break, and more a
  blank line. A line comment always ends its line. These merge with the rules' whitespace, the
  larger winning, so formatting the output asks for the same breaks again. A rule that keeps blank
  lines (`line_break_before`) looks at the gap right after the previous code; the gaps between
  comments are the comments'.
- **Comments are placed in source order** by the token helper: before a token, every comment the
  input has before it; after it, the comments that trail it or a node it ends. Dangling comments
  come out before the closing delimiter, and a rule can place them earlier, inside an indent, with
  `comments_before`.
- **A space between text that would run together.** The printer puts a space between two pieces
  of text written side by side when they weren't adjacent in the input and could lex differently
  together (`-` `-`, `int` `x`, `/` `/*`), so a rule that leaves one out can't change the program.
- **Fill separators** are their own element, `FillBreak`, which records the next item's width.
- **A disable region that runs to the end of the file** ends the output as written, with no line
  ending added, so the file is left exactly as it was from the `off` comment on.
- **Doc comments** get the same re-indenting as other block comments in A1: stars aligned when
  every later line starts with `*`, otherwise the shape kept.
- **Markers** are found in comments of every kind, doc comments included.
- **`format_tree` skips a tree with a syntax error** (`Skipped` with no diagnostics), and `verify`
  accepts an input with errors only if the output is the input.
- **`verify` reads the output as UTF-8,** with `SourceText::from_utf8`: `SourceText::decode` takes
  a file with a quarter or more NUL bytes in its first kilobyte for UTF-16, and the output can
  cross that share when the input didn't (found by the fuzz target). Equivalence also compares
  each node's subtree size and each token's node, not only the kinds in order. A `Violation` has
  the property, a message, and the byte offset in the output where it first fails.
