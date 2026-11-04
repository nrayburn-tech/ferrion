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
compared. W3's [results](#results-of-calls) have the share on each side.

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

What W3 built differs in these ways:

- **A creation's type is the class named,** whichever constructor is chosen, and whether or not one
  can be (JLS 15.9.1): `new Thread(() -> { })` is `java/lang/Thread`, with no method until B3c, rather
  than `Unknown(LeftToB3c)` as the line's words have it for a call. Its type doesn't depend on the
  choice. An error in it still makes it an error, as the scope's rule asks, and `<>` is B3c's.
- **`BodyTypes::unchosen`** has the creations, constructor calls, and enum constants with no method,
  with why: their own types can't say. `Call` has `special`, for an array's `clone()` and signature
  polymorphic methods. `Snapshot::call_text` gives javac's keys for a call.
- **Where a call is recorded:** by its node for one in the source; a constructor's implicit
  `super()` by its body's `Block`, a default constructor's by the type's declaration, an enum
  constant's creation by the `EnumConstant`. An anonymous class's creation has the superclass
  constructor it chooses, from which `call_text` gives javac's two calls
  ([observation 24](#observations)).
- **A poly conditional as an argument doesn't take the call out of the line,** as the line's words
  allow: `pr(b ? "a" : null)` is chosen by its operands, each tested in the phase's context. The
  conditional's own type is still B3c's. `Value::operands` keeps them, to a depth of 64, past which
  the call is B3c's too, so that dropping them can't recurse deeply.
- **`Why::LeftToB4`,** printed `left-to-b4 lombok`, where a type with a Lombok annotation on it or a
  supertype has an incomplete table, or a member isn't found in one: calls on such a type have no
  answer, as the design says of an incomplete table, and the comparison counts them apart. `Why::Calls`
  is gone. `Cause::NotApplicable` is new, for a call no method applies to.
- **`resolved`** also has a statically imported method's declaring type, as `Symbol::Methods`, which
  is what settles `static-import-overload`.
- **Changes to member tables,** each found by a corpus mismatch: a table's comparisons get their fuel
  per name (`Jedis`, with thousands of methods, ran out, and kept overridden ones), and a table out of
  fuel is incomplete; a local class's package is its top level class's (it had none, and inherited no
  package-private method); a table reads its supertypes' methods in its own view, since a supertype's
  table is of its own source set's (`Object` from another platform's view isn't this view's);
  in a class `Object`'s concrete method stays over an interface's abstract one, and in an interface
  one interface's replaces `Object`'s ([observation 23](#observations)). A default or canonical
  constructor has its class's access (JLS 8.8.9, 8.10.4), which the entry doesn't write.
- **Changes to W1:** unchecked conversion of arrays (`Class[]` to `Class<?>[]`, JLS 5.1.9), which
  `cargo oracle types --relations` hadn't met; a capture variable has a provisional bound before any
  `glb` of its siblings' ([observation 28](#observations)).
- **An anonymous class made through an outer instance** has no generic superclass in its entry, since
  its outer type is the instance's, which only the walk knows: the walk gives its context the
  superclass when the body is entered (`TypeContext::set_supertypes`), and its table is made from the
  class alone.
- **The adapter:** W4's waiting label and `IMPLICIT_CALLS_WAIT` are gone with what they waited for.

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

From W3, on Temurin 27.0.1 unless a release is named; the inputs of
`crates/ferrion_semantic/src/attr/call_tests.rs` are these, with javac's answers, but for what needs
the JDK's signatures (`getClass()` of a raw type), which the corpus comparison checks:

16. **The phases and most specific follow the JLS:** with `s1(int)`, `s1(long)`, `s1(Object)`,
    `s1(int...)`: `s1(1)` `(I)V`, `s1(boxed)` the `Object` one (strict beats unboxing), `s1()` and
    `s1(1, 2)` `([I)V`; `s2(1)` with `s2(Integer)`, `s2(long)` the `long` one; `obj(null)` of `obj(Object)`,
    `obj(Object...)` the array one; `ns()` of `ns(long...)`, `ns(int...)` the `int` one. `s3(boxed, 1)` of
    `s3(long, Object)`, `s3(Object, Object)` is `compiler.err.ref.ambiguous`, as are `bx(1, 2)` of
    `bx(Integer...)`, `bx(int, long...)` and `vs(1)` of `vs(int, int...)`, `vs(int...)`: no rule beyond
    15.12.2.5 for primitive and boxed parameters.
17. **The type as invoked** has formal parameters (`s1(1, 2)` `(int[])void`) and isn't captured itself:
    `w()` returning `List<?>` is `()java/util/List<?>`, of type `java/util/List<capture#1 extends
    java/lang/Object>`. `xs.get(0)` for `List<? extends Number>` is `(int)capture#1 extends
    java/lang/Number`, since the receiver is captured. An explicit type argument is substituted
    (`this.<String>id("a")` `(java/lang/String)java/lang/String`); on a non-generic method it is
    ignored (`"a".<String>length()` `()int`).
18. **Unchecked conversion** erases the result (`g(rawl)` for `List<String> g(List<String>)` is
    `(java/util/List<java/lang/String>)java/util/List`), but not to a type whose arguments are all
    unbounded wildcards, `? extends Object` among them, where JLS 15.12.2.6 would: `h(raw)` for
    `List<String> h(List<?>)`, `h2(raw)` for `Collection<? extends Object>`, `h4(rawc)` for `Class<?>[]`
    keep `java/util/List<java/lang/String>`; `h3(raw)` for `List<? extends Number>` and `h7(rawl)` for
    `List<String>[]` don't. The same for an explicit type argument's bound:
    `Comparator.<Comparable>naturalOrder()` is `()java/util/Comparator`, `Range.<C>rangeLexOrdering()`,
    `C extends Comparable` for a bound `Comparable<?>`, keeps its type (guava).
19. **A call that is an argument,** with a capture variable for its type, is recorded with its upper
    bound, captured: `g(xs.get(0))` is `java/lang/Number`, where `xs.get(0);` and `Number n = xs.get(0);`
    are `capture#1 extends java/lang/Number`; `Enum<capture#1 extends ...>` for a bound `Enum<?>`
    (guava's `Platform`), through parentheses, in `new`, `this(...)`, `super(...)`, and a generic
    method's arguments too. `gl(w())`'s `List<capture#1 ...>` keeps its capture. A field or array
    element as the argument keeps its capture (`g(bq.f)`), but as a conditional's operand in an argument
    doesn't (`g(b ? bq.f : null)`'s `bq.f` is `java/lang/Object`), and a call there is the same.
20. **`getClass()` and an array's `clone()`:** `getClass()` is `()java/lang/Class<?>` as invoked, of type
    `Class<capture#1 extends |T|>`: `Q` for `this` of a generic `Q`, `java/util/List` for `xs`,
    `java/lang/Object[]` for `T[]`, `int[]` for `int[]`. Of a raw interface type it is `()java/lang/Class`
    as invoked, of a raw class still `()java/lang/Class<?>`. `arr.clone()` is `method
    Array.clone()Ljava/lang/Object;`, `()java/lang/Object`, with no exceptions, of the array's type;
    an array's other methods are `Object`'s.
21. **Signature polymorphic methods** (JLS 15.12.3): `mh.invoke(null, ls)` is `(java/lang/Void,
    java/util/List)void` as a statement, with a type variable and its array staying, unlike class types
    (`(E:Q,E:Q[],int[])void`); `Object r = mh.invoke("a", 2)` `(java/lang/String,int)java/lang/Object`.
    A cast's type around it, through parentheses, is its result: as written up to `--release 23`
    (`(List<String>) mh.invoke()` `()java/util/List<java/lang/String>`, captured as its expression's
    type, `(E) ...` `()E:L`), and erased from 24 on (`()java/util/List`, `()java/lang/Object`), on JDK
    27 at every release, 25 and 26 too, a difference between source levels. A `for`'s init and update
    are statements. `invokeWithArguments`, which isn't native, is an ordinary method.
22. **Abstract methods inherited together** with one signature (JLS 15.12.2.5 leaves which to the
    compiler): in the order of an intersection's parts (a class, then interfaces deepest first, then
    by qualified name), with the first two swapped, the first whose return type is a subtype of the
    others'. `Ab`, `Zed` gives `Zed`, whichever is declared first; `Ab`, `Mid`, `Zed` gives `Mid`; `A1` to
    `E1` gives `B1`; `AA` (a class), `Ab`, `Zed` gives `Ab`; returns `Object`, `Object`, `String`, `String`
    give the first `String`; `Ret1.g()` returning `List<String>` over `Ret2.g()` returning `List`. Inferred
    from about 30 inputs. Two exceptions: a class's abstract method that overrides a concrete one above
    it wins outright (`B1.has` of `C1 extends B1 implements Lo`, with `B1`'s overriding `A1`'s; guava's
    `ImmutableCollection.contains`, where `Set.contains` would be next); and declarations whose
    erasures differ are ambiguous (`Gen2.put(String)` and `Gen1<String>.put(T)`, `compiler.err.ref.ambiguous`).
23. **`Object`'s methods and interfaces:** of `interface D extends Em, A`, with `A` declaring
    `hashCode()`, `d.hashCode()` is `A`'s (lucene's `CharTermAttribute.toString()`); of `interface C
    extends A, B`, both declaring it, `Object`'s (caffeine's `MutableMap`). In `abstract class I implements
    A`, `Object`'s, as JLS 8.4.8.4 has it. Of an interface with no such method, `Object`'s, by 9.2.
