# Ferrion's Java style

This is the style `ferrion format` gives, construct by construct, with an example of each. It is
Google Java Format 2.0.0's style, as observed by running GJF, except where this document marks a
**departure** from it, with the reason: the GJF output is unstable, it changes the program's
meaning, or it contradicts [SCOPE.md](SCOPE.md#formatter). One more, deep indentation in
expressions, keeps the output's size in proportion to the input's. How the formatter produces the
style is in the [formatter design](design/formatter.md).

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
  input, at most one blank line, except that a block comment on one line that trails code, other
  than an opening `(`, `[`, or `{`, isn't followed by the input's line break, as GJF does. Where
  the line breaks there is then the rules' choice: `int x = /* why */` then `5;` gives
  `int x = /* why */ 5;`. One that starts a line with code after it on the line isn't preceded
  by the input's line break either, and one naming a parameter, `/* name= */`, keeps no line
  break around it: `foo(a,` then `/* c */ b)` gives `foo(a, /* c */ b)`.

  A comment on the line of the code before it has a space before it, as GJF puts one
  (`f(x);// c` gives `f(x); // c`), except a block comment after a `(`, `.`, or `<`, which goes
  right after it with a space after it instead (`f(/* c */ x)`). A bare `//` counts as three
  columns wide, as GJF measures it, so one that ends at the limit breaks its line.

  Block comments that span lines are re-indented to their new column, with their stars aligned
  one column after the `/` when every later line starts with `*`. Otherwise the later lines keep
  their indentation relative to each other, the least indented at the comment's column, not
  counting empty lines. A doc comment (`/**`) aligns the stars it has whatever its other lines
  are, and moves those by as much as its least indented star.

  A comment's text doesn't change, except that a line comment gets a space after its `//` when
  text follows it directly, as GJF does: `//x` becomes `// x`, and `//-----` becomes `// -----`.
  `//` alone, `///`, `////`, and a `//` followed by a tab are left alone, and so are
  `//noinspection` and `//$NON-NLS-1$`, which tools read.

  ```java
  // TODO fix
  ///without a space
  //noinspection unchecked
  ```
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
- **A doc comment's lines without a star** stay without one, moved as far as its stars. GJF
  rewrites the comment with a star on every line, an empty line included. Ferrion doesn't change
  a block comment's text (SCOPE.md). Case: `doc-comment-stars`.
- **Directives keep their `//`:** formatter markers (`//@formatter:off`), `ferrion-disable*`
  suppression comments, and the comments in a disable region get no space after their `//`,
  which GJF adds. A `//noinspection` or `//$NON-NLS-1$` trailing some statements, such as a local
  variable or an assignment, gets one in GJF, and keeps none in Ferrion wherever it is. Case:
  `directive-comments`.
- **Long `//` comments** are left as they are, past the column limit. GJF 2.0.0 wraps one that
  goes over onto a new `//` line (SCOPE.md: `//` comments are not wrapped). Case: `line-comment`.

## Declarations

The compilation unit and package, type declarations and their bodies, members,
annotations, enums, records, and `module-info.java`.

### The compilation unit

The package declaration, the import section ([Imports](#imports)), and the types, in input
order. A blank line follows the package declaration and the import section, whatever the input
has. Neither the package declaration nor an import ever breaks, however long.

```java
package com.example;

import java.util.List;

class A {
  List<String> names;
}
```

### Annotations on declarations

The annotations before the first keyword modifier of a type, a method, a constructor, an enum
constant, a package, or a module each go on their own line. An annotation after a keyword modifier
stays on its line, and a block comment after an annotation stays on the annotation's line.

```java
@Override
@Deprecated
public @Nullable String name() {}
```

The exception is a `@Nullable` or `@NonNull` right before the type of a method, a field, a
parameter, or a local variable, with nothing but annotations before it, which stays on the type's
line when the file imports it from `org.jspecify.annotations` or
`org.checkerframework.checker.nullness.qual`. GJF recognizes those type-use annotations by their
import (observed), and Ferrion does the same, from the syntax alone. Such an annotation is laid
out as part of the type, and the rules below are for the annotations before it: `@Nullable
String name` is a declaration without annotations, and in `@Deprecated @Nullable String name`
only `@Deprecated` can go on a line of its own.

```java
import org.jspecify.annotations.Nullable;

class A {
  @Override
  @Nullable String name() {}

  @Deprecated
  @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
      resultsAwaitingMerging = null;
}
```

A field's annotations do the same if any of them has arguments. Annotations without arguments stay
on the field's line, unless the field doesn't fit on it, and then they go on a line of their own
before it, still together, or one per line if they don't fit together on one line or a line
comment follows one before the last.

```java
@Inject @Named private Foo foo;

@Column(name = "id")
private long id;

@NoJvmEnvironment @BrowserExcluded // not in browsers
private static final String A = "a";

@NoJvmEnvironment
@BrowserExcluded // not in browsers
@VisibleForTesting
static final String B = "b";
```

A local variable's annotations go on their own lines if there are two or more, if one has
arguments, or if the declaration doesn't fit on its line; otherwise they stay on its line. A
parameter's annotations stay inline, and when the parameter doesn't fit, they break first, all on
one line at +4 if they fit there, otherwise one per line.

```java
void m(@Nullable String name, final @Deprecated int count) {
  @A int x = 1;
  @SuppressWarnings("unchecked")
  List<String> list = null;
}
```

A `catch` parameter's annotations go on their own lines if one of them has arguments, which breaks
the `catch (` as well, and otherwise stay on its line. A parameter that catches several types keeps
them on the `(`'s line, and breaks after them when its types break (see
[`try`, `catch`, `finally`](#try-catch-finally)) (observed).

```java
} catch (
    @SuppressWarnings("unused")
    IOException e) {
} catch (@SuppressWarnings("unused") IOException | RuntimeException e) {
}
```

### Annotation arguments

On the annotation's line if they fit. Otherwise one per line at +4, and also when there are two or
more and one of them is a pair whose value is an array. A single array of values hugs the
parentheses, with the values on the lines between the braces at +2.

```java
@A(
    value = {"a", "b"},
    other = 1)
@B({
  "first value",
  "second value"
})
```

An array of values that doesn't fit is filled, as many to a line as fit, if no value is wider than
9 columns, and otherwise has one value per line, never all of them on one line between the broken
braces as an element's `default` array may (see [Arrays](#arrays)). Values in rows are kept as an
array initializer's are. A trailing comma keeps the braces broken, and so does a comment on a line
of its own before the first value. A pair's value breaks onto the next line at +4 when it doesn't
fit, as a variable's value does, unless it's an array. Comments in the arguments are placed as in a
call's (see [Arguments](#arguments)); a line comment after the last one breaks them, with the `)`
on a line of its own at +4.

### Type declarations

`class`, `interface`, `enum`, `record`, and `@interface` headers. Each clause, `extends`,
`implements`, or `permits`, moves onto the next line at +4 only if it doesn't fit on the line it
would continue, so one clause can stay on the first line while the next breaks, or if the clause
before it took more than one line. A clause's types break one per line at +8 when they don't fit
on its line. Type parameters that break are indented from the clause lines. The `{` ends the
header's last line.

```java
class Handler extends AbstractHandler
    implements FirstInterface, SecondInterface {
  ...
}

class Registry
    implements FirstVeryLongInterfaceName,
        SecondVeryLongInterfaceName,
        ThirdVeryLongInterfaceName {}
```

An enum's `implements` is the exception: when its types don't fit on the clause's line, they move
together onto the next line at +8, and fill the lines from there. A record's `implements` goes on
the next line when its components go one per line, and otherwise stays on the line of the `)` if
it fits there, as a method's `throws` does.

```java
record Config(
    String name, Set<String> acceptedEncodingNames, boolean enabled) implements Channels {}
```

### Bodies and blank lines

A body's members go on their own lines at +2; an empty body is `{}`. A blank line at the start of a
class body is kept, and one at the end is removed, even after a trailing comment. A block comment on
the line of the `{` goes on a line of its own, and a line comment stays on the line of the `{`.

A doc comment goes on lines of its own, wherever it is, and a blank line between it and the code
after it is removed: `/** Doc. */ int x;` becomes two lines, and so does `@A /** Doc. */`. Other
comments before a member keep the input's line breaks.

Between members, a blank line is required before and after every member except a plain field and
an empty declaration (`;`), but not between a member and a `;` after it. A field is plain unless it
has a doc comment or its annotations are on their own lines, because one has arguments or because
the field doesn't fit on its line; a line comment that breaks the field doesn't count. Between two
plain fields, or a plain field and a `;`, the input's choice is kept.

```java
class A {
  int a;
  int b;

  /** The count. */
  int count;

  A() {}

  void m() {}
}
```

### Enums

Each constant goes on its own line, whatever their number or length, with its annotations on their
own lines before it. Blank lines between constants are kept, and one at the start or the end of the
body is removed. The `;` that ends the constants follows the last of them, or goes on its own line
after a trailing comma or when there are none. The members after it are laid out as in a class
body.

```java
enum Color {
  RED,
  GREEN,
  BLUE;

  Color next() {}
}
```

### Records

The header's components break as a method's parameters do. A compact constructor is its name and
its body.

```java
record Point(int x, int y) {
  Point {
    check(x, y);
  }
}
```

### Fields and variables

`Type name = value`. When it doesn't fit, the value breaks onto the next line at +4, except an
array initializer, whose braces break instead. If `Type name =` doesn't fit either, the name moves
onto the next line at +4, and the value is indented from it. A value whose layout forces a break,
such as a lambda with a block or an anonymous class, starts on the next line. Several declarators
go one per line at +4 when they don't fit on one, and if the first of them, up to its comma,
doesn't fit on the type's line either, it moves onto the next line at +4 too, with its value
indented from it. After a line comment on the name, the `=` or the `,` goes on the next line at
+4, and a value that breaks after the `=` is at +4 as well.

```java
private static final String MESSAGE =
    "a string that doesn't fit on the line of the field's name";
int first = 1,
    second = 2;
private static final Method
    CONSTRUCTOR =
        lookupMethod(void.class, ConstantDescs.INIT_NAME, String.class, Object[].class),
    DISPATCH = lookupMethod(double.class, "dispatch");
```

### Methods and constructors

Three breaks, outermost first, each taken only if what it ends doesn't fit on the line: after the
type parameters, between the return type and the name (`Type name(` must fit), and in the
parameters. The parameters go all on the line; otherwise all on the next line at +4; otherwise one
per line at +4. When a break before the name is taken, the parameters are indented from the name.
`throws` goes on the next line at +4 when it doesn't fit on the line of the `)`, and always when
the parameters went one per line; parameters that all moved to the next line together keep
`) throws X {` on their line if it fits there. However the header breaks, the body is indented
+2 from its first line.

```java
void process(
    String firstArgument, String secondArgument, String thirdArgument, int count) {}

public static Map<String, List<Integer>>
    longMethodName(int first, int second) {}

void read(int first, int second)
    throws IOException {}

void write(
    String firstArgumentName, String secondArgumentName, int count) throws IOException {}

void copy(
    String firstArgumentName,
    String secondArgumentName,
    String thirdArgumentName,
    String fourthArgumentName)
    throws IOException {}
```

A parameter's name moves onto the next line at +4 when the parameter doesn't fit on its line. An
empty parameter list with a block comment in it gets a space before the `)`, `(/* none */ )`.
Comments in the parameters are placed as in a call's arguments (see [Arguments](#arguments)),
except that a line comment after the last parameter puts the `)` on a line of its own at +4
without breaking the parameters. Type arguments and parameters do the same, with the `>` at the
type's indentation.

### Annotation type elements

An element's `default` value breaks onto the next line at +4 when it doesn't fit, unless it's an
array, whose braces break instead.

```java
@interface Info {
  String[] names() default {"a", "b"};
}
```

### Modules

`module-info.java`'s annotations go on their own lines. The directives go one per line, with a
blank line between directives of different kinds and none between directives of the same kind,
whatever the input has. The modules after `to` and the classes after `with` go one per line at +4,
always.

```java
module com.example {
  requires java.base;
  requires java.sql;

  exports com.example.api to
      com.example.client;
}
```

## Imports

The import section runs from the first import to the last, with the comments trailing the last on
its line. Its imports are sorted into groups and pruned, as GJF does it, and its comments move
with the imports they belong to.

- **Groups:** static imports, then `import module`, then all other imports, with a blank line
  between groups and none inside one. `java`, `javax`, and the rest share the last group.
- **Order:** within a group, by the whole name as written with its dots (and, for a static import,
  its member), compared character by character, not segment by segment: `a.B$` sorts before
  `a.B.C`, and `$` < `*` < `.` < digits < uppercase < `_` < lowercase < other characters. Names
  compare by UTF-16 code unit, as Java strings do, so `a.b𝔸` (a surrogate pair) sorts before
  `a.bｚ`, as in GJF. A name's Unicode escapes are translated first.
- **Removed:** a single-type or single static import whose simple name has no use in the file; a
  single-type import of a type of the file's own package (`p.q.A` in `package p.q;`, but not
  `p.q.A.B`); and an import repeated after the first. A use is an identifier in the code other than
  a declaration's own name, a label, the member after `::`, or a later segment of a qualified name
  (`Pkg.a.b()` uses `Pkg`; `x.Sel.y()` doesn't use `Sel`), or, in the doc comment attached to a
  declaration, the first segment of a type in a reference: `{@link X}`, `{@linkplain X}`,
  `{@link #m(P1, P2[])}`, `{@value X#F}`, `@see X`, `@throws X`, `@exception X`, `@uses X`,
  `@provides X`, and in `///` comments from Java 23 on, `[X]` and `[text][X]`. Prose, labels,
  `{@code}`, `{@literal}`, and `{@value X}` without a member aren't uses.
- **Kept:** wildcard imports, module imports, `java.lang` imports, and any import a comment
  belongs to (below).
- **Comments:** a comment belongs to an import when it is inside it, on its line after it, or
  directly above it with no blank line between, and moves with it. A comment that belongs to no
  import, such as a group header followed by a blank line, moves to the top of the section, in
  order, with a blank line after the last (a line break after a doc comment, which stays next to
  the code after it). The comments above the first import stay at the top:
  a license header right above the imports doesn't move into the list. They keep the first
  import, as a comment keeps any import, as long as it stays first. A comment on the line after
  the last import goes with what follows.
- **Pinned:** a section that a disable region overlaps, or that holds a formatter marker or a
  `ferrion-disable*` comment, is left as written, since directives never move. So is one with a
  comment after its last import at the end of the file, which has nothing after it to go with.

```java
package com.example;

// Imports for the cache.
import static java.util.Objects.requireNonNull;

import module java.base;

import com.example.cache.Cache; // The one we wrote.
// Kept for the generated code, though nothing uses it.
import com.example.cache.Unused;
import java.util.List;

class A {
  List<Cache> caches = requireNonNull(List.of());
}
```

### Departures

- **`java.lang` and wildcard imports** stay, used or not: `import java.lang.String;`,
  `import java.lang.*;`, and the package's own `import p.q.*;`, which GJF removes. Removing them
  safely needs type information (SCOPE.md): `java.lang.String` matters if the package declares a
  `String`. Case: `import-java-lang`.
- **An import with a comment** is kept, unused, repeated, or the first import right below a
  comment. GJF removes it, moving the comment to the import before it, and drops a repeated
  import's comment (SCOPE.md: no comment is lost). Case: `import-comment-kept`.
- **A comment belongs to the import below it**, as SCOPE.md says: GJF has a `//` comment belong to
  the import above it, so on imports it sorts, a comment about one import goes with another. On a
  section GJF has sorted, the two agree. Case: `import-comment-above`.
- **Comments GJF rejects:** a header after a blank line, a block comment between imports or
  trailing one, and a comment inside an import make GJF reject the file ("Imports not
  contiguous"). Ferrion moves the header to the top and the others with their imports. Snapshot:
  `imports/floating-comments`.
- **A comment on the line after the last import** goes with what follows, as the comments design
  places it. GJF moves it into the imports, with the import above it. Case:
  `import-comment-after-last`.
- **Directives pin the section:** formatter markers and `ferrion-disable*` comments in the imports
  leave the section as written. GJF sorts across them, which can put `@formatter:on` above
  `@formatter:off` (SCOPE.md: directives never move). Case: `import-directives`.
- **Unicode escapes** in an import, which GJF rejects, are translated to sort and to match uses.
  Snapshot: `imports/escapes`.
- **`///` comments before Java 23** are plain line comments, so their links aren't uses: GJF,
  which knows no Java version, keeps `import u.Md;` for `/// [Md]` in a Java 21 file, and Ferrion
  removes it. Snapshot: `imports/markdown-before-23`.
- **Repeated wildcard and module imports** stay, both copies, which GJF merges (**proposed**).
  SCOPE.md says wildcard and module imports are never removed; a repeated one is as safe to
  remove as a repeated single-type import, so the alternative is to merge them as GJF does and
  amend SCOPE.md. Case: `import-duplicate-on-demand`.

## Statements

Blocks, control flow, switch statements and expressions, `try`, labels, and local declarations.
Each statement starts a line of its own. How an expression inside a statement breaks is the
expression's (see [Expressions](#expressions)); this section says where a statement breaks around
its expressions.

### Blocks

A block's statements go on their own lines at +2, and the `}` on a line of its own. Blank lines
between statements are kept, at most one, and a blank line before the `}` is removed, as GJF
does, except in two places where GJF keeps it: before a `}` the statement goes on after
(`} catch`, `} finally`, `} else`, and a `do`'s `} while`), and after a comment on a line of its
own.

```java
void m() {

  int a = 1;

  int b = 2;
}
try {
  a();

} catch (E e) {
  b(); // why
}
```

What happens to a blank line after the `{`, and to an empty block, depends on what the block
belongs to, as GJF does:

| Block of | Blank line after `{` | Empty block |
| --- | --- | --- |
| a method or constructor | kept | `{}` |
| `while`, `do`, `for`, enhanced `for` | kept | `{}` |
| an `if` without `else` | kept | `{}` |
| an `if` with `else`, or an `else if` | kept | `{` and `}` on two lines |
| `try` without `catch` or `finally` | kept | `{}` |
| `try` with `catch` or `finally`, `catch`, `finally` | kept | `{` and `}` on two lines |
| a lambda, a switch rule | removed | `{}` |
| `synchronized`, an initializer, a label, a block on its own | removed | `{` and `}` on two lines |

```java
while (a) {}
if (a) {
} else {
}
{
}
```

An empty block on two lines keeps a blank line between them, if it has one, when it is one whose
blank line after the `{` is kept.

An empty block with comments prints them on their own lines inside it, except a line comment on
the `{`'s line, which stays there:

```java
while (a) {
  /* nothing yet */
}
while (b) { // nothing yet
}
```

### Bodies of `if`, `else`, and loops

A block follows its header after a space. Any other statement follows after a space if it fits,
and otherwise starts the next line at +2. An empty statement, `;`, always goes on the next line at
+2.

```java
if (a) b();
for (String s : list)
  someMethodWithAVeryLongName(someArgumentWithALongName, anotherArgumentWithALongName);
while (a)
  ;
```

A body that holds a block of its own, such as a `synchronized` statement or an `if` with an `else`,
starts the next line. A comment after the body stays on its line without breaking it
(`if (a) b(); // why`). A line comment after the header ends the line, and the body then starts
the next line at the header's indentation, not +2, as GJF does:

```java
if (a) // why
b();
```

### `if` and `else`

`else` follows the `}` of a block after a space, and starts a line of its own after any other
statement. `else if` stays together, so a chain lines up.

```java
if (a) {
  b();
} else if (c) d();
else {
  e();
}
```

The condition is laid out by its expression, from the `(`, with no break of its own after the `(`.
The same holds for the conditions of `while` and `do` and a `switch`'s selector.

### Loops

`do` puts its `while` after a space when the body is a block, and on the next line otherwise
(`do b();` then `while (a);`).

A `for` header that doesn't fit breaks after each `;`, all together, with the condition and the
update at +4. A missing part leaves its `;` and a space (`for (; ; )`). Initializations that don't
fit go one to a line at +4 from the condition, as declarators do, the first on the type's line.
Updates go as many to a line as fit, continuing at +4 from the first.

```java
for (index = 0,
        otherIndex = 0,
        yetAnotherIndexWithALongName = 0,
        oneMoreIndexWithAVeryLongName = 0;
    index < someCollectionWithALongName.size();
    index++, otherIndex++, yetAnotherIndexWithALongName++, oneMoreIndex++, anotherIndex++,
        theLastIndex++) {
  b();
}
```

An enhanced `for` header that doesn't fit breaks before the variable's name, at +4, when the
type, the name, and the `:` don't fit on the line, as a variable does, and then after the `:`,
at +4, if the rest doesn't fit.

```java
for (Map.Entry<String, SomeVeryLongTypeName> entryWithALongName :
    someMapWithALongName.entrySet()) {
  b();
}
```

### Labels, `break`, `continue`, `return`, `throw`, `yield`

A label goes on a line of its own, and its statement on the next line at the same indentation,
even a block.

```java
outer:
for (; ; ) {
  break outer;
}
```

`return`, `throw`, and `yield` are followed by their expression after a space, and break only
inside it.

### `assert`

When `assert condition : message;` doesn't fit, it breaks before the `:` first, at +4, and then
inside the condition and the message, from +4.

```java
assert someCondition
    : "a message that is too long to fit on the line with the condition and the keyword";
```

### `synchronized`

When the lock doesn't fit, it starts the next line at +4 after the `(`.

### Local variable declarations

Modifiers, the type, and the declarators follow each other after spaces. A single annotation
without arguments stays on the line (`@Nullable String s = f();`). With two or more annotations,
or one with arguments, the annotations before any keyword modifier go on lines of their own.

```java
@Nullable String s = f();
@SuppressWarnings("unchecked")
List<String> list = (List<String>) object;
```

Several declarators that don't fit go one to a line at +4, after their commas. When the type and
the first declarator up to its first possible break (its `=`, or the `;`) don't fit, the line
breaks after the type, at +4; with several declarators, when the type and the whole first
declarator don't fit. That break isn't made in `for` and `try` headers.

```java
int someVariable,
    anotherVariableWithAVeryLongNameThatTakesUpMostOfTheLineAndMore,
    yetAnotherVariable;
final SomeVeryLongTypeName<WithTypeArguments, AndMoreTypeArguments>
    someVeryLongVariableNameXYZW;
```

Local classes, records, enums, and interfaces are statements like any other.

### `try`, `catch`, `finally`

`catch` and `finally` follow the `}` before them after a space. One resource stays on the `try`'s
line; two or more go one to a line at +4, after their `;`, even when they would fit. A `;` after
the last resource is kept, followed by a space.

```java
try (InputStream in = open();
    OutputStream out = create(); ) {
  copy(in, out);
} catch (IOException | RuntimeException e) {
  log(e);
} finally {
  close();
}
```

A `catch` parameter is its modifiers, type, and name after spaces. When it doesn't fit, it breaks
after the `(` first, at +4, and then before the name, at +4 from the type, as a variable does. A
union type stays on the `(`'s line instead and breaks before its `|`s (see
[Union types](#union-types)).

```java
} catch (
    SomeVeryLongExceptionNameThatTakesUpTheWholeLineAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
        exception) {
```

Annotations before a `catch` parameter's or a resource's first keyword modifier stay on the line,
unless one of them has arguments. Then each goes on a line of its own, as GJF does: a `catch`
breaks after its `(` and puts them and the type at +4, and a resource puts the lines after its
first at +4 from it. Before a union type, they stay on the `(`'s line, and the line breaks
after them when the alternatives break.

```java
} catch (
    @SuppressWarnings("unused")
    UnsupportedOperationException e) {
} catch (@SuppressWarnings("unused")
    ReflectiveOperationException
    | RuntimeExceptionWithALongName e) {
}
try (@SuppressWarnings("unused")
    InputStream in = open()) {
```

### `switch`

A switch block's rules or groups go on their own lines at +2. Blank lines between them are kept,
at most one, and blank lines after the `{` and before the `}` are removed. An empty switch block
prints as `{` and `}` on two lines. Switch expressions are laid out as switch statements.

**Groups** (`case …:`): each label goes on a line of its own, and the statements follow at +4 from
the `switch`, on their own lines, with blank lines between them kept and none after the last label.
A block in a group is a statement like any other, so it starts a line.

```java
switch (a) {
  case 1:
  case 2:
    f();
    break;
  default:
    {
      g();
    }
}
```

A label in a group that doesn't fit breaks after each comma, and a guard always starts a line,
both at the label's own indentation, as GJF does:

```java
case SOME_CONSTANT_WITH_A_LONG_NAME,
SOME_OTHER_CONSTANT_WITH_A_LONG_NAME,
YET_ANOTHER_CONSTANT:
  f();
case Integer i
when i > 0:
  g();
```

**Rules** (`case … ->`): a block follows the `->` after a space. An expression or a `throw` follows
after a space if the rule fits, and otherwise starts the next line at +4. That break comes first;
a label that still doesn't fit then breaks after each comma and before its guard, at +4.

```java
case 1 -> f();
case 2 ->
    someObject.someMethodName(argumentNumberOne, argumentNumberTwo, argumentNumberThree);
case SOME_CONSTANT_WITH_A_LONG_NAME,
    SOME_OTHER_CONSTANT_WITH_A_LONG_NAME,
    YET_ANOTHER_CONSTANT_WITH_A_LONG_NAME ->
    g();
case 3 -> {
  h();
}
```

### Comments in statements

Comments follow the general rules. A block comment on the line of the code before a statement,
or before a rule or group of a switch, stays on that line, and the statement starts the next one,
as GJF does. After a `{`, the comment goes on a line of its own instead.

```java
a(); /* why */
b();
{
  /* why */
  c();
}
```

In a switch block, a comment above a label goes at the label's indentation, and a comment after
the last rule or statement, before the `}`, goes at +4 from the `switch`, where the statements of
a group go, as GJF does.

```java
switch (a) {
  // before the first case
  case 1 -> f();
    // after the last rule
}
```

## Expressions

Operators, assignment, calls and chains, lambdas and method references, `?:`, object creation,
array initializers, casts, `instanceof`, and patterns.

Most constructs are a group that stays on one line if it fits, and otherwise breaks, with its
continuation lines at +4. Groups break outermost first: `x = a + b * c` breaks after the `=`, then
before every `+`, and only then inside `b * c`. A line comment ends its line, which breaks the
groups around that line break, but not the ones that end at the comment.

### Spacing

One space around binary operators, `?`, `:`, `->`, and assignment operators, and after a cast's
`)`; none around `.`, `::`, and unary operators, and inside parentheses and brackets. Two unary
operators that would run together keep a space: `- -a`.

```java
x = a + b * c / d % e - f << g >> h >>> i;
x = (int) a + (long) (b) + - -c + ~d;
```

### Binary operators

A run of operators of one precedence stays on one line, or breaks before every operator, all at
once, with the operators at +4 from the first operand. A run of higher precedence in an operand is
a group of its own, with its own +4. String concatenation is the same.

```java
x =
    firstOperandValue
        + secondOperandValue * thirdOperandValue
        - fourthOperandValue
        + fifthOperandValue;
if (firstConditionIsTrueAAAAAAA && secondConditionIsTrueBBBBBBBBB
    || thirdConditionIsTrueCCCCCCC && fourthDDDD) {}
```

When every operand of the run is at most 9 wide, as for arguments, the operands fill instead:
as many on a line as fit, each line after the first at +4.

```java
String label =
    (p.text() + " " + p.text() + " " + p.text() + " " + p.text() + " " + p.text() + "-b "
        + p.text() + "-b " + p.text() + "-b " + p.text() + "-b");
```

### `?:`

On one line, or the condition, then `? a` and `: b` on lines of their own at +4. A `?:` in a
branch is a group of its own, indented +4 from the one around it.

```java
x =
    someConditionHolds
        ? someVeryLongValueWhenTrueAAAAAAAAAAAAAA
        : someVeryLongValueWhenFalseBBBBBBBBBB;
```

### Assignment, casts, `instanceof`

An assignment breaks after its operator, a cast after its `)`, and `instanceof` before the
keyword, each with what follows at +4. A record pattern that doesn't fit after `instanceof` starts
a line of its own there, and its parts go one per line like arguments.

```java
this.someVariableNameAAAAAAAAAAAAAAAAAAAAAA =
    someOtherValueBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB.someMethod(
        xxxxx, yyyyyyyyyyyyyyyyyyyyy, zzzzzzzzzzzzzzzzzzzz);
x =
    (SomeLongTypeName)
        someOtherValueBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB.someMethod(xxxxx);
x =
    aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
        instanceof Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb;
```

### Arguments

All on the call's line; otherwise all on the next line at +4; otherwise one per line. When every
argument is at most 9 wide, they fill instead: as many on a line as fit. The `)` follows the last
argument. A text block as an argument breaks the list, so it starts a line of its own at +4.
Arguments whose input has them in pairs, a pair a line, keep the pairs, as an array initializer
keeps rows of two (see [Arrays](#arrays)), but with no argument left over.

```java
x =
    Map.of(
        "magenta", HIGHLIGHT_MAGENTA,
        "teal", HIGHLIGHT_TEAL);
```

A first argument that is a format string, a string literal or text block with a `%` in it, or a
concatenation of only string literals one of which has a `%`, goes on a line of its own when the
arguments break, and the arguments after it are a list of their own on the next lines: all on one
line, filled, or one per line. Any method's arguments, or a constructor's, are treated alike;
`"%s" + x` and a `{}` placeholder are not format strings.

```java
throw new StorageExhaustedException(
    "Ran out of memory while growing the slot table: from %,d to %,d",
    e, this.size + 1, slotCount);
```

```java
foobarbaz(
    argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeC, d);
foo(
    argumentNumberOneAAAAAAAAAAA,
    argumentNumberTwoBBBBBBBBBBBBB,
    argumentNumberThreeCCCCCCC,
    dd);
foo(
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28);
```

Comments: a block comment in empty parentheses keeps a space before the `)`, `foo(/* none */ )`.
A block comment on the line of a comma stays after the comma when the list breaks, except one
naming a parameter, `/* name= */`, which stays with the argument after it. A blank line after a
comment on a comma's line is removed, unless another comment follows it. A comment on a line of
its own before any argument but the first puts them one per line, even when they'd fill. A
comment on a line of its own before the first argument doesn't break the arguments onto lines of
their own.

```java
foo(
    argumentNumberOneAAAAAAAAAAA, /* comment */
    argumentNumberTwoBBBBBBBBBBBBB,
    argumentNumberThreeCCCCCCC,
    dd);
remap(
    key,
    valueAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,
    /* hints= */ null,
    /* computeIfAbsent= */ true);
```

### Method chains

A chain with two or more calls, or one call that isn't last, stays on one line, or breaks before
each `.` after its prefix, all at once, at +4. The prefix is the root and the first link when the
root is a name of up to four characters, `this`, or `super`, or a type's name, possibly
qualified, and the link after it; otherwise it is the root alone. A name is a type's if it starts
with an uppercase letter and has a lowercase one, so `FOO_BAR` is not. When the chain has a
`.stream()` or `.parallelStream()` link, the prefix is everything up to it if that fits on the
line, and otherwise everything up to it breaks before each `.` too. A chain whose only call has
fields after it has the call in its prefix, `explorer.lookup(query, 10)` then `.hitsFound`; an
array access after a chain's last call counts as a field, so `a.b(x).c()[0]` keeps `a.b(x)`. Both
of these prefixes are measured up to the call's `(`: when the line up to there fits, the call's
arguments break before its dot does.

```java
builder
    .append(argumentNumberOneAAAAAAAAAAA)
    .append(argumentNumberTwoBBBBBBBBBBBBB)
    .append(cccccc);
abcd.append(argumentNumberOneAAAAAAAAAAA)
    .append(argumentNumberTwoBBBBBBBBBBBBB)
    .append(ccccccccc);
ImmutableList.<String>builder()
    .add(argumentNumberOneAAAAAAAAAAA)
    .add(argumentNumberTwoBBBBBBBBBBBBB)
    .build();
someVeryLongName.entrySet().stream()
    .sorted(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)
    .forEach(bbbbbbbbbbbbbbbbbbbbbbbbbb);
result =
    lookup.lookup(
            span("name", "a", "z", T, T, RangeSelection.INCLUSIVE_BOUNDS_FAST_ITERATOR), 1000)
        .hitsFound;
```

A chain with one call, last, breaks its arguments first, and its dots fill: each breaks only if
what follows it up to the next dot doesn't fit. `new Foo().bar(x)`, `"a".concat(x)`, and a
parenthesized or cast root break like longer chains.

```java
foo.bar.baz.qux(
    argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeCC);
a.b.c.d.eeeeeeeeeeeeeeeeeeeeeeeeeeeeeee.ffffffffffffffffffffffffffffff
    .gggggggggggggggggggggggggg.hhhhhhhhhhhhh();
new Foo()
    .bar(argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentNumberThreeCCCC);
```

On such a root, a chain with a `.stream()` link puts what follows the root on the next line at
+4 when it doesn't fit, as a group of its own: on one line, or the links up to the `.stream()`
on that line and the ones after it at a further +4.

```java
values =
    ((Map<?, ?>) m0)
        .entrySet().stream().flatMap(e -> Stream.of(e.getKey(), e.getValue())).toList();
weight7 =
    ((CompoundRule) kit.find("body", new StringReader("kotlin|20 version|3")))
        .members().stream()
            .map(CompoundEntry::scope)
            .mapToDouble(WeightRule::getScale)
            .sum();
```

The exception is a field after a type's name: when the dot before the field breaks, so does the
one after it.

```java
RepositoryQueryMethodBenchmarks_BenchmarkParameters_jmhTypeAAAAAAAAAAAAAAAAAAAA
    .setupIterationMutexUpdater
    .get(l_benchmarkparameters1_G);
```

A call's type arguments stay with its dot. When `.<A, B>name(` doesn't fit, the name goes on the
next line at +4, with its arguments at +4 from it, and type arguments that don't fit on their line
fill at +4:

```java
legs =
    this
        .<ShipmentRoutingRegressionIT.LegRecord, QShipmentRoutingRegressionIT_LegRecord>
            createList(
                "legs",
                ShipmentRoutingRegressionIT.LegRecord.class,
                QShipmentRoutingRegressionIT_LegRecord.class,
                PathInits.DIRECT2);
```

In a chain that breaks, the root's and the prefix's arguments are at +8, and a call of a short
name, up to four characters, at the root keeps its only argument on its line when that argument
fits on no line of its own:

```java
when(remoteClient.dispatch(
        anyString(), eq(TaskStatus.DONE), any(JobRequest.class), eq(String.class)))
    .thenReturn(cannedResponse);
```

### Lambdas

A lambda that is a call's only argument is hugged: `foo(x -> {` stays on the call's line if that
fits, with the block indented from the statement, and otherwise the lambda goes on the next line
at +4. A lambda with an expression body that doesn't fit on the call's line goes on the next line
if it fits there, and otherwise stays on the call's line and breaks after its arrow, with the body
at +4. The calls in a chain's prefix hug only a lambda with an expression body, and the block's
line breaks still break the groups around the call, such as an assignment's.

```java
executor.submit(() -> {
  doSomething();
});
foo(
    a,
    b,
    x -> {
      doSomething();
    });
list.forEach(
    x -> System.out.println(xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx));
list.forEach(x ->
    System.out.println(
        xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx, yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy));
x =
    foo(a -> {
      run();
    });
gen.withArgument(configuredRetryRule ->
        configuredRetryRule.name("retryRuleArg").type(TypeSpec.builder()))
    .appendCode("this.retrier = retryRuleArg.orElseGet(")
    .appendCode(generatedType);
```

Elsewhere, an expression body is on the arrow's line, or on the next line at +4.

Parameters in parentheses stay on one line if `(params) ->` fits there, and the body breaks
first. Otherwise they go one per line at +4, the first after the `(`, and an expression body stays
on the line of the arrow if it fits there.

```java
Selector selector =
    (ctx, preamble, cookies, outputBuffer, threadId, localOverrides, peerParameters, mapper) ->
        new Answer(true, handler);
foo.bar()
    .call(
        (ScopedReleaser keeper,
            FakeResources v1,
            FakeResources v2,
            String v3,
            String v4,
            String v5) -> {
          keeper.close();
        });
```

### Method references

On one line, or a line break before the `::` at +4.

### Object creation

`new` and its arguments are formatted as a call's, and an anonymous class body follows the `)`
after a space, indented from the line the `new` expression starts on.

```java
new Foo(
    argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBBBBBBBB, argumentNumberThreeC) {
  void run() {}
};
```

A chain on an anonymous class keeps its first link on the `}`, and the rest of the chain breaks
as it would after a prefix, at +4 from the line the `new` starts on:

```java
new NameShuffler(comparator) {
  void swap(int i, int j) {}
}.sort(0, length);
new Sorter(x) {
  void get() {}
}.sort(argumentNumberOneAAAAAAAAAAAAAAAAAAA)
    .then(argumentNumberTwoBBBBBBBBBBBBBBBBBB)
    .then(ccccccccccccc);
```

### Arrays

An array initializer stays on one line, or puts its elements on lines of their own at +2, with
the `}` on a line of its own: all on one line if they fit there, filled when every element is at
most 9 wide, and otherwise one per line. A trailing comma keeps the elements off the braces'
lines. Dimensions follow the type, `new int[3][]`, and an array access that doesn't fit breaks
after its `[`, with the index at +4.

An initializer whose input has its elements in rows keeps them, one row a line, even when they'd
fit on one, and each row that doesn't fit breaks at +4 before each element that doesn't fit. The
rows are the input's lines, starting at the first element's column, and are kept when:

- no line starts left of the first element, which can follow the `{` on its line. The columns
  are GJF's: a tab counts one, a comment before the first element is its start, and an operator
  before it on its line counts the square of its length, so `->` counts 4 and `>>>` 9;
- every row has as many elements as the first, except that the last can have fewer, and at
  least two rows have them all;
- the first row has at least two elements other than arrays, counting an array's elements
  instead of the array, so a row of `{0, 1}` counts but one of `{1}` doesn't;
- the rows' first elements are all of one kind of expression, and so are more than half of each
  other column's. Kinds are javac's: `1` and `-1` are alike, as are `a` and `!a`, `a.b` and
  `A.class`, or two calls, but `a + b` and `a - b` aren't, nor are `1` and `1L`, `a` and `(a)`,
  or `"a"` and `null`.

```java
static final String[] data = {
  "apple", "banana", "avocado", "raspberry", "grape",
  "lemon", "mango", "mandarin", "quince", "guava",
  "melon"
};
int[][] values = {
  {0, 0, 0},
  {0, 10, 0}
};
String[] e = {
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cccc", "dddd",
  "eeee", "ffff", "gggg", "hhhh"
};
```

```java
x =
    new int[] {
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
      26, 27
    };
x =
    new String[] {
      "aaaaaaaaaaaaaaaaaa",
      "bbbbbbbbbbbbbbbbbbbbbbbbb",
      "cccccccccccccccccccccccccc",
      "ddddddddddddddd"
    };
x =
    new int[] {
      1, 2,
    };
```

### Departures

- **Text blocks** aren't re-indented: their content and closing delimiter are copied as written
  (SCOPE.md). The opening `"""` is placed as GJF places it, on a line of its own. Case:
  `text-block`.
- **Deep indentation:** past 100 nested indents of the expression rules, a line starts at the
  indentation of the one before it instead of +4 from it. GJF indents each `?:` in an else
  branch, each cast, and each assignment +4 from the one around it, and the parser takes chains of
  them at any length, so the output grows with the square of the input: an 800 KB chain of `?:`
  prints 40 GB. Past 100 indents a line starts beyond column 400, so no code written by hand is
  affected. Case: `deep-indentation`.

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

`A & B` in a cast fills its lines, breaking before an `&` at +4. The type is in the cast's own
+4, so those lines are at +8 from the `(`'s, as are a cast type's other breaks.

```java
Object p = (Runnable & java.io.Serializable) () -> {};
```

### Union types

`A | B` in a `catch` is on one line if it fits, and otherwise has one alternative per line at +4,
each starting with its `|`. The last alternative and the parameter's name break as a variable's
type and name do: the name on a line of its own at +8, and the alternative's own breaks at +4
from the others'.

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
