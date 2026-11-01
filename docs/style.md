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

Type parameters and arguments, arrays, wildcards, type annotations, and `throws`. The construct
around a type decides the breaks around it, such as before a field's name or before `throws`;
these rules lay out the inside of the type, and its lines break only where the construct around it
leaves it too long.

### Type arguments and type parameters

No space inside the angle brackets, none before the `<`, and one after each comma. The diamond is
`<>`.

```java
Map<String, List<? extends T>> a = new HashMap<>();
class A<T, U extends Number & Comparable<U>> {}
Foo.<String>bar();
```

A list that doesn't fit breaks after the `<`, with its items on one line at +4 if they fit there,
and otherwise one per line. The `>` follows the last item.

```java
class Pair<
    KeyTypeParameterName extends Comparable<KeyTypeParameterName>, ValueTypeParameter> {}

class Triple<
    KeyTypeParameterName extends Comparable<KeyTypeParameterName>,
    ValueTypeParameterName extends Comparable<ValueTypeParameterName>,
    ThirdTypeParameterName> {}
```

A comment on its own line before the first item doesn't put the items one per line; a line comment
after an item does.

### Bounds

A type parameter's bounds that don't fit after `extends` go on the next line at +4. They fill
their lines, breaking before an `&` at a further +4 when the bound after it doesn't fit, counting
the text that follows the last bound on its line, such as `> {}`.

```java
class A<
    T extends
        Comparable<T> & java.io.Serializable & Cloneable & SomeOtherInterface
            & AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA & BBBBBBBBBBBBBBBBBBBBBBBB> {}
```

Annotations on a type parameter go on its line, and break before the name when the parameter
doesn't fit.

### Wildcards

`?`, `? extends T`, and `? super T`, with single spaces. A bound that doesn't fit goes on the next
line at +4.

```java
List<
    ? extends
        SomeVeryLongTypeArgumentNameNumberOneXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX>
```

### Qualified types

No spaces around the dots. A qualified type that doesn't fit breaks before a dot at +4. How depends
on the case of its names, a syntactic heuristic, since the formatter has no type information:

- **A package and its types**, lowercase names followed only by uppercase ones
  (`java.util.Map.Entry`), fill their lines, breaking before a dot only when the name after it
  doesn't fit.
- **Any other chain** keeps its head on the first line and puts the rest one name per line if it
  doesn't fit. The head runs to the end of the first run of uppercase names, plus the name after
  it (`a.B.c`); with no uppercase name, it's the first name, and the second too if the first is at
  most 4 columns wide (`abcd.example`).

In a bound:

```java
    & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.Some
        .Other
    & abcde
        .example
        .some
```

When the last name has type arguments, the breaks before dots are at +8, the arguments break at
+4, and they break whenever the chain does.

```java
    & com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.and
            .Some<
        SomeVeryLongTypeArgumentNameNumberOne, B>
```

### Type annotations

A space follows each annotation in a type, and comes before an annotated dimension or varargs.

```java
java.util.@Nullable List<@A String> a;
String @NonNull [] b;
int @A [] @B [] c;
void m(@A int @B ... x) {}
```

### Arrays and varargs

No space before `[]` or `...` without annotations, and none between dimensions.

```java
int[][] a;
List<String>[] b;
int c[];
void m(String... args) {}
```

### Intersection types

`A & B` in a cast fills its lines, breaking before an `&` at +4.

```java
Object p = (Runnable & java.io.Serializable) () -> {};
```

### Union types

`A | B` in a `catch` is on one line if it fits, and otherwise has one alternative per line at +4,
each starting with its `|`.

```java
} catch (FirstVeryLongExceptionName
    | SecondVeryLongExceptionName
    | ThirdVeryLongExceptionName e) {
```

### `throws`

A `throws` list that doesn't fit has one type per line at +4, the first on the `throws` line if it
fits there.

```java
  void c()
      throws FirstVeryLongExceptionName,
          SecondVeryLongExceptionName,
          ThirdVeryLongExceptionName {}
```
