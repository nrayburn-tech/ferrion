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

Each language feature that needs a release later than 8 is a row in one table, `version::FEATURES`,
indexed by `version::Feature` and used by the version check and by later phases:

- **status:** final in release N, or preview;
- **anchor:** the token the diagnostic points at, chosen to match javac's position;
- **javac phase:** whether javac reports it while parsing, or in a later phase (see
  [syntax errors](parser-error-recovery.md#reported-earlier-than-javac));
- **javac's diagnostic:** its code, and the feature name among its arguments;
- **name:** the words used in the message.

The rows, each checked with the oracle (javac 27) at every release on an input in
`crates/ferrion_parser/tests/version/`, which has javac's output beside it. The anchor is javac's
diagnostic position; a test checks that this table and `version::FEATURES` agree.

| Feature | Needs | Anchor | javac phase |
| --- | --- | --- | --- |
| modules | 9 | `module`, after any annotations and `open` | parse |
| private interface methods | 9 | the `(` after the method's name | parse |
| variables as try-with-resources resources | 9 | the token after the variable, `;` or `)` | parse |
| '<>' with anonymous classes | 9 | the `<` | later (`cant.apply.diamond.1`) |
| 'var' in lambda parameters | 11 | the parameter's name, at 10 only | parse |
| switch expressions | 14 | `switch` | parse |
| switch rules ('case ->') | 14 | `->` | parse |
| multiple case labels | 14 | the first `,` in a switch expression, else the second label | parse |
| text blocks | 15 | the text block | parse |
| the '\s' escape sequence | 15 | the `s` | parse (javac calls it text blocks) |
| records | 16 | `record` | parse |
| patterns in 'instanceof' | 16 | the binding's name | parse |
| local enums | 16 | `enum` | parse (`local.enum`) |
| local interfaces | 16 | `interface` | later (`intf.not.allowed.here`) |
| static members of inner classes | 16 | a field's or method's name, `static`, or a type's keyword | later |
| sealed classes | 17 | `sealed`, or the `non` of `non-sealed` | parse |
| 'permits' clauses | 17 | the end of the token before `permits` | parse (`'{' expected`) |
| patterns in switch | 21 | the pattern, or `default` in `case null, default` | parse |
| 'case null' | 21 | `null` | later |
| record patterns | 21 | the `(` | parse |
| unnamed variables ('_') | 22 | `_` | parse |
| module imports | 25 | `module` | parse |
| top-level methods and fields (implicitly declared classes) | 25 | the first, after its modifiers | parse |
| statements before 'super()' or 'this()' | 25 | the call's `(` | later |
| primitive types in patterns | preview | the type | later (`preview.feature.disabled.plural`) |

javac reports each feature once per file, at its first use (three `case ->` rules in a Java 8 file
give one error). Ferrion reports every use, which is more useful in an editor; the comparison with
javac takes the first. javac also reports at most one error at a position, so where two features
meet at one token (`case null, default` at 13 is both multiple labels and a pattern in a switch),
the version check reports the one javac reports first, first.

Some rows need a note:

- **`var` in lambda parameters:** below 10, `var` is a type's name there, and the parameter only
  gets the restricted-name warning below. From 11, `var` can't be mixed with declared types,
  `(var a, int b)`, which javac 8 and 9 accept; the version check reports that
  (`invalid.lambda.parameter.declaration` at the `(`). Mixing `var` with inferred parameters,
  `(var a, b)`, is reported by the grammar at every release; at 10 javac reports only the
  feature, and this one release is a listed difference.
- **`permits` clauses:** below 17 `permits` isn't a keyword to javac, which expects the class body
  after the name and reports `'{' expected` at the end of the token before `permits`, whether or
  not the class is `sealed`. The tree has the clause at every release, and the version check
  reports this row at javac's position, so no second release-dependent reading is needed. The
  grammar's own error for a `permits` clause on a class that isn't sealed follows it.
- **Local enums:** javac 27 accepts an annotated local enum, or one with modifiers, below 16; the
  version check reports every local enum.
- **Static members of inner classes:** inner classes are non-static member classes of classes,
  enums, and records, local classes, and anonymous classes (enum constant bodies included). A
  `static final` field of a primitive type or `String` with an initializer may be a constant
  variable, which was always allowed, and whether it is needs names resolved; it isn't reported.
- **The `\s` escape:** javac reports it with the text blocks feature. Unicode escapes are
  translated first, so `\u005cs` is one too.
- **Not decidable from syntax:** primitive patterns decided by the selector's or operand's type
  (`switch` on a `long`, `i instanceof int j` with an `int` `i`), unconditional patterns in
  `instanceof` (21), and `case` on a qualified enum constant (21) are left to semantic analysis.

javac's older readings add errors the version check doesn't reproduce: below 16, a local record is
read as a variable of type `record` (a warning at its name and two `';' expected`), and a member
record as a method whose result type is `record` (a warning at `record`). They are listed as
differences in the version check's tests.

### Restricted identifiers

Some identifiers became restricted at a release: `var` (10), `yield` (14), `record` (16), `sealed`
and `permits` (17) can't name a type (`version::RestrictedName`), and `_` became a keyword at 9 and
an unnamed variable at 22. `when` and `module` aren't restricted. These checks are in the version
check too, since they depend on the release. At releases where the use is still allowed, javac
warns; Ferrion produces the same warnings, only to compare them with javac's, and never shows
them to users. At releases where it isn't, it is an error. Observed with the oracle:

| Use | Below the release | From the release |
| --- | --- | --- |
| a type's or type parameter's name (`class var`) | warning at the name | error at the name (`restricted.type.not.allowed`) |
| a local variable's type (`var x`, `yield x`, `final sealed s`) | warning at the first variable's name | `var`: none; `var[] a` or `var a[]`: `.array` at `var`; `var a, b`: `.compound` at `var`; the others: `.here` at the type |
| any other type (a field, parameter, result, cast, `catch`, type argument, `instanceof`) | warning at the type | error at the type (`restricted.type.not.allowed.here`) |
| `var` as a lambda parameter's type | warning at `var` | at 10 the feature above; from 11, nothing; with `[]` after the name, `.here` at 10 and `.array` from 11, at the name |
| an unqualified call `yield(...)` | warning at `yield` | error at `yield` (`invalid.yield`); at the start of a statement it is a `yield` statement |

`var` as a top-level record pattern's type (`o instanceof var(int x)`) is a type like the others.
`var` in a record pattern, as a nested record pattern's type, with type arguments (`var<T>`), and
after `new` (errors after parsing), and a qualified name ending in `var`, get nothing while
parsing. For `sealed int x;`, where `sealed` is read as a field's type, the restricted-type error
comes before the grammar's `<identifier> expected`, as with javac.

`_` is reported at every use, by where it is:

| Where | 8 | 9 to 21 | 22 and later |
| --- | --- | --- | --- |
| a local variable, a lambda or `catch` parameter, a pattern variable, or a nested `_` pattern | warning | unnamed variables need 22 | allowed |
| a local variable with `[]` after its name | warning | unnamed variables need 22 | error (`use.of.underscore.not.allowed.with.brackets`) |
| a field after the first in its declaration (`int a, _;`) | warning | unnamed variables need 22 | error (`use.of.underscore.not.allowed`) |
| anywhere else, `case _` included | warning | error (`underscore.as.identifier`) | error (`use.of.underscore.not.allowed.non.variable`) |

`var` and `_` are identifiers in the tree at every release. Whether `var x = 1;` declares an
inferred local (10+) or a variable of a type named `var` (8 and 9) is semantic analysis's decision,
using the tree's release.

### Severity that depends on the release

A `;` before an import is a warning ("extraneous semicolon") at releases 8 through 20, and an error
from 21; javac reports the first `;` of each run. The tree keeps the `;` as a token of the
compilation unit, and the version check reports it with the other release-dependent rules. A `;`
after the last import is an empty declaration, at every release.

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
| `module`, `open` | in the first declaration after the imports, after its annotations; later, before a name and `{`, or `open module`, as an error | none: `module m;` there is a module declaration missing its `{` |
| `requires`, `transitive`, `exports`, `opens`, `to`, `uses`, `provides`, `with` | in a module declaration's body | position in the directive |
| `module` | in `import module M;` | followed by a name, not `.` |
| `record` | where a type declaration can start | followed by a name, at every release: `record r;` is a record missing its header |
| `sealed`, `non-sealed` | in modifiers | followed by `public`, `protected`, `private`, `abstract`, `static`, `final`, `strictfp`, `sealed`, `non-sealed`, an annotation, `class`, `interface`, or `enum` |
| `permits` | in a class or interface header | after the name, type parameters, and `extends` or `implements` |
| `yield` | at the start of a statement, at 14+ | followed by `;` or a token that can start an expression; `++` or `--` only if not then followed by `;`; `(` unless a call follows (see below) |
| `when` | after a `case` pattern, or `case null, default` (an error) | always |

The lookahead for `module`, `open`, `record`, `sealed`, and `yield` is javac's, found with the
oracle, so that Ferrion reads invalid code as javac does and reports the same first error.
`non-sealed` is three adjacent tokens with no trivia between them; with trivia (`non - sealed`) it
is an expression.

javac's reading of `yield`, observed with the oracle at 14: `yield (1);`, `yield -1;`, `yield ++x;`,
and `yield;` (an error) are `yield` statements, and `yield = 1;`, `yield.f();`, `yield[0] = 1;`,
`yield++;`, `yield: ;`, `yield<T> x;`, `yield -> 1;`, and `yield * x;` use a name. At `yield (`,
javac looks ahead to the matching `)`: empty parentheses, or a `,` at their top level, make a call
(`yield(1, 2);`, `yield();`, reported as `invalid.yield`), unless `->` follows them
(`yield (a, b) -> a;` yields a lambda). A `<` before the `,` makes it part of type arguments until a
lone `>`, so `yield(a < b, c);` is a statement and `yield(a < b > c, d);` a call.

Where the parser reads an identifier as a keyword, it records the keyword kind in
the tree's token kinds (see [syntax tree](parser-syntax-tree.md#contextual-keywords)). `var` and `_`
are never recorded as keywords, as explained above.

## Testing

- The version check is tested on small inputs per feature at the release before it and the release
  it arrives in, and compared with the oracle's diagnostics at every release.
- Each release-dependent reading is tested at the releases on both sides of the change.
- A test checks that every feature row has an input, and that every row javac reports later is
  [listed as reported earlier than javac](parser-error-recovery.md#reported-earlier-than-javac).
