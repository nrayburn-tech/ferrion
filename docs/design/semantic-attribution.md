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
- **Text of floating-point constants:** `"" + 1.0e23` folds through `Double.toString`, whose
  output changed in JDK 19. W2 observes which JDK's output javac folds with (the compiling JDK's,
  or `--release`'s), and folds to match.

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
normalization](types-oracle-normalization.md). Each mismatch is categorized (`type`, `method`,
`constant`, `missing`, `error` for a non-answer where javac has a type); non-answers left to later
milestones are counted as `left-to-b3c` and `left-to-b3d`, not compared. The removed-symbols check
runs as the overview defines. W4 records:

- the counts, per project and category, and the known differences, at zero unexplained;
- the share of calls and expressions on each side of the inference line;
- cold attribution time on lucene, single-threaded and on four threads, against B2's body phase;
- an edit inside a body of lucene's largest file, end to end;
- the largest arena and result in the corpus, to set the arena budget with the time limit.

## Questions for review

Of the overview's [questions](semantic-types.md#questions-for-review), 1, 4, and 5 are this
document's. One more:

1. **Per-file results:** attribution results are per file, as body names are, and split per body
   only if W4's edit benchmark shows the language server needs it.
