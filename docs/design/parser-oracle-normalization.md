# Parser oracle normalization

The Phase 2 exit criteria in [ROADMAP.md](../ROADMAP.md) say syntax errors match javac's
parsing-phase errors for every supported release, across the corpus and javac's own test inputs,
that tree positions match the javac oracle, and that deep-nesting stress tests, javac's included,
pass. `cargo oracle syntax` and `cargo oracle tree` (`tools/oracle-check`) will check them. This
document records javac's side: what the oracle reports, how javac positions each construct, which
nodes have no positions, how parse diagnostics vary across releases, and how javac recovers from
errors. The parser work fills in Ferrion's side, in the sections marked **Ferrion side: to do**.

Everything about javac here was learned by running the oracle on inputs, never from javac's
source: on small inputs written for the purpose, most of them kept as
[snapshot cases](#snapshot-cases), on the corpus, and on javac's own tests. When javac's behavior is
in doubt, run the oracle on an input that shows it, and update this document and the comparison
together. Observations are for JDK 27 (Temurin 27.0.1).

## What is compared

For each file and release:

- **javac:** the oracle's `diagnostics` and `tree` sections (`--sections diagnostics,tree`), from
  parsing the file in a javac task of its own, with `--release <N> -proc:none`. Nothing after
  parsing runs, so errors javac reports while entering or attributing the file are never in it.
- **Ferrion:** to do. `tools/oracle-check/src/syntax.rs` defines the interface the parser
  implements (`Parser`: parse the file at each release it's compared at, since the parser reads
  some valid code differently by release, then give the errors and the tree in javac's terms).
  Until it's implemented, `NoParser` stands in, and both checks run javac, cache its output, and
  report counts without comparing: each file-release is counted as `not-compared`.

The comparisons are in `syntax.rs`:

- `syntax` compares javac's errors (warnings left out) with the parser's at each release: both
  have errors or neither does, and the first error in source order has the same code and
  `position`. This is the Phase 2 exit criterion for syntax errors, as decided in review. Later
  errors aren't compared: they depend on each parser's recovery after the first error, and
  matching them would mean reproducing javac's.
- `tree`, provisional until the parser work settles it with the normalization below, compares
  every node, as `(kind, start, end, depth)` in javac's order, for files javac reports no error
  for. Files javac rejects are skipped and listed, since javac's tree after an error depends on
  its recovery.

## The oracle's output

`tools/javac-oracle` writes one JSON object per file. Positions are UTF-16 offsets into the file
as javac reads it (a byte-order mark counts as one character; see the
[lexer normalization](lexer-oracle-normalization.md#byte-order-mark)), and -1 where javac has none.

Each **diagnostic** has:

- `kind`: `ERROR`, `WARNING`, `MANDATORY_WARNING`, `NOTE`, or `OTHER`;
- `code`: javac's diagnostic key, such as `compiler.err.expected`;
- `start`, `position`, `end`: for most parse errors all three are the same point. A few errors
  about a tree give its span, with `position` its preferred position: `else.without.if`,
  `not.stmt`, `array.dimension.missing`;
- `args`: the diagnostic's arguments. An argument that is itself a diagnostic is written as its
  code, so version errors name their feature (`compiler.misc.feature.records`) and the releases
  (`8`, `16`); token arguments are their spelling (`';'`, `token.identifier`);
- `message`: the English message, for people. Comparisons use `code` and `args`, never the
  message.

The **tree** is a list of `[kind, start, end, depth, preferred, shared]`, one per node, in the
pre-order of javac's `TreeScanner`:

- `kind` is javac's `Tree.Kind` name. Every kind but `OTHER` appears in the corpus and javac's
  tests: 117 kinds.
- `start` and `end` are what javac's `SourcePositions` gives.
- `depth` is 0 for the compilation unit.
- `preferred` is the position javac reports diagnostics about the node at: a declaration's name, a
  binary operator, and so on (see [Positions](#positions)).
- `shared` is 1 when the same node object was already written under another parent (see
  [Shared nodes](#shared-nodes)).

The oracle analyzes files on a thread with a 1 GiB stack, which the JVM reserves but commits only
as needed. With the default stack, javac overflows on 10,000 nested parentheses, blocks, `if`s,
array initializers, calls, or classes; with 1 GiB it parses all of them, so javac's output exists
for deep input. Input too deep even for that is reported as a crash.

## Inputs

### The corpus

`cargo oracle syntax` and `cargo oracle tree` take the corpus projects like `cargo oracle tokens`,
each file at its source set's release by default.

### javac's tests

`cargo corpus jdk-tests` fetches OpenJDK's javac tests as pinned in `tools/corpus/jdk-tests.toml`:
the `jdk-27-ga` tag of `https://github.com/openjdk/jdk`, `test/langtools/tools/javac` only. It
makes a shallow partial clone (`--filter=blob:none`) with a sparse checkout set before anything is
fetched, so git downloads file contents for that directory alone, and no implementation source is
ever on disk. The checkout is in `target/corpus/jdk-tests` (or under `FERRION_CORPUS_DIR`): about
36 MB of tests and 18 MB of git data, in a few seconds. Nothing from it is committed.

Every `.java` file there is an input: 5,630 files. Each is compared at the releases its test
compiles it at, read from its jtreg header and nothing else:

- `@compile` options `-source N`, `--source N`, `--release N` (also `1.N` and `${jdk.version}`),
  for the files the tag names, or the test file itself when it names none;
- `--enable-preview` in `@compile`, and the `@enablePreview` tag: compared at the latest release
  without preview features, which is what Ferrion's "preview feature, not supported" corresponds
  to;
- the `// options:` line of javac's diagnostic examples (`diags/examples`);
- otherwise javac's default, the latest release.

A file compiled at several releases is compared at each. Files compiled only at releases before 8
are left out (2 files). At `jdk-27-ga` that gives 5,702 file-releases: 5,552 at release 27, the
rest spread over 8 through 25, and 42 files compiled with preview features. One file
(`unicode/Unmappable.java`) isn't UTF-8 and is skipped.

Name `jdk-tests` on the command line to use them (`cargo oracle syntax jdk-tests`);
`cargo corpus jdk-tests list` prints each input with its releases.

### Deep nesting

`tools/corpus/jdk-tests-deep-nesting.txt` lists javac's deep-nesting stress tests among the
inputs: the 10 files whose javac tree is more than 50 levels deep, chosen by measurement with
`cargo oracle tree jdk-tests`. The deepest is 302 levels (`parser/8134007/T8134007.java`), then
the five `limits/ArrayDims*.java` at 258 and 259, `lambda/speculative/InferWeak.java` at 178, and
three more nested-lambda tests from 54 to 68. The next deepest input is 50 levels. Name
`jdk-deep-nesting` to run only these. javac parses all of them at every release without error.

Four tests whose names are about deep nesting (`DeepStringConcat.java`,
`DeeplyChainedNonPolyExpressionTest.java`, `SOEDeeplyNestedBlocksTest.java`,
`SOEForDeeplyNestedTypeTest.java`) measure as shallow (less than 36 levels), so they aren't
deep-nesting inputs; the list names them. Ferrion's own stress tests generate their deep inputs,
like the 10,000-level inputs above.

For comparison, the deepest corpus tree is 187 levels (a long chain of calls in guava's
`ArbitraryInstances.java`); chained calls and binary operators make left-deep trees as deep as the
chain is long.

### Snapshot cases

`tools/oracle-check/parser-cases/` holds small inputs written for this document, each with a
snapshot of javac's diagnostics at the releases the case names (`// releases: 8 16` on its first
line) and its tree at the last of them. `cargo oracle parser-cases` regenerates the snapshots and
fails if one changed, so a JDK that moves a position or changes an error is noticed; `--bless`
writes them. `cargo test` checks that this document links every case. The cases are cited below.

## javac's tree

### What the tree has, and doesn't

`TreeScanner` visits every child tree of every node, but much of a declaration isn't a tree:

- **Names** of classes, methods, variables, type parameters, and modules aren't nodes. The
  `preferred` position is where a declaration's name starts.
- **Modifier keywords** aren't nodes: `MODIFIERS` spans them, and only annotations are its
  children. Module `open` and `requires transitive static` aren't nodes either, and a module has
  no `MODIFIERS` node: its annotations are direct children
  ([`module-declaration.java`](../../tools/oracle-check/parser-cases/module-declaration.java)).
  Package annotations are direct children of `PACKAGE`
  ([`package-annotations.java`](../../tools/oracle-check/parser-cases/package-annotations.java)).
- **Operators** are in the kind (`PLUS`, `PLUS_ASSIGNMENT`, `POSTFIX_INCREMENT`), and their
  position is the node's `preferred`.
- **Erroneous trees** (`ERRONEOUS`) are leaves: `TreeScanner` doesn't visit the trees inside them.
  They appear only after errors.
- A constructor has no return type node, an implicit lambda parameter no type node, and a lambda
  with an expression body no `BLOCK`.

### Order

Children are in `TreeScanner`'s order, which isn't always source order:

| Parent | Order | Out of source order |
| --- | --- | --- |
| `METHOD` | modifiers, return type, type parameters, parameters, receiver, `throws`, body, default value | type parameters come after the return type |
| `CLASS` and the other type declarations | modifiers, type parameters, `extends`, `implements`, `permits`, members | a record's components are members, after `implements` ([`records.java`](../../tools/oracle-check/parser-cases/records.java)) |
| `METHOD_INVOCATION` | type arguments, method select, arguments | `this.<String>g()` lists `String` before `this.<String>g` ([`generic-calls.java`](../../tools/oracle-check/parser-cases/generic-calls.java)) |
| `NEW_CLASS` | enclosing expression, class, type arguments, arguments, body | `new <String>A<String>()` lists `A<String>` before `String` |
| `VARIABLE` | modifiers, type, name expression (`this` in a receiver), initializer | |
| `ANNOTATED_TYPE` | annotations, underlying type | in `int @A [] @B []`, `@A` comes first, wrapping the array type that contains `@B` ([`arrays-and-varargs.java`](../../tools/oracle-check/parser-cases/arrays-and-varargs.java)) |

So a comparison either produces Ferrion's nodes in this order, or sorts both sides the same way
first; siblings sorted by start position would put them in source order, but nodes without a
start (empty modifiers) and with the same start (`ARRAY_TYPE` chains) need a rule.

### Positions

A node's `start` is its first token and its `end` the end of its last token, with the exceptions
below. Comments are never inside a node's span at its edges: `COMPILATION_UNIT` starts at the first
token, after any leading comments ([`leading-comment.java`](../../tools/oracle-check/parser-cases/leading-comment.java)),
and ends at the end of the last token, before the final line break. `preferred` is the start,
except as listed.

| Construct | Observed | Case |
| --- | --- | --- |
| Class, interface, enum, record, annotation type | from the first modifier or annotation; `preferred` is the keyword (`class`, `record`, the `interface` of `@interface`), not the name | [`modifiers-and-annotations.java`](../../tools/oracle-check/parser-cases/modifiers-and-annotations.java) |
| `MODIFIERS` | from the first to the last modifier or annotation. Empty modifiers are -1..-1, except a method's that has type parameters and no modifiers: an empty span at the `<` | [`modifiers-and-annotations.java`](../../tools/oracle-check/parser-cases/modifiers-and-annotations.java) |
| `@interface` | its `MODIFIERS` include the `@`, so `@interface B {}` has modifiers spanning `@` | [`modifiers-and-annotations.java`](../../tools/oracle-check/parser-cases/modifiers-and-annotations.java) |
| Annotations after a method's type parameters (`<T> @A String m()`) | children of the method's `MODIFIERS`, outside its span | |
| Method | from modifiers, type parameters, return type, or name; to `}` or `;`; `preferred` is the name | |
| Field and local variable | to the `;`; `preferred` is the name | [`declarators.java`](../../tools/oracle-check/parser-cases/declarators.java) |
| `int a[], b;` | two `VARIABLE`s: the first is `int a[],`, ending after the comma; the second starts at the shared type and ends at `;`. In a `for` init, `int i = 0,` and `int i = 0, j = 1` | [`declarators.java`](../../tools/oracle-check/parser-cases/declarators.java) |
| `int a[]`, `int h()[]` | the `ARRAY_TYPE` spans from the element type to the last `]`, across the name (and the method's parameters) | [`declarators.java`](../../tools/oracle-check/parser-cases/declarators.java) |
| `int[][] d` | both `ARRAY_TYPE`s span `int[][]`; the inner one's end isn't its own `]`. Each one's `preferred` is a `[`, the outer's the first | [`declarators.java`](../../tools/oracle-check/parser-cases/declarators.java) |
| Varargs `String... a` | `ARRAY_TYPE` `String...`, `preferred` at `...`. With annotations on the dimensions (`String @A [] @B ...`), the `ARRAY_TYPE` can end before its `ANNOTATED_TYPE` child | [`arrays-and-varargs.java`](../../tools/oracle-check/parser-cases/arrays-and-varargs.java) |
| Parameter, record component | to the end of the name; `preferred` is the name | |
| Binary, assignment, compound assignment | left operand's start to right operand's end; `preferred` is the operator | [`expressions.java`](../../tools/oracle-check/parser-cases/expressions.java) |
| Conditional | `preferred` is the `?` | [`expressions.java`](../../tools/oracle-check/parser-cases/expressions.java) |
| Postfix `a++`, `instanceof` | `preferred` is the operator | [`expressions.java`](../../tools/oracle-check/parser-cases/expressions.java) |
| `PARENTHESIZED`, `TYPE_CAST` | include the parentheses | [`expressions.java`](../../tools/oracle-check/parser-cases/expressions.java) |
| Intersection cast `(A & B) x` | `INTERSECTION_TYPE` starts at the cast's `(` and ends at the last bound | [`expressions.java`](../../tools/oracle-check/parser-cases/expressions.java) |
| `MEMBER_SELECT` | `preferred` is the `.`; with type arguments (`this.<String>g`) the span includes them | [`generic-calls.java`](../../tools/oracle-check/parser-cases/generic-calls.java) |
| `METHOD_INVOCATION` | `preferred` is the `(` | [`generic-calls.java`](../../tools/oracle-check/parser-cases/generic-calls.java) |
| Diamond `new ArrayList<>()` | a `PARAMETERIZED_TYPE` with no arguments, `preferred` at `<` | [`generic-calls.java`](../../tools/oracle-check/parser-cases/generic-calls.java) |
| Anonymous class body | a `CLASS` from `{` to `}`, with empty modifiers | [`generic-calls.java`](../../tools/oracle-check/parser-cases/generic-calls.java) |
| Lambda | from the parameters (`(` or the name) to the body's end; `preferred` is the start, not the arrow. An implicit parameter's `VARIABLE` is just the name | [`lambdas-and-method-references.java`](../../tools/oracle-check/parser-cases/lambdas-and-method-references.java) |
| Method reference | from the qualifier to the name or `new`; `preferred` is the start | [`lambdas-and-method-references.java`](../../tools/oracle-check/parser-cases/lambdas-and-method-references.java) |
| Enum constant | `VARIABLE` from its annotations or name to the end of its arguments or body; see [Synthesized nodes](#synthesized-nodes) | [`enum-constants.java`](../../tools/oracle-check/parser-cases/enum-constants.java) |
| Record | compact constructor `METHOD` from the name to `}`; see [Synthesized nodes](#synthesized-nodes) | [`records.java`](../../tools/oracle-check/parser-cases/records.java) |
| `switch` | selector is a `PARENTHESIZED`. Each `case` or `default` is a `CASE` from its keyword to the end of its statements or body; `case 1: case 2:` is two `CASE`s, the first with no statements. Labels are `CONSTANT_CASE_LABEL`, `PATTERN_CASE_LABEL`, `DEFAULT_CASE_LABEL` (`case null, default` has two); a guard is a child of the `CASE` after its labels | [`switches-and-patterns.java`](../../tools/oracle-check/parser-cases/switches-and-patterns.java) |
| Patterns | `BINDING_PATTERN` wraps a `VARIABLE` with the same span; `DECONSTRUCTION_PATTERN` from the type to `)` | [`switches-and-patterns.java`](../../tools/oracle-check/parser-cases/switches-and-patterns.java) |
| Module | the compilation unit and `MODULE` start at the first annotation, after the doc comment; `preferred` is `module`. Each directive spans its keyword to `;` | [`module-declaration.java`](../../tools/oracle-check/parser-cases/module-declaration.java) |
| Package with annotations | the compilation unit and `PACKAGE` start at the first annotation; `preferred` is `package` | [`package-annotations.java`](../../tools/oracle-check/parser-cases/package-annotations.java) |
| A file with only comments | `COMPILATION_UNIT` with `start` at the end of the file and `end` 0. An empty file is 0..0 | [`only-a-comment.java`](../../tools/oracle-check/parser-cases/only-a-comment.java) |

### Nodes without positions

Run over the corpus and javac's tests, the files javac accepts have these nodes without a
position:

- **No start (and no end): empty `MODIFIERS`,** and nothing else. About 370,000 nodes.
- **A start but no end:** about 24,000 nodes, all [synthesized](#synthesized-nodes): 17,500
  `IDENTIFIER`s and 6,100 `NEW_CLASS`es of enum constants, and the copies of record components in
  compact constructors.

### Synthesized nodes

javac's parser makes some nodes that have no source of their own:

- **Enum constants** are `VARIABLE`s whose type is a synthesized `IDENTIFIER` (start at the
  constant's name, no end) and whose initializer is a synthesized `NEW_CLASS` whose class is
  another such `IDENTIFIER`. Without arguments or a body, the `NEW_CLASS` starts at the name and
  has no end; with arguments it spans them, from `(`; with only a body it starts at `{`. The body is
  a class of kind `ENUM` spanning from the constant's name to `}`, so it starts before its
  `NEW_CLASS` parent ([`enum-constants.java`](../../tools/oracle-check/parser-cases/enum-constants.java)).
- **A record's compact constructor** has the record's components as its parameters: copies whose
  nodes keep the components' starts and `preferred` positions but have no end
  ([`records.java`](../../tools/oracle-check/parser-cases/records.java)).
- **Implicitly declared classes** (top-level methods and fields) are wrapped in a `CLASS` with
  empty modifiers spanning all the top-level code. javac does this even at releases before 25,
  and while recovering from errors (see [Error recovery](#error-recovery)).

### Shared nodes

In `int a, b;` and `for (int i = 0, j = 1; ...)`, every declarator after the first has the same
`MODIFIERS` and type node objects as the first, so the tree lists them twice, the second time with
`shared` set to 1. In `int a[], b;`, `b` shares the modifiers and the element type `int`, which
`a`'s own `ARRAY_TYPE` wraps
([`declarators.java`](../../tools/oracle-check/parser-cases/declarators.java)). About 3,300 nodes
are shared in the corpus and javac's tests, mostly `MODIFIERS` and `PRIMITIVE_TYPE`.

### Spans that don't contain their children

Besides enum bodies and annotations after type parameters (above), an annotated varargs array type
can be shorter than its child. In the corpus and javac's tests, 500 nodes lie outside their
parent: 415 enum bodies, 82 annotations after type parameters, and 3 annotated array types. 67
compilation units have `start` after `end` (files with no tokens).

### Ferrion side: to do

- How Ferrion's nodes map to javac's kinds (the analogue of the lexer's `javac_name`), including
  declarations with several declarators, which javac splits into `VARIABLE`s with shared nodes.
- Which of javac's quirks above Ferrion reproduces in its javac-terms tree, and which become
  allowed differences: the comma in a first declarator's span, `int[][]`'s inner span, the
  intersection type's `(`, empty modifiers at `<`, the comment-only compilation unit.
- What Ferrion emits for synthesized nodes (enum constant types and initializers, compact
  constructor parameters, implicit classes), or whether they're dropped from javac's side before
  comparing.
- Whether the comparison uses javac's child order or sorts siblings, and whether it compares
  `preferred` too.

## javac's parse diagnostics

### What is reported while parsing

The `diagnostics` section is everything javac reports while parsing:

- **lexical errors** from its scanner (`unclosed.str.lit`, `illegal.char`, `illegal.esc.char`,
  ...), the same the lexer comparison stops at;
- **syntax errors** (`expected`, `illegal.start.of.expr`, `not.stmt`, `premature.eof`, ...);
- **language-version errors**: `feature.not.supported.in.source.plural`, or
  `feature.not.supported.in.source` for a feature with a singular name (pattern matching in
  `instanceof`), with the feature's `compiler.misc.feature.*` code, the release, and the first
  release that has it as arguments, and errors with their own codes (`underscore.as.identifier`,
  `local.enum`, `restricted.type.not.allowed*`, `invalid.yield`);
- **warnings** about names that later releases restrict (`warn.restricted.type.not.allowed` for
  `var`, `warn.underscore.as.identifier`, `warn.invalid.yield`,
  `warn.restricted.type.not.allowed.preview` for `record` before 16) and
  `warn.extraneous.semicolon`;
- `bad.file.name` for an implicitly declared class in a file whose name isn't an identifier: the
  class takes its name from the file, so this parse error depends on the file name
  ([`implicit-classes-and-module-imports.java`](../../tools/oracle-check/parser-cases/implicit-classes-and-module-imports.java),
  [`recovery-top-level.java`](../../tools/oracle-check/parser-cases/recovery-top-level.java)).

Diagnostics are in the order javac reports them, which isn't always source order: in
[`recovery-top-level.java`](../../tools/oracle-check/parser-cases/recovery-top-level.java), `a.b.;`
reports `<identifier> expected` after the second `.` before `not a statement` at `a`. javac seems
to report at most one error at a position: at release 8, `record R(int x) {}` raises its version
error twice at the same place (a diagnostic handler installed in the log sees both), and it's
reported once.

Errors javac reports after parsing never appear, however syntactic they look: illegal modifier
combinations (`abstract final`), `this(...)` outside a constructor, and preview features (see
[Preview features](#preview-features)). Some checks are made while parsing, though: a repeated
modifier (`repeated.modifier`), and a method without a return type
(`invalid.meth.decl.ret.type.req`).

Across javac's tests at their own releases, 229 of 5,702 file-releases have parse errors, with 98
distinct codes. The most common, by file-releases: `expected` (36), `feature.not.supported.in.
source.plural` (27), `illegal.start.of.type` (14), `illegal.start.of.expr` (13), `expected2` (11),
`not.stmt` (11), `restricted.type.not.allowed.here` (7), `class.method.or.field.expected` (6),
`invalid.lambda.parameter.declaration` (6), `illegal.char` (6), `expected3` (6), `premature.eof`
(6). In the corpus at each source set's release, the only parse errors are guava's two
`module-info.java` files, which are in release-8 source sets ("modules are not supported in
-source 8"); guava's build compiles them separately.

### By release

The same file gives different diagnostics at different releases. Observed at every release from 8
to 27 (`feature.not.supported...` abbreviated to "not supported", with its feature):

| Syntax | Releases with an error | Diagnostic |
| --- | --- | --- |
| `private` interface methods | 8 | not supported, `private.intf.methods` |
| a variable as a try-with-resources resource | 8 | not supported, `var.in.try.with.resources` |
| `module-info.java` | 8 | not supported, `modules` |
| `_` as an identifier | 8: warning; 9 and later: error | `underscore.as.identifier`; from 22, `use.of.underscore.not.allowed.non.variable` for a method named `_` |
| `_` as an unused local | 9–21 | not supported, `unnamed.variables` |
| `var` as a local's type | 8–9: warning | `warn.restricted.type.not.allowed` |
| `var` lambda parameters | 8–9: warnings; 10: error | not supported, `var.syntax.in.implicit.lambda` |
| `yield(1)` as a call | 8–13: warning | `warn.invalid.yield` |
| switch rules, multiple case labels, switch expressions | 8–13 | not supported: `switch.rules`, `multiple.case.labels`, `switch.expressions` |
| text blocks | 8–14 | not supported, `text.blocks` (from the scanner) |
| `instanceof` patterns | 8–15 | `feature.not.supported.in.source`, `pattern.matching.instanceof` |
| records, local records, interfaces, and enums | 8–15 | not supported, `records`; locals cascade into `';' expected` and `local.enum` |
| `sealed`, `non-sealed`, `permits` | 8–16 | not supported, `sealed.classes`, then `'{' expected` at `permits` |
| patterns in switch, record patterns | 8–20 | not supported: `pattern.switch`, `deconstruction.patterns` |
| `import module` | 8–24 | not supported, `module.imports` |
| implicitly declared classes | 8–24 | not supported, `implicit.classes` |
| flexible constructor bodies (statements before `super()`) | none | checked after parsing |

Snapshot cases: [`var-and-underscore.java`](../../tools/oracle-check/parser-cases/var-and-underscore.java),
[`sealed-and-text-blocks.java`](../../tools/oracle-check/parser-cases/sealed-and-text-blocks.java),
[`records.java`](../../tools/oracle-check/parser-cases/records.java),
[`switches-and-patterns.java`](../../tools/oracle-check/parser-cases/switches-and-patterns.java),
[`implicit-classes-and-module-imports.java`](../../tools/oracle-check/parser-cases/implicit-classes-and-module-imports.java).

A feature that was a preview at a release (records at 14 and 15, sealed classes at 15 and 16,
patterns in switch at 17 to 20, ...) is reported as not supported there, exactly as at earlier
releases: javac allows `--enable-preview` only at the latest release.

### Preview features

At JDK 27, no syntax is a preview feature at parse time. Primitive types in patterns are a preview
feature in 27, but javac parses `case int i ->` without complaint and reports
`preview.feature.disabled.plural` (`primitive.patterns`) only when attributing the file
([`preview-primitive-patterns.java`](../../tools/oracle-check/parser-cases/preview-primitive-patterns.java)).
None of the 42 javac tests compiled with preview features has a parse error without them.

So Ferrion's "preview feature, not supported" error has no counterpart in javac's parse
diagnostics at JDK 27. If Ferrion reports it while parsing, it's a syntax-shaped error Ferrion
reports earlier than javac, which ROADMAP.md says is documented.

### Error recovery

javac doesn't stop at the first error, but it reports few errors per mistake. In broad terms, from
running the oracle on the cases and javac's tests:

- After an error, javac skips to the end of the statement or member and goes on: `int z = 1 2 3;`
  reports one `';' expected` at `2`, then nothing until the next statement. A statement or member
  with several mistakes usually gets one error.
- A missing `;` is reported where it was expected, at the end of the previous token, and the tree
  goes on as if it were there: a `VARIABLE` missing its `;` ends at the next token's
  start ([`recovery-statements.java`](../../tools/oracle-check/parser-cases/recovery-statements.java)).
- A missing expression becomes an empty `ERRONEOUS` node at the error, so the tree keeps the
  statement: `int x = ;`, `g(1, );`.
- Unclosed braces give one `premature.eof` at the end of the file, and the open blocks end there
  ([`recovery-end-of-file.java`](../../tools/oracle-check/parser-cases/recovery-end-of-file.java)).
  An unclosed string literal is one lexical error; the literal becomes an empty `ERRONEOUS` and
  parsing goes on with the next line.
- Top-level code that isn't a type declaration (`garbage here;`) makes javac read the whole file as
  an implicitly declared class, at every release: the top-level classes become its members, and
  the stray code a field or method. Before 25 this adds an `implicit.classes` version error, and
  the file name can add `bad.file.name`
  ([`recovery-top-level.java`](../../tools/oracle-check/parser-cases/recovery-top-level.java)).
- Some version errors cascade, because javac doesn't parse the newer syntax at older releases:
  `sealed ... permits` adds `'{' expected`, a local record adds `';' expected` twice.

### Ferrion side: to do

- How Ferrion's errors map to javac's codes and positions, and which of javac's diagnostics are
  compared: errors only, or warnings too; the first error, or all of them; codes, or codes and
  arguments.
- The syntax-shaped errors Ferrion reports earlier than javac (preview features, and any others),
  each with an input that shows it.
- Allowed differences, in SCOPE.md's categories, with a row each, as in the
  [lexer's list](lexer-oracle-normalization.md#allowed-differences).

## Running it

```bash
cargo corpus jdk-tests
```

```bash
cargo oracle syntax jdk-tests
```

`cargo oracle syntax` and `cargo oracle tree` take projects, `jdk-tests`, and `jdk-deep-nesting`,
and the options of `cargo oracle tokens`. Until the parser is plugged in, each prints the usual
table with a `not-compared` column, the diagnostic codes javac reported (how often, and in how many
file-releases), and for `tree` the number of nodes, those without positions, and the deepest
trees. Observed when this was written, with 10 JVMs:

| Run | File-releases | javac's time | Parse errors | Deepest |
| --- | --- | --- | --- | --- |
| `syntax jdk-tests` | 5,702 | 21 s | 229 | |
| `tree jdk-tests` | 5,702 | 21 s | 229 | 302 |
| `tree jdk-deep-nesting --all-releases` | 200 | 3 s | 0 | 302 |
| `tree` (the corpus) | 10,577 | 37 s | 2 | 187 |

javac's output is cached as for tokens, keyed by the oracle jar, JDK, and sections, so `syntax` and
`tree` have caches of their own.

## Changing this

When javac's behavior (a new JDK), the oracle, or the corpus changes what's compared, update the
comparison in `tools/oracle-check`, the snapshot cases, and this document together. When the tag in
`jdk-tests.toml` moves to a new release, recheck `jdk-tests-deep-nesting.txt` and the numbers here.
