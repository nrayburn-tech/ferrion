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

- **Symbols** are identified by stable keys: a package by name, a type by its source set view and
  binary name, a member by its type, kind, name, and parameter types as written, a local by its
  `NodeId` in one tree. Project-wide data never holds a `NodeId` or an offset.
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
- **Types** by `(view, binary name)`. The view maps a binary name to one origin: a source type
  `(file, declaration path)` or a class file `(classpath entry, entry name)`. Two source sets that
  resolve `com.acme.Foo` to the same file share the symbol; a type in a jar used by both sets is
  shared through B1's archive cache.
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
    (8.9.3), and a record's canonical constructor, accessors, and fields (8.10.3) when they aren't
    declared;
  - whether the type is `@Generated` or Lombok-annotated by an annotation's simple name (resolved
    later, see [Generated sources](#generated-sources)).
- **Module declaration:** name, `open`, and each directive with its names as written.
- **Diagnostic positions** aren't stored. A diagnostic about a declaration finds the declaration
  again in the current tree by its path.

"As written" is the syntax of a type with its name segments, type arguments, array dimensions, and
annotations, as a small owned structure rather than tree nodes, so a summary outlives its tree.

Left out: method bodies, initializer expressions of non-`final` fields, instance and static
initializer blocks, local and anonymous classes, comments, and doc comments. Lombok (B4) will add
the members it generates to the summary.

## Source set views

### Order

For a type name `p.T` in a source set S, the view answers from the first of:

1. **The platform API,** for packages that a module of the platform owns (B1's `PlatformApi`).
   The design assumes other entries can't add types to those packages; what javac does with a
   source file or a classpath class in `java.lang` is an observation still to make.
2. **S's own sources,** every source root and generated-source root. A source type shadows any
   class file of the same name: javac does the same for the files it compiles (observation 2).
3. **S's classpath, in order:** each entry is a jar, a directory, or another source set, which is
   read from source (its summaries). The first entry that has the type wins, as with javac
   (observed in B1).

Packages exist if any of these has a type in them or, for sources, a file declares them. Packages of
the platform that aren't exported to everyone aren't visible (B1), but their classes still resolve
as supertypes of visible classes.

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
  unit imports `java.base` this way implicitly (JLS 7.3).

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

A qualified type name `Q.T` resolves `Q` as a package or type name first (JLS 6.5.4, 6.5.5):
`T` is then a member type of type `Q`, declared or inherited, or a top-level type of package `Q`.

Access isn't checked in B2, except where JLS makes it part of what is found: a private member type
isn't inherited (8.5), and a type in an unexported package isn't visible.

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
- a type with any supertype that doesn't resolve, or that has an incomplete hierarchy, has an
  incomplete hierarchy (SCOPE.md): rules treat its members, overrides, and subtyping as unknown,
  and member lookups that find nothing in it give an error symbol rather than "not found".

### Class-file types

A class file gives its binary name, flags, supertypes (`Signature` when present, else
`super_class` and `interfaces`), its fields and methods (synthetic and bridge members left out),
its member types and its outer class from `InnerClasses`, and its record components and permitted
subclasses. Member types are looked up through `InnerClasses`, never by splitting a binary name at
`$`, since `$` is legal in names. A class file that's malformed gives an error symbol with B1's
error as the root cause.

## Errors and incompleteness

- A name that doesn't resolve gets an error symbol carrying a root cause: the name, where it was
  used, and why (no such type, an ambiguity, a cycle, a missing classpath entry, a malformed class
  file). Names whose resolution reads an error symbol get error symbols that point at the same root
  cause, so B5 reports one diagnostic.
- Lombok-annotated types (any annotation from `lombok`) have incomplete hierarchies until B4.
- Syntax errors: names in a subtree with an error node resolve as usual where they can; a missing
  name is an error symbol with "syntax" as its cause.

## What B2 leaves to B3

- the field in `e.f` where `e` is any expression but a resolved variable of a non-generic declared
  type;
- the method a call chooses, and anything after it in a chain;
- `var`'s type, lambda parameters' types, and anything that needs inference;
- the type of `this`-qualified and `super`-qualified field access beyond its lookup class;
- members of a type variable (through its bounds) and of an intersection type.

The javac comparison in B2 covers B2's part only; the other names are recorded by the oracle from
the start, and compared from B3.

## Observations

Run with Temurin 27.0.1's `javac`, in a scratch directory. Each is a test in `ferrion_semantic`
once resolution exists.

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
   reports `reference to List is ambiguous` (`java.awt.List` and `java.util.List`).

Still to observe before the behavior is implemented, each as a small input:

- a source type in a package the platform owns (`java.lang.Foo` in the unnamed module), and a
  classpath jar with one;
- a class whose type parameter and member type have the same name, used in its body;
- an inherited member type and a same-named type from a single-type import, used in a subclass;
- a static-import-on-demand and a type-import-on-demand giving two types of the same name;
- `import module` of a module the unnamed module doesn't read by default (`jdk.incubator.vector`
  at release 21);
- `import` of a type in an unexported platform package (`jdk.internal.misc.Unsafe`), and the
  message javac gives;
- a supertype cycle through a member type (`class A extends B.C {}`, `class B extends A { class C
  {} }`): javac's error and which uses it reports.
