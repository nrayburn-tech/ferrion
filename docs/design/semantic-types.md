# Semantic analysis: type attribution

B3 in the [roadmap](../ROADMAP.md) gives every expression in the corpus a type, every call a
chosen method, and every constant its value, on top of B2's
[names](semantic-names.md) and [engine](semantic-incremental.md). This document is the overview
of all four milestones: what B3 serves, the type model, how errors and limits spread, where the
code lives, how each milestone is checked against javac, the order of the work, and the questions
for review. Two milestones have designs of their own now; the other two get theirs, and a review
stop, when their turn comes.

| Milestone | Document | In short |
| --- | --- | --- |
| B3a, type relations | [Type relations](semantic-type-relations.md) | Types hash-consed in an arena per computation, translated from B2's `Ty`; subtyping, containment, capture, conversions and contexts, erasure, member types as seen from a type, least upper bounds; every answer `yes`, `no`, or `unknown`. |
| B3b, attribution and overloads | [Attribution](semantic-attribution.md) | A second walk over each body, after B2's names: standalone expression types, field and method lookups on expression types, overload resolution for calls that need no inference, constant folding, and member tables as shared entries. |
| B3c, inference | Later | JLS chapter 18, poly expressions, lambdas, method references, diamond, `var` from poly initializers. Its design is written when B3b closes. |
| B3d, patterns and flow | Later | Switch expressions and patterns, records and sealed types, exhaustiveness, definite assignment, effective finality, reachability. Its design is written when B3c closes. |
| The oracle | [Types oracle normalization](types-oracle-normalization.md) | `javac-oracle types`, with relation queries in the same run, and `cargo oracle types`: what javac reports, the type and constant string formats, what is compared, twenty observations, the corpus run. Written with the oracle, in W0. |

## What B3 serves

- **Linter rules that need types** (C3): `equals` on incompatible types, boxed primitives compared
  with `==`, ignored return values, format strings, narrowing compound assignments. Every one of
  them must stay silent on code whose types are unknown, so an unknown type is never a guess.
- **Incomplete analysis (B5):** root causes, and which expressions depend on them, so a file can
  be reported partial and only its root causes reported.
- **B3's own later milestones:** B3c builds on B3a's relations and B3b's walk; B3d on all three.
- **The language server (C5):** an edit inside a body re-attributes that body's file and no other,
  and attribution stops within a few milliseconds of cancellation, as names do.
- **B4 (Lombok):** the members Lombok generates are added to type entries, and attribution reads
  them like any other member.

## The type model

