# Language versions

How the parser handles syntax from every supported release (8 through 27), how constructs the target
release doesn't allow are reported, how preview syntax is reported, and how contextual keywords are
read. Part of the [parser design](parser.md).

## Decision

1. **One grammar for all releases.** The parser accepts the syntax of every supported release, and
   the preview syntax it recognizes, and builds the same complete node whatever the target release:
   a record in a Java 11 file is a record declaration, never an error node.
2. **A closed list of release-dependent readings.** Where code that is valid at one release means
   something else at another, the parser takes the release into account. Today the list has one
   entry, `yield` (see [Release-dependent readings](#release-dependent-readings)).
3. **A separate version check.** After parsing, one pass over the tree reports every construct the
   target release doesn't allow, from a table of language features. Messages name the feature and
   the release it needs, for example "records require Java 16+". Preview features get "... is a
   preview feature, not supported".
4. **Contextual keywords are decided by the parser,** from context, the same way at every release.
   The tree records the decision.

The reasons: the formatter, the linter, and the language server all get one tree shape per
construct; the version check is small, table-driven, and testable alone; and adding a release means
adding table rows, not grammar variants.

## The parser's input

`parse(text, ParseOptions { release })`. The release is required, as it is for formatting and
linting. The parser uses it only for the readings below; the version check and doc comment
classification use it through the tree, which records it.

## Release-dependent readings

javac's parser reads some code differently depending on `--release`. Observed with the oracle:

| Input | Releases 8–13 | Releases 14+ |
| --- | --- | --- |
| `yield(1);` in a method | a call to a method named `yield` (with a warning) | a `yield` statement |
| `yield x;` in a method | a local variable `x` of type `yield` (with a warning) | a `yield` statement |
| `record R(int x) {}` in a class | a method `R` returning `record`, plus an error: records need 16 | a record (an error below 16) |
| `sealed interface S permits R {}` | `sealed` is reported as needing 17, then `'{' expected` at `permits` | at 17+: a sealed interface |

Only `yield` changes the meaning of code that is valid at both releases; the others are errors at
the older release however they are read. So:

- **`yield`:** at release 13 and below, the parser reads `yield` at the start of a statement as an
  identifier, as javac does; at 14 and above, as a `yield` statement. The tree differs between the
  two, so it records its release.
- **Everything else** is parsed with the newest reading, and the version check reports the feature.
  Where javac's older reading adds errors (the `'{' expected` above), the difference is javac
  recovery after an error, and is listed with the other differences.

A new entry needs an input showing that javac reads valid code differently by release, and a row
here. The alternative, a parser that ignores the release entirely, would give `yield(1);` a
`yield` statement in a Java 11 file, where it calls a method; the formatter would print it as a
statement and semantic analysis would have to undo the parser's decision.

## The version check

`version::check(&tree)` walks the tree once, iteratively, and returns diagnostics. It runs as part
of `parse`, and its diagnostics have their own category, so the formatter can say why it skipped a
file and the language server can show them distinctly.

### The feature table

Each language feature that needs a release later than 8 is a row in one table, `version::Feature`,
used by the version check and by later phases:

- **status:** final in release N, or preview;
- **anchor:** the token the diagnostic points at, chosen to match javac's position;
- **javac phase:** whether javac reports it while parsing, or in a later phase (see
  [syntax errors](parser-error-recovery.md#reported-earlier-than-javac));
- **name:** the words used in the message.

Rows checked against the oracle so far (javac 27; the anchor is javac's diagnostic position):

| Feature | Needs | Anchor | javac phase |
| --- | --- | --- | --- |
| modules (`module-info.java`) | 9 | `module` | parse |
| private interface methods | 9 | the `(` after the method's name | parse |
| variables in try-with-resources (`try (r)`) | 9 | the `)` after the variable | parse |
| diamond with anonymous classes | 9 | | later |
| switch expressions | 14 | `switch` | parse |
| switch rules (`case ... ->`) | 14 | `->` | parse |
| multiple case labels (`case 1, 2`) | 14 | the `,` | parse |
| text blocks | 15 | the text block | parse |
| records | 16 | `record` | parse |
| pattern matching in `instanceof` | 16 | the binding's name | parse |
| local enums | 16 | `enum` | parse ("enum classes must not be local") |
| local interfaces, static members of inner classes | 16 | | later |
| sealed classes | 17 | `sealed` or `non-sealed` | parse |
| patterns in switch | 21 | the pattern | parse |
| record patterns | 21 | the `(` | parse |
| unnamed variables (`_`) | 22 | `_` | parse |
| module imports (`import module`) | 25 | `module` | parse |
| implicitly declared classes | 25 | the first member | parse |
| flexible constructor bodies | 25 | | later |
| primitive types in patterns | preview | | later |

The table is completed during implementation, one row per feature from the JLS and JEPs, each with
an oracle input. javac reports each feature once per file, at its first use (three `case ->` rules
in a Java 8 file give one error). Ferrion reports every use, which is more useful in an editor; the
comparison with javac takes the first.

### Restricted identifiers

Some identifiers became restricted at a release: `var` (10), `yield` (14), `record` (16), `sealed`
and `permits` (17) can't name a type, and `_` became a keyword at 9 and an unnamed variable at 22.
These checks are in the version check too, since they depend on the release:

- at releases where the use is still allowed, javac warns (`var` as a type name at 8, `_` as an
  identifier at 8, `yield(1)` at 13). Ferrion produces the same warnings, only to
  compare them with javac's, and never shows them to users;
- at releases where it isn't, it is an error, at javac's anchor (the declared name, or the `_`).

`var` and `_` are identifiers in the tree at every release. Whether `var x = 1;` declares an
inferred local (10+) or a variable of a type named `var` (8 and 9) is semantic analysis's decision,
using the tree's release.

### Severity that depends on the release

A `;` before the first import is a warning ("extraneous semicolon") at releases 8 through 20, and an
error from 21. The parser reports the `;`, and the diagnostic's severity comes from the release, in
the version module, with the other release-dependent rules.

### Messages

- **Too old:** "records require Java 16+ (the target is Java 11)". One wording per feature, in the
  table.
- **Preview:** "primitive types in patterns are a preview feature, not supported". Ferrion never
  enables preview features, so there is no "use --enable-preview".
- **Not decidable from syntax:** some version rules need names or types (primitive patterns in a
  `switch` on a `long`, for example). Those are reported by semantic analysis, using the same
  table, so the feature's name and release are defined once.

## Preview syntax

A preview feature here is one that is preview in the latest supported release (27 today) and final
in no supported release. That matches javac: with `--release 15`, javac 27 says records "are not
supported in -source 15", not that they are a preview feature, since preview features can only be
enabled for the latest release. Its syntax is in the grammar when the parser can recognize it
without ambiguity, so that a file using it gets the preview message rather than a generic syntax
error. At JDK 27 the only preview language feature found so far is primitive types in patterns,
as in `o instanceof int i`, whose syntax the grammar accepts anyway; javac reports it after
parsing.

Each new GA release is a language-update work item (ROADMAP.md): its new final features become
table rows with their release, its preview features get rows marked preview, and their syntax is
added to the grammar if needed. A feature that leaves preview changes status.

Withdrawn preview features (string templates, preview in 21 and 22) are not recognized: javac 27
doesn't know them at any release, and they get a syntax error.

## Contextual keywords

The lexer makes contextual keywords `Identifier`s at every release
([lexer oracle normalization](lexer-oracle-normalization.md#contextual-keywords)). The parser reads
one as a keyword only where the grammar has that keyword and the lookahead confirms it:

| Keyword | Read as a keyword | Lookahead |
| --- | --- | --- |
| `module`, `open` | at the start of a module declaration, after imports and annotations | `open`? `module` name `{` |
| `requires`, `transitive`, `exports`, `opens`, `to`, `uses`, `provides`, `with` | in a module declaration's body | position in the directive |
| `module` | in `import module M;` | followed by a name, not `.` |
| `record` | where a type declaration can start | followed by a name and `(` or `<` |
| `sealed`, `non-sealed` | in the modifiers of a class or interface | followed by more modifiers or a class or interface keyword |
| `permits` | in a class or interface header | after the name, type parameters, and `extends` or `implements` |
| `yield` | at the start of a statement, at 14+ | not followed by `=`, `.`, `[`, `++`, or `--` (exact set settled with the oracle) |
| `when` | after a `case` pattern | followed by an expression |

`non-sealed` is three adjacent tokens with no trivia between them; with trivia (`non - sealed`) it
is an expression. Where the parser reads an identifier as a keyword, it records the keyword kind in
the tree's token kinds (see [syntax tree](parser-syntax-tree.md#contextual-keywords)). `var` and `_`
are never recorded as keywords, as explained above.

## Testing

- The version check is tested on small inputs per feature at the release before it and the release
  it arrives in, and compared with the oracle's diagnostics at every release.
- Each release-dependent reading is tested at the releases on both sides of the change.
- A test checks that every feature row has an input, and that every row javac reports later is
  [listed as reported earlier than javac](parser-error-recovery.md#reported-earlier-than-javac).
