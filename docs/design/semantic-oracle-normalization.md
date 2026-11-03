# Semantic oracle normalization

B2's exit criterion in [ROADMAP.md](../ROADMAP.md) says names in the corpus resolve to the same
symbols as in javac, except for a documented, categorized list of disagreements, with code where
javac reports errors excluded ([semantic analysis](semantic.md#what-the-exit-criterion-means)).
`cargo oracle names` (`tools/oracle-check/src/names`) checks it, with javac's side from
`javac-oracle names` (`tools/javac-oracle`, `Names.java`). This document records what the oracle
reports, how javac's names and keys are matched with Ferrion's, what javac does that has no
counterpart in Ferrion, and what the corpus gives.

Everything about javac here was learned by running the oracle, never from javac's source: on small
inputs written for the purpose, and on the corpus. Observations on small inputs are for JDK 27
(Temurin 27.0.1); each corpus project runs on the JDK its lock names (see [Releases](#releases)).
When javac's behavior is in doubt, run the oracle on an input that shows it, and update this
document and the comparison together.

## What is compared

For each source set of each corpus project's model (`corpus::model`, from its lock):

- **javac:** `javac-oracle names` compiles the source set (see [The compilation](#the-compilation))
  and writes, per file, its errors and every identifier and member select tree with its range and
  the key of the element `Trees.getElement` gives.
- **Ferrion:** for each file, a range and a key for every name `ferrion_semantic` resolves, in the
  oracle's terms (see [Ferrion's side](#ferrions-side)). `--javac-only` runs javac's side alone,
  and counts every name as not compared. `--replay` also resolves each project with an engine
  that was given every file as an edit, and fails a source set whose names differ
  ([Checking the engine](semantic-incremental.md#checking-the-engine)).

The check then:

1. **Leaves out whole files** javac reports a language error in, files javac never analyzed, and
   files javac threw on, counting each by cause (decision 3 of the
   [oracle review](semantic.md#the-oracle-review)): what annotation processors report isn't a
   language error and leaves nothing out (see [Annotation processors](#annotation-processors)).
   `module-info.java` is compiled on its own and compared like the others (see
   [Modules](#modules)). Generated files are compared like the others, and counted apart.
2. **Leaves out names inside Lombok-annotated types:** each class that has an annotation from a
   `lombok` package, or a member with one, until B4. The oracle reports the classes' ranges.
3. **Sorts the rest into parts** ([B2's part](#b2s-part)): B2's are compared, B3's and the others
   are counted.
4. **Matches each B2 name** with Ferrion's name at the same range, in UTF-16 code units (Ferrion's
   byte offsets are converted, with a byte-order mark counted as one unit, as in the
   [parser comparison](parser-oracle-normalization.md#the-oracles-output)), and compares the keys
   as strings. For a method name the key compared is the type searched for the method, not the
   method javac chose.
5. **Categorizes each mismatch:** `missing` (Ferrion has no name at the range), `b3` (Ferrion
   leaves the name to B3), `kind` (the keys name different kinds of symbol), `origin` (the same
   type from somewhere else), `symbol` (another symbol of the same kind). A mismatch listed in
   `tools/oracle-check/differences/names-known.txt` is counted under its known category instead
   ([Known differences](#known-differences)). Mismatches that aren't known, source sets the
   oracle's JVM gave no output for, and source sets Ferrion failed on fail the run.

Ferrion's names at a range where javac has no B2 name are counted, not compared: those it leaves
to B3 apart from the others ("extra"): names in javac's other parts that B2 resolves anyway, and
ranges javac has no name at.

## The oracle's output

One JSON object per line: one per file but `module-info.java`, one per file an annotation
processor generated (`generated: 1`), one per `module-info.java` (`module: 1`), and a last
`sourceSet` object with the processors registered on the processor path (`processors`), each
processor that ran with the files it created and the errors it reported (`processorRuns`), the
errors that belong to no file written (an error in a file on the source path) of the main
compilation (`diagnostics`) and of the modular ones (`moduleDiagnostics`), `crash` when javac
threw on the whole source set, and `millis`. Paths are
relative to the corpus directory (`projects/jedis/src/...`, `generated/jedis/main/...`,
`jars/org/...`), so the output doesn't depend on where the corpus is. Each file has:

- `diagnostics`: its errors, as the other commands write diagnostics;
- `lombok`: `[start, end]` of each Lombok-annotated class;
- `names`: `[start, end, context, qualifier, key]` for each `IDENTIFIER` and `MEMBER_SELECT` tree,
  in the pre-order of `TreePathScanner`, with a sixth element for method names: the key of the
  type searched for the method, or `""` when that is B3's;
- `unanalyzed: 1` when javac never started analyzing a class of the file (see
  [The compilation](#the-compilation));
- `crash`, instead of the rest, when javac threw on the file even on its own.

A tree written under two parents (the type in `int a, b;`) is written once.

### Contexts

A name's context is where the chain it's part of appears; `java.util.List` is a chain of three
names: the select `java.util.List`, the select `java.util`, and the identifier `java`. The context
comes from the chain's outermost tree and its parent:

| Context | Where |
| --- | --- |
| `package`, `import`, `module` | a package declaration, an import, a module directive |
| `annotation` | an annotation's name |
| `annotation-element` | `value` in `@A(value = 1)` |
| `type` | a variable's or method's type, `throws`, `extends`, `implements`, `permits`, type arguments and bounds, the class `new` creates, casts, `instanceof`, record patterns, array creation |
| `method` | `m` in `m()`, `a.m` in `a.m()`; the qualifiers of `a.m` are `expression` |
| `reference` | the qualifier of a method reference (`A` in `A::m`), a type or an expression |
| `expression` | anywhere else |
| `keyword` | `this`, `super`, and `class`, which javac's trees have as names (`this`, `A.this`, `A.class`) |

### Qualifiers

A member select's qualifier is what its expression is: `p` a package, `t` a type, `this` for
`this` and `super` (also `A.this`), `v` a field or variable of a declared type without type
arguments whose class has no type parameters, declared with its type written (not `var`, not a
lambda parameter without a type), when the expression is a chain of names, and `e` anything else
(a call, an array, a generic type, a raw type). An identifier's is `-`.

### Keys

| Element | Key |
| --- | --- |
| Package | `package java.util` |
| Module | `module java.base` |
| Type | `type java/util/Map$Entry platform`, `type p/A source projects/x/src/p/A.java`, `type q/B class jars/q/b/1.0/b-1.0.jar` |
| Local or anonymous class, or a type declared in one | `type local <file>:<offset>` |
| Type variable | `typevar <owner key> T` |
| Field, enum constant | `field java/lang/System.out`, `field local <file>:<offset>.x` |
| Method, constructor | `method java/util/Collections.emptyList()Ljava/util/List;` |
| Local, parameter, exception parameter, resource, binding | `local <file>:<offset>` |
| No element | `none` |
| An unresolved type | `error <name>` |

- **Binary names** are written with `/` for the package and `$` for nesting, from
  `Elements.getBinaryName`.
- **Where a type came from** is `Elements.getFileObjectOf`: a source file (on the command line, on
  the source path, or generated) is `source <path>`; a class file in a jar or directory on the
  classpath is `class <entry>`; anything from the JDK (`ct.sym` with `--release`, or `jrt:`) is
  `platform`. A jar a manifest's `Class-Path` adds is named as itself.
- **Offsets** of locals and local classes are the position of the declaration's tree, found by a
  first walk over the file that records each local's and local class's element. For a variable
  that is its name. For a local class, or a member type of a local or anonymous class, it is its
  `class`, `interface`, `enum`, or `record` keyword, after any modifiers; for an anonymous class,
  its body's `{`, or for an enum constant's body the constant's name; for a compact constructor's
  parameter, which javac makes from the record component, the preferred position of the
  component's type (`String` in `@A String a`, the last `.` of `java.lang.String`, the `<` of
  `List<String>`, the first `[` of an array type, the `...` of `String...`), as observation 16
  in [names](semantic-names.md#observations) records. That is javac 27's position for the
  parameter; javac 21, 25, and 26 put it at the component's start, its annotations included
  (observed on a record with each kind of type), so the oracle takes the type's preferred
  position on every JDK. Javac's binary names for local classes
  (`Outer$1Local`) are its own choice of digits, so they're never in a key; a member type of a
  local class is keyed by its own declaration. Descriptors write a local class as `L<local
  key>;`.
- **Ferrion's keys** (`SymbolKey` in `ferrion_semantic`) have the same form and positions, but for
  absolute paths, byte offsets, and `class platform` for the platform; `ferrion.rs` converts
  them, each `local <file>:<offset>` of the file to UTF-16 units. A class a `--patch-module`
  adds to the platform is `class <the patch's jar or directory>`, as javac's is (see
  [Releases](#releases)). Its `member` keys, for a
  single-static import's member, meet no javac key: javac gives those names no element.
- **Descriptors** erase each parameter and the return type with `Types.erasure`; constructors
  return `V`. Synthetic parameters aren't in `javax.lang.model`, so an inner class's constructor
  has none for its outer instance.

## B2's part

From [names](semantic-names.md#decision), as the oracle's contexts and qualifiers give it:

- every name in a `package`, `import`, `module`, `annotation`, `annotation-element`, or `type`
  context, qualifiers included;
- in an `expression` or `reference` context: identifiers, and selects qualified by a package, a
  type, `this`, or `super`; a select qualified by `v` is B2's only when its qualifier is (so in
  `a.b.c`, `.c` is B2's when `a` is a local and `b` a field, each of a plain declared type);
- in a `method` context: identifiers and selects qualified by a package, a type, `this`, or
  `super`, compared by the type searched (JLS 15.12.1);
- everything else is B3's: selects on calls, arrays, generic or raw types, `var` variables and
  untyped lambda parameters, and calls on any expression.

Not compared at all: `keyword` names, implicit names, names whose key is `none`, and names inside
Lombok-annotated types.

The searched type for a simple method name is the innermost enclosing class (anonymous and local
ones included) with a member method of that name in `Elements.getAllMembers`; without one, the
method came from a static import, and the type is the method's owner. For `Q.m()` it's `Q`; for
`super.m()` and `this.m()`, the type of `super` or `this`.

## Ferrion's side

`Ferrion` in `tools/oracle-check/src/names/ferrion.rs` runs `ferrion_semantic` as the CLI will:

- **One engine per project:** the project's model (`corpus::model`, with the JDK the project's
  lock names as each source set's, the oracle's), loaded by `ferrion_project` with its type
  inputs opened, made
  into a `ProjectInput` by `ProjectInput::from_project`, so the compile options, processor and
  module paths, and generated roots in the model are the oracle's too.
- **Source set by source set,** as the oracle finishes each: the files the oracle generated for it
  are added to the engine (or changed, or removed), the project symbols are brought up to date
  (`Snapshot::run_symbols`, computed once and verified after), and its files' names resolved
  (`Snapshot::run_names`), each file's mapped to ranges and keys and then dropped. A project's
  source sets are resolved one at a time, since a generated file added for one would cancel
  another's snapshot; the engine is dropped after the project's last source set in the run.
- **Mapping:** each `Name`, `ClassType`, `NameExpr`, field access, method call, and element-value
  pair name is one name, with [javac's range](#javacs-ranges), and its symbol's key in the
  oracle's form ([Keys](#keys)); for a method name, the key of the type searched. Names
  `ferrion_semantic` leaves to B3 (`Symbol::LeftToB3`) are given as `b3 <why>`.

The report gives Ferrion's time per project: loading the model and its jars, the project symbols,
and the names of every source set the run compares.

## javac's ranges

- An identifier's range is the identifier's. A member select's starts where its first qualifier
  starts and ends after its identifier: `java.util.List` is `[0, 14]`, `java.util` `[0, 9]`, `java`
  `[0, 4]`. The outer select comes first in pre-order, then its qualifiers.
- A class type's name ends at its last identifier: `List` in `List<String>` is `[0, 4]`, and in
  `@A List`, the `IDENTIFIER` is `List` alone. One exception: a variable arity parameter's element
  type with annotations on its `...` alone, `Object @A ... args`, ends after the annotations (as
  in the [tree comparison](parser-oracle-normalization.md#spans-that-dont-contain-their-children)).
- A resource that is a variable, `try (r; ...)`, ends after its `;` (`r;` is the `IDENTIFIER`),
  as observed with the oracle on a resolution case (`body-corpus.case`).
- Ferrion's side gives these ranges (`javac_range` in `ferrion.rs`).
- Offsets are UTF-16 code units of the file as written: a Unicode escape in an identifier
  (`ab`) counts six units for its one character, and a character outside the BMP two.
- javac attributes every file of the source set even when one has a syntax error, and attributes
  the file with the error too: names after a parse error in `S.java` still have keys, and so do
  those of the other files. So one bad file doesn't cost the rest; it's left out whole by its own
  errors.

### Implicit names

Trees javac makes up have a start but no end (-1), or an empty range. They're counted as implicit
and never compared:

- the default constructor's `super()` call, an `IDENTIFIER` at the class declaration whose key is
  the superclass's constructor (`method java/lang/Object.<init>()V`), also for anonymous and local
  classes and for an enum (`java/lang/Enum.<init>(Ljava/lang/String;I)V`) and a record
  (`java/lang/Record.<init>()V`);
- each enum constant's type and `new`, two `IDENTIFIER`s of the enum's type at the constant;
- a record component's type again, for the implicit field or accessor, at the component's type
  (`String` in `record B(int c, String d)` appears once with its range and once without an end);
- the superclass of an anonymous class a qualified `new` creates: in `o.new Inner<String>() {}`,
  a `MEMBER_SELECT` of `Inner` with an empty range at the end of `Inner`, after the `IDENTIFIER`
  with its own range, and, without ends, its qualifiers.

Corpus counts are in [The corpus](#the-corpus).

### Trees without a Ferrion counterpart

- `this` and `super` are `IDENTIFIER`s whose element is a field named `this` or `super` of the
  enclosing class (`field p/A.this`); `A.this`, `A.super`, and `A.class` are `MEMBER_SELECT`s whose
  element is a field named `this`, `super`, or `class`. All are `keyword`.
- `var` is a `VAR_TYPE` tree, not a name, and an untyped lambda parameter has no type tree.
- An array's `length` has the element `field Array.length`: its owner prints as `Array`.

### No element

`Trees.getElement` gives nothing for an on-demand import's last name (`java.util.*`), a static
import's member (`java.util.Collections.emptyList`), or a static on-demand import's `*`; their
qualifiers have elements. These are B2's names, but javac's side can't say what they are, so they
are counted, not compared.

## The compilation

`javac-oracle names` gives javac, through `JavacTask` with the standard file manager, what the
build gives it (decision 2 of the [oracle review](semantic.md#the-oracle-review)), from the
model, which has it from the lock:

- every `.java` file of the source set on the command line but `module-info.java`, those under
  the roots the build's other plugins generate sources into included;
- `--release` from the model, or `-source` and `-target` for a source set whose build gives
  them (`platform: jdk`; see [Releases](#releases)); the build's options for the JDK's modules
  (`--enable-preview`, `--add-modules`, `--add-exports`, `--add-reads`, `--patch-module`), as
  given; `-encoding UTF-8`;
- `-classpath` with the model's jars and directories, in order (a `.pom` a build lists is left out
  and named in the report), then each entry of the source sets it depends on that it doesn't have
  (see [Dependencies' classpaths](#dependencies-classpaths));
- `-sourcepath` with the source roots and generated-source roots of the source sets this one
  depends on, `-implicit:none`, and `-Xprefer:source`, so a type on the source path wins over a
  class file of the same name whatever their modification times are (javac's default,
  `-Xprefer:newer`, depends on them: [observation 2](semantic-names.md#observations));
- `-proc:full` and `-s <corpus>/generated/<project>/<id>`, the model's first generated-source
  root, which `cargo oracle names` empties first; the build's processors (see
  [Annotation processors](#annotation-processors)), with its `-A` options. Without a `-proc`
  option, javac 27 ran no processor: a processor registered on `-cp` that generates a type didn't
  run, and the type wasn't found. Hence `-proc:full`; and `--should-stop=ifError=FLOW`, so that an
  error a processor reports doesn't stop javac before attribution;
- `-d` to a temporary directory deleted afterwards, for class files and resources processors write;
- `-Xmaxerrs` at its maximum, since javac otherwise stops reporting after 100 errors and later files
  with errors would look clean; `-nowarn -Xlint:none`.

`analyze()` attributes everything. Source sets are compiled after the ones they depend on, so the
sources those generated are on the source path.

An error in some file doesn't stop javac from attributing the others: an `ANALYZE` task event
starts for each of their classes, though flow analysis, which ends the event, is skipped for all
of them. An error reported while processors run is different: one a processor reports
(`compiler.err.proc.messager`, with no file), or one in a file on the source path that javac read
for a processor, means no `ANALYZE` event starts for any file, though no file has an error of its
own. What names resolve to then can't be trusted: in a small input whose processor reports an
error, the names still had their elements, but in Helidon's JMH benchmarks, after a processor
reported errors in other files, names had error types (`error
org.openjdk.jmh.runner.InfraControl.isFailing`) in files javac reported nothing in. The oracle
marks each file no `ANALYZE` event started for as `unanalyzed`, and the check leaves those out,
counted apart. `--should-stop=ifError=FLOW` (the option Guava's build gives javac for Error
Prone) makes javac attribute every file after such errors: with it, an `ANALYZE` event started for
every file of a source set whose processor reported an error, so no file is left unanalyzed for
that. Javac's own errors in files on the source path still count; so does the processor's error,
under its code.

### Dependencies' classpaths

javac reads the sources of a dependency on `-sourcepath` against the one classpath and source path
it was given, while Ferrion resolves a dependency's names in the dependency's own view
([names](semantic-names.md#dependent-source-sets)). A dependency's classpath can have entries its
dependents' doesn't: an optional dependency, or one only its processors need, a jar or another
source set. Helidon has many (`package io.helidon.config.metadata does not exist`, in a
dependency's file, was the most common). javac then reports errors in files on the source path,
which aren't compared, but once processors run, such an error stops javac before it attributes
anything (see above): in the Helidon experiment below it first left 1,080 files of 92 source sets
unanalyzed. So the oracle appends, after the source set's own entries, the classpath entries of
every source set it reads from source, transitively, and puts their source roots and generated
sources on the source path too, after its direct dependencies'. Appended last, they serve only
names that the source set's own entries don't have.

### Crashes

If javac throws on the source set, each file is compiled again on its own, with the source set's
roots and generated sources on the source path and no processors, so a crash loses only the files
it happens in, and the source set is reported as having crashed. A processor that throws makes
`analyze()` throw (`RuntimeException` wrapping the processor's exception), observed with a
processor written for the purpose and with Lombok 1.18.42 on javac 27
(`ExceptionInInitializerError`, caused by `ClassNotFoundException:
com.sun.tools.javac.tree.EndPosTable`). So the oracle's wrapper catches what a processor throws,
reports it (`threw` in `processorRuns`), and leaves the processor out from then on, as if it
weren't there, rather than losing the processors that work. Javac, given the exception, went on in
a way that lost a type: the class Lombok annotated wasn't found in another file that names it.

### Modules

A source set with `module-info.java` would need its dependencies on the module path, and javac
can't read a module's dependencies from `-sourcepath`. The oracle compiles every source set in the
unnamed module instead: `module-info.java` is left out of the command line, and hidden on the
source path by a forwarding file manager, since a `module-info.java` there makes javac compile the
files as members of that module: with a dependency's `module-info.java` in `-sourcepath dep`,
`javac -sourcepath dep src/u/U.java` reports `file should be on source path, or on patch path for
module` at `U.java`'s package declaration. In the unnamed module every platform module that exports
a package is readable, so a name that resolves in the named module resolves the same way here; the
difference is only that names the named module can't see (a package of a module it doesn't
`require`) resolve instead of being errors.

Module declarations are compared (decision 4 of the
[oracle review](semantic.md#the-oracle-review)) through a modular compilation of each
`module-info.java` on its own, after the source set's main compilation:

- the file alone on the command line, with `-implicit:none` and `-proc:none`;
- `--module-source-path <module>=<roots>` (the module-specific form) for its own module, with its
  source set's roots and generated sources, and one for each source set it reads from source that
  has a `module-info.java`, named as that file declares (the oracle parses it for the name);
- `--module-path` with the build's module path for the source set (or its classpath's jars, for
  a build that gives none), then the build's module paths of the source sets it reads from
  source: a module read from source requires what its own build put on its module path, and
  without it (`module not found: org.junit.jupiter.api` in Helidon's testing module, read for a
  module that requires it) javac also reported the declaration's own `requires java.logging` as
  `module not found`;
- release 9 or later, which a module declaration needs (`--release`, or `-source` and `-target`
  as the main compilation has them): Guava's build compiles its `module-info.java` in a separate
  execution with `--release 9` for the same reason;
- the build's options for the JDK's modules, as in the main compilation.

What javac needed, observed with javac 27 on a module `m.a` requiring a module of another source
root, a module in a jar, and `java.sql`, exporting a package, and using and providing a service:
the declaration alone compiles, javac reading on the module source path only what the directives
name (an `exports` of a package with no compilation unit there is `package is empty or does not
exist`); a module on both the module source path and the module path is taken from source,
without an error; and nothing else of the module needs compiling. Its names: in `requires m.b`,
the select `m.b` has the key `module m.b`, and its qualifier `m` and the declared module's own
name (`module m.a {`) have no element (`none`), so they aren't compared; `exports p` and `opens
p` name `package p`; `uses` and `provides ... with` name types, with where they came from. All
are in the `module` context.

### Releases

`--release N` gives javac the platform API of release N only. A build that compiles with
`-source N -target N` on a newer JDK sees that JDK's whole API, internal packages included: every
package before release 9, and the packages the module graph exports to the unnamed module from 9
([observations](project-inputs.md#observations)). The lock records which (`platform: jdk` for
`-source`), and the oracle compiles as the build does. Jedis and Guava compile with `-source 1.8
-target 1.8`: with `--release 8`, Guava's files that use `sun.misc` and `sun.security.jca`, and a
test that uses `java.lang.classfile`, were left out for errors, though the build compiles them;
with `-source 8` they compile. The oracle runs each project on the JDK its lock names, the one
its build ran on (decision 1 of the [corpus inputs review](semantic.md#the-corpus-inputs-review)),
so it sees the API the build saw: on JDK 21, Guava's test that uses `java.lang.classfile` has
`package java.lang.classfile does not exist`, and is left out, while JDK 27 has the package
(Guava's build leaves that test out). Ferrion's side gives such a source set the JDK's image
(`Jdk::image_platform`), not `ct.sym`'s release, which from JDK 25 has the JDK's own release
with only what its modules export, and no `sun.security.jca`.

Lucene's build (`gradle/java/core-mrjar.gradle`) compiles `lucene/core/src/java21` without
`--release`, with `--add-exports java.base/java.lang.foreign=ALL-UNNAMED`, `--add-exports
java.base/jdk.incubator.vector=ALL-UNNAMED`, and `--patch-module java.base=<an API jar it commits,
extracted from JDK 21>`, with JDK 21's preview foreign memory API and the incubating vector
packages in it. With `--release 21`, 15 of those files reported `is a preview API and is disabled
by default` and 7 `package jdk.incubator.vector is not visible`. With the build's options, the
names of the patched classes are javac's from the API jar (`class
projects/lucene/lucene/core/src/generated/jdk/jdk21.apijar`), not the platform's. Ferrion's
platform says which patch a class comes from (`Platform::patched_from`), and its keys name it
too.

### Annotation processors

The oracle runs each source set's own processors (decision 1 of the
[oracle review](semantic.md#the-oracle-review)): those its build's `-processor` names, or else
those registered (`META-INF/services/javax.annotation.processing.Processor`) on the processor path
its build gives javac, or else on its classpath, as javac finds them without one. An empty
processor path (Maven's `-proc:none`, Gradle's empty `annotationProcessor` configuration) runs
none. The lock reads the path from what the build ran: the processor path a Maven compiler plugin
execution logs (`annotationProcessorPaths` resolved, with what they depend on), and Gradle's
`annotationProcessor` configuration. Helidon's processors are its own modules; the lock records
each as the module's published jar, which `corpus fetch` downloads, so the oracle needn't build
Helidon to run them. `--processor-path` on `cargo oracle names` replaces every source set's, as an
experiment.

The oracle loads the processors itself and gives javac each through a wrapper (`setProcessors`),
which charges to the processor what happens while it runs: each file created through the file
manager, and each error it reports. The wrapper passes on javac's own processing environment,
since some processors need javac's: Lombok, given a wrapped environment, did nothing, and a class
it annotated wasn't found in the files that use it.

Only language errors leave a file out (decision 3). What a processor reports through its
`Messager` is an error with the code `compiler.err.proc.messager`, attached to a file when it
names an element: the check doesn't count it as a language error, and leaves no file out for it.
Processors that only check code and generate nothing, such as Helidon's API stability checks,
are recognized by what they did: one that created no file in a source set only checked it there,
and the report lists it so. They aren't skipped in advance, since nothing tells them apart before
they run; but with their reports not counted against files and `--should-stop=ifError=FLOW`
keeping their errors from stopping javac, running one changes nothing the comparison sees, which
is what not running it would give.

The build's other plugins generate sources too: Helidon's templating plugin writes
`io.helidon.common.Version` into `common/common/target/generated-sources/templates`, ANTLR's into
`generated-sources/antlr4`, protobuf's into `generated-test-sources/protobuf`, and caffeine's code
generator (a Gradle task) into `build/generated/sources`. The lock finds them by what the build
does: a source root of a Maven compiler execution that's in the module's build directory, and is
not the processors' (`-s`) directory, and a source directory of a Gradle source set in the
project's build directory. The model lists them as generated-source roots, and the oracle compiles
their files with the source set's own and puts them on dependents' source paths. They exist where
the lock's build ran; after `corpus fetch`, `corpus generate` makes them (decision 3 of the
[corpus inputs review](semantic.md#the-corpus-inputs-review)), as it does the jar on caffeine's
processor paths, which its build downloads.

## Known differences

Each mismatch that isn't fixed is listed in `tools/oracle-check/differences/names-known.txt`, by
file and javac's range, under one of these categories (the counts are of the run in
[The comparison](#the-comparison)):

- **`case-label`** (1,012): a `case` label's simple name in a switch whose selector's type needs
  attribution (`switch (range.getLowerBoundType()) { case OPEN: ... }`, a call's result). Whether
  it is an enum constant of the selector's type or a constant in scope depends on that type,
  which is B3's. Ferrion takes a variable in scope that could be a `case` constant (of a
  primitive type, `String`, or an enum type), which is right for `switch (count()) { case LOW:
  }`, and leaves the rest to B3 (observation 24 in [names](semantic-names.md#observations)). So
  the listed names are B3's in Ferrion (1,010) or, in 2, a `String` constant of the name in scope
  where the selector is an enum (helidon's `case ENTITY ->`, with a `String ENTITY` inherited).
  B3 resolves them from the selector's type.
- **`static-import-overload`** (4,830, 4,743 of them in caffeine's generated code): a method named
  alone that only static imports have, where several imported types declare methods of the name
  (`filter` from both `Sets.*` and `Collections2.*`). JLS 15.12.1 gives no type to search; the
  oracle's searched type is the declaring type of the overload javac chooses, and choosing is
  B3's, so Ferrion gives the first import's type. Every listed name's file imports both types
  statically.
- **`lombok-member`** (5, hertzbeat): a method Lombok generates, named qualified by its type
  (`QueryResult.builder()` for a `@Builder` class). On JDK 25, its lock's, Lombok runs, so javac
  has the method and searches `QueryResult`, while Ferrion sees no member Lombok generates and
  reports the type's members incomplete. Names inside Lombok-annotated types are left out until
  B4 (below); these are outside them. B4 resolves them.
- **`dependency-view`** (none): a dependency's type resolved in the dependency's own view
  (decision 1 of the [project symbols review](semantic.md#the-project-symbols-review)), where
  javac compiles the dependent source set as one namespace, so a type the dependent redeclares
  replaces the dependency's in the dependency's code too. The corpus has one such type, in
  hertzbeat (`CollectorAutoConfiguration`, redeclared by `hertzbeat-collector-collector` over a
  dependency's), and no name resolves differently because of it: the count is negligible, and
  dependencies' references stay resolved in their own views.

Lombok-annotated types aren't a category: their names are left out before comparing, until B4
(554 types in hertzbeat).

Three categories W6b listed were Ferrion's or the comparison's, and are fixed (decisions of the
[corpus inputs review](semantic.md#the-corpus-inputs-review), and W6c): `patched-platform`
(1,014 names; Ferrion's platform names the patch a class comes from), `platform-per-source-set`
(8; source sets compiled alike share one platform), and `jdk-internal-package` (11; a `jdk`
source set sees the JDK's image, and guava runs on JDK 21, its lock's, whose `ct.sym` has no
classes for its own release anyway).

## The corpus

`cargo oracle names` on every locked project, on a 10-core Mac with two JVMs of 5 GB at a time,
with each build's own processors, release options, generated roots, and module paths, each
project on the JDK its lock names, on 2 November 2026. "Compared" files are those not left out;
files include those processors generated (in brackets) and module declarations.

| Project | JDK | Source sets | Files | Module declarations | Language errors | Never analyzed | Compared | B2 names | B3 names |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| jedis | 21 | 2 | 1,080 | 0 | 0 | 0 | 1,080 | 351,154 | 74,161 |
| guava | 21 | 6 | 1,597 | 2 | 1 | 0 | 1,596 | 465,099 | 65,791 |
| caffeine | 26 | 29 | 1,309 (601) | 4 | 2 | 0 | 1,307 | 300,781 | 64,097 |
| spring-data-jpa | 21 | 4 | 901 (280) | 0 | 0 | 0 | 901 | 215,039 | 25,810 |
| lucene | 21 | 75 | 6,239 (271) | 37 | 0 | 0 | 6,239 | 1,694,558 | 220,760 |
| hertzbeat | 25 | 46 | 1,384 | 0 | 442 | 0 | 942 | 80,810 | 6,904 |
| helidon | 27 | 376 | 4,901 (462) | 214 | 0 | 0 | 4,901 | 1,092,038 | 186,631 |

No file was left unanalyzed or lost to a crash, and every module declaration compiled. Before
(W3's run, with processors from the classpath, `--release` everywhere, and no module
declarations), the four projects run then were:

| Project | Files | Module declarations | With javac errors | Unanalyzed | Compared | B2 names |
| --- | --- | --- | --- | --- | --- | --- |
| jedis | 1,080 | 0, left out | 0 | 0 | 1,080 | 351,154 |
| guava | 1,533 | 2, left out | 11 | 0 | 1,520 | 449,819 |
| helidon | 4,439 + 36 generated | 214, left out | 855 | 0 | 3,406 | 572,753 |
| lucene | 5,968 | 37, left out | 18 | 0 | 5,913 | 1,555,324 |

- **jedis** and **guava** compile with `-source 1.8 -target 1.8` now, as their builds do; Guava's
  11 files that `--release 8` failed compile. Guava's files gain `guava-tests/benchmark`, a test
  root build-helper adds, which the lock now reads from the compiler's source roots. On JDK 21,
  guava's `AbstractFutureTest` names `java.lang.classfile`, which JDK 21 doesn't have, and is
  left out; on JDK 27 (W6b's run) it compiled.
- **lucene's** `java21` sources compile with the build's `--add-exports` and `--patch-module`
  (see [Releases](#releases)).
- **helidon** compiles whole: its builder, service, feature, and copyright processors generate
  2,580 files (462 compared; the rest are resources and class files), the templating plugin's
  `Version` is in `common/common`'s generated roots, and the API stability enforcer runs in the 43
  source sets whose build names it, creating nothing and reporting nothing. Its 214 module
  declarations compile with each module's dependencies' module paths (see [Modules](#modules)):
  with only the module's own, 4 failed (`module not found: org.junit.jupiter.api` in a module
  read from source, and then `module not found: java.logging` in the declaration itself). This
  is the projected 99.8% and more: the projection's 7 files with errors were from a processor
  path shared by every source set.
- **caffeine's** 601 generated files are its code generator's (`build/generated/sources`), which
  its `codeGen` source set compiles against main's runtime classpath (its own compile task's,
  which the lock now records), and JMH's and jcstress's. Its 2 files with errors are test sources
  that name a type the code generator writes, which their source sets don't have.
- **spring-data-jpa** compiles whole, with Hibernate's and Querydsl's processors (352 and 242
  files) and JMH's.
- **hertzbeat** is Lombok's, and runs on JDK 25, its lock's. On javac 27, Lombok 1.18.42 throws
  (see [Crashes](#crashes)) and is left out, so the files that use the getters, setters, and
  constructors it would generate had language errors (`cannot find symbol`, 490 of the 610 left
  out then, 774 compared). On JDK 25, Lombok runs: 554 Lombok types are left out, and 442 files
  still have errors, all in test source sets that read their main source set's Lombok-annotated
  types from source, where Lombok doesn't run (`cannot find symbol` in 440); the report counts
  them so (`in source sets that read Lombok-annotated types from source`). They wait for B4.

The B2 names, in all seven, by context and kind of symbol: types in `type` contexts (783,376,
plus 59,105 type variables), locals (1,038,116), fields (529,482), and types (286,270) in
expressions, imports (558,411 packages and 163,547 types, qualifiers included), method names'
searched types (509,565), annotations (142,773), package declarations (80,495), and in module
declarations 1,409 modules, 6,317 packages, and 686 types. Implicit names were 174,569, `keyword`
names 116,688, and names with no element 22,880. javac took 1,394 s for Helidon, 527 s for
lucene, and 138 s for hertzbeat (summed over the JVMs), and under 70 s for each of the others;
the run took 1,119 s.

The processors that ran, by the files they created: Helidon's `AptProcessor` (234 source sets,
2,580 files), JMH's (5, 740), Hibernate's (4, 352), Querydsl's (4, 242), Spring Boot's
configuration metadata (28, 90), jcstress's (1, 20); and, creating nothing, Helidon's API
stability enforcer (43), Lombok's two (46), and Log4j's two (11). None reported an error.

### The comparison

`cargo oracle names` on every locked project, from the cache of the run above, with Ferrion's
side on 4 threads (`--threads`), on 2 November 2026. "Extra" are Ferrion's names where javac has
no B2 name, apart from those Ferrion leaves to B3. Ferrion's time is loading each project's
model and jars, its symbols, and every source set's names, under the heavy-job lock with other
work on the machine.

| Project | B2 names | Matched | Known | Unexplained | Extra | Left to B3 | Ferrion |
| --- | --- | --- | --- | --- | --- | --- | --- |
| jedis | 351,154 | 351,154 | 0 | 0 | 2,522 | 74,274 | 0.6 s |
| guava | 465,099 | 464,995 | 104 | 0 | 7,291 | 64,630 | 0.8 s |
| caffeine | 300,781 | 296,005 | 4,776 | 0 | 3,258 | 64,378 | 0.8 s |
| spring-data-jpa | 215,039 | 215,017 | 22 | 0 | 524 | 25,868 | 0.8 s |
| lucene | 1,694,558 | 1,694,275 | 283 | 0 | 3,402 | 220,261 | 2.3 s |
| hertzbeat | 80,810 | 80,805 | 5 | 0 | 23,585 | 10,188 | 2.4 s |
| helidon | 1,092,038 | 1,091,381 | 657 | 0 | 11,198 | 188,574 | 3.4 s |
| total | 4,199,479 | 4,193,632 | 5,847 | 0 | 51,780 | 648,173 | 11.1 s |

Known differences by project: guava 21 `case-label` and 83 `static-import-overload`; caffeine
33 and 4,743; spring-data-jpa 22 `case-label`; lucene 279 `case-label` and 4
`static-import-overload`; hertzbeat 5 `lombok-member`; helidon 657 `case-label`. 99.86% of the
B2 names match; without caffeine's generated static-import calls, 99.97%. W6b's run, before the
fixes of W6c, had 4,188,020 B2 names, 4,181,145 matched, and 6,875 known, hertzbeat on JDK 27.

The first full run of W6c had two problems of the comparison's own, both fixed: javac 21 to 26
key a compact constructor's parameter at the component's start, not its type's preferred
position as javac 27 does, so locals in records' compact constructors mismatched in caffeine,
spring-data-jpa, lucene, and hertzbeat (78 names; see [Keys](#keys)); and four of Helidon's
source sets failed on Ferrion's side (`cancelled`), whose generated files the oracle wrote again
after the engine had read them: the first change to the engine cancelled the snapshot comparing
the rest.

A run of one project, from the cache, peaked at 864 MB of memory for lucene and 505 MB for
helidon, the oracle's output included.

The first run, before any fix, had 50,167 mismatches in guava and lucene alone. What it found was
fixed in `ferrion_semantic`, each with a resolution case checked against javac (observations 16
to 24 in [names](semantic-names.md#observations)):

- the keys of local classes (their keyword, not their name), anonymous classes (their body's
  `{`, not `new`), and compact constructors' parameters (the component's type);
- a `module-info.java` below release 9, which made guava a named module reading nothing (40,000
  of the mismatches);
- `Object`'s methods as members of an interface; a local of a `switch` group in later groups; a
  type variable bounded by another in descriptors; `new Outer().new Inner()`; `C::new` with a
  field `C`; readability through modules other than the platform's (helidon's `java.sql`); and
  `case` labels of a selector Ferrion can't type, where a field of the name was taken;

and in the comparison's ranges: class types before their type arguments, annotated varargs, a
resource's `;`, and the empty range of a qualified `new`'s anonymous superclass.

### Helidon with its own processors

Before the locks recorded processor paths, an experiment gave every source set, through
`--processor-path`, the classes the lock-time Maven build left in each module's `target/classes`,
then the lock's jars: what Maven's `annotationProcessorPaths` resolves to, roughly, but for every
source set alike.

- With every module's classes, Helidon's API-stability processor ran everywhere and reported 1,375
  files (`Public API ... is missing stability annotation`) that the build compiles: the build runs
  it only in the 43 source sets that name it, where it reports nothing.
- Without the API-stability modules: 450 files generated, 7 files with errors (5 `cannot find
  symbol`, 2 a processor's `Failed to write source file`), and 3,898 of 4,675 files compared. The
  other 770 were left unanalyzed for one cause: `io.helidon.common.Version`, which a Maven
  templating plugin generates into `common/common/target/generated-sources/templates`, wasn't in
  any root of the model.

With each source set's own processor path and the templating plugin's root, as now, every file
compiles (see [The corpus](#the-corpus)).

## Still to observe

- A static import whose member is inherited by the imported type: which type javac's method gives
  as the searched one, against the type the import names. The corpus has no mismatch of it.
- Type annotations inside a qualified type (`java.lang.@A String`): which ranges javac gives the
  select and its qualifier. The corpus has no mismatch of it either.