B2's types are `Ty`: a flat, pre-order list of parts, as written in a declaration or read from a
class file, with class-file names still binary names. They are values in project-wide entries and
stay so. Attribution needs more: types it builds (capture variables, intersections from `lub`,
substituted member types, and in B3c inference variables), fast equality, and substitution. So B3
works on a second representation, a `TypeId` into a hash-consed **arena**, and translates a `Ty`
into it on first use ([type relations](semantic-type-relations.md#the-arena)).

- **One arena per computation:** a file's attribution, or a type relation query, owns its arena.
  Nothing is shared between threads, nothing is locked, and an arena dies with its result. A
  file's result keeps its arena, so its types stay readable while the result is.
- **Translation is memoized per arena,** so a member type read a thousand times in a file is
  translated once there. Project-wide entries never hold a `TypeId`.
- **Identity:** a class type is its `TypeKey` with its outer type and arguments; a type variable is
  its declaration (`TypeVarOwner` and index, as B2 keys it); a capture variable is fresh in its
  arena, with the node and argument position that made it.

The alternative, one interner for the whole engine, would let results share types and compare
them across files, at the cost of a concurrent map on every type built and a memory that only
grows. Nothing in B3's milestones compares types across files (constants cross files as values,
not types), so the arena per computation is recommended (question 3).

## Errors, unknowns, and limits

The scope's rule is that rules stay silent on code involving an error type, and an error-typed
argument never yields a chosen method. B3 makes that a property of the representation:

- **Two kinds of non-answer type:** `Error(root)` for code that depends on something that didn't
  resolve (B2's error symbols, a type with an incomplete hierarchy, a Lombok-annotated type until
  B4), carrying the root cause so B5 can report it once; and `Unknown(why)` for code Ferrion can't
  type yet (left to a later milestone, or a limit hit). Both behave the same in relations; they
  differ only in what B5 reports.
- **Three-valued relations:** every relation answers `Yes`, `No`, or `Unknown`. A relation
  involving an error or unknown type answers `Unknown`; so does one that needs a supertype a
  hierarchy is missing, unless the answer is already found through the parts that are complete
  (`A <: Object` is always `Yes`, and `A <: B` is `Yes` if `B` is reached through resolved
  supertypes; `No` needs the whole hierarchy).
- **Spreading:** an expression with a non-answer operand, receiver, or argument has a non-answer
  type, with the first root cause found. A call whose candidates can't all be judged chooses no
  method. A method chosen is therefore always one javac would choose too, if Ferrion's inputs are
  right; that is the property the removed-symbols check tests.
- **Limits:** every recursive algorithm (subtyping through F-bounds, `lub`, capture, and B3c's
  inference) runs on a **fuel** budget per body: a count of steps, so the same input always stops
  at the same place on every machine. Running out gives `Unknown(Limit)` for the expression being
  typed and what depends on it, and the rest of the body carries on. A wall-clock limit per file,
  checked where cancellation is checked, is a backstop that should never fire; when it does, the
  file is partial and the run reports a Ferrion bug.

### The per-file time limit

The exit criterion asks that no file of the corpus or of javac's test inputs exceed a per-file
time limit, and nothing defined one before B3. The proposal is made from data, in two steps:

1. **Now:** W0's oracle records how long javac spends analyzing each file (attribution and flow,
   timed per compilation unit), across the corpus and javac's stress inputs. The distribution,
   the slowest files, and the provisional limits are in [the measurements](#measurements) below.
2. **At B3c's review,** when inference exists and is the part most likely to blow up, Ferrion's own
   distribution is measured the same way, and the limits are set: the fuel per body at a multiple
   of the corpus's largest use, and the wall-clock backstop per file at a multiple of the slowest
   file's time, with the margin chosen from the two distributions. Until then B3a and B3b run with
   provisional limits about ten times above anything measured in the corpus, so a limit firing
   there is a bug report rather than a result.

### Measurements

From W0's corpus run ([analysis time per file](types-oracle-normalization.md#analysis-time-per-file)):
javac's `ANALYZE` time (attribution and flow) for each of the 16,709 compared files, on each
project's own JDK (21 to 27). A source set's first files carry the JVM's warm-up, so they are left
out of the row that measures a file's own cost:

| | Median | p90 | p99 | p99.9 | Max |
| --- | --- | --- | --- | --- | --- |
| Every compared file | 1.6 ms | 17.5 ms | 122 ms | 431 ms | 1,129 ms |
| But each source set's first 5 files | 1.4 ms | 11.9 ms | 74 ms | 323 ms | 1,129 ms |

- **The slowest corpus files** are large API surfaces with thousands of overloaded calls (jedis's
  `CommandObjects`, 1,129 ms, and `Jedis`, 867 ms), ANTLR's generated parsers (spring-data-jpa's
  `EqlParser`, 906 ms), and guava's collection classes (`Maps`, 563 ms).
- **javac's stress tests** (975 files under `generics/inference`, `generics/diamond`, `lambda`,
  `patterns`, `expswitch`, `perf`, and files named for nesting or chains): one outlier,
  `generics/inference/8048838/T8048838.java` at 5,031 ms, a method with 100 type parameters each
  bounded by the next, called once; then 488 ms and 458 ms; everything else under 165 ms, each
  with about 40 ms of warm-up.
- **Noise:** the machine was shared during the run, and the same file measured up to three times
  apart between two runs, so these are upper bounds. The numbers set at B3c's review are measured
  again on a quiet machine.

What follows for the limits:

- The second row is the measure of a file's own cost, and the fuel budget is set against
  Ferrion's own distribution of the same kind.
- A fixed input like `T8048838` is what the limit is for: it should run out of fuel and be
  `Unknown(Limit)` within the backstop, rather than the limit being raised to fit it.
- **Provisional limits for B3a and B3b:** a wall-clock backstop of 10 s per file, about ten times
  the slowest corpus file, and fuel that W1 sizes so that no corpus file uses a tenth of it.

## The crates

Everything is in `ferrion_semantic`, beside B2's modules; there is no new crate and no new external
dependency.

- `types`: the arena, translation from `Ty`, and B3a's relations (`subtype`, `contains`,
  `capture`, `lub`, `glb`, `erasure`, conversions and contexts, member types as seen from a type).
