# Ferrion's Java style

This is the style `ferrion format` gives, construct by construct, with an example of each. It is
Google Java Format 2.0.0's style, as observed by running GJF, except where this document marks a
**departure** from it, with the reason: the GJF output is unstable, it changes the program's
meaning, or it contradicts [SCOPE.md](SCOPE.md#formatter). How the formatter produces the style is
in the [formatter design](design/formatter.md).

Each departure has a case in `tools/format-check/differences/`, which `cargo format-check
differences` checks against GJF and Ferrion both.

## General

- **Line width:** 100 columns, counted in UTF-16 code units. A line can still go over: a long
  token, a long string, or text that is copied.
- **Indentation:** +2 for a block's contents, +4 for a continuation line.
- **Line endings:** the file's line ending throughout, and one at the end of the file.
- **Blank lines:** one or more blank lines in the input become one where the style keeps the
  input's choice; each construct below says where blank lines are kept, required, or removed.
- **Comments** stay where they are relative to the code: a comment that leads a token stays before
  it, and one that trails a token stays after it. The line breaks around a comment come from the
  input, at most one blank line. Block comments that span lines are re-indented to their new
  column, with their stars aligned when every later line starts with `*`.
- **Modifiers** are reordered within each run of adjacent keyword modifiers into
  `public protected private abstract default static final sealed non-sealed transient volatile
  synchronized native strictfp`. Annotations stay where they are.
- **Formatter disable regions:** the text from an `@formatter:off` comment to the matching
  `@formatter:on` comment, or to the end of the file, is copied as written.

### Departures

- **A comment between modifiers** pins the modifiers around it: `final /* c */ public static int b`
  is unchanged, where GJF gives `public /* c */ static final int b`. Comments never move across
  code (SCOPE.md and the comments design). Case: `modifier-comment`.
- **Disable regions:** GJF doesn't recognize the markers and formats the code between them;
  Ferrion copies it (SCOPE.md). Case: `formatter-off`.

## Declarations

The compilation unit, package and imports, type declarations and their bodies, members,
annotations, enums, records, and `module-info.java`.

## Statements

Blocks, control flow, switch statements and expressions, `try`, labels, and local declarations.

## Expressions

Operators, assignment, calls and chains, lambdas and method references, `?:`, object creation,
array initializers, casts, `instanceof`, and patterns.

### Departures

- **Text blocks** aren't re-indented: their content and closing delimiter are copied as written
  (SCOPE.md). The opening `"""` is placed as GJF places it, on a line of its own. Case:
  `text-block`.

## Types

Type parameters and arguments, arrays, wildcards, type annotations, and `throws`.