24. **Calls javac makes up,** with W0's observations 14 and 15: a compact constructor's `super()` at its
    `{`; a record's implicit canonical constructor's at `record`, `java/lang/Record.<init>()V`; an enum's
    `java/lang/Enum.<init>(Ljava/lang/String;I)V`, `(java/lang/String,int)void`. An anonymous class's
    constructor has the superclass constructor's parameters as invoked, erased: `new Base<String>("x") { }`
    is `local ...<init>(Ljava/lang/String;)V`, over `Base.<init>(Ljava/lang/Object;)V`,
    `(java/lang/String)void`; through an outer instance first its type, `b.new In<String>() { }`
    `<init>(LBase;)V`; of an interface `<init>()V` over `Object`'s. An enum constant with a body is the
    same, `PINK(1, 2) { }` `<init>([I)V` over `Color.<init>([I)V`.
25. **Accessibility:** `new Base(1) { }` reaches a protected constructor another package's `new Base(1)`
    doesn't, which chooses the public `long` one; `b.pick(1)` in a subclass in another package, through a
    `Base`, takes the public `long` one over the protected `int` one, where `s.pick(1)`, `this.pick(1)`,
    `super.pick(1)`, and `pick(1)` take the protected one.
26. **A reference conditional as an argument** is chosen by its operands, and its own type is the
    target's: `println(b ? "a" : "b")` the `String` one, `println(b ? 1 : "b")` the `Object` one, by
    boxing, in the second phase; `println(b ? 'a' : 1)`, a numeric conditional, the `char` one.