- `members`: per type and view, its member fields and methods, declared and inherited, with
  overriding and hiding worked out: a derived entry, shared by every file
  ([attribution](semantic-attribution.md#member-tables)).
- `constants`: constant folding (JLS 15.29), and a derived entry per constant field, since a
  constant crosses files ([attribution](semantic-attribution.md#constants)).
- `attr`: B3b's walk and its per-file result, with B3c's and B3d's additions later.

```rust
impl Snapshot {
    // B3a:
    /// A relations context for `view`, with its own arena: the entry point for tests, the
    /// relations oracle comparison, and `attr`.
    pub fn types(&self, view: SourceSetId) -> TypeContext<'_>;
    // B3b:
    /// A type's member fields and methods in `view`, declared and inherited.
    pub fn members(&self, view: SourceSetId, ty: TypeKey) -> Result<Arc<Members>, Cancelled>;
    /// A constant field's value, or `None` if it isn't a constant variable.
    pub fn constant(&self, field: FieldRef) -> Result<Option<Constant>, Cancelled>;
    /// Every expression's type, every call's chosen method, and every constant's value in
    /// `file`, by `NodeId` of its current tree, with the arena the types live in.
    pub fn body_types(&self, file: FileId) -> Result<Arc<BodyTypes>, Cancelled>;
}
```

`body_types` reads `body_names`, so a file's names are resolved once and attribution starts from
them; like `body_names`, the CLI drops each file's result after its rules run, and the language
server keeps open files' results.

## Verification

Every milestone is checked against `javac-oracle` across the corpus, as B2 was, with the same
exclusions: files with language errors, files javac never analyzed, and Lombok-annotated types
until B4 ([names oracle](semantic-oracle-normalization.md#what-is-compared)). W0 builds the
oracle's side for all four milestones at once, so each milestone only adds Ferrion's side and the
known differences. What each milestone compares:

| Milestone | Compared with javac | Counted, not compared |
| --- | --- | --- |
| B3a | Relation queries over the corpus's declared types (2,180,279 in W0's run): subtype, assignable, same type, erasure, direct supertypes, capture, member types as seen from a type, subsignature, overrides; `lub` through the types of conditional expressions whose operands B2 already types. | Queries involving a type that didn't resolve, or a class file javac couldn't read (`threw`, 182). |
| B3b | Types of standalone expressions, chosen methods of calls that need no inference, constant values; B2's two B3 categories (`case-label`, `static-import-overload`) settled. | Anything that needs inference or a later milestone: `left-to-b3c`, `left-to-b3d`. |
| B3c | Everything else of types and chosen methods: generic calls, diamond, lambdas, method references, poly conditionals, `var` from poly initializers, inferred type arguments. | Patterns and switches. |
| B3d | Switch and pattern types, exhaustiveness and other switch errors, effective finality, definite assignment as javac reports it, reachability as javac reports it, the remaining constant values. | Nothing: B3's exit. |

Each milestone ends with its corpus comparison at zero unexplained, a categorized list of known
differences, and a review stop (the decision made when B3 started). The comparison for each also
runs the **removed-symbols check** from the exit criterion: for each project, a seeded choice of
classpath jars (and, separately, of single classes) is removed from Ferrion's inputs, and javac's
full-classpath answers are the reference. Ferrion must not crash, and every answer that differs
from javac's must be a non-answer whose root cause is a removed symbol; a confident answer that
differs fails the run. That is the scope's "never a guess", made testable.

Beside the oracle, each milestone has:

- **Tests** from small inputs, each observed with javac when written (the observation becomes the
  test), as in B2.
- **Property tests and a fuzz target** for relations: reflexivity and transitivity of subtyping
  on generated types, `lub` an upper bound of its operands and below every other common
  supertype found, capture idempotent on its result's bounds, every algorithm stopping within
  its fuel. The `semantic_edits` fuzz target attributes what it resolves, so the differential
  check covers attribution too.
- **javac's test inputs,** fetched as the parser's are: no file exceeds the time limit, and none
  panics.
- **Benchmarks:** `semantic/corpus-cold` gains the attribution phase, and `semantic/edits` an edit
  inside a body that changes an expression's type.

## How javac and the JLS are weighed

The exit criterion compares with javac, so where javac and the JLS disagree, Ferrion follows
javac and records the departure as an observation, with the small input that shows it, in the
milestone's design. Ferrion never learns javac from its source (the [clean-room
policy](../../CONTRIBUTING.md#clean-room-policy)): only from running it, through the oracle and
on small inputs. W0 recorded twenty observations in
[types oracle normalization](types-oracle-normalization.md#observations); the departures from the
JLS among them, and what Ferrion does about each:

- **`lub` cuts its recursion off** (its observation 1): `var x = c ? 1 : "a";` is
  `Serializable & Comparable<? extends Serializable & Comparable<?> & Constable & ConstantDesc> &
  Constable & ConstantDesc`: javac computes one level more than the operands and then writes
  `Comparable<?>`, where JLS 4.10.4 gives an infinite type. Inference's `lub` (`List.of(1, 2.0)`)
  is cut off the same way. Ferrion's `lub` cuts off at the same depth (question 2). (javac's
  diagnostics print `Object` as an extra first part; the API's type doesn't have it.)
- **An intersection's parts are ordered** (observation 2): the class first, then interfaces
  deepest first, then by qualified name, whichever operand comes first. The JLS doesn't order
  them; Ferrion orders `lub`'s and `glb`'s results the same way, and W1 observes the rule on more
  inputs, since it was inferred from four.
- **`Types.directSupertypes` and `Types.asMemberOf` depart from JLS 4.10.2 and 4.5.2**
  (observations 16, 17): for `int[]` one intersection rather than three types, `Object` among an
  interface's supertypes even when it has superinterfaces, wildcards substituted rather than
  captured. These are javac's API, not how javac types programs, so Ferrion implements the JLS and
  its relations adapter converts its answers to the API's shape for the comparison
  ([type relations](semantic-type-relations.md#checking)).
- **Folded `float` and `double` text is the compiling JDK's** (observation 13): `"" + 1.0E23` is
  `1.0E23` on JDK 27 and `9.999999999999999E22` on JDK 17, since `Double.toString` changed in
  JDK 19. Every corpus project builds on JDK 21 or later; Ferrion folds with JDK 19's algorithm
  ([constants](semantic-attribution.md#constants)).

## Work order

On `b3/integration`, cut from main at 3273dd6:

0. **W0, the oracle side** (done, alongside this design): `javac-oracle types`, which reports
   relation queries in the same run as types so each source set compiles once; `cargo oracle
   types` (and `--relations` for B3a alone) with a stand-in for Ferrion; the type and constant
   string formats in [types oracle normalization](types-oracle-normalization.md); per-file
   analysis times; CI jobs; and a corpus run of javac's side alone: 4,447,710 expressions,
   1,329,951 calls, 849,929 constants, 460,541 variables, and 2,180,279 relation queries in 16,709
   compared files, 514 MB of output, in 864 s.
1. **W1, type relations (B3a):** `types`: the arena, translation, relations, `lub` and `glb`,
   conversions and contexts, member types as seen from a type; Ferrion's relations side for
   `cargo oracle types --relations`; property tests and the fuzz target. One agent: its parts
   build on each other. (Done: each of the corpus's 2,180,279 relation queries matches, is left
   to B4, is about a type javac couldn't resolve, or was lost to javac throwing, with no known
   differences; the most fuel a query used is 145 steps;
   [results](semantic-type-relations.md#results).)
2. **B3a review stop,** after W1's corpus comparison. (Done: W1's five questions accepted as
   recommended; [decisions](semantic-type-relations.md#decisions-made-in-review).)
3. **W2, attribution core (B3b):** `members` (member tables), `attr`'s walk and result,
   literals, names, field access, operators and promotions, casts, `instanceof` without
   patterns, array creation and access, `this` and class literals; `constants`.
4. **W3, calls (B3b),** after W2's walk lands: method invocation and instance creation (15.9,
   15.12): potentially applicable candidates, the three phases, most specific, the result type,
   explicit type arguments; the `static-import-overload` and `case-label` differences.
5. **W4, the B3b comparison:** the Ferrion adapter for `cargo oracle types`, the corpus run,
   fixes, known differences, the removed-symbols check, benchmarks. It can start beside W3 on what
   W2 types, and finishes after W3.
6. **B3b review stop.** Then B3c's design, its review, its work, its stop; then B3d's.

At most two B3 agents run at once while A2's agents run, all through the shared heavy-job lock,
keeping the machine near four agents in total.

## What was consulted

- **Specifications:** the Java Language Specification, Java SE 27 edition: chapter 4 (types,
  subtyping, `lub`), 5 (conversions and contexts), 8.4 (method signatures, overriding), 15
  (expressions, especially 15.12 and 15.29); chapter 18 for what B3c will need, to keep B3a's
  representation ready for it.
- **Observation:** javac (Temurin 27.0.1) on small inputs written for this design, recorded as
  observations here and in the milestone documents.
- **Not consulted:** javac's source, or any other Java compiler's or tool's type checker.

## Questions for review

1. **Constant folding in B3b** rather than B3d: assignment conversion of constants (`byte b = 1;`)
   and `case` labels need constant values, so B3b can't type assignments without them. B3d keeps
   flow and switches.
2. **Following javac over the JLS,** and recording each departure as an observation: `lub`'s
   recursion cut off as javac cuts it (observation 1), and the departures W0 finds.
3. **An arena per computation** rather than one interner for the engine.
4. **Member tables as shared derived entries** (per type and view), which answers W5's open
   question from B2: body lookups of fields and methods become shared entries, since overload
   resolution needs each type's inherited and overridden methods worked out, and doing that per
   call is what made B2's body phase scale worse than the others.
5. **B3b's line:** a call whose candidates or arguments need inference, and every expression that
   depends on one, is `left-to-b3c` and compared from B3c. Is that the right line for B3b's
   comparison?
6. **The time limit,** as a deterministic fuel budget per body plus a wall-clock backstop per
   file, with the numbers set at B3c's review from both distributions, and provisional limits ten
   times above anything measured until then.
7. **The removed-symbols check** as defined: every answer differing from javac's must be a
   non-answer rooted in a removed symbol.
8. **The work order:** W1 alone for B3a; W2, then W3 and W4 for B3b; at most two B3 agents at
   once while A2 runs.

From W0, the oracle (each recommended by W0, and by this design):

9. **Two javac internals in the oracle:** constant values come from `Type.constValue()` and
   effective finality from javac's `EFFECTIVELY_FINAL` flag, read by name, since the public API
   has no other route to either. Both were checked against the JLS on small inputs, on JDK 21,
   25, 26, and 27; `names` already reads javac's trees.
10. **Relations inside `types`,** not a separate `relations` command, so each source set compiles
    once; `cargo oracle types --relations` compares them alone for B3a.
11. **Type strings without origins:** a class in a type string is its binary name, without where it
    came from, since `names` already compares that.
12. **What the public API doesn't expose isn't compared:** inferred type arguments on their own,
    constructor and method reference types, and which phase chose a method. A call's type as
    invoked covers inferred arguments wherever the signature mentions them.

## Decisions made in review

The user reviewed this design, with [type relations](semantic-type-relations.md),
[attribution](semantic-attribution.md), and W0's oracle, and accepted every recommendation
("Accept, start W1"):

1. **Constant folding** is B3b's, with a derived entry per constant field; B3d keeps flow and
   switches.
2. **javac over the JLS** where they disagree, each departure an observation with its input.
3. **An arena per computation,** not one interner for the engine.
4. **Member tables** are shared derived entries per type and view, which settles W5's question
   from B2.
5. **B3b's line:** calls that need inference, and what depends on them, are `left-to-b3c`, counted
   and compared from B3c.
6. **The time limit:** fuel per body and a wall-clock backstop per file, provisionally 10 s, with
   both set at B3c's review from distributions measured on a quiet machine.
7. **The removed-symbols check** as defined: an answer differing from javac's must be a non-answer
   rooted in a removed symbol.
8. **The work order:** W1 alone for B3a, then W2, W3, and W4 for B3b, at most two B3 agents at
   once while A2 runs.
9. **The oracle's two javac internals** (constant values, effective finality) stay.
10. **Relations inside `types`,** compared alone with `--relations`.
11. **Type strings without origins.**
12. **What the public API doesn't expose isn't compared.**

And the milestone documents' own questions:

- **Casting legality** ([type relations](semantic-type-relations.md#questions-for-review)): the
  kind of conversion for correct programs, and `Unknown` for legality typing doesn't need.
- **Per-file results** ([attribution](semantic-attribution.md#questions-for-review)), split per
  body only if W4's edit benchmark needs it.
