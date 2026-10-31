# Error recovery and syntax errors

How the parser recovers from syntax errors, what "affected by a syntax error" means to the formatter
and the linter, how the quality of recovered trees is tested, and how Ferrion's syntax errors are
made comparable with javac's. Part of the [parser design](parser.md).

## Decision

- **Never fail.** `parse` always returns a tree that holds every token, and diagnostics. It never
  panics and never loops without consuming input.
- **Recover at the structure Java code has:** lists (members, statements, arguments, parameters,
  case labels) stop at their own terminators or at tokens an enclosing construct expects; tokens
  nothing expects go into an `ERROR` node; a missing token or child is reported and left absent.
- **One diagnostic per mistake.** After an error, nothing more is reported until the parser has
  consumed a token normally again.
- **Errors are local:** each error affects its innermost statement, member, or import, and the
  formatter and linter work on everything else.
- **Quality is measured** on corpus files broken at random, with a metric tracked in CI, plus
  reviewed snapshots.
- **Each diagnostic kind has a position rule** that gives the offset javac reports for the same
  mistake, so diagnostics are compared by kind and position, not by message.

## Recovery

### Lists and recovery sets

Most of Java's grammar is lists, and recovery happens there. A list knows:

- its terminator (`}` for members and statements, `)` for arguments, `;` for a `for` header, and so
  on);
- what can start an item (a type, a modifier, a statement keyword, an expression);
- the tokens its enclosing constructs are waiting for, passed down as a recovery set.

On a token that can't start an item:

1. if it is in the recovery set, the list ends there, and the missing terminator is reported. The
   enclosing construct continues from that token;
2. otherwise the parser puts tokens into one `ERROR` node until a token can start an item or is in
   the recovery set, and reports one error at the first of them. Balanced delimiters inside the
   skipped run are skipped as a unit, so a stray `(` doesn't swallow the rest of the file.

Characters the lexer can't make a token of (its `Error` tokens, already reported) are skipped by
the parser and placed like trivia, in the innermost node around them. javac's scanner skips them
the same way, so `import a.#b;` gets one error, for the `#`, and its import is still `a.b`.

### Missing pieces

- A missing token (`;`, `)`, a name) is reported, not invented. Its node ends at the last token it
  has. `expect(Semi)` is the common case.
- A missing child (`int b = ;`) is absent. Typed accessors return `None`, and the node is flagged
  `CONTAINS_ERROR`.
