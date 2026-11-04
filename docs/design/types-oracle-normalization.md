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
named, and on the corpus; the [observations](#observations) list what each input showed.

Two of the oracle's values aren't in javac's public API (`javax.lang.model`, `com.sun.source`),
and come from two of javac's internal methods, as `names` takes a tree's preferred position from
one: a constant's value (`Type.constValue()` of the expression's type), and whether a variable is
effectively final (the `EFFECTIVELY_FINAL` flag of its symbol, found by name, since its bit isn't
the same on every JDK). What both give was checked against the JLS on the inputs of observations
11 and 12, and is the same on JDKs 21, 25, 26, and 27.

## The oracle's output

One JSON object per line, in this order: one per file but `module-info.java`, one per file a
processor generated (`generated: 1`), a `relations` object (see [Type relations](#type-relations)),
one per `module-info.java` (`module: 1` and `skipped`, since a module declaration has no
expressions), and the last `sourceSet` object, as `names` writes it. Paths are relative to the
corpus directory. Each file has:

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

Every tree javac's API calls an expression, with its range and `Trees.getTypeMirror`, but:

- **names of packages, types, type variables, and methods** (an identifier or member select whose
  element is one): `Collections` in `Collections.emptyList()`, `String` in `String::length`, the
  method name `m` in `a.m()`. Their resolution is the names comparison's; a call's method is in
  [Calls](#calls). `this`, `super`, `A.class`, and an array's `length` are expressions;
- **type trees** (primitive, array, parameterized, wildcard, union, intersection, annotated, and
  `var`), annotations, and an annotation's `name =`; the annotation's values are expressions;
- **package declarations, imports, and module declarations**;
- **trees javac made up**, with no end (the default constructor's `super()`, an enum constant's
  `new`): they have no source to type. Their calls are in [Calls](#calls).

A tree that is both an expression and a constant is in both lists. Ranges are javac's, as in
[names](semantic-oracle-normalization.md#javacs-ranges); two observed oddities: an enum constant
with a body has a `NEW_CLASS` whose range is the body alone (`{ }`), and a parenthesized `switch`
selector is a `PARENTHESIZED` tree of its own.

The type is what javac attributed: a poly expression's type is the one its context gave
(`Function<String,Integer>` for `String::length` assigned to one), a lambda's and a method
reference's is its target type, and a conditional or `switch` expression's is javac's least upper
bound where the operands need one (observations [1 to 6](#observations)).

## Calls

For each `METHOD_INVOCATION` (explicit `this(...)` and `super(...)` included), `NEW_CLASS`, and
`MEMBER_REFERENCE`, the method or constructor `Trees.getElement` gives, keyed as `names` keys it:
the declaring class's binary name, the name, and the erased descriptor. That is the method javac
chose, not the type searched (which `names` compares).

- **The type as invoked** is the method select's `Trees.getTypeMirror`: an `ExecutableType` with
  the inferred type arguments substituted, `(java/lang/Integer[])java/util/List<java/lang/Integer>`
  for `Arrays.asList(1, 2)`. javac's public API gives no type for a `NEW_CLASS`'s constructor or a
  method reference's method, nor the inferred type arguments themselves, nor which phase of
  JLS 15.12.2 found the method (whether a variable arity call was made in the third); those are
  -1, and are seen only through the expression's type.
- **Calls javac made up** have end -1: a constructor's implicit `super()`, at its body's `{`; the
  default constructor's, at the class's keyword (`class`, `enum`, `record`) after its modifiers;
  an anonymous class's, at its body's `{`; and an enum constant's `new`, at the constant's name.
  Ferrion's side matches them by their start alone.
- **An anonymous class's creation** chooses the anonymous class's own constructor (`method local
  <file>:<offset>.<init>(...)V`, its parameters the superclass constructor's, as JLS 15.9.5.1 has
  it); the superclass constructor chosen is the implicit `super(...)` call inside it, at the body's
  `{`, which is the anonymous class's key.

## Type relations

After a source set's files, one `relations` object: javac's answers, through
`javax.lang.model.util.Types` (and `Elements.overrides`), about a sample of the types the source
set declares, so that B3a's relations can be checked before expressions are typed.

- **Sites:** every class's superclass and interfaces, field types, parameter types, and return
  types, in the source set's files and in the classes javac declares implicitly (a record's
  accessors, `toString`): `type p/A source projects/... superclass`, `type p/A ... interface 0`,
  `field p/A.f type`, `method p/A.m(I)V parameter 0`, `method p/A.m(I)V return`, a names key and
  a role. Anonymous classes, and types with an error type in them, are left out.
- **The sample:** the first `--relation-types` (48 by default) distinct type strings, ordered by
  FNV-1a of the string, then the string, then the site; each with its first site in that order.
  The same source set gives the same sample on every run and JDK. `sites` lists them:
  `[site, type]`.
- **Queries,** with types as indices into the object's own `types` list: for each sampled type
  `erasure`, `capture`, and `supertypes` (`directSupertypes`), and `subtype` and `assignable`
  between it and each direct supertype, both ways; for each ordered pair of sampled types
  `subtype`, `assignable`, and `same`; for each sampled declared type, `member` (`asMemberOf`) for
  three of its class's members (by FNV-1a of their keys), and for four of its class's own methods
  and each method of the same name in its supertypes, `subsignature` (of both as members of the
  type) and `overrides`. A yes-or-no answer is `true`, `false`, or `"error"` when javac threw; a
  type answer is -1 when it threw.
- **What javac can't read:** each sampled type's own queries (its unary ones and those with its
  supertypes, and its members') are asked as a block, and a block javac throws on is one
  `["threw", type, exception]` instead, which isn't compared. In the corpus that is a dependency's
  class file javac can't read: `BadClassFile` for `RateLimiter$Metrics` in resilience4j 1.7.1's
  jar, which jedis's tests have on their classpath, thrown while completing a supertype. Before
  blocks, it lost a quarter of the corpus's source sets' relations. Anything else that throws
  gives the object an `error` instead of its queries.

48 types make about 7,600 queries per source set (jedis: 15,204 for two). `lub` isn't in the API;
the types of conditional expressions and `var` variables in the files give it.

## Analysis time

`analyzeMicros` is the time between javac's `ANALYZE` task events for the file's classes. javac
27 starts every class's event (attribution) and then finishes each (flow analysis), in the same
order ([observation 19](#observations)); other policies do both for a class before the next. So
the time from an event to the next is charged to the class whose event started it if that is a
start, else to the class whose event ends it if that is a finish; time between a finish and a
start is no class's. Under javac 27's order, the last class's attribution and the first's flow
analysis can't be told apart, and both are charged to the last class. The time includes what
javac does lazily for the class: completing classes from the classpath and reading files on the
source path. Each source set runs in a JVM of its own, which is warming up (loading javac's classes
and compiling them) while it analyzes its first classes, so those files' times are inflated; the
distribution over the corpus is in [The corpus](#the-corpus).

## What is compared

For each source set, as in `names`:

1. **Files left out:** files with a language error (switch errors among them: the report counts
   files whose only errors are switch errors), files javac never analyzed or threw on, and module
   declarations, each counted. Entries inside Lombok-annotated classes are left out, until B4.
2. **Matching:** each javac entry with Ferrion's at the same range in UTF-16 units (Ferrion's
   byte offsets converted as in `names`), an implicit call by its start, a variable by its
   offset, a relation query by its index. Values are compared as strings.
3. **Categories:** `missing` (Ferrion has nothing there), `order` (intersections with the same
   parts in another order), `type`, `value` (a constant), `method` (another method chosen),
   `instantiation` (the same method, another type as invoked), `finality`, and `answer` (a
   relation). A Ferrion value starting with `left-to-` (`left-to-b3c inference`) is a later
   milestone's, and counted apart, per aspect; `waiting-for-calls` is one that waits for overload
   resolution, counted apart under its own label until W3's calls land. So is a constant javac
   has at an expression with such a value, which can't have one yet. A mismatch listed in
   `tools/oracle-check/differences/types-known.txt` (`<category> <file> <aspect> <start>-<end>`,
   and for a relation query `<category> <source set> relation <i>-<i>`) counts under its known
   category. An entry Ferrion has where javac has none is `extra`, and fails the run as a mismatch
   does.
4. **Relation queries left out:** a query about a type javac couldn't resolve, where javac's
   answer or one of the query's types has an `error` type in it, or a method key whose
   descriptor names a class with dots (`builder()Lcom.google.Foo.Builder;`, javac's erasure of a
   class it couldn't resolve), is counted apart, not compared: the class is one a jar names that
   javac's compilation lacks, and that Ferrion's view has
   ([W1's observation 12](semantic-type-relations.md#observations)). A query that needs a class or
   member that a Lombok-annotated class's annotations make, which Ferrion has from B4, is Ferrion's
   `left-to-b4 lombok`.

Ferrion's side is the `Typer` trait in `tools/oracle-check/src/types/compare.rs`. For the relation
queries it is `tools/oracle-check/src/types/ferrion.rs`, over B3a's relations
([type relations](semantic-type-relations.md#checking)), and for the files' entries
`tools/oracle-check/src/types/attribution.rs`, over B3b's attribution
([below](#ferrions-side-of-the-files-entries)). `--relations` compares the relation queries only,
for B3a. Ferrion's type strings name local classes as javac's keys do, with the file relative to
the corpus and UTF-16 offsets, as `names`' `oracle_key` converts keys.

## Ferrion's side of the files' entries

The adapter gives, for each file of a source set, its attribution at the body fuel B3b runs with,
each node's type and constant in the oracle's forms, at javac's range. Most nodes are a tree of
javac's at the same range. What isn't, each observed (the [observations](#observations) from 21 on)
and checked together by `tools/oracle-check/types-cases/shapes.java`, whose `shapes.jsonl` is the
oracle's output on it:

| javac has | Ferrion has | The adapter |
| --- | --- | --- |
| `PARENTHESIZED` around the condition of `if`, `while`, `do`, `switch`, and `synchronized`, a `switch` expression's selector too, with the expression's type and constant | Two tokens of the statement | Adds an entry at the parentheses' range, from the expression inside |
| One `STRING_LITERAL` over a run of adjacent string literals in a chain of `+` ([which chains](parser-oracle-normalization.md#folded-strings)), with the joined value | A literal per string, and a `+` per operand | Adds the run, and leaves out its literals and the `+`s before its last one, and the `+` of a first run. Only constants of what remains are printed |
| `INT_LITERAL` or `LONG_LITERAL` over `-` and a decimal literal | `PrefixExpr` and a literal | Leaves out the literal |
| `NEW_CLASS` for an enum constant with arguments or a body: from its arguments, or body if it has none, through its body, or its arguments if it has none; of the anonymous class's type with a body, else the enum's | No expression | Adds it, and its call (W3's) at that range |
| `NEW_ARRAY` for `new int[] {1}` as one tree | The creation and its initializer | Leaves out the initializer. A nested `{1}`, or a variable's `int[] a = {1}`, is a tree of javac's |
| A resource that is a variable through its `;`, if one follows | The variable | Extends the range, as `names` does |
| A compact constructor's parameter, at the [preferred position of the component's type](semantic-oracle-normalization.md#keys) | The record component | A component is a variable only for a record with a compact constructor |
| A `PARAMETER` for each of an implicit constructor's, a record's at its keyword and an anonymous class's at its `local` key's offset, several at one offset | Nothing | Lists those offsets; javac's parameters there are counted apart, never compared |
| No tree for an annotation on a record component that doesn't apply to fields, by `@Target` ([observation 24](#observations)) | The annotation's values | Lists the ranges of annotations on components; an entry in one that javac lacks is counted apart |

Ferrion's types print as the oracle's, and `local <path>:<byte offset>` in any of them is converted
as above. Until B3d, a variable's finality is `left-to-b3d finality`. A call with no chosen method
is `waiting-for-calls` or `left-to-b3c inference`, by what its type says, and javac's implicit calls
all wait, until W3 sets `IMPLICIT_CALLS_WAIT`.

W3's chosen methods are read from `BodyTypes::calls` by node (`call_position` in the adapter): a
method invocation, an explicit constructor call, or a creation at its range; a call recorded at a
class body or an enum constant without a body but with arguments at the [enum creation
range](#calls); javac's made-up calls by their start alone, recorded at a constructor or its block
(its `{`), an anonymous class's body (its `{`), a class without a constructor (its keyword), or an
enum constant (its name).

## Generated text that changes

An annotation processor can write different text from the same inputs. Helidon's
`__ServiceDescriptor`s do: run after run of `javac-oracle types` on `common/mapper/main`, with one
scratch `--generated` and nothing else changed, `MappersFactory__ServiceDescriptor.java` came out one
of two ways, 4 runs of 8 each, its `RTYPE_2` and `RTYPE_3` swapped and sets reordered accordingly.
Across the cached runs, the same input hash has more than one shape of trees for 8 of helidon's
generated files, all `__ServiceDescriptor`s, and none outside helidon.

A cached output stays in use while the files it lists exist, so a later run of either command that
writes the same directory can leave another text under a cached run: the input hash covers a source
set's own files and its dependencies' generated files, not those it generates itself. So does a
miss in one source set: it rewrites its own directory, and if the text comes out otherwise, its
dependents' hashes change, and they miss too, the next time either command runs. The cache's own
times show it happening: `types` ran on 2 November; `names` ran under a new cache key on 3 November
(18:47 to 19:05), and every generated file's time is from then or later; `types` ran again for 262 of
helidon's source sets (19:21 to 19:28). What W2's rough comparison counted as 174 expressions with no
Ferrion expression was the `__ServiceDescriptor`s between those runs. W4's own `cargo oracle names`
then missed for 81 of helidon's source sets, and `cargo oracle types --relations` for 4.
A run of the oracle that writes the text on disk gives exactly the trees that line up with it, and
one that writes the other text gives exactly the cached output, byte for byte.

Both commands therefore check a generated file's entries against its text before comparing
(`fits_text`): for `types` a `STRING_LITERAL`, `METHOD_INVOCATION`, `PARENTHESIZED`, or `IDENTIFIER`
that couldn't be at its range, for `names` a field's or method's name that doesn't end at its range,
means this isn't the text javac saw. The file is left out, counted as generated with other text, and
named `STALE`. Nothing is counted as a mismatch, since nothing was compared. A text with trees at the
same ranges passes for `types`, and then every entry is still compared; none differs. On 3 November
that leaves out, of 1,614 generated files, one for `types`
(`common/mapper/main`'s `MappersFactory__ServiceDescriptor.java`), and five for `names` (those of
`messaging/messaging/main`, `data/jdbc/jdbc/main` twice, `webserver/observe/observe/main`, and
`webclient/api/main`).

To bring one such file back in line without disturbing anything else, replace the cached output and
keep the text on disk: `FERRION_ORACLE_SHOW` gives the source set's command; run it with
`--generated` set to a scratch directory until the processors write what is on disk, replace the
scratch path in its output with the generated directory's path relative to the corpus, and put that
in place of `target/oracle/types/<key>/<hash>.jsonl`. Removing the cached entry and letting the run
make it isn't the same: it rewrites the directory, half the time with the other text, which
invalidates what `names` has cached for the file, and, by the input hash, every source set that
depends on it.

## Observations

On Temurin 27.0.1 unless a JDK is named, each on an input written for it.

1. **`lub` cuts its recursion off.** `var x = b ? 1 : "x";` has
   `java/io/Serializable&java/lang/Comparable<? extends java/io/Serializable&java/lang/Comparable<?>&java/lang/constant/Constable&java/lang/constant/ConstantDesc>&java/lang/constant/Constable&java/lang/constant/ConstantDesc`.
   By JLS 4.10.4, `lcta(Comparable<Integer>, Comparable<String>)` is `? extends lub(Integer,
   String)`, an infinite type; javac computes one more level and then writes `Comparable<?>`.
   This departs from the JLS, which leaves the infinite type in place. `b ? new StringBuilder() :
   "s"` and `List.of(1, 2.0)` (inference's lub) are cut off the same way.
2. **The order of a `lub`'s parts** is a class first, then interfaces deepest first (the longest
   chain of superinterfaces), then by qualified name: `Ab&Zed` for two classes implementing `Zed,
   Ab` and `Ab, Zed`, whichever operand comes first; `z/Deep8&z/Aaa` where `Deep8` extends two
   interfaces; `Base&Deep&Other&Zed`. The JLS doesn't order an intersection's parts.
3. **`lub` keeps only minimal candidates,** as JLS 4.10.4 says: `b ? new ArrayList<String>() :
   new LinkedList<String>()` is `java/util/AbstractList<java/lang/String>&java/io/Serializable&java/lang/Cloneable`
   (`List` is a supertype of `AbstractList`), and with a `HashSet` instead,
   `java/util/AbstractCollection<java/lang/String>&java/io/Serializable&java/lang/Cloneable`.
   `b ? list : set` (two interfaces) is `java/util/Collection<java/lang/String>`.
4. **Inference infers lubs too:** `Arrays.asList(1, "a")` is a `List` of observation 1's type.
   `var e = Collections.emptyList();` is `java/util/List<java/lang/Object>`.
5. **Conditional expressions** follow JLS 15.25: `Object o = true ? 1 : "x";` types the conditional
   `java/lang/Object` (a reference conditional in an assignment context is a poly expression);
   `b ? 1 : 'a'` is `char` (a constant `int` representable as `char`), `b ? 'a' : 1000000` is
   `int`, `b ? (short) 1 : (byte) 1` is `short`, `b ? (Integer) 1 : (Character) 'a'` is `int`,
   `b ? Integer.valueOf(1) : Double.valueOf(2)` is `double`, and `b ? Integer.valueOf(1) : null`
   is `java/lang/Integer`.
6. **`switch` expressions** (JLS 15.28.1): `case 1 -> 1; default -> 2L;` is `long`, and `case 1 ->
   "a"; default -> 1;` is observation 1's lub.
7. **Capture's bounds** are the glb JLS 5.1.10 gives: `ls.get(0)` for `List<? super Integer>` is
   `capture#1 extends java/lang/Object super java/lang/Integer`; `g.get()` for `G<?>` with `class
   G<T extends Comparable<T>>` is `capture#1 extends java/lang/Comparable<capture#1>`; and
   `en.getDeclaringClass()` for `Enum<?>` is
   `java/lang/Class<capture#1 extends java/lang/Enum<capture#1>>`. Nested wildcards are captured
   one level at a time: `ll.get(0)` for `List<? extends List<? extends Number>>` is `capture#1
   extends java/util/List<? extends java/lang/Number>`, and `ll.get(0).get(0)` `capture#1 extends
   java/lang/Number`.
8. **A generic method's return type is captured** (JLS 15.12.3): `Collectors.toList()` in
   `stream.collect(Collectors.toList())` is
   `java/util/stream/Collector<java/lang/String,capture#1 extends java/lang/Object,java/util/List<java/lang/String>>`.
9. **`var` takes the upward projection** (JLS 14.4.1): `var x = xs.get(0);` for `List<? extends
   Number>` is `java/lang/Number`, `var dc = en.getDeclaringClass();`
   `java/lang/Class<? extends java/lang/Enum<?>>`, and `var c = g.get();`
   `java/lang/Comparable<?>`. `var` of an anonymous class's creation is the anonymous class.
10. **A multi-catch parameter's** type is the union, `java/io/IOException|java/lang/RuntimeException`,
    and it is `final` (JLS 14.20).
11. **Constants** are what JLS 15.29 says: `1 << 3 + 2` is `int 32`, `(byte) 300` `byte 44`,
    `Long.MAX_VALUE + 1` `long -9223372036854775808`, `1.0f / 3` `float 0x1.555556p-2`, `0.0 /
    0.0` `double NaN`, `-0.0` `double -0x0.0p0`, `"a" + K + 'c' + 1.5f` `String "a32c1.5"`, `1 < 2
    && "a" == "a"` `boolean true`, `true ? 'a' : 0` `char "a"`, and a `final` local with a constant
    initializer is a constant where it's used. `1 / 0` isn't a constant (it would complete
    abruptly), nor is `... + null`, though its left operand is. A parenthesized constant is one.
    javac holds a `boolean`'s, `char`'s, `byte`'s, and `short`'s value as an `Integer`; the
    oracle writes it by the expression's type.
12. **Effective finality** is JLS 4.12.4's: a local assigned once after its declaration without an
    initializer, an unassigned parameter, catch parameter, pattern binding, lambda parameter, and
    enhanced `for` variable are `effectively-final`; a variable assigned again, or a basic `for`
    loop's incremented variable, `mutable`; a declared `final` variable is `final` (javac doesn't
    also mark it effectively final, as the JLS doesn't), and so are a resource and a multi-catch
    parameter. JDKs 21, 25, and 26 give the same, at release 21.
13. **String conversion of `float` and `double` constants is the running JDK's.** `"" + 1.0E23`
    folds to `String "1.0E23"` on JDK 27, and to `9.999999999999999E22` on Corretto 17 (with the
    same internal method), whose `Double.toString` predates JDK 19's shortest-decimal one; `"" +
    2.0e-3` is `0.002` on both. JLS 5.1.11 defers to `Double.toString`, whose specification
    changed in 19. Every corpus project runs on JDK 21 or later, so Ferrion follows JDK 19's.
14. **Implicit calls' positions:** see [Calls](#calls) (a class `T1` with no constructor: the
    `super()` at `class`; `T2() {}`: at its `{`; `Sub() { }` in a subclass of `T2<String>`: at
    its `{`, choosing `T2.<init>()V`; an enum: at `enum`, choosing
    `java/lang/Enum.<init>(Ljava/lang/String;I)V`).
15. **Enum constants:** `RED` has an implicit `NEW_CLASS` at its name choosing `Color.<init>()V`;
    `GREEN { }` has a `NEW_CLASS` with the range of `{ }` choosing the anonymous class's
    constructor, and an implicit `super()` at `GREEN` choosing `Color.<init>()V`.
16. **`directSupertypes` departs from JLS 4.10:** for `int[]` it gives one intersection,
    `java/io/Serializable&java/lang/Cloneable` (without `Object`, as W1 found; for another array,
    the array of its component's first supertype:
    [W1's observation 8](semantic-type-relations.md#observations)), where JLS 4.10.3 has three
    direct supertypes; for an interface it includes `Object` even when the interface has
    superinterfaces (`List`: `Object` and `SequencedCollection`), as the API's documentation says
    and JLS 4.10.2 doesn't; and for a wildcard-parameterized type it substitutes the wildcards
    instead of capturing (`List<? extends Number>`: `SequencedCollection<? extends Number>`;
    `Enum<?>`: `Comparable<?>`), where JLS 4.10.2 takes the supertypes of the capture.
17. **`asMemberOf` doesn't capture:** `List.get` as a member of `List<? extends Number>` is
    `(int)? extends java/lang/Number`, a wildcard as a return type.
18. **`isSubsignature` compares signatures only,** names aside, as JLS 8.4.2: `n2(List)` is a
    subsignature of `n(List<String>)` and of `<T> m(List<T>)` (the erasure rule), and not the
    reverse.
19. **javac 27 attributes every class first,** starting each one's `ANALYZE` event, and then
    analyzes flow for each, finishing its event, in the same order.
20. **Switch errors:** exhaustiveness is checked in flow analysis, so a class with an attribution
    error has none reported (a `switch` missing a sealed type's case went unreported beside a
    dominance error). The codes, which the report counts: `not.exhaustive` and
    `not.exhaustive.statement` (with `.details` on javac 27, without on 21), `pattern.dominated`,
    `duplicate.case.label`, `duplicate.default.label`, `duplicate.unconditional.pattern`,
    `unconditional.pattern.and.default`, `flows.through.to.pattern`, and
    `flows.through.from.pattern`, each under `compiler.err.`.

21. **An enum constant's creation** (correcting observation 15, which had a body alone): `ARGUMENT(1)`
    has a `NEW_CLASS` over `(1)`, of the enum's type, choosing `Q$E.<init>(I)V`; `BOTH(2) { ... }`
    one over `(2) { ... }`, of the anonymous class's type, choosing its constructor, and an implicit
    `super(...)` at `BOTH`; `BODY { ... }` one over `{ ... }`; `PLAIN` an implicit one alone, and no
    expression. (`types-cases/shapes.java`.)
22. **Implicit constructors' parameters are variables:** `record Implicit(int a, List<String> b) {}`
    has two `PARAMETER`s at the offset of `record`, of the components' types; `new Base(1, "a") {}`
    two at its body's `{`, of the superclass constructor's; `BOTH(2) { ... }` one at `BOTH`.
    An explicit canonical constructor's are its own, and a compact one's are at the component's type.
    An implicit canonical constructor's are in no other tree.
23. **An array creation's initializer** isn't a tree of its own: `new int[][] {{1}, {2, 3}}` is one
    `NEW_ARRAY` over all of it, with a `NEW_ARRAY` for `{1}` and for `{2, 3}`; `int[] a = {1, 2}`'s
    `{1, 2}` is a `NEW_ARRAY`, as observed for annotations
    ([attribution's observation 15](semantic-attribution.md#observations)).
24. **An annotation on a record component** is in javac's trees only if it applies to a field: with
    `record R(@P("p") int a, @Q("q") int b, @C("c") int c, @F("f") int d, @M("m") int e, @U("u") int
    f, @N("n") int g) {}` and `@Target`s of `PARAMETER` and `RECORD_COMPONENT` (`P`), `PARAMETER`
    (`Q`), `RECORD_COMPONENT` (`C`), `FIELD` (`F`), `METHOD` (`M`), `TYPE_USE` (`U`), and none (`N`),
    only `"f"`, `"u"`, and `"n"` are expressions. A compact constructor doesn't bring them back
    (`record S(@P("ps") int a, @F("fs") int d)`: `"fs"` alone), and an explicit constructor's own
    parameters have their own (`T(@P("ptc") int a, ...)`: `"ptc"`). The same on JDKs 21, 25, 26, and
    27 for no `@Target`. (hertzbeat's `@DefaultValue` on its `Properties` records: 66 entries.)
25. **Processors can write different text from the same inputs:** see
    [Generated text that changes](#generated-text-that-changes).

## Known differences

None for the relation queries (B3a): every one javac answered is matched, left to B4, or about a
type javac couldn't resolve ([the corpus](#the-relations-comparison)). None for what W2's walk types
either ([the comparison](#the-attribution-comparison)): every difference found was a difference of
trees, which [the adapter](#ferrions-side-of-the-files-entries) normalizes. W3's calls bring the
first, if any.

## The corpus

`cargo oracle types --javac-only` on every locked project, on a 10-core Mac with two JVMs of 5 GB
at a time, each project on the JDK its lock names, on 2 November 2026, with the oracle jar of
SHA-1 `e61ea3af7772dc7df0801dfe5f140dd9e8b76efa` (built reproducibly, as W6c made it). Files
include those processors generated and module declarations; "compared" are those not left out,
as in [names](semantic-oracle-normalization.md#the-corpus) (the same 445 files with language
errors, 442 of them hertzbeat's tests). Entries are counted in compared files, outside Lombok
types. javac's time is its own, summed over the JVMs; the output is the cached JSON.

| Project | JDK | Source sets | Files | Compared | Expressions | Calls | Constants | Variables | Relation queries | javac | Output |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| jedis | 21 | 2 | 1,080 | 1,080 | 369,831 | 140,681 | 70,792 | 46,509 | 15,204 | 28 s | 37 MB |
| guava | 21 | 6 | 1,597 | 1,594 | 427,754 | 153,966 | 98,287 | 44,949 | 30,925 | 43 s | 47 MB |
| caffeine | 26 | 29 | 1,309 | 1,303 | 293,589 | 107,858 | 34,437 | 34,631 | 138,888 | 83 s | 35 MB |
| spring-data-jpa | 21 | 4 | 901 | 901 | 191,400 | 59,528 | 36,490 | 21,004 | 30,596 | 35 s | 24 MB |
| lucene | 21 | 75 | 6,239 | 6,202 | 2,069,301 | 470,731 | 410,240 | 197,615 | 409,828 | 327 s | 176 MB |
| hertzbeat | 25 | 46 | 1,384 | 942 | 44,975 | 15,840 | 11,174 | 5,165 | 171,875 | 114 s | 40 MB |
| helidon | 27 | 376 | 4,901 | 4,687 | 1,050,860 | 381,347 | 188,509 | 110,668 | 1,382,963 | 1,035 s | 155 MB |
| total | | 538 | 17,411 | 16,709 | 4,447,710 | 1,329,951 | 849,929 | 460,541 | 2,180,279 | 1,664 s | 514 MB |

- **Calls:** 31,765 of them implicit (`super()` and enum constants' `new`).
- **Expressions by kind:** identifiers 1,442,787, method invocations 1,139,535, string literals
  326,202, int literals 324,318, member selects 245,789, class instance creations 146,132,
  parenthesized 135,735, assignments 112,814, `+` 72,950, and fewer of the rest.
- **Variables:** 379,483 effectively final, 29,168 final, 51,890 mutable.
- **Relation queries:** `subtype` and `assignable` 701,655 each, `same` 647,573, `erasure`,
  `capture`, and `supertypes` 16,003 each, `member` 41,529, `subsignature` and `overrides` 19,838
  each, and 182 blocks javac threw on, all on class files it couldn't read.
- **Switch errors:** none: no corpus file has one.
- **Error types in clean files:** 8 type strings, where a jar's method names a class its classpath
  lacks (`(error graphql.execution.ExecutionStrategy)graphql/GraphQL$Builder` in helidon's GraphQL
  server); javac reports nothing, since nothing needs the class.
- **Type strings:** 666,527 in the files' tables, median 46 characters, the longest 5,540;
  captures up to `capture#5` in one string (caffeine's `LocalAsyncCache`).
- **Size:** 514 MB in all, each file's type strings listed once and named by index. The largest
  output, one of lucene's source sets at 37 MB, gzips to 6 MB; compressing the cache isn't needed
  yet, and would cut it about six times if CI's cache needs it.

The whole run took 864 s. javac's time per source set includes starting the JVM, the
processors, and the relations, which `millis` covers.

### The relations comparison

`cargo oracle types --relations` on every locked project, from the cached javac side above, with
B3a's relations as Ferrion's side (W1, on 2 November 2026: 13.8 s, Ferrion's side 9.4 s; the
[design's results](semantic-type-relations.md#results) have memory, fuel, and arenas):

| Project | Queries | Matched | Left to B4 (Lombok) | javac unresolved | javac threw | Mismatched |
| --- | --- | --- | --- | --- | --- | --- |
| caffeine | 138,888 | 138,802 | 0 | 54 | 32 | 0 |
| guava | 30,925 | 30,925 | 0 | 0 | 0 | 0 |
| helidon | 1,382,963 | 1,382,802 | 0 | 98 | 63 | 0 |
| hertzbeat | 171,875 | 148,905 | 22,712 | 194 | 64 | 0 |
| jedis | 15,204 | 15,203 | 0 | 0 | 1 | 0 |
| lucene | 409,828 | 409,790 | 0 | 25 | 13 | 0 |
| spring-data-jpa | 30,596 | 30,559 | 0 | 28 | 9 | 0 |
| total | 2,180,279 | 2,156,986 | 22,712 | 399 | 182 | 0 |

Every query but those left out matches; there are no known differences. Sampled sites whose owner
is a local class match too, their keys and type strings converted as above.

### The attribution comparison

`cargo oracle types` on every locked project, from the cached javac side above, on 3 November 2026,
with what W2's walk types; each call and whatever depends on one waits for W3, and is counted apart.
The run took 34 s, Ferrion's side 32 s, on four threads, on a shared machine.

| Project | Matched | Expressions | Constants | Variables | Waiting for calls | Left to B3c, B3d | Parameters of implicit constructors | In annotations javac dropped | Stale generated | Mismatched | Extra |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| caffeine | 355,931 | 157,503 | 34,433 | 25,193 | 239,295 | 48,510 | 106 | 0 | 0 | 0 | 0 |
| guava | 443,989 | 271,045 | 98,287 | 43,732 | 305,446 | 50,403 | 496 | 0 | 0 | 0 | 0 |
| helidon | 2,278,384 | 611,765 | 188,471 | 95,346 | 784,299 | 158,856 | 1,491 | 0 | 1 | 0 | 0 |
| hertzbeat | 193,247 | 28,381 | 11,110 | 4,851 | 31,777 | 28,612 | 150 | 66 | 0 | 0 | 0 |
| jedis | 358,446 | 226,594 | 70,792 | 45,857 | 280,748 | 50,039 | 146 | 0 | 0 | 0 | 0 |
| lucene | 2,548,761 | 1,535,782 | 409,986 | 193,203 | 995,171 | 207,462 | 1,949 | 0 | 0 | 0 | 0 |
| spring-data-jpa | 210,126 | 124,912 | 34,283 | 20,372 | 125,980 | 23,645 | 117 | 0 | 0 | 0 | 0 |
| total | 6,388,884 | 2,955,982 | 847,362 | 428,554 | 2,762,716 | 567,527 | 4,455 | 66 | 1 | 0 | 0 |

"Matched" includes the relation queries' 2,156,986, which are unchanged; the three columns after it
are the files' entries. By aspect, what is counted apart:

| Aspect | Waiting for calls | Left to B3c | Left to B3d |
| --- | --- | --- | --- |
| Expression | 1,431,634 | 59,489 | 402 |
| Constant | 2,534 | 3 | 0 |
| Call | 1,317,345 | 12,518 | 0 |
| Variable's type | 11,203 | 16,309 | 14 |
| Variable's finality | 0 | 0 | 456,080 |

With [one cached output replaced](#generated-text-that-changes), in a private copy of the cache,
helidon has 2,278,527 matched and no stale file, and everything else is the same.

Of javac's expressions, every one has a Ferrion expression at its range. A type or constant that
differs would be a `type` or `value` mismatch: none. What W2 counted as 174 expressions without one
was the text of `__ServiceDescriptor`s, as above; before the check, this run had 12 expressions and
a constant without a Ferrion entry, and 6 entries extra, all in the one file. The 3 constants left to B3c are fields inherited through an anonymous class made with `<>`
(helidon's `JdbcConnectionPool`), which Ferrion doesn't type until inference gives its supertype.

### Analysis time per file

`analyzeMicros` of the 16,709 compared files (javac 21 to 27, each project's own):

| | Median | p90 | p99 | p99.9 | Max | Sum |
| --- | --- | --- | --- | --- | --- | --- |
| Every compared file | 1.6 ms | 17.5 ms | 122 ms | 431 ms | 1,129 ms | 147 s |
| But each source set's first 5 files | 1.4 ms | 11.9 ms | 74 ms | 323 ms | 1,129 ms | 88 s |

A source set's first file took 39 ms in the median, against 1.4 ms from the sixth on: the JVM's
warm-up, and the first completions of the platform's classes, are charged to the first classes
analyzed, so the second row is the better measure of a file's own cost. The machine was shared with
other work, and an earlier run of the same jar's predecessor gave the same files up to three times
less (`CommandObjects.java` 379 ms there, 1,129 ms here), so these are upper bounds within a factor
of about three. The 20 slowest files, by position in their source set (0 is the first analyzed):

| Time | # | File |
| --- | --- | --- |
| 1,129 ms | 13 | jedis/src/main/java/redis/clients/jedis/CommandObjects.java |
| 920 ms | 107 | spring-data-jpa/spring-data-jpa/src/test/java/org/springframework/data/jpa/repository/AotUserRepositoryTests.java |
| 906 ms | 216 | spring-data-jpa/spring-data-jpa/target/generated-sources/antlr4/org/springframework/data/jpa/repository/query/EqlParser.java |
| 867 ms | 32 | jedis/src/main/java/redis/clients/jedis/Jedis.java |
| 730 ms | 0 | jedis/src/main/java/redis/clients/jedis/AbstractPipeline.java |
| 721 ms | 0 | caffeine/guava/src/test/java/com/github/benmanes/caffeine/guava/CaffeinatedGuavaTest.java |
| 629 ms | 6 | caffeine/caffeine/src/test/java/com/github/benmanes/caffeine/cache/BoundedLocalCacheTest.java |
| 585 ms | 65 | hertzbeat/hertzbeat-common-spring/src/main/java/org/apache/hertzbeat/common/support/valid/SqlSecurityValidator.java |
| 581 ms | 63 | jedis/src/main/java/redis/clients/jedis/RedisClusterClient.java |
| 563 ms | 225 | guava/guava/src/com/google/common/collect/Maps.java |
| 554 ms | 0 | helidon/webclient/http2/src/test/java/io/helidon/webclient/http2/Http2AltSvcCacheTest.java |
| 548 ms | 200 | guava/guava/src/com/google/common/collect/ImmutableSetMultimap.java |
| 474 ms | 8 | caffeine/caffeine/src/eclipseTest/java/com/github/benmanes/caffeine/eclipse/mutable/ConcurrentHashMapTest.java |
| 465 ms | 0 | caffeine/caffeine/src/test/java/com/github/benmanes/caffeine/cache/AsMapTest.java |
| 463 ms | 0 | caffeine/jcache/src/test/java/com/github/benmanes/caffeine/jcache/CacheManagerTest.java |
| 462 ms | 0 | caffeine/caffeine/build/generated/sources/local-cache/com/github/benmanes/caffeine/cache/SI.java |
| 457 ms | 0 | caffeine/guava/src/osgiTest/java/com/github/benmanes/caffeine/guava/OSGiTest.java |
| 431 ms | 222 | spring-data-jpa/spring-data-jpa/target/generated-sources/antlr4/org/springframework/data/jpa/repository/query/HqlParser.java |
| 426 ms | 5 | jedis/src/main/java/redis/clients/jedis/BuilderFactory.java |
| 421 ms | 0 | helidon/openapi/openapi-31/src/test/java/io/helidon/openapi/v31/OpenApi31DocumentMapperTest.java |

The slow files that aren't first are large API surfaces (jedis's `CommandObjects` and `Jedis`,
thousands of overloaded calls), ANTLR's generated parsers, and guava's collection classes, heavy
in generic inference.

### javac's stress tests

`javac-oracle types --release 27 --javac --enable-preview` on 975 of javac's own tests
(`cargo corpus jdk-tests`): every file under `generics/inference`, `generics/diamond`, `lambda`,
`patterns`, `expswitch`, and `perf`, and every file named for deep nesting, chains, or stress,
each in a JVM of its own, since they declare clashing classes, on Temurin 27.0.1. Each file is its
JVM's first, so about 40 ms of every time is warm-up; files javac reports errors in are counted
too. The slowest:

| Time | Expressions | Errors | File |
| --- | --- | --- | --- |
| 5,031 ms | 102 | 0 | generics/inference/8048838/T8048838.java |
| 488 ms | 303 | 0 | generics/inference/8067767/T8067767.java |
| 458 ms | 100 | 0 | lambda/speculative/NestedLambdaGenerics.java |
| 164 ms | 220 | 0 | lambda/speculative/InferWeak.java |
| 141 ms | 442 | 24 | lambda/deduplication/DeduplicationTest.java |
| 129 ms | 41 | 0 | lambda/speculative/InferStrict.java |
| 122 ms | 119 | 0 | lambda/speculative/T8055984.java |
| 119 ms | 168 | 0 | patterns/T8326404.java |
| 116 ms | 384 | 42 | generics/inference/EagerReturnTypeResolution/EagerReturnTypeResolutionTestb.java |
| 110 ms | 958 | 0 | lambda/ByteCodeTest.java |

`T8048838` declares a method with 100 type parameters, each bounded by the next, and calls it: a
fixed input that takes javac five seconds, which a per-file limit has to allow for or cut off as
"unknown". The rest are under half a second, nested generic lambdas and speculative attribution
the slowest of them. The whole set took 371 s.

## Still to observe

- Whether a variable arity call javac chose in phase 3 can be told from the oracle's output where
  the instantiated type alone doesn't say (a single array argument).

W1 observed the order of a `lub`'s parts on more inputs (it holds:
[W1's observation 2](semantic-type-relations.md#observations)), and wrote local classes in type
strings, as `names`' `oracle_key` converts keys.
