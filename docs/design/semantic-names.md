# Symbols and name resolution

What B2 resolves, how symbols are identified, what a file summary holds, how a source set sees
sources, dependencies, class files, and the platform API, and the order in which names are looked
up. Part of the [semantic analysis design](semantic.md); how the results are kept between edits is
in [incremental computation](semantic-incremental.md).

The rules come from the Java Language Specification (cited by section, Java SE 27 edition) and from
javac's observable behavior, which wins where the two differ (SCOPE.md). Every javac behavior this
document relies on was observed by running javac on small inputs; the
[observations](#observations) record the commands.

## Decision

- **Symbols** are identified by stable keys: a package by name, a type by where it is declared (a
  source set and its path of names, or a class source and binary name), a member by its type,
  kind, name, and parameter types as written, a local by its `NodeId` in one tree. Project-wide
  data never holds a `NodeId` or an offset.
- **Each file has a summary:** its package, imports, and declarations with types as written,
  without bodies or offsets. Summaries are the only part of a source file that project-wide
  resolution reads.
- **Each source set has a view:** its own sources, then its classpath in order (dependent source
  sets read from source, jars, directories), with the platform API ahead of all of them for the
  packages it owns. A type declared in more than one of its own source roots is ambiguous.
- **Resolution follows the JLS's scopes,** with javac's choices where they differ, and gives every
  name in B2's part either a symbol or an error symbol that records the root cause.
- **B2's part** is every package name, type name, and annotation name; every simple expression
  name; expression names qualified by a package or type; and, for method names, the type whose
  members are searched. Names qualified by an expression's type, and which overload a call
  chooses, are B3's ([What B2 leaves to B3](#what-b2-leaves-to-b3)).

## What name resolution serves

- **Linter rules that need names only** (C2): unused imports and private members, naming rules that
  depend on what a name is, and rules that match a specific type or annotation
  (`java.util.Optional`, `@CheckReturnValue`).
- **Type attribution (B3):** the symbols of every name, each type's supertypes and members, the
  candidate types for method calls, and error symbols that spread.
- **Incomplete analysis (B5):** root causes (a name that doesn't resolve, a classpath entry
  missing), and which types have incomplete hierarchies.
- **The formatter (A2):** nothing; import cleanup uses syntax only (SCOPE.md).
- **The javac comparison:** a symbol key for every name that can be compared with javac's element
  for the same source range.

## Symbols

### Kinds

| Kind | From source | From class files |
| --- | --- | --- |
| Module | `module-info.java` | `module-info.class`, the platform's descriptors, automatic modules |
| Package | `package` declarations and directories of source roots | package listings |
| Type (class, interface, enum, record, annotation interface) | summaries, and local and anonymous classes in bodies | class files |
| Type parameter | summaries, and generic methods and local classes in bodies | `Signature` attributes |
| Field (including enum constants) | summaries | field tables |
| Method and constructor | summaries, plus the implicit ones (default constructors, enum and record members) | method tables, without synthetic and bridge methods |
| Record component | summaries | `Record` attributes |
| Local (variable, parameter, exception parameter, resource, pattern binding) | bodies | none |

### Identity

- **Packages** by name, within a view. A package is "the same" across sources and the classpath:
  `p.A` in sources and `p.B` in a jar are both members of `p`.
- **Types** by where they are declared. The view maps a name to one origin: a source type
  `(declaring source set, package, path of simple names)` or a class file `(class source, binary
  name)`, a class source being a platform API or a classpath entry numbered across the engine. Two
  source sets that resolve `com.acme.Foo` to the same declaration share the symbol; a class in a
  jar or the platform that several source sets use is one symbol. A source type's path
  (`Outer.Inner`), unlike its binary name, can't be confused with a top-level type whose name has
  a `$`.
- **Members** by `(type, kind, name, parameter types as written)` for source members, and by
  descriptor for class-file members. Overloads differ by their parameter types as written, as in
  the [syntax tree design](parser-syntax-tree.md#node-identity).
- **Locals, local classes, and anonymous classes** by `NodeId`, valid for one tree. Their binary
  names (`Outer$1Local`, `Outer$1`) are javac's choice of digits (JLS 13.1 leaves them open), so the
  javac comparison keys them by declaration position.
- **Error symbols** by the name that failed and where, so that one missing type gives one root
  cause.

## File summaries

A summary holds what other files can see of a file, and nothing that changes when a body or an
offset does. It is computed by one declaration-only walk of the tree, which skips bodies by
jumping to their `subtree_end`.

- **Kind:** ordinary, compact (a top-level method or field outside any class, JLS 7.3),
  `package-info.java`, or `module-info.java`.
- **Package:** its name, and for `package-info.java` its annotations as written.
- **Imports,** in order: single-type, on-demand, single-static, static-on-demand, and single-module,
  each with its name as written.
- **Types,** for each class, interface, enum, record, and annotation interface that is top level or
  a member, in order, with its path (`Outer.Inner`):
  - kind, modifiers, and annotations as written (annotation names, and a fingerprint of their
    arguments);
  - type parameters with their bounds as written;
  - `extends`, `implements`, and `permits` clauses as written;
  - fields: modifiers, annotations, type as written, name, and, for a `final` field with an
    initializer, a fingerprint of the initializer's tokens (B3 folds constants from it);
  - methods and constructors: modifiers, annotations, type parameters, parameter types and names,
    varargs, return type, `throws`, and whether it has a body (and a `default` value for an
    annotation interface element, as a fingerprint);
  - enum constants (with whether each has a class body), and record components;
  - the implicit members: the default constructor (JLS 8.8.9), an enum's `values` and `valueOf`
    (8.9.3), and a record's canonical constructor, accessors, fields, `equals`, `hashCode`, and
    `toString` (8.10.3) when they aren't declared;
  - whether the type is `@Generated`, by an annotation's simple name (see
    [Generated sources](#generated-sources)). Whether it is Lombok-annotated is decided by its
    type entry, from the resolved annotations.
- **Module declaration:** name, `open`, and each directive with its names as written.
- **Diagnostic positions** aren't stored. A diagnostic about a declaration finds the declaration
  again in the current tree by its path.

"As written" is the syntax of a type with its name segments, type arguments, array dimensions, and
annotations, as a small owned structure rather than tree nodes, so a summary outlives its tree. It
is flat, its parts in pre-order each with a count of children, so a type nested ten thousand deep
is built, compared, hashed, and dropped without recursion. Names have their Unicode escapes
translated, so `\u0041` and `A` declare the same type.

Every name in a summary (identifiers, dotted names, and later binary names) is interned by the
engine: a summary holds 32-bit ids, and the engine's interner holds each distinct text once. Ids
follow the order names were first seen, so they are compared only within one engine; the
differential check compares summaries across engines as text. A part of a type as written is 12
bytes, with the few annotations on types kept beside the parts. On lucene this took the summaries
and package tables from 49 MB to 36 MB (see
[the engine as built](semantic-incremental.md#the-engine-as-built)).

Details of `ferrion_semantic::summary` as built:

- **Compact units:** their top-level fields and methods, and any type declarations beside them,
  are members of an implicitly declared class, the summary's first type, which has no name (it is
  the file's) and isn't in the package table, since no other file can name it.
- **Constants:** an interface's or annotation interface's fields are implicitly final (JLS 9.3),
  so their initializers are fingerprinted too.
- **Implicit members** are recorded by the rule that declares them (default constructor, `values`,
  `valueOf`, canonical constructor, accessor and field per component), not as signatures. A
  declared constructor is canonical when its parameters have the components' shapes as written:
  the same types without annotations, with `T...` and C-style dimensions counted as array
  dimensions. Resolution can refine that. A record's `equals`, `hashCode`, and `toString` are
  listed unless a method of that name with the same parameters is declared: `equals` with one
  parameter whose type is written with the simple name `Object` (resolution can refine that), and
  the others with none.
- **Generated:** a member type of a generated type is generated too.
- **Receiver parameters** aren't recorded: they don't change what other files see.
- **Fingerprints** of arguments, constants, and defaults hash each token's kind and text, escapes
  translated, without trivia, so comments in them don't change the summary.

Left out: method bodies, initializer expressions of non-`final` fields, instance and static
initializer blocks, local and anonymous classes, comments, and doc comments. Lombok (B4) will add
the members it generates to the summary.

## Source set views

### Order

For a type name `p.T` in a source set S, the view answers from the first of:

1. **The platform API,** for packages that a module of the platform owns (B1's `PlatformApi`).
   Nothing else can add types to those packages: javac reports a source file in `java.lang` as
   `package exists in another module: java.base`, and finds neither it nor a jar's
   `java/lang/Foo.class` (observation 4).
2. **S's own sources,** every source root and generated-source root. A source type shadows any
   class file of the same name: javac does the same for the files it compiles (observation 2).
3. **S's classpath, in order:** each entry is a jar, a directory, or another source set, which is
   read from source (its summaries). The first entry that has the type wins, as with javac
   (observed in B1).

Packages exist if any of these has a type in them or, for sources, a file declares them. Packages of
the platform that the reading module can't see (B1's `is_visible` for the unnamed module, see
[Modules](#modules) for a named one) give an error symbol for a type named in them, `not visible`,
but their classes still resolve as supertypes of visible classes and as class files' names.

As built (`ferrion_semantic::view`), a lookup by package and simple name reads only that type's
part of the package entries it consults, so a new type in a package reaches only the lookups of
its name. A class file's names are binary names, resolved in the reader's view by
`Snapshot::resolve_binary`: the platform for its packages, then own sources (trying each way to
split the name at `$` into a top-level type and members), then the classpath.

### Duplicates

- **Within S's own sources,** a binary name declared in more than one file is ambiguous: every use
  resolves to an ambiguous error symbol and rules stay silent (SCOPE.md). javac keeps the first file
  on its command line and reports the others as `duplicate class` (observation 1). Since code where
  javac reports errors is excluded from the comparison, the two never disagree there.
- **Across S and its classpath,** S's own wins, as above, with no ambiguity.
- **The same file in two source sets' roots** (a shared root) is analyzed once per source set, as
  javac would.

### Dependent source sets

A source set that depends on another (test on main, a module on its sibling) reads it from source:
the dependency's summaries, through its own view. A dependency's type is resolved in the
dependency's view, not the dependent's: `main`'s `Foo extends Bar` means `main`'s `Bar` even if
`test` declares its own.

javac differs here (observation 5): it compiles `test` as one namespace, with `main` on
`-sourcepath` or as classes, so `m.Bar` is `test`'s everywhere in that compilation, `main`'s
`Foo` included. Ferrion keeps the design, so that a dependency's types are resolved once and
shared by every source set that reads them; a test source set that redeclares a type of the
source set it tests is rare, and the javac comparison lists it as a known difference.

Dependencies are acyclic in the project model; a cycle is a usage error (see
[inputs](project-inputs.md#validation)).

### Generated sources

Files under generated-source roots are summarized and resolved like any other, and marked
generated, so the linter and formatter skip them. A type is also generated when it is annotated
with an annotation whose simple name is `Generated`, resolved or not, so that it works without a
classpath: `javax.annotation.Generated`, `javax.annotation.processing.Generated`, and the
`Generated` annotations of code generators and Lombok all qualify (decision 4 in the
[overview](semantic.md#decisions-made-in-review)).

## Modules

All code is in the unnamed module until a source set has a `module-info.java`:

- **Unnamed module:** reads every module of the platform's default set and sees their unqualified
  exports (B1's `is_visible`), and every classpath entry.
- **A named module in sources:** reads `java.base`, the modules it `requires` and, transitively,
  those they `requires transitive`. It sees its own packages, the unqualified exports of the modules
  it reads, and exports qualified to it. Its readability is computed from its summary's directives
  and the descriptors of the modules named.
- **The module path,** for now, is read like the classpath, with every package visible (SCOPE.md:
  "treated leniently"). Its jars' module names (from `module-info.class`, the
  `Automatic-Module-Name` manifest attribute, or the file name) are still recorded, so that
  `requires` and `import module` find them.
- **`import module M`** (JLS 7.5.5) imports the public top-level types of every package that M
  exports to the current module, and of the modules M `requires transitive`. A compact compilation
  unit imports `java.base` this way implicitly (JLS 7.3). M must be read: in the unnamed module, a
  module outside the default set is `unnamed module does not read: jdk.incubator.vector`
  (observation 6).

As built: a source set's module is a derived entry read from its `module-info.java` (found through
the package index, which lists the files in no package). A named module's platform packages are
visible when it reads their module (from the platform's descriptors, with `requires transitive`
followed) and that module exports them to everyone or to it. Classpath and module path packages
are always visible. `import module` finds a platform module, a source set's own or a
dependency's module by its `module-info.java` (its `exports`), or a module path jar by its
recorded name (all of its packages, leniently).

## Resolving names

### Contexts

JLS 6.5.1 classifies a name by its syntactic context, and the syntax tree gives the context: a
`Name` in a `ClassType` is a type name with a package-or-type qualifier, a `NameExpr` is an
expression name or an ambiguous one, an import's name is a type or package name, a method call's
name is a method name, and so on. A table maps each node kind and child position to its JLS
context; the parser's node catalog has a row for each.

### Type names

A simple type name `T` at a point resolves to the first match, innermost first (JLS 6.3, 6.4.1):

1. a local class, interface, enum, or record declared earlier in an enclosing block, or a type
   parameter of an enclosing generic method or constructor;
2. for each enclosing class from the innermost outward: a type parameter of that class, or a member
   type it declares or inherits;
3. a type in the compilation unit, or one imported by a single-type or single-static import;
4. a type in the same package, from any file or classpath entry;
5. a type imported on demand (type-import-on-demand, static-import-on-demand, and the implicit
   `java.lang.*`); two different types are an ambiguity, reported where the name is used;
6. a type imported by a single-module import (and a compact compilation unit's implicit
   `import module java.base`); shadowed by every kind above, including same-package types and
   on-demand imports, and ambiguous only when two module imports give different types
   (observation 3).

Observed details (observation 7), as built in `ferrion_semantic::resolve`:

- In a class's body, a member type wins over a type parameter of the same name; in the class's
  header (type parameters, `extends`, `implements`, `permits`), its own members aren't in scope,
  so the type parameter, or a top-level type of the name, is found.
- An inherited member type wins over a single-type import, and a member type the inner class
  inherits wins over one its outer class declares.
- A single-type import wins over the same package, and the same package over an on-demand import.
- A static-import-on-demand and a type-import-on-demand giving two types of a name are ambiguous
  where the name is used, and fine while it isn't.
- A single-static import brings in a member type of its name.

A qualified type name `Q.T` resolves `Q` as a package or type name first (JLS 6.5.4, 6.5.5):
`T` is then a member type of type `Q`, declared or inherited, or a top-level type of package `Q`.
A type obscures a package of the same name (observation 7). An import names a type by its
canonical name, so its first segment is a package. Members of a type variable (`T.In`, which javac
accepts through `T`'s bound) are left to B3, and are an error symbol in B2.

Access isn't checked in B2, except where JLS makes it part of what is found: a private member type
isn't inherited (8.5), nor is a package-access one in another package, and a type in an unexported
package isn't visible. javac finds the private member and reports `X has private access`; with
another `X` imported, it uses the import (observation 7), as Ferrion does.

### Ambiguous names and obscuring

A name like `a.b.c` in an expression is ambiguous until its first segment is classified (JLS
6.5.2): `a` is an expression name if a variable named `a` is in scope, else a type name if a type
named `a` is, else a package name. A variable obscures a type, and a type obscures a package (JLS
6.4.2). Each following segment is classified from the one before: after a package, a type or a
subpackage; after a type, a member type or a static field; after an expression, a field of its
type, which is B3's unless the expression is one B2 has resolved to a field or variable of a
declared, non-generic type (then the field is looked up by name in that type and its supertypes).

### Expression names

A simple expression name `x` (JLS 6.5.6.1) resolves to the innermost of: a local variable,
parameter, exception parameter, resource, or pattern binding in scope at that point (pattern
bindings by the flow-scoping rules of JLS 6.3.1); a field declared or inherited by an enclosing
class, innermost class first; a field imported by a single-static import; a field imported by a
static-import-on-demand. A qualified name `Q.x` with `Q` a type is a static field of `Q`, declared
or inherited.

### Method names

For a method call, B2 finds the type whose members are searched (JLS 15.12.1): for a simple name,
the innermost enclosing class with a member method of that name, declared or inherited, else the
static imports; for `Q.m()` with `Q` a type, `Q`; for `super.m()`, the superclass. It records the
candidate methods by name. Choosing among them, and every call on an expression (`foo().m()`,
`x.m()`), is B3's.

### Inheritance

Member types and fields are inherited from the direct superclass and superinterfaces, unless
private or hidden by a declaration of the same name in the subtype (JLS 8.3, 8.5, 9.3, 9.5). A
name inherited from two different supertypes is ambiguous when used. This needs every type's
supertypes resolved, which needs names resolved in its `extends` clause, whose scope includes the
member types of enclosing classes and their supertypes. So:

- supertypes are resolved per type on demand, as a type entry ([incremental
  computation](semantic-incremental.md#project-symbols));
- a type whose supertype resolution reaches itself is a cycle: every type in the cycle gets an
  error supertype in its place, and an incomplete hierarchy;

As built, a source type's supertypes are resolved erased, in an entry of their own, before
anything that needs members: the classes named in `extends` and `implements`, without their type
arguments. A member type lookup in a type needs its supertypes, so `class A extends A.B { static
class B {} }` reads itself and is a cycle, as javac reports (`cyclic inheritance involving A`,
observation 8), while `class S implements Supplier<S.In>` is not, since type arguments are resolved
with the rest of the type's entry. To find a cycle through supertypes alone (`P extends Q`, `Q
extends P`), a type's supertypes entry reads its source supertypes' supertypes entries. A cycle is
found by the engine's tracking of entries in progress
([incremental computation](semantic-incremental.md#project-symbols)), and every entry found in it
gives its cyclic value.

Member types are an entry per type and view: the declared ones, each with who can inherit it, and
the inherited ones, hidden by a declared one of the name; one inherited as two different types is
ambiguous, and the same type by two paths is one (observation 7). The entry is incomplete when a
supertype doesn't resolve, is cyclic, or is incomplete itself.
- a type with any supertype that doesn't resolve, or that has an incomplete hierarchy, has an
  incomplete hierarchy (SCOPE.md): rules treat its members, overrides, and subtyping as unknown,
  and member lookups that find nothing in it give an error symbol rather than "not found".

### Class-file types

A class file gives its binary name, flags, supertypes (`Signature` when present, else
`super_class` and `interfaces`), its fields and methods (synthetic and bridge members left out),
its member types and its outer class from `InnerClasses`, and its record components and permitted
subclasses. Member types are looked up through `InnerClasses`, never by splitting a binary name at
`$`, since `$` is legal in names: `p.A$B` names a top-level class `A$B`, and `p.A.B` isn't it
(observation 9). A class file that's malformed gives an error symbol with B1's error as the root
cause.

As built, a class file is read once into a class entry (`ferrion_semantic::symbols::ClassEntry`):
its flags (an `InnerClasses` entry's for a member), simple name and outer class, type parameters,
erased and generic supertypes, member types, fields, and methods with their generic signatures,
names as binary names. It is the same for every view; whoever reads it resolves its names in its
own view. A part that is malformed is left out, and the first problem kept.

## Errors and incompleteness

- A name that doesn't resolve gets an error symbol carrying a root cause: the name, where it was
  used, and why (no such type, an ambiguity, a cycle, a missing classpath entry, a malformed class
  file). Names whose resolution reads an error symbol get error symbols that point at the same root
  cause, so B5 reports one diagnostic.
- Lombok-annotated types (any annotation from `lombok`) have incomplete hierarchies until B4.

As built, an error symbol is the name as written and a cause: not found; no such package (`package
nope does not exist`); ambiguous, with the candidates; not visible; a module not read, or none of
the name; a cycle; nothing found in an incomplete hierarchy, with the type; a class file that
couldn't be read; syntax; or a qualified name of more than 1,024 segments, of which only the first
1,024 are resolved, since each prefix of a name is a name of its own and resolving them all costs
the square of the name's length (the parser's stress patterns have 100,000; no Java code has
packages that deep). Where is known by what holds it: a type entry's errors are its
header's, listed once each, and a file's names are by node. A name qualified by one that failed
holds the same error. Lombok is decided by the type entry: an annotation on the type or one of its
fields that resolves to a type in a `lombok` package. A subclass of a Lombok type isn't made
incomplete by it.
- Syntax errors: names in a subtree with an error node resolve as usual where they can; a missing
  name is an error symbol with "syntax" as its cause.

## Names in declarations

For the javac comparison to start on declarations before W5 resolves bodies, every name outside
bodies has a resolution per file, keyed by `NodeId` of the file's current tree with its byte range
(`ferrion_semantic::decl_names`): the package declaration, imports, the module declaration,
annotations on declarations and their element names, type parameter bounds, `extends`,
`implements`, and `permits`, field and record component types, method and constructor signatures,
and `throws`. Each `Name` and `ClassType` node is one name, so `java.util.List` gives the packages
`java` and `java.util` and the type. Each resolves to a symbol (a package, module, type, type
variable, annotation element, or a single-static import's member name) or an error symbol, and
`Snapshot::symbol_key` gives the key of semantic.md's table: the exchange with W3's oracle is left
for integration.

## Names in bodies

Every name inside a body has a resolution too, per file and keyed by `NodeId` of its current tree
(`ferrion_semantic::body`): in method, constructor, and initializer bodies, field initializers,
lambdas, enum constants' arguments and bodies, annotation arguments (of declarations too, whose
annotation names are the declaration's), annotation elements' defaults, and every local and
anonymous class those declare, its header and members included. `Snapshot::body_names` gives
them, and `Snapshot::file_names` the names of declarations and bodies together, in node order.

- **The walk** is the tree's flat pre-order walk. Outside bodies it follows the summary's types
  (it meets them in the summary's order) and their methods, for the type parameters' owners;
  inside, it keeps an explicit stack of scopes, each opened by a node and closed when the walk
  leaves it. A scope's declarations are bindings in maps by name, innermost last, so a lookup is
  one map read however deep the scopes are: locals, parameters, exception and lambda parameters,
  resources, and pattern bindings; local classes and the type parameters of local classes and of
  the methods around; and the fields, member types, and methods of each local or anonymous class
  around, declared and inherited, bound when its body is entered. What the bindings don't have,
  the summary's class around the body and its outer classes answer, then the file's imports,
  through the lookups of declarations (a scope in a body asks the body first, `BodyTypes`).
- **Local scopes** (JLS 6.3, 6.4): a local is in scope from its declarator, its own initializer
  included; an enhanced `for` variable in the body only, not its expression; a resource in the
  rest of the resources and the `try` block, not in `catch` or `finally`; a local class from its
  declaration, its own declaration included. A local shadows a field; a field of a local or
  anonymous class shadows a local of the method around the class.
- **Pattern bindings** (JLS 6.3.1, 6.3.2): what an expression introduces when true and when
  false is worked out as the walk leaves it (`instanceof`, `!`, `&&`, `||`, parentheses), and
  bound in the right operand of `&&` and `||`, the branches of `?:` and `if`, a `while` or `for`
  body and update, a `case` label's guard and body (with the guard's when true), and after an
  `if`, `while`, `for`, or `do` in a block where the rules introduce them. Whether a statement can
  complete normally follows JLS 14.22 as far as the rules need: `return`, `throw`, `break`,
  `continue`, and `yield`, blocks ending in one, `if`-`else` and `try` whose parts all can't, and
  loops on `true` with no `break` (observation 10).
- **Contexts** (JLS 6.5.1) come from the parent: the qualifier of a field access or a call, and of
  `::`, is an ambiguous name; of `.class`, `.this`, and `.super` a type name, its own qualifiers
  then package-or-type names; a name in a class type is a type name, and `var` there isn't one
  (JLS 14.4). Ambiguous names are classified as each node is left (JLS 6.5.2): a variable, else
  a type, else a package, so a variable obscures a type and a type a package (observation 11).
  After a package, a type or a subpackage; after a type, a field, or a member type where a type
  can be; after a variable, `this`, or `super`, a field of its class. A qualified name of more
  than 1,024 segments is `Cause::TooLong` after them, as in declarations.
- **Expression names** (JLS 6.5.6): the innermost local, or field of a local or anonymous class
  around; then a field of the summary's classes around, declared or inherited, innermost first;
  then a single-static import; then a static-import-on-demand, two different fields being
  ambiguous. An inherited field wins over a single-static import (observation 12), and a
  superclass's private field isn't inherited, even within one top-level class (observation 13).
  A `case` label's simple name is an enum constant of the selector's type when it is an enum;
  when the selector's type needs attribution, a name no variable has is left to B3.
- **Fields and methods by name** are worked out once per body result for each type asked
  about, its supertypes first, by a loop: declared ones, then the non-private ones each
  supertype has (package-access ones only in the same package, an interface's static methods
  never), a field inherited as two different fields being ambiguous. Every method of the name is
  kept, overridden ones too, since choosing is B3's.
- **Method names** (JLS 15.12.1): for `m()`, the innermost class around with a member method
  `m`, declared or inherited, is the type to search, else the types the static imports of `m`
  name; for `Q.m()`, `this.m()`, `Q::m`, and `this::m`, `Q`'s or this class's; for `super.m()`
  and `Q.super.m()`, the superclass of this class or of `Q`, or for a superinterface `Q`, `Q`. The
  name records the type and its candidates. A call on any other expression, and a constructor
  reference, are B3's.
- **Local and anonymous classes** get what a summary's types have, made the same way: a summary
  of their declaration (`summary::local_type`, `summary::anonymous_type`), a type entry resolved
  with `resolve::members_of` in the scope of their body, erased supertypes, and member types
  declared and inherited, kept in the body result and keyed `TypeKey::Local` (the file and the
  index in its result). An anonymous class extends its class, or implements its interface and
  extends `Object`; an enum constant's body extends its enum. Member types of a local class are
  registered before the walk reaches them, so a field's type can name them. A local record,
  enum, or interface is static, but a local of the method around it is still found there:
  javac finds it and reports an error rather than looking further (observation 14). A field
  inherited by an anonymous class shadows the method's local of the name (observation 15).
- **Keys:** a local is `local <file>:<offset of its name>`; a local or anonymous class is `type
  local <file>:<offset>`, of its name, or of its `new` or enum constant's name for an anonymous
  class, a member type of one included (javac's binary names for them have digits it chooses);
  its members are keyed by that, `field local <file>:<offset>.f`. A field is `field <declaring
  type>.f`, a method name with one candidate `method <type>.m<descriptor>`, and one with several
  `methods <type searched>.m:` and its candidates' keys. A compact constructor's parameters are
  its record's components, keyed at their names. How javac's positions are matched with these
  is for the comparison's normalization (W6).
- **Errors and B3:** a name that doesn't resolve is an error symbol as in declarations, and a
  name qualified by one, or by a variable whose type didn't resolve, has the same error. A name
  whose meaning needs types is `Symbol::LeftToB3` with a reason: a member of an expression whose
  type needs attribution (a call's result, `var`, an implicitly typed lambda parameter, an array
  element, a parenthesized or created value), of a type variable, or of an array, and the method
  of a call on an expression.

Labels aren't names javac gives elements for, so they aren't recorded; they are followed for the
statements a `break` leaves. Not done: inherited members of a local class's member type named
before the walk reaches that member's header resolve its declared members only.

## What B2 leaves to B3

- the field in `e.f` where `e` is any expression but a resolved variable of a non-generic declared
  type;
- the method a call chooses, and anything after it in a chain;
- `var`'s type, lambda parameters' types, and anything that needs inference;
- the type of `this`-qualified and `super`-qualified field access beyond its lookup class;
- members of a type variable (through its bounds) and of an intersection type.

The javac comparison in B2 covers B2's part only; the other names are recorded by the oracle from
the start, and compared from B3.

As built, `e.f` is looked up in `e`'s declared class type whether or not it was written with
type arguments, since which field a name is doesn't depend on them; a field whose type is a type
variable then leaves what follows it to B3. `this.m()` and `this::m` are searched in this class,
like `super.m()`. The calls left to B3 are those on any other expression.

## Observations

Run with Temurin 27.0.1's `javac`, in a scratch directory. Each is a test in `ferrion_semantic`
(`tests/resolution/*.case`, resolved against a stand-in platform with the JDK's names, and each
case's files run through javac when written).

1. **Duplicate classes.** `r1/p/A.java` and `r2/p/A.java` both declare `public class p.A`, with
   different methods, and `r1/p/U.java` calls `A.one()`, which only the first declares.
   `javac -d out r1/p/A.java r2/p/A.java r1/p/U.java` reports `r2/p/A.java:1: error: duplicate
   class: p.A` and nothing about `U`: the first file on the command line is kept.
2. **Sources and the classpath.** `b.jar` holds `q.B` with `fromJar()`; `src/q/B.java` declares
   `q.B` with `fromSource()`; `src/q/V.java` calls both.
   - With both source files on the command line (`javac -cp b.jar src/q/B.java src/q/V.java`), the
     only error is `cannot find symbol: method fromJar()`: the source type wins.
   - With `B.java` only on `-sourcepath src`, javac takes the newer of the source and the class
     file (its default `-Xprefer:newer`): an older source gives `cannot find symbol: method
     fromSource()`, a newer one `... fromJar()`. Ferrion reads every file of a source set as javac
     reads its command line, so the source always wins. The oracle passes every file of the source
     set on the command line for that reason.
3. **Module imports** (`--release 27`). With `import module java.base;`, a same-package class
   `x.List` is used for `List` with no error, and so is `java.awt.List` with an added
   `import java.awt.*;`. With `import module java.base;` and `import module java.desktop;`, javac
   reports `reference to List is ambiguous` (`java.awt.List` and `java.util.List`). In a compact
   unit, `import java.awt.*;` and `List l;` is `java.awt.List` over the implicit module import.
4. **The platform's packages.** `src/java/lang/Foo.java` (`package java.lang; public class Foo`)
   and `src/u/U.java` with a field `java.lang.Foo f`: `javac -d out src/java/lang/Foo.java
   src/u/U.java` reports `package exists in another module: java.base` and `cannot find symbol`
   for the field. With `Foo` compiled into `foo.jar` (`javac --patch-module java.base=jarsrc`),
   `javac -cp foo.jar -d out src/u/U.java` reports `cannot find symbol`: a package the platform
   owns has only the platform's classes.
5. **Dependent source sets.** `main` declares `m.Bar` (`fromMain()`) and `m.Foo extends Bar`;
   `test` declares its own `m.Bar` (`fromTest()`) and `t.T`, which calls `foo.fromMain()` on an
   `m.Foo`. `javac --release 21 -implicit:none -sourcepath main -d out test/m/Bar.java
   test/t/T.java` reports `cannot find symbol: method fromMain()`, and so does compiling `test`
   against `main`'s classes: in `test`'s compilation, `main`'s `Foo extends Bar` is `test`'s `Bar`.
   Ferrion resolves `main`'s types in `main`'s view by design (see
   [Dependent source sets](#dependent-source-sets)).
6. **Module imports and readability.** `import module jdk.incubator.vector;` with a use of
   `VectorSpecies`: `javac --release 21` reports `module imports are not supported in -source 21`;
   `--release 25` reports `unnamed module does not read: jdk.incubator.vector`; with
   `--add-modules jdk.incubator.vector` it compiles. In a named module, `requires java.sql` makes
   `java.sql`, `javax.xml`, and `java.util.logging` readable (`requires transitive`), and
   `java.awt.List` is `package java.awt is not visible`. `import jdk.internal.misc.Unsafe;`, single
   or on demand, is `package jdk.internal.misc is not visible` in the unnamed module at release 27,
   and `package jdk.internal.misc does not exist` at `--release 21`, whose `ct.sym` doesn't have
   the package; javac gives either message for the same import depending on what else it compiles
   with it. `import nope.*;` and `import nope.Missing;` are `package nope does not exist`.
7. **Scopes and shadowing**, each compiling unless said: `class A<T> { static class T { void
   member() {} } T field; T method(T p) { p.member(); return p; } }` (the member type wins in the
   body); `class A<T> extends java.util.ArrayList<T> { class T {} }` used as `String s =
   a.get(0)` (the type parameter in the header); `class A extends B { static class B {} }` with a
   top-level `B` (the top-level `B`, whose method `a.top()` resolves), and alone, `cannot find
   symbol`; `class Sub extends p.Base { Entry e; }` with `import java.util.Map.Entry;` (`Base`'s
   member wins); `import java.util.List;` in package `p` with a `p.List` (the import wins);
   `import java.util.*;` with a `p.List` (the package wins); `import static p.Holder.*;` (a member
   `List`) with `import java.util.*;` and a use of `List`: `reference to List is ambiguous`, and
   no error when unused; two interfaces with a member `X` and a class implementing both:
   `reference to X is ambiguous` when used, none when not, and none when the second interface
   inherits the first's `X`; a private member type `X` of a superclass: `X has private access in
   B`, or the imported `p.X` when one is imported; an inner class's inherited `X` over its outer
   class's `X`; `import p.p; ... p.q.Z z;` with a type `p.p` having a member `q.Z` and a package
   `p.q` with a `Z`: the type's (types obscure packages); `class A<T extends A.Holder> { ... T.In
   x; }`: compiles (members through a bound).
8. **Supertype cycles.** `class A extends B.C {}` and `class B extends A { class C {} }`, with
   `class D extends A {}` and `class E { A a; B.C c; }`: `cyclic inheritance involving B`, once,
   whichever file is first, and nothing for `D` and `E`. `class A extends A.B { static class B {} }`
   and `interface I extends I.J { interface J {} }` are cyclic; `class A implements
   java.util.function.Supplier<A.In> { ... static class In {} }` compiles.
9. **Binary names with `$`.** A library with a top-level `p.A$B` (with a member `C`) and a `p.A`:
   `p.A$B ab; p.A$B.C c;` compiles, and `p.A.B b;` is `cannot find symbol`.

Observations 10 to 15 were made for the names in bodies. Each case's files were written out per
source set and compiled with `javac --release 27 -implicit:none -d out <files>` (dependency sets on
`-sourcepath`), and which variable each use read was read from `javap -c -p` of the classes:
`getfield`/`getstatic` and their owner for a field, a load of a local slot for a local.

10. **Pattern bindings** (`body-patterns.case`). A class with a field `String s` and methods using
    `s` after or inside each form: `if (!(o instanceof String s)) return; s.length();` reads the
    binding; with a then-branch that completes normally, the field (`getfield s`); `o instanceof
    String s && s.length() > 0` the binding in the right operand and the then-branch, the field
    in the else-branch; `!(o instanceof String s) || s.isEmpty()` the binding after the `return`;
    `!(o instanceof String s && s.isEmpty())` too; after `while (!(o instanceof String s))`, the
    binding, and the field when the body has a `break`; `?:` the binding in the then-expression and
    the field in the else; `for (; !(o instanceof String s); )` and `do ... while (!(o instanceof
    String s))` the binding after; `if (o instanceof String s) {} else { return; }` the binding
    after; one introduced in a nested block, the field after the block; `case String s when
    s.length() > 1 -> s.length()` the binding, and `default -> s.length()` the field; a `case
    String t:` group's statements the binding; a record pattern's `int x, String y` their
    bindings. Every file compiles.
11. **Obscuring in bodies** (`body-obscuring.case`). With a class `p.q` that has a static field `Z`
    and a member `In`, and a package `q` with a class `Z`, `q.Z` in package `p` is `getstatic
    p/q.Z` (the type obscures the package); `q.In.W` is `p/q$In.W`; a local `String Math` makes
    `Math.length()` a call on the local and `java.lang.Math.abs(-1)` still `java/lang/Math.abs`;
    with a local `String q`, `q.In in` and `q.In.class` are the type, since a type context looks
    for types and packages only. Compiles.
12. **A single-static import and an inherited field** (`body-fields.case`). `class Sub extends
    p.Base` with `import static q.C.X;` and `import static q.C.*;`, where `Base` and `C` both have
    a static `X` and a method `m`: `X` is `getstatic X` through `Sub`, `Base`'s, and `m()` is
    `invokevirtual m` on `Sub`, `Base`'s, over `q.C.m` from the on-demand import; a name only
    `C` and `D` have through on-demand imports is theirs, and `W`, which `C` has by a single-static
    import and `D` on demand, is `q/C.W`. `this.count`, `super.count`, `Sub.this.count`, `b.count`,
    and `b.next.count` read the fields of the types as declared. Compiles.
13. **Private members aren't inherited** (`body-fields.case`, p/Outer.java). In `class Outer { int
    x; static class A { private int x; } class B extends A { int f() { return x; } } }`, `x` is
    `getfield p/Outer.x` through `this$0`, not `A`'s private `x`, though both are in one top-level
    class. With `A` declaring a private `m()` and `Outer` an `m(int)`, a call `m()` in a subclass
    of `A` is `method m in class Outer cannot be applied to given types`: the type searched is
    `Outer`.
14. **A local record and the method's local** (`body-types.case`, p/Records.java). In a method
    with a local `int x` of a class with a static field `x`, a local record's method returning `x`,
    and a local enum's, are both `non-static variable x cannot be referenced from a static
    context`: javac finds the local, not the field.
15. **Local and anonymous classes** (`body-types.case`). A local `class Member extends Base`
    shadows the member type `Member` after its declaration (`new p/Types$1Member`) and not before
    (`p/Types$Member.fromMember`); its body reads `Base`'s inherited field (`getfield inherited`);
    a member type `List` shadows the import `java.util.List` in a body. In an anonymous `new
    Base() { ... }` in a method with a local `inherited`, `inherited` is the inherited field
    (`getfield inherited` in `p.Types$1`), and the method's other locals are captured (`val$value`,
    `val$other`). A generic local class's field of the method's type variable `T`, and a local
    class's member class used as `Outer.Inner`, compile.
