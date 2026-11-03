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
4.10.4 allows are where javac departs: observation 1 in the [overview](semantic-types.md) shows
javac cutting the recursion off after one level, with `Comparable<?>` at the bottom. Ferrion cuts
off at the same depth, so its types print the same. W0's corpus run and the oracle's conditional
expression types give more cases; each departure found becomes an observation and a test.

The order of an intersection's parts is part of the type's printed form. javac printed `Object`
first, then the interfaces (observation 1); W1 observes the order on inputs with several classes
and interfaces and matches it.

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

- **The relations oracle** (W0's `javac-oracle relations`): javac's answers, through
  `javax.lang.model.util.Types`, to questions about pairs of types taken from each corpus source
  set's own declarations, sampled deterministically. Ferrion's adapter rebuilds each type from
  where the oracle says it was declared, asks the same question, and compares. `lub` isn't in that
  API, so it is compared through the types of conditional expressions in the `types` dump whose
  operands are names or literals B2 already types.
- **Tests:** the JLS's examples for each section above, and each observation, as tests in
  `ferrion_semantic` with javac's answer written beside them.
- **Properties:** on types generated from a small fixed hierarchy (generic classes, F-bounds,
  inner classes of generic classes, arrays, wildcards), subtyping is reflexive and transitive
  wherever all three answers are `Yes`; `lub` is a supertype of each operand; `capture` of a
  captured type is the same type; every query stops within its fuel.
- **Fuzzing:** a `relations` fuzz target builds that hierarchy and random queries from its bytes,
  and checks the properties and that nothing panics.

## Observations

Run with Temurin 27.0.1's `javac`, in a scratch directory. Each becomes a test in W1.

1. `lub(Integer, String)`: see observation 1 in [type attribution](semantic-types.md). The cut-off
   is after one level, and `Object` is the intersection's first part.

W0 records more in [types oracle normalization](types-oracle-normalization.md); W1 adds the ones
it needs here (intersection order, `glb` dropping redundant parts, capture of nested wildcards).

## Questions for review

Of the overview's [questions](semantic-types.md#questions-for-review), 2 and 3 are this
document's. One more:

1. **Casting legality:** B3a decides the kind of conversion of a cast in a correct program, and
   leaves legality `Unknown` where typing doesn't need it, since code javac rejects is never
   compared. The alternative is implementing 5.5's legality in full now, which only a future
   "this cast always fails" rule would use.
