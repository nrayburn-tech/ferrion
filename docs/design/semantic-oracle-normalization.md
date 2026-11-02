# Semantic oracle normalization

B2's exit criterion in [ROADMAP.md](../ROADMAP.md) says names in the corpus resolve to the same
symbols as in javac, except for a documented, categorized list of disagreements, with code where
javac reports errors excluded ([semantic analysis](semantic.md#what-the-exit-criterion-means)).
`cargo oracle names` (`tools/oracle-check/src/names`) checks it, with javac's side from
`javac-oracle names` (`tools/javac-oracle`, `Names.java`). This document records what the oracle
reports, how javac's names and keys are matched with Ferrion's, what javac does that has no
counterpart in Ferrion, and what the corpus gives.

Everything about javac here was learned by running the oracle, never from javac's source: on small
inputs written for the purpose, and on the corpus. Observations are for JDK 27 (Temurin 27.0.1).
When javac's behavior is in doubt, run the oracle on an input that shows it, and update this
document and the comparison together.

## What is compared

For each source set of each corpus project's model (`corpus::model`, from its lock):

- **javac:** `javac-oracle names` compiles the source set (see [The compilation](#the-compilation))
  and writes, per file, its errors and every identifier and member select tree with its range and
  the key of the element `Trees.getElement` gives.
- **Ferrion:** for each file, a range and a key for every name in B2's part, in the oracle's terms.
  `tools/oracle-check/src/names/ferrion.rs` defines the format (`ResolvedName`) and the interface
  (`Resolver`); until name resolution exists (W5), a stand-in gives nothing, and every name is
  counted as not compared.

The check then:

1. **Leaves out whole files** javac reports an error in, files javac threw on, files javac didn't
   analyze, and `module-info.java` (see [Modules](#modules)), counting each by cause. Generated
   files are compared like the others, and counted apart.
2. **Leaves out names inside Lombok-annotated types:** each class that has an annotation from a
   `lombok` package, or a member with one, until B4. The oracle reports the classes' ranges.
3. **Sorts the rest into parts** ([B2's part](#b2s-part)): B2's are compared, B3's and the others
   are counted.
4. **Matches each B2 name** with Ferrion's name at the same range, in UTF-16 code units (Ferrion's
   byte offsets are converted, with a byte-order mark counted as one unit, as in the
   [parser comparison](parser-oracle-normalization.md#the-oracles-output)), and compares the keys
   as strings. For a method name the key compared is the type searched for the method, not the
   method javac chose.
5. **Categorizes each mismatch:** `missing` (Ferrion has no name at the range), `kind` (the keys
   name different kinds of symbol), `origin` (the same type from somewhere else), `symbol` (another
   symbol of the same kind). A mismatch listed in `tools/oracle-check/differences/names-known.txt`
   is counted under its known category instead, and the file gives each category's explanation.
   Mismatches and source sets the oracle's JVM gave no output for fail the run.

Ferrion's names at a range where javac has no B2 name are counted as extra, not compared.

## The oracle's output

One JSON object per line: one per `module-info.java` (`skipped`), one per other file, one per file
an annotation processor generated (`generated: 1`), and a last `sourceSet` object with the
processors registered on the classpath, the errors that belong to no file written (an error in a
file on the source path), `crash` when javac threw on the whole source set, and `millis`. Paths are
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
- **Offsets** of locals and local classes are the declaration's preferred position (its name, as
  in the [tree comparison](parser-oracle-normalization.md#positions)), found by a first walk over
  the file that records each local's and local class's element. Javac's binary names for local
  classes (`Outer$1Local`) are its own choice of digits, so they're never in a key; a member type
  of a local class is keyed by its own declaration. Descriptors write a local class as
  `L<local key>;`.
- **Ferrion's keys** (`SymbolKey` in `ferrion_semantic`) have the same form but for absolute paths
  and `class platform` for the platform; `oracle_key` in `ferrion.rs` converts them. Its `member`
  keys, for a single-static import's member, meet no javac key: javac gives those names no element.
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

## javac's ranges

- An identifier's range is the identifier's. A member select's starts where its first qualifier
  starts and ends after its identifier: `java.util.List` is `[0, 14]`, `java.util` `[0, 9]`, `java`
  `[0, 4]`. The outer select comes first in pre-order, then its qualifiers.
- Offsets are UTF-16 code units of the file as written: a Unicode escape in an identifier
  (`ab`) counts six units for its one character, and a character outside the BMP two.
- javac attributes every file of the source set even when one has a syntax error, and attributes
  the file with the error too: names after a parse error in `S.java` still have keys, and so do
  those of the other files. So one bad file doesn't cost the rest; it's left out whole by its own
  errors.

### Implicit names

Trees javac makes up have a start but no end (-1). They're counted as implicit and never compared:

- the default constructor's `super()` call, an `IDENTIFIER` at the class declaration whose key is
  the superclass's constructor (`method java/lang/Object.<init>()V`), also for anonymous and local
  classes and for an enum (`java/lang/Enum.<init>(Ljava/lang/String;I)V`) and a record
  (`java/lang/Record.<init>()V`);
- each enum constant's type and `new`, two `IDENTIFIER`s of the enum's type at the constant;
- a record component's type again, for the implicit field or accessor, at the component's type
  (`String` in `record B(int c, String d)` appears once with its range and once without an end).

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

`javac-oracle names` gives javac, through `JavacTask` with the standard file manager:

- every `.java` file of the source set on the command line but `module-info.java`;
- `--release` from the model; `-encoding UTF-8`;
- `-classpath` with the model's jars and directories, in order (a `.pom` a build lists is left out
  and named in the report), then each entry of the source sets it depends on that it doesn't have
  (see [Dependencies' classpaths](#dependencies-classpaths));
- `-sourcepath` with the source roots and generated-source roots of the source sets this one
  depends on, `-implicit:none`, and `-Xprefer:source`, so a type on the source path wins over a
  class file of the same name whatever their modification times are (javac's default,
  `-Xprefer:newer`, depends on them: [observation 2](semantic-names.md#observations));
- `-proc:full` and `-s <corpus>/generated/<project>/<id>`, the model's generated-source root, which
  `cargo oracle names` empties first; processors are found on the classpath, as javac finds them
  without `-processorpath`. Without a `-proc` option, javac 27 ran no processor: a processor
registered on `-cp` that generates a type didn't run, and the type wasn't found. Hence
`-proc:full`;
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
counted apart.

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
processor written for the purpose: the fallback then compiles without processors, and names of
generated types become errors in the files that use them.

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
`require`) resolve instead of being errors. Module declarations themselves aren't compared yet.

### Releases

`--release N` gives javac the platform API of release N only. A build that compiles with
`-source N -target N` on a newer JDK sees that JDK's whole API, internal packages included, and
the lock records its release as N either way. Guava's files that use `sun.misc` and one test that
uses `java.lang.classfile` fail with `--release 8` for that reason, though the build compiles
them. They're left out as javac errors.

The models have no compiler flags either. Lucene's build (`gradle/java/core-mrjar.gradle`)
compiles `lucene/core/src/java21` without `--release`, against an API jar it extracts from JDK 21
with the incubating vector packages patched into `java.base` and `java.lang.foreign` exported. With
`--release 21`, 15 of those files report `is a preview API and is disabled by default` (the
foreign memory API was a preview in 21) and 7 `package jdk.incubator.vector is not visible`.

### Annotation processors

Processors run from the classpath, as the design says. That covers jedis (JMH's
`BenchmarkProcessor` is registered on its test classpath), but not projects whose build gives
javac a separate processor path (Maven's `annotationProcessorPaths`): the corpus locks record
classpaths only. Helidon is the main case: its builder and service processors are its own modules,
named in `annotationProcessorPaths` of each module's compiler plugin, so none is on any classpath.
See [The corpus](#the-corpus) for what that costs.

## The corpus

`cargo oracle names jedis guava helidon lucene`, on a 10-core Mac with two JVMs of 4 GB at a time,
with the processors found on each classpath, on 1 November 2026. "Compared" files are those not
left out; javac's time is the sum of the JVMs' wall times, from an empty cache.

| Project | Source sets | Files | Module declarations | With javac errors | Compared | B2 names | B3 names | javac |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| jedis | 2 | 1,080 | 0 | 0 | 1,080 | 351,154 | 74,161 | 14 s |
| guava | 6 | 1,533 | 2 | 11 | 1,520 | 449,819 | 64,315 | 25 s |
| helidon | 376 | 4,439 + 36 generated | 214 | 855 | 3,406 | 572,753 | 97,799 | 664 s |
| lucene | 75 | 5,968 | 37 | 18 | 5,913 | 1,555,324 | 206,462 | 270 s |

- **jedis** compiles whole; JMH's processor is registered on its test classpath but finds nothing
  to do. Its two source sets take 14 s, so pull requests run it. Two of its files are commented out
  whole and have no names.
- **guava's** 11 are the [release](#releases) difference (`sun.misc`, `sun.security.jca`,
  `java.lang.classfile` with `--release 8`) and a test's `close()` that overrides nothing at
  release 8.
- **lucene's** 18 are its `java21` sources (see [Releases](#releases)) and one more file there.
- **helidon** without its processors: 855 files (20% of those compared or left out for errors)
  name types its processors generate (`cannot find symbol` and `package ... does not exist` for
  builder and service types), all in source sets whose build runs processors.

The B2 names, in all four, by context and kind of symbol: types in `type` contexts (554,368, plus
35,719 type variables), locals (777,444), fields (373,739), and types (183,931) in expressions,
imports (378,430 packages and 111,066 types, qualifiers included), method names' searched types
(352,600), annotations (92,056), and package declarations (54,700). Implicit names were 45,997,
`keyword` names 55,944, and names with no element 12,726.

### Helidon with its own processors

Helidon's processors aren't on any classpath (see [Annotation processors](#annotation-processors)).
As an experiment, `--processor-path` gave every source set the classes the lock-time Maven build
left in each module's `target/classes`, then the lock's jars: what Maven's
`annotationProcessorPaths` resolves to, roughly, but for every source set alike.

- With every module's classes, Helidon's API-stability processor ran everywhere and reported 1,375
  files (`Public API ... is missing stability annotation`) that the build compiles, so the build
  must run it only where it's configured, or with other options.
- Without the API-stability modules: 450 files generated, **7 files with errors** (5 `cannot find
  symbol`, 2 a processor's `Failed to write source file`), and 3,898 of 4,675 files compared,
  832,264 B2 names. The other 770 were left unanalyzed in 59 source sets, all for one cause:
  `io.helidon.common.Version`, which a Maven templating plugin generates into
  `common/common/target/generated-sources/templates`, isn't in any root of the model, so the files
  of `common/common` that use it fail on the source path, which stops attribution once processors
  run. If those compile once that root is in the model, 4,668 of 4,675 files (99.8%) are compared.
- javac took 908 s (summed), 456 s of wall time.

So once the corpus records each build's processor path and its other generated-source roots,
nearly all of Helidon compiles; until then, the default run leaves out a fifth of it.

## Still to observe

- Lombok on the classpath runs as a processor (hertzbeat): whether it works through the javac API
  on JDK 27, or makes `analyze()` throw.
- A static import whose member is inherited by the imported type: which type javac's method gives
  as the searched one, against the type the import names.
- Type annotations inside a qualified type (`java.lang.@A String`): which ranges javac gives the
  select and its qualifier.
