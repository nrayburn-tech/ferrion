# Types oracle normalization

B3's exit criterion in [ROADMAP.md](../ROADMAP.md) says chosen methods, attributed types, constant
values, variable finality, and switch errors in the corpus match javac's, except for a documented,
categorized list of disagreements, with code where javac reports errors excluded. `cargo oracle
types` (`tools/oracle-check/src/types`) checks it, with javac's side from `javac-oracle types`
(`tools/javac-oracle`, `Types.java`). This document records what the oracle reports, the textual
forms of types and constants that both sides write, how javac's entries are matched with
Ferrion's, and what javac does that has no counterpart in Ferrion.

`javac-oracle types` compiles a source set exactly as `javac-oracle names` does (see
[the names oracle](semantic-oracle-normalization.md#the-compilation)): the same command line,
processors, source path, crash handling, and keys. Everything about javac here was learned by
running the oracle on small inputs written for the purpose, on Temurin 27.0.1 unless a JDK is
named, and on the corpus.

## The oracle's output

One JSON object per line, as `names` writes them: one per file but `module-info.java`, one per
file a processor generated (`generated: 1`), one per `module-info.java` (`module: 1` and
`skipped`, since a module declaration has no expressions), a `relations` object (see
[Type relations](#type-relations)), and the last `sourceSet` object, as `names` writes it. Paths
are relative to the corpus directory. Each file has:

- `diagnostics`, `lombok`, `unanalyzed`, and `crash`, as `names` writes them;
- `analyzeMicros`: the time javac spent analyzing the file's classes (see
  [Analysis time](#analysis-time));
- `types`: every type string the file's entries use, each once, in order of first use; the
  entries below refer to a type by its index in this list;
- `expressions`: `[start, end, kind, type]` for each expression tree (see
  [Expressions](#expressions)), in the pre-order of `TreePathScanner`; `kind` is javac's
  `Tree.Kind`;
- `constants`: `[start, end, value]` for each expression with a constant value (see
  [Constants](#constant-values));
- `calls`: `[start, end, kind, method, type]` for each method invocation, explicit or implicit
  constructor call, class instance creation, and method reference: the key of the method or
  constructor javac chose, as `names` keys it (`method java/util/List.of(Ljava/lang/Object;)Ljava/util/List;`),
  and the index of its type as invoked, or -1 where javac doesn't expose it (see [Calls](#calls));
- `variables`: `[offset, kind, finality, type]` for each local variable and parameter: the offset
  its `local` key has, javac's `ElementKind`, `final`, `effectively-final`, or `mutable`, and its
  type (see [Variables](#variables)).

Offsets are UTF-16 code units, as in `names`. An entry javac made up (an implicit `super()`) has
end -1.

A small example, for

```java
class A<T> {
  java.util.List<? extends Number> xs;
  long f(final int a) { var n = xs.get(0); return a + 1L; }
}
```

```json
{"file":"projects/p/src/A.java","diagnostics":[],"lombok":[],"analyzeMicros":812,
 "types":["void","java/util/List<? extends java/lang/Number>",
          "capture#1 extends java/lang/Number","int","long","(int)capture#1 extends java/lang/Number",
          "java/lang/Number"],
 "expressions":[[52,54,"IDENTIFIER",1],[52,61,"METHOD_INVOCATION",2],[59,60,"INT_LITERAL",3],
                [70,76,"PLUS",4],[70,71,"IDENTIFIER",3],[74,76,"LONG_LITERAL",4]],
 "constants":[[59,60,"int 0"],[74,76,"long 1"]],
 "calls":[[9,-1,"METHOD_INVOCATION","method java/lang/Object.<init>()V",0],
          [52,61,"METHOD_INVOCATION","method java/util/List.get(I)Ljava/lang/Object;",5]],
 "variables":[[42,"PARAMETER","final",3],[52,"LOCAL_VARIABLE","effectively-final",6]]}
```

(Offsets here are illustrative; the oracle's are javac's.)

## Type strings

A type string is a canonical text for a type, which Ferrion's side writes byte for byte from its
own types. It is built on the names oracle's keys, and is unambiguous to parse, so a type the
oracle gives (a supertype, a capture) can be rebuilt on Ferrion's side.

| Type | String |
| --- | --- |
| Primitive, `void` | `int`, `boolean`, `void` |
| Class or interface, not parameterized | its binary name with `/`: `java/lang/String`, `java/util/Map$Entry` |
| Raw type | the same: `java/util/List` (a generic class with no arguments is raw) |
| Parameterized | `java/util/Map<java/lang/String,java/lang/Integer>`, no spaces after commas |
| Member of a parameterized type | the enclosing type's string, `$`, the simple name, its arguments: `p/Outer<java/lang/String>$Inner<java/lang/Integer>` |
| Local or anonymous class, or a member of one | `local <file>:<offset>`, its key as an owner in `names`, then its arguments |
| Type variable | `<name>:<owner>`: `E:java/util/List`, `T:java/util/Collections.emptyList()Ljava/util/List;`, `T:p/A.<init>(Ljava/lang/Object;)V` |
| Captured type variable | `capture#<n>` and, at its first appearance in the string, ` extends <upper>` and ` super <lower>` when it has a lower bound: `capture#1 extends java/lang/Number` |
| Wildcard | `?`, `? extends java/lang/Number`, `? super java/lang/Integer` |
| Array | the component and `[]`: `int[][]`, `java/lang/String[]` |
| Intersection | the bounds in javac's order, joined by `&`: `java/lang/Number&java/lang/Comparable<?>` |
| Union (a multi-catch parameter's alternatives) | joined by `\|`: `java/io/IOException\|java/lang/RuntimeException` |
| Null | `null` |
| Method type | `<T:owner,...>` when generic, `(P1,P2)R`, and ` throws X,Y` when it throws: `(int)java/lang/Object` |
| No type | `none` |
| Error | `error <javac's text for it>` |

The rules behind the table:

- **Class names** are binary names, as `names` keys write them: the package with `/`, nested
  classes with `$`. Where a type came from (`platform`, `source <file>`) isn't in the string: the
  names comparison checks that, and it would make every string long.
- **A member type's enclosing type** is written only when it, or a type enclosing it, has type
  arguments (`DeclaredType.getEnclosingType`); otherwise the binary name says it all. A local
  class's enclosing type is never written.
- **A type variable** is its name and, after `:`, its declaration's owner: a class's binary name
  (or `local <file>:<offset>`), or a method's or constructor's `names` key without the word
  `method`. Its bounds are not written; they are the declaration's.
- **Captured type variables** are numbered from 1 in the order they first appear in the string,
  reading left to right; the same capture appearing again is `capture#<n>` alone, so a capture
  whose bound mentions itself (`capture#1 extends java/lang/Enum<capture#1>`) is finite. The
  bounds written are the capture's own (JLS 5.1.10: the glb of the wildcard's bound and the type
  parameter's), not the wildcard's, so they check capture conversion and glb together. Numbers
  are per string: two strings can both have a `capture#1` that are different captures.
- **Intersections** keep javac's order of bounds, which for a type variable's bound is the
  declaration's, and for a least upper bound is javac's own (see [Observations](#observations)).
  The comparison counts a mismatch that is only an order apart under its own category.
- **Type annotations** are not written.
- **Spaces** appear only in ` extends `, ` super `, ` throws `, and inside the `local` and `error`
  forms.

## Constant values

A constant is `<type> <value>`, the type being the expression's (`byte`, `short`, `char`, `int`,
`long`, `float`, `double`, `boolean`, `String`):

| Type | Value | Examples |
| --- | --- | --- |
| `byte`, `short`, `int`, `long` | decimal, with `-` when negative | `int 32`, `byte 44`, `long -9223372036854775808` |
| `float`, `double` | Java's `Float.toHexString` and `Double.toHexString` | `float 0x1.555556p-2`, `double -0x0.0p0`, `double NaN`, `double Infinity`, `double -Infinity` |
| `boolean` | `true` or `false` | `boolean true` |
| `char` | the character, quoted and escaped as a string is | `char "A"`, `char "\u000a"` |
| `String` | quoted, escaped | `String "a32c1.5"`, `String "tab\u0009hereé😀"` |

- **Hexadecimal floating point** is exact, and has one form per value: `0x1.` then the
  significand's hexadecimal digits without trailing zeros (at least one digit), `p`, and the
  binary exponent in decimal; a subnormal is `0x0.` with the exponent of the smallest normal
  (`p-126` for `float`, `p-1022` for `double`); zero is `0x0.0p0`, with `-` for negative zero.
- **Escaping:** printable ASCII (U+0020 to U+007E) is written as it is, but `"` as `\"` and `\` as
  `\\`; every other UTF-16 code unit is `\u` and four lowercase hexadecimal digits, so a character
  outside the BMP is two escapes, and a lone surrogate, which a Java string can hold, one. The
  string is all ASCII, whatever the source's encoding.

## Variables

Each local variable, parameter (of a method, constructor, lambda, or `catch`), resource, and
pattern binding, keyed by the offset its `local` key has in `names`, with its type and:

- `final`: declared `final`, or implicitly final: a resource, a multi-catch parameter (JLS 14.20,
  14.20.3);
- `effectively-final`: not final, and effectively final by JLS 4.12.4;
- `mutable`: neither.

## Expressions

## Calls

## Type relations

## Analysis time

## Observations