27. **Through an outer instance:** `new From<Integer>() { }.new To<String>()` is
    `From<java/lang/Integer>$To<java/lang/String>`, its outer type found through the anonymous class;
    `new From<Integer>().new To<String>() { }.a()` sees `F` as `java/lang/Integer` (guava's
    `TypeTokenTest`).
28. **Capture variables' bounds can read each other:** a variable `B<? extends B<?, ? extends R>, ?
    extends R>`, with `B<S extends B<S, T>, T>`, is `B<capture#1 extends B<capture#1,capture#2 extends
    R>,capture#2>`: its `glb` drops the wildcard's bound, once it can see that the second variable is
    below `R` (helidon's `RouterImpl`).

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
| No Ferrion expression at the range: `PARENTHESIZED` 97,883, folded `STRING_LITERAL` 2,767, implicit `NEW_CLASS` 1,968; the 174 others are in a few of helidon's generated files, whose text changed when processors ran again after the cached run ([W4 saw the cause](types-oracle-normalization.md#generated-text-that-changes)) | 102,792 |

So no type Ferrion gave differs from javac's. Of 849,929 constants, 843,483 are compared and
all match, none extra; the rest are at ranges with no Ferrion expression or waiting for W3. Every
variable's type with an answer matches too, but for 18 that differ by the same offsets in `local`
names. (The parameters of implicit constructors, an anonymous class's or a record's, which javac
lists, have no Ferrion variable.)

What the run doesn't say: it doesn't check `conversions` (javac doesn't report them; tests do),
or anything W3 or W4 will: chosen methods, or the comparison's normalization and known differences.
javac's stress tests (`jdk-tests`) weren't attributed; `oracle attribute` takes corpus projects only.

**W4's comparison,** with the [adapter](types-oracle-normalization.md#ferrions-side-of-the-files-entries)
this rough one asked for, normalizes those shapes and the others W4 observed ([results per
project](types-oracle-normalization.md#the-attribution-comparison)): of the files' entries, 2,955,982
expressions, 847,362 constants, and 428,554 variables' types match, 0 differ, and Ferrion gives none
javac lacks; 2,762,716 entries wait for W3, and 567,527 are left to B3c and B3d (456,080 of them
variables' finality). What W2's walk types needed no fix.

### Results of calls

W3's, with the calls, on 3 November 2026, on the same machine, shared; `cargo oracle attribute` on
every locked project. *Calls* are the method invocations, creations, and explicit constructor calls
in the source; *chosen* of them have a method.

| Project | Files | Bodies | Panics | Expressions | Typed | B3c | B3d | B4 | Error | Incomplete | Limit | Calls | Chosen | Calls left to B3c | Calls left to B4 | Member tables | Their bytes | Time | Largest arena |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| caffeine | 1,309 | 20,679 | 0 | 291,607 | 233,968 | 57,630 | 9 | 0 | 0 | 0 | 0 | 104,589 | 61,324 | 43,265 | 0 | 3,935 | 15.5 MB | 0.9 s | 1,347 |
| guava | 1,597 | 35,481 | 0 | 428,135 | 366,963 | 61,163 | 0 | 0 | 9 | 0 | 0 | 145,531 | 89,309 | 56,216 | 0 | 2,989 | 7.4 MB | 0.8 s | 2,379 |
| helidon | 4,901 | 72,140 | 0 | 1,041,112 | 880,283 | 160,522 | 307 | 0 | 0 | 0 | 0 | 365,544 | 239,217 | 126,309 | 0 | 19,654 | 35.4 MB | 1.9 s | 830 |
| hertzbeat | 1,384 | 13,076 | 0 | 196,917 | 151,096 | 18,393 | 13 | 26,021 | 1,391 | 3 | 0 | 73,968 | 33,011 | 14,412 | 25,231 | 6,074 | 13.8 MB | 1.2 s | 716 |
| jedis | 1,080 | 20,118 | 0 | 370,686 | 325,505 | 45,181 | 0 | 0 | 0 | 0 | 0 | 137,673 | 94,817 | 42,856 | 0 | 1,283 | 3.5 MB | 1.0 s | 651 |
| lucene | 6,239 | 69,112 | 0 | 2,117,479 | 2,082,625 | 34,773 | 81 | 0 | 0 | 0 | 0 | 458,985 | 433,357 | 25,626 | 0 | 12,239 | 45.1 MB | 2.6 s | 627 |
| spring-data-jpa | 901 | 19,795 | 0 | 199,849 | 185,065 | 14,774 | 10 | 0 | 0 | 0 | 0 | 57,760 | 44,976 | 12,784 | 0 | 2,732 | 7.5 MB | 0.8 s | 5,223 |
| total | 17,411 | 250,401 | 0 | 4,645,785 | 4,225,505 | 392,436 | 420 | 26,021 | 1,400 | 3 | 0 | 1,344,050 | 996,011 | 321,468 | 25,231 | 48,906 | 128.1 MB | 9.2 s | 5,223 |

- **The inference line:** of 1,344,050 calls, 74.1% are chosen, 23.9% left to B3c, and 1.9% left to B4;
  91.0% of expressions are typed, 8.4% left to B3c. Most of what is left is a project's use of generic
  APIs: lucene's calls are 94.4% chosen, jedis's 68.9%, caffeine's 58.6%.
- **Fuel per body,** with calls: median 6, p90 55, p99 235, p99.9 527, p99.99 1,069, most 2,300
  (`ClimberInvariants` in caffeine); no body of 250,401 is above 10,000, about 4.3 times the most, and
  nothing in the corpus is `Unknown(Limit)`. Without calls the most was 709.
- **Member tables and memory:** 48,906 tables, against 765 before, each project's counted in its own
  engine; what they hold is 128 MB in all, 35 MB for helidon's 19,654 over its 376 views, about 52 per
  view and 1.8 KB per table, and 45 MB for lucene's 12,239. The whole run, one project's engine at a
  time, peaked at 767 MB. Dependencies aren't counted. Sharing tables between views with identical
  classpaths isn't needed for memory, and wasn't built.
- **Time:** 9.2 s of attribution in all, with the member tables and constants a file is the first to
  read, against 2.0 s without calls and javac's 147 s of `ANALYZE`; per file median 0.22 ms, p90 1.16
  ms, p99 4.76 ms, p99.9 18.12 ms, most 77 ms (`UAX29URLEmailTokenizerImpl`).
- **Errors:** the 1,400 are all in files javac reports errors in, and aren't compared: 1,391 and 3
  incomplete in hertzbeat's tests, 9 in guava's `AbstractFutureTest`.

**Against javac,** `cargo oracle types` at zero unexplained: of javac's 1,329,863 calls, 1,005,368 have
the same method, 324,493 are counted apart (323,015 left to B3c, 20 to B3d, 1,458 to B4), and 2 differ, known
([types oracle](types-oracle-normalization.md#the-comparison-with-calls)). Of those with a type as
invoked, all that have the same method have the same type but 3, known. Ferrion gives no call javac
lacks. `cargo oracle names` matches 4,192,580 names with 4,859 known, and `cargo oracle types
--relations` still matches every query.

**B2's two differences:** `case-label` is 42 of 1,012, each in a switch on a call that needs inference;
`static-import-overload` is 4,812 of 4,830, each a call with a generic candidate, 4,743 of them caffeine's
`assertThat`, which Truth's generic overloads keep from being chosen. The names comparison reads
attribution's answers (`BodyTypes::resolved`), and the list now has just those. B3c settles them.

**What else ran:** the edit fuzzer (`semantic_edits`, whose fragments now make calls and overloads) and
`relations` 180 s each on the final code without a failure; the deep inputs (`tests/deep.rs`), with a
new test of conditionals nested 5,000 deep as an argument. **What the run doesn't say:** the
removed-symbols check and javac's stress tests are W4's, and neither ran here; the conversions
recorded for arguments and `Call::phase` have no counterpart in javac's output, so only tests check
them, and not many; a call in a lambda's body is compared where its type needs no inference, like
any other.

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

W4's, for the same stop, on what the comparison needed:

9. **Text a processor writes differently from the same inputs:** helidon's `__ServiceDescriptor`s
   do, so a cached run can stand beside text it didn't see ([the
   cause](types-oracle-normalization.md#generated-text-that-changes)), and a miss in one source set
   can rewrite its text and make its dependents miss. Both comparisons now name such a file and
   leave it out, when javac's entries can't be trees of the text. Recommended: keep that check, and
   make the cache robust when oracle work next invalidates it anyway: a run after the first would
   compile the text already generated for those inputs, with processors off, rather than running
   them again, so that `names` and `types` always see one text and nothing cascades. It needs an
   oracle change and a new cache, so it isn't worth doing for this alone.
10. **Entries Ferrion gives where javac has none fail the run,** as mismatches do; the design counted
    them. Recommended: as built; each is a form the adapter doesn't know, or a tree Ferrion made up,
    and there are none now. Those in annotations on record components are counted apart, since
    javac's dropping them depends on `@Target`, which attribution doesn't read.

W3's, for the same stop:

11. **A creation's type without a method:** the class named, unless an error is in it, rather than the
    non-answer the spreading rule says of a call with a non-answer argument. Its type doesn't depend on
    the choice, and 3,730 creations keep their type. Recommended: as built.
12. **`BODY_FUEL`:** the most a body uses is 2,300 with calls, so 10,000 is 4.3 times that, not the ten
    the overview wants, and inference will use more. Recommended: leave it, since nothing reaches it,
    and set it at B3c's review as planned; if a number is wanted now, 25,000.
13. **Member tables per view:** 128 MB in all, 35 MB for helidon's 376 views. Recommended: no sharing,
    and measure again when B3c's calls read more of them.
14. **Calls on a Lombok-annotated type,** or one that inherits from it, have no answer until B4, even for
    a method the source declares: `Why::LeftToB4`, 25,231 calls, all hertzbeat's, 1,458 of them
    compared. Recommended: as built; choosing among the declared methods could be wrong by a generated
    overload, and B4 is close.
15. **The choice among abstract methods** follows [observation 22](#observations), a rule inferred from
    inputs, with no corpus mismatch; JLS 15.12.2.5 calls it arbitrary, and rules shouldn't care which
    was chosen. Recommended: keep it, since the comparison is by key.
16. **The signature polymorphic cast by release** ([observation 21](#observations)): Ferrion follows the
    source set's release. Only caffeine, at release 21 on JDK 26, shows the difference in the corpus,
    and helidon's at 27 the other way. Recommended: as built.
17. **The names comparison now attributes each file:** it is what settles the two categories, and costs
    its share of the 9 s. Recommended: keep it; nothing else reads both.
18. **The remaining `case-label` and `static-import-overload` entries** stay in `names-known.txt`, under
    explanations that say B3c settles them. Recommended: remove them with B3c's calls.