- No node is ever empty (see [syntax tree](parser-syntax-tree.md#layout)), so the language server
  never has to place a zero-width node.

### Unclosed blocks

An unclosed `{` would otherwise swallow the rest of the file into one method. Inside a block or a
switch block, a token that can only start a member, not a statement (an access modifier, `static`,
`native`, `transient`, or `volatile`), closes the block, with an error at that token. javac does the
same: given a method missing its `}` followed by `private void b()`, it reports "illegal start of
expression" at `private` and then parses `b` as a method. `abstract`, `final`, and `strictfp` can
start a local class, so they don't close a block. Further heuristics (indentation of a `}`
compared with its block's first line, for example) are added only when the quality metric below
shows they help.

### Progress

Every loop in the parser either consumes a token or exits. A debug assertion checks it. In release
builds, a loop that stalls puts the current token in an `ERROR` node and continues, so a bug in
recovery costs an extra error, never a hang. The fuzz targets run with debug assertions.

### Contextual decisions under errors

Where the parser needs lookahead to choose between readings (a cast or a parenthesized expression, a
lambda or a parenthesized expression, a generic call or a comparison, a local variable or an
expression statement), it decides from tokens alone, never by trying one reading and backtracking
after errors. A wrong guess in broken code is then a local error, not a cascade.

The lookahead is linear in what it scans. The one scan that could make parsing quadratic, for a
generic type before `::` (`List<String>::size`), which `a < b < c ...` would start at every name,
gives up after 1,024 tokens of type arguments, and the name is then read as an expression.

## What a syntax error affects

SCOPE.md says rules run on the parts of a file a syntax error doesn't affect, and range formatting
works where the range isn't affected. An error **affects its unit**: the innermost statement,
member declaration, import, package declaration, or module directive containing the error. If the
error is in a declaration's header (its modifiers, name, type parameters, or `extends`), the unit is
the whole declaration, since everything in it depends on the header.

- The tree flags every node containing an error (`CONTAINS_ERROR`) and offers `error_unit(node)`.
- **Linter:** a rule runs on a node if no error affects it. A rule that needs a whole method (unused
  variables) skips a method containing an affected statement; a rule that looks at one expression
  doesn't. The C1 framework turns this into rule-level policy.
- **Formatter:** whole-file formatting skips files with any error. Range formatting proceeds if no
  affected unit overlaps the range.
- **Fixes** are never applied to files with syntax errors (SCOPE.md), so they don't need this.

## Testing recovered trees

1. **Snapshots.** Inputs with errors in `recovery` directories under
   `crates/ferrion_parser/tests/snapshots/`, each with its printed tree and diagnostics as an
   expected file, reviewed like code, and written with the other snapshots (`FERRION_BLESS=1`).
   They cover each recovery rule above, in each area's own `recovery` directory, and the language
   server's mid-edit states, in `recovery/mid-edit/`: a half-typed member access (`foo.`), an
   unclosed `(`, `{`, string, or comment, a missing `;`, a statement typed at class level, an
   annotation with no declaration, an unfinished lambda or switch. A test checks that each reports
   an error.
2. **The corpus mutation test** (`cargo parser-check mutate`). For corpus files that parse
   without errors, at random tokens (seeded, with a fixed sample size, so reproducible): delete
   the token, duplicate it, insert a token from a small vocabulary (`{`, `}`, `(`, `)`, `;`, `.`,
   `=`, an identifier), truncate the file there, or delete a delimiter (`(`, `)`, `[`, `]`, `{`,
   `}`), which unbalances them. Inserted text has spaces around it, so it never joins a
   neighbouring token. Parse the clean and the broken file, and check:
   - **invariants:** no panic, the tree reproduces the broken text, the tree is valid, and at least
     one error is reported for a change that breaks the syntax. Whether a change breaks the syntax
     isn't decidable without a parser to trust, so the check is made where it is certain: when the
     broken file's delimiters don't balance. The others without an error are counted;
   - **preservation:** each member declaration of the clean file that doesn't overlap the change is
     present in the broken file's tree, at the same place shifted by the change, with the same
     kind, name, and text. Overlap is generous, so that the metric doesn't depend on how the change
     is attributed to units: a declaration that touches the change, such as one that starts right
     where a token is inserted, is left out, and so is each one enclosing the change. The share
     preserved, per mutation kind, is the quality metric. CI fails if it drops below a threshold set
     from the first measurement and raised as recovery improves, in
     `tools/parser-check/thresholds.toml`. The same is measured for statements in bodies, by kind
     and text, and for package and import declarations.

   The A2 range-formatting test (break files at random tokens and check that unaffected ranges
   format as in the clean file) is the same harness with a different check.
3. **Fuzzing:** the parser fuzz targets check the tree's invariants and that the parser terminates.
4. **javac's first error:** on the snapshot inputs and javac's own test inputs, Ferrion's first
   error is expected to match javac's first parsing-phase error by kind and position. Later errors
   depend on each parser's recovery and aren't compared (see
   `parser-oracle-normalization.md`).

## Syntax errors and javac

The Phase 2 exit criteria compare Ferrion's syntax errors with javac's parsing-phase errors, by
position and kind, not by message. The oracle's `diagnostics` section gives each javac diagnostic's
`code`, `start`, `position`, and `end`. The normalization and the kind mapping are in
`parser-oracle-normalization.md`, written separately. The parser's side is below.

### Diagnostics

```rust
pub struct ParseDiagnostic {
    pub kind: ParseDiagnosticKind, // lexical, syntax, version, or preview, with details
    pub severity: Severity,        // error or warning
    pub span: Span,                // what an editor underlines
    pub anchor: u32,               // the offset javac's `position` corresponds to
    pub javac_phase: JavacPhase,   // when javac reports it: Parse, Later, or Never
}
```

`span` is for people: it can cover a whole unexpected run of tokens. `anchor` is for the comparison:
each kind has a rule, documented on the kind, that gives the offset javac reports as `position` for
the same mistake. The tree has every token, so any such offset is computable. Observed with
javac 27:

| Mistake | javac code | javac `position` |
| --- | --- | --- |
| `int a = 1` then a newline and the next member | `expected` (`';' expected`) | end of `1`, the previous token |
| `int b = ;` | `illegal.start.of.expr` | start of `;`, the unexpected token |
| `foo(1, ;` | `illegal.start.of.expr` | start of `;` |
| `x.;` | `expected` (`<identifier> expected`), then `not.stmt` | end of `.`; `not.stmt` spans `x.`, with its position at the `.` |
| `return 1 }` | `expected` (`';' expected`) | end of `1` |
| `foo(a b)` | `expected2` (`')' or ',' expected`) | end of `a` |
| `x instanceof ;` | `illegal.start.of.type` | start of `;` |
| `int;` in an expression, `a[];` | `dot.class.expected` | start of `;`, the token after the type |
| `new int[];` | `array.dimension.missing` | start of `;`, the token after the `]` |
| `new int[1] {2}` | `illegal.array.creation.both.dimension.and.initialization` | the `{` |
| `(a, int b) -> 1` | `invalid.lambda.parameter.declaration` | the lambda's `(` |
| `}` among top-level declarations | `expected3` before 16, `expected4` to 24, then `class.method.or.field.expected` | start of `}` |
| `class A { 5 }`, `class A { public }` | `illegal.start.of.type` | start of the token that can't start a member |
| `void f()` then the class's `}` | `expected2` (`'{' or ';' expected`) | end of `)` |
| `B() {}` in class `A` | `invalid.meth.decl.ret.type.req` | the name |
| `public public int x;` | `repeated.modifier` | the second `public` |
| `enum E { int x; }` | `enum.constant.expected` | start of `int` |
| `class A permits B {}` | `invalid.permits.clause` | `permits` |
| `record R(final int x)` | `record.cant.declare.field.modifiers` | start of the modifiers |
| `module m { foo a; }` | `invalid.module.directive` | start of `foo` |
| `open m {}` | `expected.module` | start of `m` |
| `module m {} class A {}` | `expected` (end of input expected) | end of the module's `}` |
| an unclosed class at the end of the file | `premature.eof` | end of the last token |
| a `;` before an import | `extraneous.semicolon` (a warning before 21) | the first `;` of a run |
| a feature too new for the release | `feature.not.supported.in.source(.plural)` | the feature's anchor (see [language versions](parser-language-versions.md#the-feature-table)) |
| `class var {}`, `<var>` | `restricted.type.not.allowed` (a warning before the release) | the name |
| `var` as a field's type | `restricted.type.not.allowed.here` (a warning before 10) | `var` |
| `var x = 1;` at 8 or 9 | `restricted.type.not.allowed` (a warning) | the variable's name |
| `var a[] = ...`, `var a = 1, b = 2;` | `restricted.type.not.allowed.array`, `.compound` | `var` |
| `(var a, int b) -> a` from 11 | `invalid.lambda.parameter.declaration` | the lambda's `(` |
| `int _;` | `underscore.as.identifier` (a warning at 8), from 22 `use.of.underscore.not.allowed.non.variable` | the `_` |
| `int a, _;` as a field, `int _[]` as a local, from 22 | `use.of.underscore.not.allowed`, `.with.brackets` | the `_` |
| `yield(1)` in an expression | `invalid.yield` (a warning before 14) | `yield` |

So "expected" errors anchor at the end of the previous non-trivia token, and "illegal start" errors
at the start of the offending token. Errors about a construct that is complete but wrong anchor at
a token of their own, given with each diagnostic kind. Ferrion uses the same span conventions for display, which also
read well: the caret follows the token that should have been followed by a `;`.

Ferrion's kinds are its own (`Expected { what }`, `UnexpectedToken`, `NotAStatement`,
`UnclosedDelimiter`, `ElseWithoutIf`, `NestingTooDeep`, ...), named for Ferrion's messages, not
javac's codes. Each kind maps to one or more javac codes in the comparison's table.

### Reported earlier than javac

Some errors are syntax-shaped but javac reports them after parsing. Ferrion reports them while
parsing, because they are decidable from the tree. These are the feature table's rows with phase
`later` ([language versions](parser-language-versions.md#the-feature-table)), named as there; a
test checks that each is listed:

- '<>' with anonymous classes, below 9;
- local interfaces, and static members of inner classes, below 16;
- 'case null', below 21;
- statements before 'super()' or 'this()' (flexible constructor bodies), below 25;
- primitive types in patterns (preview).

The version check's tests compare them with javac's full compile, on the inputs in
`crates/ferrion_parser/tests/version/`.

The mechanism, so the list stays complete:

- every diagnostic kind and every feature row records `javac_phase: Parse | Later | Never`;
- the comparison with javac's parsing phase leaves out `Later` and `Never` diagnostics;
- each `Later` kind or feature has a case in `tools/oracle-check/differences/` whose snapshot shows
  javac's full compile reporting it, and no error from javac's parse;
- the normalization document lists them, and a test checks that the list, the cases, and the table
  agree, as the lexer's differences are checked.

Diagnostics javac never reports (`NestingTooDeep`) have `javac_phase: Never`. They are deliberate
departures, listed the same way.

### Warnings

javac's parsing phase also warns: `_` as an identifier at 8, `var` and `yield` used where they
will be restricted, an extraneous `;` before release 21. The version check produces the same
warnings, so they are compared like errors. Ferrion never shows them to users: they are compiler
warnings, not Ferrion findings, and they never stop formatting.
