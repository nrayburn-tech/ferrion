# Type relations (B3a)

B3a builds the relations between types that every later part of B3 asks: subtyping, containment,
capture, conversions and the contexts that allow them, erasure, the members of a type as seen
from a parameterized type, and least upper bounds. It has no expressions yet; it is checked
against javac by asking both the same questions about the corpus's declared types. The overview
is [type attribution](semantic-types.md).

## Decision

- **Types in an arena:** a `TypeId` into a hash-consed arena owned by one computation, translated
  from B2's `Ty` on first use and memoized there ([the arena](#the-arena)).
- **Three-valued answers:** every relation is `Yes`, `No`, or `Unknown`, and anything that touches
  an error type, an unknown type, or a missing part of a hierarchy answers `Unknown` unless the
  parts that are known already settle it ([unknowns](#unknowns)).
- **Correct programs first:** Ferrion isn't a compiler, and code javac rejects is left out of every
  comparison. B3a implements each relation exactly where its `No` changes how a correct program
  is typed (applicability, which drives overload choice, needs strict and loose invocation
  contexts exactly); where a relation only decides whether a program is legal (most of casting's
  legality, 5.5), it gives the kind of conversion for correct programs and `Unknown` for what it
  doesn't decide.
- **javac where it departs from the JLS,** each departure an observation with its input
  ([observations](#observations)).
- **Fuel:** every recursive relation draws on its computation's step budget, so expansive
  inheritance, which makes subtyping undecidable in general, ends in `Unknown(Limit)` rather than
  a hang ([termination](#termination)).

## The arena

```rust
/// A type in one computation's arena. Two equal types have equal ids, except capture
/// variables, which are fresh each time.
pub struct TypeId(u32);

pub enum Type {
    Primitive(Primitive),
    Null,
    /// A class or interface type. `outer` is the type it is a member of, when that type is
    /// parameterized (written, or implied by where the type is used inside a generic class);
    /// `args` is `None` for a non-generic class and for a raw type, which `TypeEntry`'s type
    /// parameters tell apart.
    Class { key: TypeKey, outer: Option<TypeId>, args: Option<ArgList> },
    Array(TypeId),
    /// A declared type variable: B2's `TypeVarOwner` and index.
    Var(TypeVarOwner, u32),
    /// A capture variable (5.1.10); its bounds are in the arena's side table, set after it is
    /// made, since they can name it (`Enum<?>`'s capture is bounded by `Enum<CAP#1>`).
    Capture(u32),
    /// `A & B & ...`, in the order the oracle compares (see the oracle's format).
    Intersection(TypeList),
    /// Depends on a name that didn't resolve, or an incomplete hierarchy: the root cause.
    Error(ErrorId),
    /// Ferrion can't type it yet: left to a later milestone, or a limit hit.
    Unknown(Why),
    // B3c adds inference variables.
}

pub enum TypeArg {
    Type(TypeId),
    /// `?`, `? extends T`, `? super T`.
    Wildcard(Option<(BoundKind, TypeId)>),
}
```

- **Hash-consing:** each `Type` and argument list is interned in the arena's map, so equality of
  types (the JLS's "same type", once both are in canonical form) is equality of ids, and memo
  tables key on ids.
- **Translation from `Ty`:** a `Ty` from a type entry or a class entry is translated in a view:
  `Binary` parts resolve through `Snapshot::resolve_binary` in that view (a class file's names are
  the same for every view, the classes they name aren't); `UnknownTypeVar` and `Error` become
  `Error` with their root cause; a member class of a generic class used without a qualifier, inside
  that class, gets the enclosing class's own type variables as its `outer`. Translation is memoized
  per (`Ty`'s owner, index) in the arena.
- **Type variables' bounds** are read from their owner's entry when first needed, not when the
  variable is made, so a variable bounded by itself (`T extends Comparable<T>`) translates in one
  step.
- **Arena size:** an arena only grows. A file's arena is dropped with its result; a body's
  intermediate types (capture variables made for one call) stay until then. If a file's arena
  grows past a budget, that file's remaining bodies get `Unknown(Limit)`; W1 measures the corpus's
  largest and sets the budget with the time limit.

## The relations

Each takes and returns ids in one arena, and memoizes its answers there (subtyping and
containment by pair, capture and erasure by type). Section numbers are the JLS's (Java SE 27).

| Relation | JLS | Notes |
| --- | --- | --- |
| `subtype(S, T)` | 4.10 | Primitives (4.10.1); class types by finding `T`'s class among `S`'s supertypes, substituted, then containment of each argument (4.10.2); a parameterized type with wildcards through its capture; raw types; arrays (4.10.3, with `Object`, `Cloneable`, `Serializable`); type variables and capture variables through their bounds; intersections on either side; `null` below every reference type. |
| `contains(A, B)` | 4.5.1 | Type arguments, wildcards against wildcards and types. |
| `same(S, T)` | 4.3.4 | Id equality after canonical form; capture variables only equal themselves. |
| `capture(T)` | 5.1.10 | Fresh variables with `glb` of the declared bound and the wildcard's bound as upper bound; idempotent on a type without wildcards. |
| `erasure(T)` | 4.6 | |
| `direct_supertypes(T)` | 4.10.2 | Substituted from the class's entry; erased for a raw type (4.8). |
| `as_super(T, C)` | 4.10.2 | `T`'s supertype whose class is `C`, substituted, if any: the step `subtype`, member lookup, and B3c's inference all use. |
| `member_type(T, member)` | 4.5.2, 4.8, 4.9 | A field's or method's type as seen from `T`: substituted for a parameterized `T`, erased for a raw `T`, through bounds for a type variable or intersection. |
| `subsignature(m, n)`, `override_equivalent` | 8.4.2 | Erasure counts, as for raw types. |
| `return_substitutable(m, n)` | 8.4.5 | |
| `lub(T1..Tn)`, `glb(T1..Tn)` | 4.10.4, 5.1.10 | Below. |
| `upward_projection(T)` | 4.10.5 | For `var` (B3b where the initializer is standalone). |
| Conversions | 5.1 | Below. |

### Least upper bounds

`lub` follows 4.10.4: the erased candidate set, its minimal elements, and the least containing
invocation of each generic candidate, with `lub` of type arguments recursive. The infinite types
4.10.4 allows are where javac departs: javac computes one level more than the operands and then
writes `Comparable<?>` ([the oracle's observation 1](types-oracle-normalization.md#observations)).
Ferrion cuts off at the same depth, so its types print the same.

The order of an intersection's parts is part of the type's printed form. javac puts the class
first, then interfaces deepest first (the longest chain of superinterfaces), then by qualified
name, whichever operand comes first (observation 2, from four inputs). Ferrion orders `lub`'s and
`glb`'s results by that rule, and W1 observes it on more inputs: interfaces of equal depth in
different packages, and a class deeper than the interfaces.

### Conversions and contexts

A conversion is returned as a value, not a boolean, so attribution and the linter can see which
steps it took (rules such as `no-boxed-primitive-equality` need to know an unboxing happened):

```rust
pub enum Conversion {
    Identity,
    WideningPrimitive, NarrowingPrimitive, WideningNarrowingPrimitive,
    WideningReference,
    /// Allowed only in casts.
    NarrowingReference,
    Boxing, Unboxing,
    Unchecked,
    String,
}

/// `Yes` with the conversions in the order applied, `No`, or `Unknown`.
pub fn convert(&mut self, context: Context, from: TypeId, to: TypeId, constant: Option<&Constant>)
    -> Tri<SmallVec<[Conversion; 2]>>;

pub enum Context { Assignment, StrictInvocation, LooseInvocation, String, Casting, Numeric }
```

- **Assignment** (5.2), including the narrowing of a constant expression of type `int` to `byte`,
  `short`, or `char` when its value fits, which is why the constant is passed (B3b's constant
  folding provides it).
- **Strict and loose invocation** (5.3), exactly, since overload resolution's phases are built on
  them.
- **Numeric contexts** (5.6): unary and binary promotion, with unboxing.
- **Casting** (5.5): the kind of conversion for a cast in a correct program; legality is decided
  only as far as typing needs it, and otherwise `Unknown`. Provably distinct parameterized types
  (4.5) are implemented, since a reference cast's kind depends on them.
- **String conversion** (5.4), for `+`.

## Unknowns

`Tri<T>` is `Yes(T)`, `No`, or `Unknown(Why)`. The rules:

- an `Error` or `Unknown` type on either side of any relation gives `Unknown`, with its root cause
  as the reason;
- a class whose hierarchy is incomplete (B2's `TypeEntry::incomplete`, or its `Supertypes` with an
  error) gives `Yes` when the answer is found through the supertypes that did resolve, and
  `Unknown` where `No` would need the missing ones;
- a Lombok-annotated type, until B4, keeps its supertypes, which Lombok doesn't change, but a
  lookup of a member it might generate is `Unknown`
  ([attribution](semantic-attribution.md#member-tables));
- `lub` of a set containing a non-answer type is that type, and `glb` likewise.

## Termination

Subtyping between parameterized types can recurse without end when inheritance is expansive
(a class whose supertypes mention it with a growing argument, `class C<X> extends D<D<C<X>>>`).
Ferrion doesn't try to recognize this: every relation's recursion takes one unit of fuel per step,
from the computation's budget, and a query that runs out answers `Unknown(Limit)` and memoizes
nothing it was computing. Re-entry into a query already being computed (the same pair asked
again further down) is a cycle and answers `Unknown(Limit)` at once, so cycles cost no fuel.
`lub`'s recursion is bounded by its cut-off depth, and translation by the `Ty` it reads.

## Checking

- **The relations oracle** (W0's relation queries, in the same `javac-oracle types` run, compared
  alone with `cargo oracle types --relations`): javac's answers, through
  `javax.lang.model.util.Types`, to questions about types taken from each corpus source set's own
  declarations (fields, parameters, results, supertypes; 48 per source set by default), sampled
  deterministically ([type relations](types-oracle-normalization.md#type-relations)). Ferrion's
  side rebuilds each type from the declaration site the oracle names, asks the same question, and
  compares. `lub` isn't in that API, so it is compared through the types of conditional
  expressions in the `types` dump whose operands are names or literals B2 already types; W1
  leaves that to W4 ([departures](#departures)).
- **The API's own departures:** `directSupertypes` gives `int[]` one intersection rather than three
  types, lists `Object` among an interface's supertypes even when it has superinterfaces, and
  substitutes wildcards rather than capturing them; `asMemberOf` doesn't capture
  (observations 16 and 17, and [W1's 8 and 9](#observations)). That is the API, not how javac
  types programs. Ferrion's relations follow the JLS, and its side of the comparison
  (`tools/oracle-check/src/types/ferrion.rs`) converts their answers to the API's shape: an
  array's supertype as one type, `Object` added first for an interface, a type variable's bounds
  after `Object` when the first is an interface, a member type or supertype substituted without
  capture (printed with each capture variable as the wildcard it captured).
- **Tests:** the JLS's examples for each section above, and each observation, as tests in
  `ferrion_semantic` with javac's answer written beside them.
- **Properties:** on types generated from a small fixed hierarchy (generic classes, F-bounds,
  inner classes of generic classes, arrays, wildcards), subtyping is reflexive and transitive
  wherever all three answers are `Yes`; `lub` is a supertype of each operand; `capture` of a
  captured type is the same type; every query stops within its fuel.
- **Fuzzing:** a `relations` fuzz target builds that hierarchy and random queries from its bytes,
  and checks the properties and that nothing panics.

## Departures

What W1 built differs from the design above in these ways, each for a reason given:

- **`Steps`, not `SmallVec`:** `convert` returns `Tri<Steps>`, the conversions in an inline array
  of at most three (a widening reference conversion, an unboxing, a widening primitive
  conversion), since the workspace has no `smallvec` and three suffice.
- **The constant** `convert` takes is the value of a constant expression of type `int`, `short`,
  `char`, or `byte`, as an `Option<i32>`: the only constants assignment's narrowing reads. B3b's
  `Constant` gives it.
- **`capture` is memoized per type,** so relations asked twice of one type agree, and
  `fresh_capture` makes new variables each time, for attribution, where each expression's capture
  is its own.
- **A method's result** with none is `Type::Void`, so a method type is types throughout; `Why` is
  `Limit`, `Later`, `Incomplete(key)`, or `Error(id)`.
- **Fuel** is a context's budget, set with `refuel(budget)`: the comparison gives each query
  `DEFAULT_FUEL`, 10,000 steps, far above the 145 any corpus query used
  ([results](#results)); W2 gives each body its own, and B3c's review sets the limits.
- **`lub` captures** its operands and each parameterization it combines, and **`glb` compares
  classes erased**, as javac does ([observations 4 and 5](#observations)); JLS 4.10.4's
  `lcta` rules for wildcards are never reached, and one of them (`lcta(? extends U, ? super U) =
  U`) isn't above its operands, which the fuzz target found.
- **A raw qualified inner class** (`Outer.Inner`, inside a subclass of `Outer<String>`)
  translates as an unqualified `Inner` would there, `Outer<String>.Inner`, where javac makes it
  raw ([observation 11](#observations)): B2's `Ty` records an outer type only when it is written
  with arguments, so it can't tell the two apart. Not seen in the corpus; B2's `TyPart::Class`
  could say that a qualifier was written, if attribution meets one.
- **Lombok:** a Lombok-annotated type's relations are its own, since Lombok doesn't change
  supertypes, and member lookups are W2's member tables. In the comparison, a query that needs a
  class or member Ferrion can't find, where a Lombok-annotated class's annotations make it (a
  `@Builder`'s builder, a getter), answers `left-to-b4 lombok` and is counted apart.
- **`lub` through conditional expressions isn't compared yet.** Typing the operands needs
  locals', fields', and parameters' declared types and literals' types, and JLS 15.25 to tell a
  standalone reference conditional from a poly one (javac reports the target type of a poly
  one, observation 5), which is more than a few lines of B2's names; W4 compares it with
  attribution. `lub` is checked by its tests (W0's observations 1 to 3, W1's 1 to 4) and the
  fuzz target.
- **W2 changed capture, upward projection, and outer types** as attribution observed javac
  ([W2's departures](semantic-attribution.md#departures)): `? super Object` captures to `Object`,
  the outer type is captured too, upward projection has JLS 4.10.5's declared-bound rule, and a
  written outer type is normalized to the class declaring the member.
- **Two fixes outside `types`:** `Snapshot::resolve_binary` missed a source type whose name no
  summary had interned yet (it looked the name up rather than interning it), so a context made
  before any summary was read found no source types; and another source set's entries name
  class files by the keys their own view found, so translation resolves such a name again in
  the context's view, as javac compiling this source set does (a dependency's `Comparable`,
  compiled against another platform, is this platform's here).

## Observations

Run with Temurin 27.0.1's `javac`, in a scratch directory: the types oracle's types of variables
and conditionals, and `javax.lang.model.util.Types` and `Elements` asked directly. Each is a test
in `crates/ferrion_semantic/src/types/tests.rs`, with javac's answer beside it.

1. `lub(Integer, String)` printed in a diagnostic: `INT#1 extends
   Object,Serializable,Comparable<? extends INT#2>,Constable,ConstantDesc`, with `INT#2` the same
   with `Comparable<?>`. The diagnostic adds `Object` as a first part; the API's type, which the
   oracle compares, doesn't.
2. **The order of an intersection's parts** on more inputs, all as W0's observation 2 has it
   (class first, interfaces deepest first, then by qualified name, whichever operand is first):
   `z/Base3&a/Ia&b/Ia` for two classes implementing `b.Ia, a.Ia` and `a.Ia, b.Ia` (interfaces of
   equal depth in two packages); `z/Base3&z/Deep3&z/Zed` (a class deeper than the interfaces,
   `Deep3` three interfaces deep); `z/Mid&z/Zed`, `z/Deep1&z/Zed`, `z/Base2&z/Zed`;
   `java/io/Serializable&java/lang/CharSequence&java/lang/Comparable<...>` for
   `lub(StringBuilder, String)`.
3. **A nested `lub` is cut off by its operands:** `b ? listOfInteger : listOfString` is
   `java/util/List<? extends ` and observation 1's whole type `>`, and with `List<Long>`,
   `List<? extends Number&Comparable<? extends Number&Comparable<?>&Constable&ConstantDesc>&...>`:
   the same operands are computed while they are on the stack at most twice, however deep the
   first is; a third time, the argument is `?`. `lub(String[], Integer[])` is observation 1's
   type with `[]`, and `lub(int[], long[])` `java/io/Serializable&java/lang/Cloneable`.
4. **`lub` captures its operands, and each parameterization it combines:** `b ? superInteger :
   extendsInteger` (`List<? super Integer>`, `List<? extends Integer>`), with `var`, is
   `java/util/List<? extends java/lang/Object>`, as are `b ? List<? super Number> : List<? super
   Integer>` and `b ? List<? super Comparable<? super C>> : List<String>`; `b ? List<? extends
   Number> : List<? extends Integer>` is `java/util/List<? extends java/lang/Number>`. JLS
   4.10.4's `lcta` would give `List<Integer>`, `List<? super Integer>`, and `List<? super
   String>` for the first three, and the first isn't above `List<? super Integer>`.
5. **`glb` compares classes erased** and orders as `lub` does, in capture (`Types.capture`):
   `J<? extends Dml>` with `J<S extends DbS<S>>` and `Dml extends DbS<Dml>` is `J<capture#1
   extends Dml>`, and `G<? extends Integer>` with `G<T extends Comparable<T>>` `G<capture#1
   extends java/lang/Integer>`, where JLS 5.1.10's glb keeps `DbS<capture#1>` and
   `Comparable<capture#1>`; `M<? extends Comparable<Integer>>` with `M<T extends Number>` is
   `capture#1 extends java/lang/Number&java/lang/Comparable<java/lang/Integer>`, `M3<? extends
   Comparable<String>>` with `M3<T extends Serializable>` `capture#1 extends
   java/io/Serializable&java/lang/Comparable<java/lang/String>`, and `H<? extends Deep1>` with
   `H<T extends Base & Zed>` `capture#1 extends z/Base&z/Deep1&z/Zed`. Of two parameterizations
   of one class, the one below the other stays (`DbClientMetricBuilder<capture#1,capture#2>` over
   a wildcard's `DbClientMetricBuilder<?,?>`, in helidon).
6. **An interface's method doesn't override `Object`'s protected one:** `Elements.overrides` is
   false for `Cl.clone()` over `Object.clone()` in `interface Cl { Cl clone(); }` (and for
   `javax.naming.Name.clone()`), and true for `Cl.equals(Object)` over `Object.equals`: an
   interface has `Object`'s public methods only (JLS 9.2). A static method, and a private one,
   override nothing; `p(String)` in `B extends A<String>` overrides `A.p(T)`.
7. **`?` and `? extends Object` are the same argument:** `isSubsignature` of
   `Properties.computeIfAbsent(Object, Function<? super Object, ?>)` and `Hashtable`'s, as
   members of `Properties` (`(Object, Function<? super Object, ? extends Object>)`), is true.
8. **`directSupertypes`' shapes,** which correct W0's observation 16 for arrays: `int[]`'s and
   `Object[]`'s is one intersection, `java/io/Serializable&java/lang/Cloneable`, without
   `Object`; any other array's is the array of its component's first supertype:
   `java/lang/Number[]` for `Integer[]`, `java/lang/Object[]` for `String[]`, an interface's
   array, and `List<String>[]`, `java/lang/Enum<E>[]` for an enum's array, and
   `java/io/Serializable&java/lang/Cloneable[]` for `Object[][]`. A type variable's are its bounds,
   after `java/lang/Object` when the first bound is an interface (`[Object, Data]` for `B extends
   Data`, `[JsonValue]` for a class bound, `[Object]` unbounded); a primitive type's are none.
9. **`asMemberOf`'s substitution of a wildcard,** for `Box<? extends Number>` and `List<? extends
   List<? extends Number>>`: a type parameter where a type is written becomes the wildcard
   itself (`()? extends java/lang/Number`, `()? extends java/lang/Number[]`); inside `? super`,
   `? super ` and the wildcard (`List<? super ? extends java/lang/Number>`); inside `? extends`,
   the wildcard's bound for `? extends X` (`List<? extends java/lang/Number>`), `? super X`
   itself (`? extends ? super java/lang/Integer`), and for `?` its type parameter's declared
   bound, unsubstituted (`? extends java/lang/Number` for `G<T extends Number>`, `? extends
   java/lang/Comparable<T:H>` for `H<T extends Comparable<T>>`, `? extends java/lang/Object`
   unbounded). `directSupertypes` substitutes the same way (`Comparable<?>` for `Enum<?>`).
10. **A raw type's members are erased, its static ones aren't:** a raw `List`'s `toArray(T[])` is
    `(java/lang/Object[])java/lang/Object[]`, its `of(E)` `<E:java/util/List.of(...)>(E:...)...`
    as declared; a raw `Outer.Inner`'s field `T t` is `java/lang/Object`.
11. **An inner class of a generic class** used without a qualifier inside the class is
    `Outer<T:Outer>$Inner`, inside `Q extends Outer<String>` `Outer<java/lang/String>$Inner`, and
    `Outer.Inner` written in a class that isn't a subclass is raw, `Outer$Inner`; a static
    nested class's is `Outer$Nested`. `Outer.Inner` inside `Q` is raw for javac too.
12. **javac's answers name classes its classpath lacks:** `error opennlp.tools.util.model.SerializableArtifact`
    among a type's supertypes, `(...)error com.typesafe.config.ConfigMemorySize` as a member's
    type, and method keys whose descriptors name such a class with dots
    (`builder()Lcom.google.common.collect.ImmutableSortedSet.Builder;`), where Ferrion's view
    has the class (a jar a dependency names that javac's compilation doesn't have, as in W0's
    "error types in clean files"). Those queries are counted apart, not compared.

W0's observations in [types oracle normalization](types-oracle-normalization.md#observations) are
the rest of this milestone's: 1–3 (`lub`), 7 (capture's bounds), 9 (upward projection), 16–18
(the API's supertypes, members, and subsignatures), each a test too.

## Results

`cargo oracle types --relations` on every locked project (W0's cached javac side, each project on
its lock's JDK), on 2 November 2026, on a 10-core Mac shared with other work:

| Project | Source sets | Queries | Matched | Left to B4 | javac unresolved | javac threw | Mismatched | Ferrion | Most fuel | Largest arena |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| caffeine | 29 | 138,888 | 138,802 | 0 | 54 | 32 | 0 | 0.9 s | 78 | 619 |
| guava | 6 | 30,925 | 30,925 | 0 | 0 | 0 | 0 | 0.5 s | 47 | 563 |
| helidon | 376 | 1,382,963 | 1,382,802 | 0 | 98 | 63 | 0 | 3.1 s | 128 | 449 |
| hertzbeat | 46 | 171,875 | 148,905 | 22,712 | 194 | 64 | 0 | 2.0 s | 115 | 344 |
| jedis | 2 | 15,204 | 15,203 | 0 | 0 | 1 | 0 | 0.4 s | 145 | 317 |
| lucene | 75 | 409,828 | 409,790 | 0 | 25 | 13 | 0 | 2.2 s | 64 | 297 |
| spring-data-jpa | 4 | 30,596 | 30,559 | 0 | 28 | 9 | 0 | 0.4 s | 75 | 549 |
| total | 538 | 2,180,279 | 2,156,986 | 22,712 | 399 | 182 | 0 | 9.4 s | 145 | 619 |

- **Categories:** no mismatches, so no known differences. *Left to B4* are hertzbeat's queries
  that need a class or member Lombok makes (its `@Builder`s' builder classes, sampled as method
  results); *javac unresolved* are about a type javac couldn't resolve (observation 12); *javac
  threw* are W0's blocks lost to class files javac can't read.
- **Time and memory:** Ferrion's own time, summed over source sets, is each project's engine
  loaded (its inputs, and the entries the queries read) and the queries answered: 9.4 s for 2.2
  million queries. The whole run, javac's side read from the cache (514 MB of JSON), took 13.8 s
  with two source sets at a time, and its largest resident size was 743 MB, which holds a
  project's engine and the source sets' oracle output.
- **Fuel:** the most any query used is 145 steps (`AbstractTransaction <: MultiDbTransaction` in
  jedis's tests), so the provisional `DEFAULT_FUEL` of 10,000 is about seventy times above the
  corpus. The `relations` fuzz target gives each query 20,000, and a query over expansive
  inheritance stops there, `Unknown(Limit)`.
- **Arenas:** one context per source set holds every query's types: at most 619 types, in one of
  caffeine's source sets. A file's arena with attribution is W4's to measure, with the budget the
  design asks for.
- **The javac-oracle jar** is W0's (SHA-1 `e61ea3af7772dc7df0801dfe5f140dd9e8b76efa`); the PR job
  now compares jedis's relations with Ferrion's side (`cargo oracle types --relations jedis`),
  and so does the weekly oracle workflow for every project.

## Questions for review

Of the overview's [questions](semantic-types.md#questions-for-review), 2 and 3 are this
document's (accepted). One more (accepted):

1. **Casting legality:** B3a decides the kind of conversion of a cast in a correct program, and
   leaves legality `Unknown` where typing doesn't need it, since code javac rejects is never
   compared. The alternative is implementing 5.5's legality in full now, which only a future
   "this cast always fails" rule would use.

W1's questions, for B3a's review stop, each with a recommendation:

2. **Raw qualified inner classes** ([departures](#departures)): should B2's `TyPart::Class` record
   that a qualifier was written, so `Outer.Inner` inside a subclass of `Outer<X>` is raw as javac
   makes it? Recommended: in W2, and only if attribution meets the case; the corpus has none.
3. **Lombok:** keep queries that need a class or member Lombok generates counted apart as
   `left-to-b4 lombok`, rather than as known differences? Recommended: yes; B4 answers them.
4. **Types javac couldn't resolve:** keep queries whose javac answer holds an `error` type, where
   Ferrion's view has the class, counted apart rather than as known differences? Recommended: yes.
5. **The weekly oracle workflow** compares relations with Ferrion's side for every project, which
   W1's brief didn't ask for: keep it? Recommended: yes; it costs seconds over javac's side.
6. **Fuel:** keep `DEFAULT_FUEL` at 10,000 per query until W2 measures each body's use?
   Recommended: yes.

## Decisions made in review

At B3a's review stop (2 November 2026), questions 2 to 6 were accepted as recommended:

- **Raw qualified inner classes:** W2 makes B2's `TyPart::Class` record that a qualifier was
  written only if attribution meets the case.
- **Lombok:** queries that need a class or member Lombok generates stay counted apart, as
  `left-to-b4 lombok`.
- **Types javac couldn't resolve** stay counted apart, not as known differences.
- **The weekly oracle workflow** keeps comparing relations, with Ferrion's side, for every project.
- **Fuel:** `DEFAULT_FUEL` stays 10,000 per query until W2 measures each body's use.
