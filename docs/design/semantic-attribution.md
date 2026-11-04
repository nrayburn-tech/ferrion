# Attribution and overloads (B3b)

B3b types every expression that needs no inference, chooses the method of every call whose choice
needs none, and folds constants. It runs as a second walk over each file's bodies, after B2's
names, using B3a's [type relations](semantic-type-relations.md). The overview is
[type attribution](semantic-types.md).

## Decision

- **A second walk, after names:** `body_types` reads `body_names` and walks the same tree again,
  rather than extending B2's walk. Names are decided by scope as each node is entered; types are
  decided as each node is left, with what its context expects set as it is entered. Keeping them
  apart keeps B2's results, its comparison, and its replay check as they are.
- **Member tables as shared entries:** each type's member fields and methods, declared and
  inherited, with overriding and hiding worked out, are a derived entry per type and view, read by
  every call that searches that type ([member tables](#member-tables)).
- **Constants in B3b:** constant folding (15.29) moves here from B3d, with a derived entry per
  constant field, since a constant's value crosses files ([constants](#constants)).
- **The inference line:** a call needs inference when one of its potentially applicable candidates
  is a generic method without explicit type arguments, or it creates an instance with `<>`, or an
  argument is a lambda, a method reference, or another expression whose type needs inference. Such
  a call, and everything whose type depends on it, is `Unknown(LeftToB3c)`
  ([the line](#the-inference-line)).
- **No compile errors:** B3b reports none. Where a correct program couldn't be what Ferrion sees
  (no applicable method, an ambiguous call), the result is an `Error` with its cause, so rules stay
  silent; in the corpus, outside code javac rejects, each one is a mismatch to explain.

## The walk

Per body: each method, constructor, and initializer body, each field initializer, each enum
constant's arguments, and each annotation's arguments. It is the tree's flat pre-order walk with
an explicit stack, as B2's is, so deep nesting never recurses on the native stack:

- **Entering a node** pushes what its context expects of it: an assignment context with the
  target type, an invocation context, a numeric or string context, a condition, a statement with
  no expectation. B3c turns these into targets for poly expressions; in B3b they decide constant
  narrowing and which conversion is recorded.
- **Leaving a node** types it from its children's types (already on the stack), and records the
  type, and the conversion its context applied, in the result.
- **Names** come from `body_names` by `NodeId`: a local's or parameter's declared type, a field's
  type as seen from its qualifier ([`member_type`](semantic-type-relations.md#the-relations)), a
  type name's class. What B2 left to B3 (`LeftToB3`) is looked up here, in the member table of the
  qualifier's type now that it is known: fields of an expression's type, members of a type
  variable through its bounds, an array's `length` and `clone()`.
- **Polling:** every 1,024 nodes, as B2's walk polls. **Fuel** is drawn per body, and running out
  types the rest of that body's current expression `Unknown(Limit)`.

What each expression gets in B3b, by JLS section; "B3c" and "B3d" mark what is left to them:

| Expression | JLS | B3b |
| --- | --- | --- |
| Literals, `this`, qualified `this`, class literals | 15.8 | Typed. |
| Names, field access, `super.f` | 6.5.6, 15.11 | Typed, with B2's left names looked up. |
| Array access, array creation, array initializers | 15.10 | Typed. |
| Method invocation | 15.12 | Chosen and typed, inside the line. |
| Class instance creation, anonymous classes | 15.9 | Chosen and typed, without `<>`. |
| Unary, binary, shift, relational, equality, logical, conditional-and/or | 15.15–15.24 | Typed, with unary and binary numeric promotion and the conversions recorded. |
| String concatenation | 15.18.1 | Typed. |
| Assignment, compound assignment | 15.26 | Typed, with the implied cast of compound assignment recorded. |
| Conditional `? :` | 15.25 | Boolean and numeric conditionals typed; a reference conditional typed as standalone (`lub`) outside assignment and invocation contexts, and B3c's in them. |
| Casts, `instanceof` with a type | 15.16, 15.20.2 | Typed, with the cast's conversion kind. |
| `instanceof` with a pattern, `switch` expressions | 15.20.2, 15.28 | B3d (the binding's declared type is B2's). |
| Lambdas, method references | 15.27, 15.13 | B3c. Inside a lambda body, explicitly typed parameters have their types and the expressions that don't depend on implicit ones are typed. |
| `var` locals, enhanced `for` | 14.4.1, 14.14.2 | Typed when the initializer or iterable is: the upward projection of its type, the array's component or `Iterable`'s argument through `as_super`. |
| Switch statements | 14.11 | The selector typed, and `case` constants resolved and checked against it. |

## Method invocation

Following 15.12, with the type to search already given by B2's `Methods` symbol for a simple name,
`super.m()`, and `this.m()`, and found here from the receiver's type otherwise:

1. **Candidates (15.12.2.1):** the member methods of the name in the searched type's [member
   table](#member-tables), accessible from the class the call is in (6.6, including 6.6.2's rule
   on the qualifier of a protected member), whose arity fits (with varargs), and whose explicit
   type arguments, if given, fit their type parameters' number.
2. **The inference line** is checked here ([below](#the-inference-line)).
3. **Phases (15.12.2.2–4):** applicable by strict invocation, then loose, then variable arity,
   each argument tested with B3a's `convert` in that context. The first phase with any applicable
   method decides.
4. **Most specific (15.12.2.5):** for non-generic methods, by the subtyping of their parameter
   types (with the varargs rule); among maximally specific methods with override-equivalent
   signatures, the one that is not abstract, or else the abstract and default rules of 15.12.2.5's
   last part, with the throws clauses intersected.
5. **The result (15.12.2.6, 15.12.3):** the chosen method's return type as seen from the searched
   type, erased if an unchecked conversion was needed for applicability, then captured. `getClass()`
   is typed as 15.8.2 and 4.3.2 say. A `void` call has no value type.

Class instance creation (15.9.3) chooses among constructors the same way, with an anonymous class's
superclass constructors, and an inner class's outer instance from its qualifier or the enclosing
instance.

What is recorded for each call: the chosen method (a `MethodRef`, keyed for the oracle as B2 keys
methods), its type as invoked (parameters and result after substitution), and which phase chose
it. Settling B2's two known differences:

- **`static-import-overload`** (4,830 in the corpus): the candidates are every single-static
  import of the name, gathered from all of them as 15.12.1 says, so the chosen method's type can
  be one the first import doesn't declare.
- **`case-label`** (1,012): a `case` name in a switch whose selector is an enum is that enum's
  constant, now that the selector has a type.

### The inference line

The question for each call is whether choosing its method, or typing its result, needs JLS
chapter 18. B3b chooses and types a call when all of these hold:

- every potentially applicable candidate is non-generic, or the call gives explicit type
  arguments (substitution is enough);
- no argument is a lambda, a method reference, a `<>` creation, a generic method call without
  explicit type arguments whose result mentions its type parameters, or a conditional or `switch`
  whose type depends on one;
- for a creation, it isn't `<>`.

A call outside the line is `Unknown(LeftToB3c)`, and so is everything whose type it determines,
up to the statement. The arguments of such a call are still typed when they are standalone, since
their types don't depend on the call. W0's first corpus counts give how many calls fall on each
side, and W4 records it.

The oracle reports a call's chosen method and its type as invoked, but not which phase chose it
(W0's question 12): which phase chose a call is recorded in Ferrion's result for rules, and isn't
compared.

## Member tables

```rust
/// A type's member fields and methods in a view: declared and inherited, by name.
pub struct Members {
    pub fields: Box<[(Name, FieldRef)]>,
    /// Every member method of each name: declared ones, and inherited ones not overridden or
    /// hidden (8.4.8), each with the supertype it is inherited through, so its signature is
    /// substituted as seen from this type.
    pub methods: Box<[(Name, Box<[InheritedMethod]>)]>,
    /// A supertype that didn't resolve, or a Lombok-annotated type until B4: lookups that find
    /// nothing, or might be overloaded by a missing member, are `Unknown`.
    pub incomplete: bool,
}
```

- **Overriding** is decided on signatures as seen from this type (8.4.2, 8.4.8.1), so
  `compareTo(A)` in `class A implements Comparable<A>` overrides `compareTo(T)`; abstract and
  default methods inherited from interfaces follow 8.4.8.4 and 9.4.1.3. An interface's members
  include `Object`'s public methods (9.2).
- **Fields** hide by name (8.3); one inherited from two supertypes is ambiguous, an error only
  when used.
- **Entries:** one per type and view, computed from the type's entry, its supertypes' member
  tables, and B3a's relations, with its own small arena. It reads its supertypes' tables through
  `Reads`, so an edit inside a body never invalidates it, and a signature change cuts off at the
  first table whose value is unchanged. This answers W5's question from B2: body lookups of fields
  and methods become shared entries, rather than walked per call.
- **Local and anonymous classes** get their tables inside the file's result, as B2 keeps their
  type entries.

## Constants

Constant expressions (15.29) are folded as they are typed: literals, casts to primitive types and
`String`, the operators 15.29 lists, and names of constant variables. Arithmetic is Java's: `int`
and `long` wrap, division by zero isn't a constant, `float` and `double` follow IEEE 754 with
Java's rounding, and `String` conversion of a constant follows 5.1.11.

- **A constant field crosses files:** `Snapshot::constant(field)` is a derived entry per `final`
  field with an initializer and a constant type. It reads the summary's initializer fingerprint
  (B2 records it for exactly this), the file's tree from the cache, and resolves and folds the
  initializer alone, reading other constants through `Reads`. An edit elsewhere in the file
  doesn't change it. A cycle of constants (an error javac reports) reads as not constant.
- **Class files:** a field's `ConstantValue` attribute, which B1 reads but B2's `ClassField`
  doesn't keep yet; W2 adds it.
- **Text of floating-point constants:** `"" + 1.0E23` folds through `Double.toString`, whose
  output changed in JDK 19, and javac folds with the JDK it runs on, not `--release`'s
  (observation 13 in [types oracle normalization](types-oracle-normalization.md#observations):
  `1.0E23` on JDK 27, `9.999999999999999E22` on JDK 17). Ferrion folds with JDK 19's shortest
  decimal, which every corpus project's JDK (21 or later) uses; a build on an older JDK can differ,
  and only in such text.

## The result

```rust
/// A file's attributed bodies, by `NodeId` of the tree they were attributed on.
pub struct BodyTypes {
    /// Every type below lives here.
    pub arena: Arena,
    /// Each expression node's type, in node order, binary searched by `NodeId`.
    pub types: Box<[(NodeId, TypeId)]>,
    /// Each call's and creation's chosen method and its type as invoked.
    pub calls: Box<[(NodeId, Call)]>,
    /// Each constant expression's value.
    pub constants: Box<[(NodeId, Constant)]>,
    /// The conversions an expression's context applied to it, where any but identity was.
    pub conversions: Box<[(NodeId, SmallVec<[Conversion; 2]>)]>,
    /// The local and anonymous classes' member tables.
    pub local_members: Box<[Members]>,
}
```

It is a derived entry per file, like `body_names`, and reads the file's tree, its `body_names`,
and project-wide entries (type entries, member tables, constants) through `Reads`. An edit inside a
body recomputes that file's types and no other file's; a changed signature recomputes the files
that read the changed member table. The CLI drops each file's result after its rules run; the
language server keeps open files'.

Per-file granularity is kept from B2. If W4's benchmark of an edit in lucene's largest files is
too slow for the language server, results split per body, which the walk's per-body structure
already allows.

## Errors

- An `Error` or `Unknown` operand, receiver, or argument makes the expression's type a non-answer,
  with the first root cause; a call with one chooses no method (the scope's rule).
- A call whose searched type's member table is incomplete chooses no method when the missing part
  could hold a candidate: always, since its names aren't known.
- No applicable candidate, or no single most specific one, is an `Error` with that cause. In the
  corpus outside code javac rejects, these are counted and explained, as B2's mismatches were.

## Checking

`cargo oracle types` (W0's) compares, per expression in B3b's part, the type, and per call the
chosen method's key, and per constant expression the value, in the formats of [types oracle
normalization](types-oracle-normalization.md#what-is-compared)). Each mismatch is categorized
with the oracle's categories (`missing`, `order`, `type`, `value`, `method`, `instantiation`, and
for later milestones `finality` and `answer`), and a non-answer where javac has a type is a `type`
mismatch like any other; Ferrion's values starting `left-to-` (`left-to-b3c`, `left-to-b3d`) are
counted apart, not compared. The removed-symbols check
runs as the overview defines. W4 records:

- the counts, per project and category, and the known differences, at zero unexplained;
- the share of calls and expressions on each side of the inference line;
- cold attribution time on lucene, single-threaded and on four threads, against B2's body phase;
- an edit inside a body of lucene's largest file, end to end;
- the largest arena and result in the corpus, to set the arena budget with the time limit.

## Departures

What W2 built differs from the design above in these ways, each for a reason given.

- **No stack of expectations.** What a node's context does to it is decided by the parent when
  the parent is left, since it is the parent's types that say so: the conversion of a variable's
  initializer, an assignment's value, an operand. Only two things need the context earlier, and
  each reads it from the tree when it needs it: an array initializer's type, at its entry, and
  whether a conditional is in an assignment or invocation context, from its ancestors. B3c can add
  a stack for targets if it needs one.
- **A value stack with marks.** Each node entered remembers where its children's values start; a
  node left takes what its children pushed, so a statement or an unexpected tree can't unbalance
  the stack. `Arguments` leaves its children's values on the stack, for W3's call.
- **Types written in bodies are built from the syntax,** with each `ClassType` node's symbol from
  the file's names, not through a `Ty` and W1's translation: B2 keeps only a resolved class per
  node there. This also settles [W1's raw qualified inner class](semantic-type-relations.md#departures)
  for types written in bodies: `Outer.Inner` is raw, as javac has it, since the qualifier is seen.
  Types in declarations go through `Ty` as before; no corpus type needed more.
- **The fuel of a body that runs out:** the rest of that body's relation queries answer
  `Unknown(Limit)`, not only the current expression's, and the walk carries on: what needs no
  query (a literal, a declared type already translated) is still typed. Fuel is drawn per
  top-level body; a method of a local class shares the body around it. Between bodies the context
  is refueled, so that one body's running out doesn't reach the next method's parameters.
  ([Results](#results): no corpus body comes near the budget.)
- **A constant field's entry doesn't reuse the file's walk:** it finds its initializer through
  the summary (the type's path of member types, each by its position among its outer type's), and
  B2's walk starts at the initializer, in that type's scope, so that a file of many constants
  doesn't walk its declarations once per constant (`HqlParser` took 193 ms before, 18 ms after).
  An initializer with anything a constant expression can't have isn't resolved at all. Fields
  that aren't `final` get no entry.
- **Strings are ropes.** `Constant::String` holds a `JavaString`: units, or two strings
  concatenated, flattened every 64 levels, with a content hash combined in constant time.
  Lucene's JFlex tables join thousands of literals into one string, and every prefix is a constant
  of its own: 735 million units in `UAX29URLEmailTokenizerImpl`, 1.5 GB, had each been kept whole.
  W4 should print only the constants javac reports: it folds such a run into one
  ([observation 10](#observations)).
- **`BodyTypes` has more than the sketch:** `variables` (each local's and parameter's type by its
  declaring node, which the oracle compares), `compound` (the implied cast of a compound
  assignment, which has no node of its own), `resolved` (names B2 left that the walk settled),
  `fuel` (per body, for measuring), and `local_members` holds the tables the walk looked up.
  `conversions` holds W1's `Steps`.
- **Members:** `FieldMember` says when a name is ambiguous; `MemberMethod` keeps the direct
  supertype a method came through. A type's table follows JLS 8.3 and 8.4.8, not javac's
  `Elements.getAllMembers`, which keeps hidden fields and abstract methods a superclass implements
  ([observation 14](#observations)).
- **`instanceof` with a pattern** is `boolean` here, not left to B3d: that much doesn't depend on
  the pattern. A binding's type is its declared type, and `var` in a pattern is B3d's.
- **`Why`** has three new values: `Calls` (W3's), `LeftToB3c`, `LeftToB3d`.
- **`FieldRef`** is new in `symbols`, beside `MethodRef`.
- **`TypeContext` records what it reads** when a derived entry made it
  (`Snapshot::recording_types`): the entries of every type it loads. Its reads go into the entry's
  dependencies when the computation ends.
- **Changes to B2:** an anonymous class's entry has its generic supertype (it had only the erased
  one in `Supertypes`), but not with `<>`, whose arguments are inference's; a local class records
  the class around it (`LocalType::lexical`); the names of one initializer can be walked alone.
- **Changes to W1:** `convert` takes `Option<&Constant>`; capture leaves `? super Object` as
  `Object` and captures the outer type too; upward projection has JLS 4.10.5's declared-bound
  rule; a written outer type is normalized to the class that declares the member; an implicit
  outer type is found through a local class's lexically enclosing class; `Type::Union` exists only
  to be recorded and printed ([observations 5 to 9](#observations)). `cargo oracle types
  --relations` still matches every query.
- **A case name is decided by the selector,** whatever B2 found: when a selector's type needed
  attribution, B2 takes a variable in scope if one could be a constant, and the walk overrules it
  for an enum selector (`resolved` has the field). While the selector has no type, a call's until
  W3, its names wait with it, so a corpus switch on a call has no constants for now.
- **`oracle attribute`** is the corpus harness, not a test: `cargo oracle attribute`, with
  `--dump` for each file's expressions and variables as JSON lines.

## Observations

Run with Temurin 27.0.1's `javac` through the types oracle, in a scratch directory, each on a small
input; the inputs of `crates/ferrion_semantic/src/attr/tests.rs` are each one of these, with javac's
answer beside it.

1. **What is captured:** a variable read for its value is (`Object a = xs;` with `List<? extends
   Number> xs` is `java/util/List<capture#1 extends java/lang/Number>`, `this.xs` too, `(xs)` and
   its `xs`, the iterable of an enhanced `for`, `xs == null`'s). A qualifier isn't, through
   parentheses too (`xs` in `xs.size()` and `(xs).size()`), nor an assignment's target (`xs =
   null`'s `xs`, while the assignment is captured). A cast is captured (`(List<?>) o`). An array
   access isn't: `la[0]` with `List<?>[] la` is `java/util/List<?>`.
2. **`this` and `super`:** `this` and `Q.this` are `Q<E:Q>`, the class's declared type; `super` is
   the declared superclass as a type of the class (`Base<java/lang/String>` for `class Q<E> extends
   Base<String>`); `I.super` the interface as a supertype of the class around it
   (`I<java/lang/String>` in `D implements I<String>`, `I<java/lang/Integer>` in `new I<Integer>() {
   ... }`, `java/util/ArrayList<java/lang/Object>` for `super` in `new ArrayList<>() { ... }`).
3. **Class literals:** `String.class` is `java/lang/Class<java/lang/String>`, `int.class`
   `java/lang/Class<java/lang/Integer>`, `void.class` `java/lang/Class<java/lang/Void>`,
   `int[].class` `java/lang/Class<int[]>`, and a generic class's erased, `List.class`
   `java/lang/Class<java/util/List>`.
4. **A conditional's type:** `i > 0 ? "a" : 1` in `Object o = ...` is `java/lang/Object`, the target
   (B3c's); under a cast it is standalone (`(String) (flag ? null : "a")`, `java/lang/String`), and
   with `var` its `lub` (`flag ? null : 1` is `java/lang/Integer`). `flag ? c : 1` is `char`, `flag ?
   boxed : 1L` `long`.
5. **`? super Object` captures to `Object`:** `List<? super Object>` reads as
   `java/util/List<java/lang/Object>`, and `eo.c` with `List<? super K> c` as a member of `E<Object>`
   too, while `List<? super Integer>` has `capture#1 extends java/lang/Object super java/lang/Integer`.
   JLS 5.1.10 has a fresh variable there; this departs from it. (guava's `LocalCacheTest`.)
6. **The outer type is captured too:** a variable `Mall<? extends Number>.Shop<Integer>` reads as
   `Q$Mall<capture#1 extends java/lang/Number>$Shop<java/lang/Integer>`.
7. **A written outer type is the declaring class's:** `Outlet<X>.Shop`, with `Shop` a member of
   `Outlet`'s superclass `Mall`, is `Q$Mall<X>$Shop`, wildcards kept (guava's `TypeTokenSubtypeTest`).
8. **Upward projection's declared-bound rule** (JLS 4.10.5): `var v = (F<String, ?>) o;` with
   `F<T, A extends Annotation>` is `Q$F<java/lang/String,?>`, and with `H<T, A extends List<T>>`
   `Q$H<java/lang/String,? extends java/util/List<java/lang/String>>`, since that bound mentions a
   type parameter.
9. **A multi-catch parameter** is recorded as its union, variable and identifier and parentheses,
   `java/lang/InterruptedException|java/lang/IllegalStateException`. Its type for everything else
   is the `lub` (JLS 14.20), which is what Ferrion's relations see.
10. **javac's trees, for W4:** its parser folds a run of adjacent string literals in a `+` chain
    into one `STRING_LITERAL`, over the whole run: `"a" + "b" + "c"` is one, `x + "a" + "b"` is
    `PLUS(x, "a" + "b")` and so regrouped, `"a" + K + "b" + "c"` `PLUS(PLUS("a", K), "b" + "c")`,
    and `"a" + 1 + "b"` is left alone. A `-` before a decimal `int` or `long` literal is part of the
    literal (`-129` and `- 5` are `INT_LITERAL`s, `-1L` a `LONG_LITERAL`), but not before `0x10`,
    `010`, `1.0`, `(5)`, or a name. The parentheses of `if`, `while`, `do`, `switch`, and
    `synchronized` are `PARENTHESIZED` trees of their own, which Ferrion's tree has no node for;
    a `for`'s condition has none. A resource that is a variable followed by another, `try (c; d)`,
    has a range through its `;` (`c;`), and the last, `d`, doesn't.
11. **Constants:** `self.K` and `this.K` aren't constants, though `K` is, as JLS 15.29 has it; a
    `final var n = 200_000;` is a constant variable; `"x" + null` isn't; everything else is W0's
    observation 11, with more forms in the `more_forms` test: `(byte) -129` is `byte 127`, `(char)
    -1` `char "￿"`, `(long) 1e30` `long 9223372036854775807`, `1e38f * 10` `float Infinity`, `0.0
    / 0 == 0.0 / 0` `boolean false`, `1 + 2 + "a" + 1 + 2` `String "3a12"`.
12. **`Double.toString` and `Float.toString`,** run directly: 799,372 values (random bits, random
    decimals, values at the plain notation's bounds, the extremes) all match
    [`double_to_string`](../../crates/ferrion_semantic/src/constants/value.rs), a sample of 1,225 of
    them as a test. Two rules aren't what Rust's shortest form gives: of two closest decimals of
    the shortest length, the one with an even last digit (the `float` `419025.625` is `419025.62`,
    where Rust's is `.63`), and two digits where one would round to the value but two are closer
    (`4.9E-324`).
13. **A `case` name follows the selector:** with `static final String DESC` in scope, `case DESC` on
    an `Op[]` element or `Op`-returning call is `Op`'s constant; on a `String` selector it is the
    field (helidon's `JakartaPersistenceDataQueryBuilder`).
14. **`Elements.getAllMembers`** lists a field hidden by another of its name (`A.x` beside `B.x`,
    `I.F` beside `J.F`), and an interface's abstract method a concrete superclass method implements
    (`I.f()` beside `R.f()` in `S extends R implements I`); it drops methods overridden by a declared
    one, and an interface's are `Object`'s public ones and not `clone()` or `finalize()`. So it isn't
    the JLS's member set, and the tables follow the JLS (`members/tests.rs`).
15. **Annotation values:** `@Ann(names = {"a", "b"})`'s `{...}` is `NEW_ARRAY` of the element's type
    `java/lang/String[]`, as is a default `{}`; a single value for an array element is itself
    (`@SuppressWarnings("x")`'s literal is `java/lang/String`).

## Results

`cargo oracle attribute` on every locked project, on 3 November 2026, on a 10-core Mac shared with
other work, four threads, each file with fuel far above any body's so that its use is measured.
*Other* would be a form the walk doesn't know; *Calls* wait for W3.

| Project | Files | Bodies | Panics | Expressions | Typed | Calls | B3c | B3d | Error | Other | Time | Largest arena | Member tables | Constants |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| caffeine | 1,309 | 20,132 | 0 | 291,607 | 156,662 | 125,722 | 9,214 | 9 | 0 | 0 | 0.1 s | 577 | 107 | 2,650 |
| guava | 1,597 | 32,227 | 0 | 428,135 | 272,827 | 150,681 | 4,624 | 0 | 3 | 0 | 0.2 s | 1,326 | 117 | 2,822 |
| helidon | 4,901 | 70,866 | 0 | 1,041,112 | 612,362 | 396,365 | 32,093 | 292 | 0 | 0 | 0.4 s | 299 | 37 | 15,749 |
| hertzbeat | 1,384 | 12,517 | 0 | 196,917 | 113,814 | 79,384 | 3,626 | 13 | 80 | 0 | 0.1 s | 584 | 12 | 1,940 |
| jedis | 1,080 | 19,104 | 0 | 370,686 | 228,236 | 139,961 | 2,489 | 0 | 0 | 0 | 0.1 s | 493 | 11 | 1,419 |
| lucene | 6,239 | 68,418 | 0 | 2,117,479 | 1,604,380 | 504,453 | 8,567 | 79 | 0 | 0 | 0.8 s | 327 | 431 | 14,114 |
| spring-data-jpa | 901 | 19,753 | 0 | 199,849 | 135,773 | 62,330 | 1,736 | 10 | 0 | 0 | 0.1 s | 4,289 | 50 | 1,827 |
| total | 17,411 | 243,017 | 0 | 4,645,785 | 3,124,054 | 1,458,896 | 62,349 | 403 | 83 | 0 | 2.0 s | 4,289 | 765 | 40,521 |

- **Fuel per body:** median 1, p90 9, p99 42, p99.9 108, p99.99 215, most 709 (`LatLonBounds` in
  lucene's spatial3d); no body of 243,017 is above 10,000, about 14 times the most. That is
  without overload resolution, so it is a floor for what B3b will use.
- **Time per file** (attribution alone, its names and project-wide entries first, except the
  member tables and constants a file is the first to read): median 0.04 ms, p90 0.26 ms, p99 0.94
  ms, p99.9 2.52 ms, most 78 ms (`UAX29URLEmailTokenizerImpl`, 1.8 MB with 250,000 expressions); the
  next is 17 ms (`HqlParser`). The whole run, loading every project's engine and resolving its
  names, took 12 s (with the JSON dump). javac's `ANALYZE` of the same corpus sums to 147 s, with
  method resolution, inference, and flow, which attribution doesn't do yet.
- **Arenas:** 4,289 types at most (`HqlParser`), 1,326 in hand-written code (guava's `Maps`).
- **Entries:** 765 member tables, since only fields of expressions' types read them until W3, and
  40,521 constant entries.
- **Errors:** 83 expressions, all with names that don't resolve: 80 in hertzbeat, variables of the
  builder classes Lombok generates, which B4 adds, and 3 in guava's `AbstractFutureTest`, on
  `java.lang.classfile`, which its JDK 21 lacks (javac reports that file). Nothing in the corpus
  is `Unknown(Limit)` or `Incomplete`.
- **The edit fuzzer** (`semantic_edits`, whose differential check now attributes the files it asks
  about and compares each with a fresh engine's, among new fragments that make constants and
  conversions) ran 180 s locally on the final code without a failure, and so did `relations`. The
  deep inputs (`tests/deep.rs`) attribute on a 256 KiB stack, with chains of 20,000 member tables
  and 5,000 constants.
- **Other comparisons:** `cargo oracle names` still matches (4,193,632 matched, 0 mismatched, 5,847
  known), and `cargo oracle types --relations` too (2,156,986 matched, 0 mismatched).

**A rough comparison with javac,** not W4's: each file's expressions by range, against the cached
output of `javac-oracle types`, with a script of about a hundred lines, no normalization but UTF-16
offsets, and this document's tree shapes uncounted. Of javac's 4,447,710 expressions in the compared
files:

| | Expressions |
| --- | --- |
| Ferrion's type the same | 2,891,732 |
| Differing, all only by byte and UTF-16 offsets in a `local` name in a non-ASCII file, which W4's adapter converts | 59 |
| Waiting for W3 | 1,393,390 |
| Left to B3c, B3d | 59,342, 395 |
| No Ferrion expression at the range: `PARENTHESIZED` 97,883, folded `STRING_LITERAL` 2,767, implicit `NEW_CLASS` 1,968; the 174 others are in a few of helidon's generated files, whose text changed when processors ran again after the cached run | 102,792 |

So no type Ferrion gave differs from javac's. Of 849,929 constants, 843,483 are compared and
all match, none extra; the rest are at ranges with no Ferrion expression or waiting for W3. Every
variable's type with an answer matches too, but for 18 that differ by the same offsets in `local`
names. (The parameters of implicit constructors, an anonymous class's or a record's, which javac
lists, have no Ferrion variable.)

What the run doesn't say: it doesn't check `conversions` (javac doesn't report them; tests do),
or anything W3 or W4 will: chosen methods, or the comparison's normalization and known differences.
javac's stress tests (`jdk-tests`) weren't attributed; `oracle attribute` takes corpus projects only.

## Questions for review

Of the overview's [questions](semantic-types.md#questions-for-review), 1, 4, and 5 are this
document's. One more:

1. **Per-file results:** attribution results are per file, as body names are, and split per body
   only if W4's edit benchmark shows the language server needs it.

W2's questions, for B3b's review stop, each with a recommendation:

2. **Fuel per body:** the budget stays `BODY_FUEL`, equal to `DEFAULT_FUEL`'s 10,000, against a
   corpus maximum of 709 without calls. Overload resolution will use more, so W3 or W4 should
   measure again before B3c's review sets the limits. Recommended: keep 10,000 per body until then.
3. **Out of fuel,** the rest of the body's queries are `Unknown(Limit)`, rather than the current
   expression's alone with some new allowance for the next. Recommended: as built; it is simple
   and deterministic, and no corpus body comes near it, so it only has to be safe.
4. **`Type::Union` only recorded and printed,** with the `lub` as the type relations see,
   rather than a type relations handle. Recommended: as built; JLS 14.20 says the parameter's type
   for everything else is the `lub`, and every corpus use agrees.
5. **A `case` name on a selector without a type waits,** rather than keeping B2's guess: until W3
   types the selector, it has no constant. Recommended: as built; W3 settles them. A selector's
   type is the only thing that can settle B2's `case-label` difference, and for selectors the walk
   already types, it does (`resolved`).
6. **Member tables per view for class files:** a class file's table is computed per source set,
   since its names resolve per view. Only 765 tables exist in the corpus now, since only fields of
   expressions' types read them; W3's method lookups will make many more, and helidon has 376
   views. Recommended: keep it, and have W3 measure the count; tables of views with identical
   classpaths could share one if memory says so.
7. **`oracle attribute` and its `--dump`:** keep them beside W4's comparison? Recommended: yes,
   for what it measures without javac; the dump is what found this document's observations 5 to 9
   and 13.
8. **Inside an anonymous class made with `<>`,** its supertype is `LeftToB3c`, so `super` and
   inherited members are non-answers until B3c. Recommended: as built.
