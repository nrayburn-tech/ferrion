# Project inputs

How `ferrion_project` turns command-line flags or a project model into source sets, builds each
source set's classpath, and validates what it was given. B1 in the [roadmap](../ROADMAP.md) lists
this work; it was left out of the [class file design](classfile.md) and is done as B2's first
workstream, since source sets drive name resolution. Part of the
[semantic analysis design](semantic.md).

## Decision

- **Two stages.** Loading the inputs reads flags, the environment, and the project model, and gives
  the source sets with their roots and releases, which is all the formatter needs. Opening the type
  inputs (the JDK, jars, and directories) happens only when an enabled rule needs types, as SCOPE.md
  requires.
- **`ferrion_project` doesn't parse the command line.** The CLI (A3, and C's `lint`) parses flags
  into a plain `InputFlags` struct, and reads `JAVA_HOME` and the working directory into an
  `Environment`, so loading is a pure function and tests need no process environment.
- **Every value carries its origin:** a flag, the classpath file, the project model (with the
  source set), a jar's manifest, or `JAVA_HOME`. Every message names it (SCOPE.md).
- **The project model is JSON with a format version,** shaped like the corpus lock files: a list
  of source sets, each with roots, release, JDK, and a classpath whose entries are paths or other
  source sets, in order, and from format version 2, how it is compiled where that changes which
  names its code sees (decision 2 of the [oracle review](semantic.md#the-oracle-review)).

## Single source set from flags

Without `--project-model`, there is one source set:

| Value | From | When missing |
| --- | --- | --- |
| Release | `--java-version` | Usage error for `format` and `lint` (SCOPE.md) |
| Sources | the files the run covers (SCOPE.md's default file set and ignores) | |
| Classpath | `--classpath`, or `--classpath-file` | Unset: type inputs missing |
| Module path | `--module-path` | Empty |
| JDK | `--jdk`, else `JAVA_HOME` | Type inputs missing |

- **Sources** are every `.java` file the run's default file set covers, even when only some files
  are passed on the command line, so that a pre-commit hook passing two files still resolves names
  against the rest (decision 5 in [review](semantic.md#decisions-made-in-review)). There are no
  generated-source roots in this mode; they come from a project model.
- **Classpath syntax:** entries separated by the platform's path separator (`:`, or `;` on
  Windows), as javac takes them. In a classpath file, line breaks also separate entries and blank
  lines are ignored, so both `mvn dependency:build-classpath -Dmdep.outputFile=...` output and one
  entry per line work. Empty entries (`a.jar::b.jar`) are left out. Relative entries, in a flag or
  in the file, are against the working directory, as they would be on javac's command line.
  Giving both `--classpath` and `--classpath-file` is a usage error.
- **Every file is in the one source set,** wherever it is: its id is `default`, its root the
  working directory, and the CLI decides which files the run covers.
- **Unset and empty** differ: no classpath flag means the type inputs are missing; `--classpath ""`
  means the code has no dependencies beyond the platform.

## The project model

Written by integrations (the Maven plugin, IntelliJ), never by hand, never committed (SCOPE.md).
Paths are absolute, or relative to the model file's directory.

```json
{
  "formatVersion": 2,
  "sourceSets": [
    {
      "id": "core/main",
      "javaRelease": 21,
      "jdk": "/opt/jdk-21",
      "sourceRoots": ["core/src/main/java"],
      "generatedSourceRoots": ["core/target/generated-sources/annotations"],
      "classpath": [
        { "sourceSet": "api/main" },
        { "path": "/home/me/.m2/repository/com/google/guava/guava/33.4.0-jre/guava-33.4.0-jre.jar" }
      ],
      "modulePath": []
    },
    {
      "id": "core/test",
      "javaRelease": 21,
      "jdk": "/opt/jdk-21",
      "sourceRoots": ["core/src/test/java"],
      "classpath": [
        { "sourceSet": "core/main" },
        { "sourceSet": "api/main" },
        { "path": "/home/me/.m2/repository/junit/junit/4.13.2/junit-4.13.2.jar" }
      ],
      "processorPath": [
        { "path": "/home/me/.m2/repository/org/openjdk/jmh/jmh-generator-annprocess/1.37/jmh.jar" }
      ]
    },
    {
      "id": "core/main21",
      "javaRelease": 21,
      "platform": "jdk",
      "addModules": ["jdk.incubator.vector"],
      "addExports": ["java.base/jdk.internal.misc=ALL-UNNAMED"],
      "patchModules": { "java.base": ["core/src/generated/jdk21.apijar"] },
      "sourceRoots": ["core/src/java21"],
      "classpath": [{ "sourceSet": "core/main" }]
    }
  ]
}
```

- **`id`** is any unique, non-empty string; the Maven plugin uses `<module path>/<main|test>`.
- **`generatedSourceRoots`, `classpath`, and `modulePath`** may be left out, for none. A model
  always gives a classpath, so in a model the type inputs are missing only without a JDK.
- **`--classpath`, `--classpath-file`, and `--module-path`** can't be combined with a model, which
  gives each source set's; `--jdk` and `--java-version` can, as fallbacks.
- **Dependencies on source sets** are classpath entries, so their order relative to jars is kept,
  as javac's first-match rule needs. Integrations write a `sourceSet` entry where the build has
  that source set's output directory (`target/classes`), since Ferrion reads it from source.
- **`jdk`** may be left out: then `--jdk`, then `JAVA_HOME`, with the origin recorded.
- **`javaRelease`** is required.
- **How a source set is compiled** (format version 2), as javac's options for it, each left out
  for javac's default:
  - **`platform`** is `release` (the default) for a build that gives `--release`, so the code
    sees the API of `javaRelease` alone, and `jdk` for one that gives `-source` and `-target` (or
    neither), so the code sees the JDK's whole API, internal packages included: every package
    before release 9 (`-source 8` resolves `sun.security.jca` and `jdk.internal.misc`), and from 9
    what the JDK's module graph exports to the unnamed module ([observations](#observations)).
  - **`enablePreview`** is `--enable-preview`.
  - **`addModules`** are `--add-modules`' values: more root modules of the unnamed module's
    module graph (`ALL-DEFAULT`, `ALL-SYSTEM`, and `ALL-MODULE-PATH` too).
  - **`addExports`** and **`addReads`** are `--add-exports`' and `--add-reads`' values,
    `<module>/<package>=<target>(,<target>)*` and `<module>=<target>(,<target>)*`, the targets
    module names or `ALL-UNNAMED`.
  - **`patchModules`** are `--patch-module`'s: for each module, the jars and directories whose
    classes are in the module, before its own (lucene compiles `core/src/java21` with JDK 21's
    preview and incubating APIs patched into `java.base` that way).
- **`processorPath`** is the annotation processor path the build gives javac (Maven's
  `annotationProcessorPaths` resolved, Gradle's `annotationProcessor` configuration), entries
  like the classpath's; left out, processors are found on the classpath, as javac finds them
  without `-processorpath`, and `[]` is none (`-proc:none`). Ferrion runs no processors (their
  output is in `generatedSourceRoots`); it's for tools that compare with javac, such as the
  javac oracle.
- **Format version 2** added the compile options and `processorPath`, since a Ferrion that
  ignored them would resolve names differently from the build; an older Ferrion refuses such a
  model and says to upgrade, and this one reads version 1 models as ones without them.
- **Unknown fields** are a usage error, and a `formatVersion` newer than this Ferrion knows says to
  upgrade Ferrion, since the Maven plugin may be newer than the binary.
- **A file under the roots of more than one source set** is analyzed in each and its findings
  merged; it is formatted with the release of the first source set listing it. A file under no
  source set is formatted with `--java-version`, and is a usage error without it.

### Corpus models

The corpus lock's shape (modules, source sets, classpath entries pointing at source sets or jars)
is the same idea, so `cargo corpus project-model [--jdk <path>] [<project>...]` writes a model for
each fetched corpus project (all of them by default) to `target/corpus/models/<project>.json` (or
under `FERRION_CORPUS_DIR`). The oracle comparison and the semantic benchmark use it, through
`corpus::model` or the file.

- **Ids** are `<module path>/<source set>`, or just `<source set>` for the root module (`main`
  in jedis, `lucene/core/main` in lucene).
- **Paths** are absolute: source roots in the checkout, jars under `target/corpus/jars`, and files
  committed to the project in the checkout.
- **Generated sources:** each source set lists `target/corpus/generated/<project>/<id>` as its
  first generated-source root, where the oracle writes what the build's annotation processors
  generate (decision 3 in [review](semantic.md#decisions-made-in-review)), then the roots the
  build's other plugins generated sources into, in the checkout, as the lock records them
  (Helidon's templates, ANTLR's and protobuf's output, caffeine's code generator's). Until the
  oracle has run, the first doesn't exist, and the others exist only where the lock's build ran;
  each missing one is a warning.
- **How a source set is compiled** (`platform` and the options) and its **processor path** are
  as the lock records them: the build's own, read from what it ran (see the corpus tool's
  `lock`), with only the options for the JDK's own modules, since the corpus compiles every
  source set in the unnamed module and a build's options for its own modules (Maven's for a
  modular test compile, `--patch-module io.helidon.common=src/test/java`) name modules that
  don't exist there. A processor path entry that is one of the build's own modules is its
  published jar, so that fetching the corpus, without building it, gives the oracle the
  processors.
- **The module path** is the jars and directories of the build's `--module-path`, where it gives
  one (Helidon's and lucene's modular compiles); the build's own modules on it are left out, since
  they're source sets. The oracle compiles module declarations with it; Ferrion reads it after
  the classpath, as from any model, and an entry the classpath has too is read there.
- **The release** is the lock's, or the lock's JDK when the build sets none, as the parser checks
  use.
- **The JDK** is `--jdk`, else `JAVA_HOME`, for every source set, as the other corpus tools find
  theirs. The newest GA JDK serves every release the corpus uses, and the oracle runs with it.
  With neither, the model leaves `jdk` out and its reader's `--jdk` or `JAVA_HOME` applies.
- **What is left out** is printed: entries the lock marks unavailable, and dependencies on source
  sets the lock doesn't have. None of the current locks has either.

## Classpath construction

When type inputs are opened, each source set's classpath becomes a `ferrion_classfile::ClassPath`
at its release:

- **Jars** are opened through the shared `ArchiveCache`, resolved for the release when
  multi-release.
- **`Class-Path` manifest attributes** are expanded, as javac does (observed in B1, and the
  details [below](#observations)): each listed path, relative to the jar's directory, is inserted
  right after the jar, recursively and depth first (`a.jar` listing `b.jar c.jar`, and `b.jar`
  listing `d.jar`, gives `a b d c`). An entry already on the classpath is skipped where it repeats,
  so a jar a manifest lists keeps the manifest's place even when the classpath names it later.
  Entries are URLs: `%` escapes are decoded and `file:` URLs read, while other schemes are skipped
  with a warning. One that doesn't exist is skipped with a warning naming the jar whose manifest
  listed it, and a manifest that can't be parsed is a warning, its `Class-Path` ignored. A stack
  of what is left, and the repeat check, keep chains and cycles from recursing.
- **Directories** of class files are read as they are; one that doesn't exist is a warning, since
  it may be build output not produced yet (SCOPE.md), and is kept as an entry, so the language
  server sees it once it is built. A missing entry is told apart from a missing jar by its
  extension (`.jar` or `.zip`, ignoring case).
- **Source sets** become views over their summaries (see
  [names](semantic-names.md#source-set-views)). They keep their place among the other entries, and
  the `ClassPath` holds only the compiled ones.
- **Other files** are opened as zips whatever their name, as javac does. One named `.jar` or `.zip`
  that can't be read is a usage error, as in javac; any other (a `.pom` a build lists, as two
  corpus projects do) is skipped with a warning.
- **The module path** is read as more classpath entries, after the classpath (SCOPE.md: leniently),
  with each module's name recorded. An entry is a jar, an exploded module (a directory with
  `module-info.class`), or a directory of them, read in name order. A module's name is its
  `module-info.class`'s, else its manifest's `Automatic-Module-Name`, else the name derived from
  the file name as `ModuleFinder`'s documentation describes it (checked against
  `jar --describe-module`). Manifests' `Class-Path` aren't followed there, which javac doesn't do
  either.
- **The JDK** is opened with `Jdk::open` and asked for `platform(release)`, or for a `jdk`
  source set `platform(<the JDK's own version>)`, its whole image; B1's usage errors (a JDK 9 or
  10, a release the JDK can't serve) are passed through with the JDK's origin added. A JDK older
  than the release gets its own message, with help. Each JDK is opened once per run, and a
  problem that many source sets share (one JDK, one missing jar) is reported once. Which of its
  packages the code sees, as the compile options change it, is applied by `ferrion_semantic`'s
  input adapter.
- **Patched modules** (`patchModules`) are opened as jars and directories at the release, apart
  from the classpath, without following manifests: a missing jar is a usage error, and a missing
  directory a warning, as on the classpath.

## Validation

| Condition | Result |
| --- | --- |
| No release for a file being formatted or linted | Usage error |
| Release outside the supported range | Usage error |
| `--classpath` with `--classpath-file`, or either or `--module-path` with `--project-model`; an unreadable classpath file | Usage error |
| JDK path isn't a JDK, is older than the release, or can't serve it | Usage error, when types are needed |
| A classpath or module path jar doesn't exist, or isn't a zip | Usage error, when types are needed |
| A classpath directory doesn't exist | Warning |
| A classpath file that isn't a jar or a directory, and isn't named `.jar` or `.zip` | Warning, and skipped |
| Type inputs missing and an enabled rule needs them | Usage error |
| Project model: unreadable, malformed, unknown field, newer format version | Usage error |
| Project model: empty or duplicate `id`, unknown `sourceSet` reference, an entry with both or neither of `path` and `sourceSet`, a cycle of source set dependencies | Usage error |
| Project model: an `addModules` or `patchModules` module that isn't a module name, an `addExports` or `addReads` entry not in javac's form | Usage error |
| A `patchModules` jar doesn't exist, or isn't a zip; a `patchModules` directory doesn't exist | Usage error, or a warning for a directory, when types are needed |
| Project model: a source root or generated-source root that doesn't exist | Warning |
| A manifest `Class-Path` entry that doesn't exist, or isn't a local file; a manifest that can't be parsed | Warning |
| A module path entry's `module-info.class` that can't be read | Warning, and the module has no name |

`Project::load` reports the rows about flags and the model; `TypeInputs::open` the rest, since
only it opens anything. "No release for a file" is `Project::load`'s error without a model, and
`Project::no_release(file)` for a file under no source set of one.

Usage errors exit with status 2 and stop the run before any file is changed. Each message names
the value, its origin, and what to do:

```
error: classpath entry `/home/me/lib/gone.jar` doesn't exist
  from: --classpath-file build/cp.txt, line 3
error: release 17 can't be served by the JDK at `/usr/lib/jvm/java-8` (JDK 8)
  from: JAVA_HOME
  help: pass --jdk with JDK 17 or later
error: source sets `core/main` and `core/test` depend on each other
  from: project model `target/ferrion-model.json`
warning: classpath entry `/home/me/lib/b.jar` doesn't exist
  from: Class-Path in the manifest of `/home/me/lib/a.jar`
```

Values are shown made absolute; origins as they were given. The wording is checked with the rest
of the CLI's messages in A3 and C1.

## The crate

As implemented:

```rust
/// Flag values as the CLI parsed them. `None` is "not given".
pub struct InputFlags {
    pub java_version: Option<u32>,
    pub classpath: Option<String>, // `Some("")` is an empty classpath
    pub classpath_file: Option<PathBuf>,
    pub module_path: Option<String>,
    pub jdk: Option<PathBuf>,
    pub project_model: Option<PathBuf>,
}

/// What loading reads from the process, passed in.
pub struct Environment {
    pub working_dir: PathBuf, // absolute; relative flag values are against it
    pub java_home: Option<PathBuf>,
}

/// Where a value came from, for messages.
pub enum Origin {
    Flag(&'static str),
    ClassPathFile { path: PathBuf, line: u32 },
    ProjectModel { path: PathBuf, source_set: String }, // "" for the model as a whole
    Manifest { jar: PathBuf },
    JavaHome,
}

/// A value with where it came from.
pub struct Given<T> { pub value: T, pub origin: Origin }

pub struct Project; // the source sets
impl Project {
    /// Usage errors are returned with any warnings; on success, warnings are kept.
    pub fn load(flags: &InputFlags, env: &Environment) -> Result<Project, Vec<InputError>>;
    pub fn source_sets(&self) -> &[SourceSet];
    pub fn source_set(&self, id: SourceSetId) -> &SourceSet;
    pub fn is_from_model(&self) -> bool;
    pub fn warnings(&self) -> &[InputError];
    /// The source sets whose roots (generated ones included) hold `file`, in model order.
    pub fn source_sets_of(&self, file: &Path) -> Vec<SourceSetId>;
    /// Under a generated-source root: analyzed, never formatted or linted.
    pub fn is_generated(&self, file: &Path) -> bool;
    /// The release to format `file` with, and where it came from.
    pub fn release_of(&self, file: &Path) -> Option<(u32, Origin)>;
    /// The usage error for a file `release_of` has no release for.
    pub fn no_release(&self, file: &Path) -> InputError;
}

/// A source set's position in the model (0 for the one source set of flags).
pub struct SourceSetId; // index()

/// A source set as given; paths are absolute.
pub struct SourceSet {
    pub id: SourceSetId,
    pub name: String, // the model's `id`
    pub roots: Vec<PathBuf>,
    pub generated_roots: Vec<PathBuf>,
    pub release: Given<u32>,
    pub jdk: Option<Given<PathBuf>>,
    pub classpath: Option<Vec<ClassPathItem>>, // `None`: not given, so type inputs are missing
    pub module_path: Vec<Given<PathBuf>>,
    pub compile: CompileOptions, // the default for the source set of flags
    pub processor_path: Option<Vec<ClassPathItem>>, // `None`: processors from the classpath
}

/// javac's options that change which names a source set's code sees.
pub struct CompileOptions {
    pub platform: PlatformKind, // Release (`--release`) or Jdk (`-source` and `-target`)
    pub enable_preview: bool,
    pub add_modules: Vec<String>,
    pub add_exports: Vec<AddExports>, // module, package, targets; `to_unnamed()`
    pub add_reads: Vec<AddReads>,     // module, targets
    pub patch_modules: Vec<PatchModule>, // module, paths
}

pub enum ClassPathItem { Path(Given<PathBuf>), SourceSet(Given<SourceSetId>) }

pub struct TypeInputs;
impl TypeInputs {
    /// Opens the JDKs and classpaths. Errors are usage errors; warnings are returned with them.
    pub fn open(project: &Project, cache: &ArchiveCache)
        -> Result<(TypeInputs, Vec<InputError>), Vec<InputError>>;
    pub fn source_set(&self, id: SourceSetId) -> &SourceSetTypes;
    pub fn source_sets(&self) -> &[SourceSetTypes];
}

pub struct SourceSetTypes {
    pub jdk: Arc<Jdk>,
    pub platform: Arc<PlatformApi>, // the JDK's own for a `jdk` source set
    pub patches: Vec<(String, Vec<ClassesEntry>)>, // `--patch-module`'s, by module
    /// The classpath, manifests expanded, then the module path; source sets in their places.
    pub entries: Vec<TypeEntry>,
    /// The compiled entries of `entries`, in order.
    pub class_path: ClassPath,
}

pub enum TypeEntry { SourceSet(Given<SourceSetId>), Classes(ClassesEntry) }

pub struct ClassesEntry {
    pub entry: ClassPathEntry,
    pub path: PathBuf,
    pub origin: Origin,
    pub on_module_path: bool,
    pub module: Option<String>, // a module path entry's module name
}

pub struct InputError {
    pub severity: Severity, // Error or Warning
    pub message: String,
    pub origin: Origin,
    pub help: Option<String>,
}

/// The JSON format, with serde, for integrations that write it (the corpus tool does).
pub mod model; // ProjectModel, ModelSourceSet, ModelEntry, FORMAT_VERSION
```

It depends on `ferrion_classfile`, `ferrion_java_version`, `serde`, and `serde_json`. B1's jar
manifest gained `Automatic-Module-Name`, for module names.

## Observations

Observed with javac and the jar tool of Temurin 27.0.1, on jars written for this document (`jar
cfm a.jar manifest -C classes .`), each with a class `lib.X` whose one method tells the jars
apart. Compiling a file that calls one of the methods with `javac -cp a.jar` shows which jar's
`lib.X` came first.

- **Order:** `a.jar` listing `b.jar c.jar`, and `b.jar` listing `d.jar`: `d.jar`'s `lib.X` is
  found, not `c.jar`'s, so `b`'s list is inserted right after `b`, before the rest of `a`'s.
- **Repeats:** `-cp a2.jar:d.jar:c.jar`, where `a2.jar` lists `c.jar`: `c.jar`'s class is found,
  so the manifest's place wins over the later one on the command line.
- **Cycles:** `e.jar` listing `f.jar`, and `f.jar` listing `e.jar`, compiles.
- **Missing entries** are ignored; `-Xlint:path` warns `bad path element ... no such file or
  directory`. A missing jar named on the command line is ignored too, which Ferrion makes a usage
  error (SCOPE.md).
- **Paths:** `sub/c.jar` is against the jar's directory, `sp%20ace/c.jar` finds `sp ace/c.jar`,
  and `file:/abs/c.jar` and plain absolute paths work. A directory works with or without a
  trailing `/`.
- **Files that aren't zips:** `-cp x.pom:c.jar` with `x.pom` an XML file compiles, while a file
  named `bad.jar`, `bad.zip`, or `bad.JAR` that isn't a zip is `error: error reading bad.jar; zip
  END header not found`, on the command line or listed in a manifest. A zip named `zipped.pom` is
  read as an archive.
- **Module path:** a `Class-Path` of a jar on the module path isn't followed (with
  `--add-modules ALL-MODULE-PATH`, `package lib does not exist`); a directory of jars works.
- **The platform without `--release`** (javac 27, on a file naming `sun.misc.Unsafe`,
  `sun.security.jca.Providers`, `jdk.internal.misc.VM`, and `java.lang.classfile.ClassFile`):
  `--release 8` finds none of the four, and `--release 11` only `sun.misc` (`does not exist` for
  the others). `-source 8 -target 8` finds all four, with warnings for the internal ones, so
  before release 9 every package of the JDK is visible; `-source 11 -target 11` and `-source 21
  -target 21` find `sun.misc` and `java.lang.classfile` but report `package sun.security.jca is
  not visible`, the module graph's rule. On JDK 21, `-source 8` doesn't find
  `java.lang.classfile`, which JDK 21 doesn't have: the API is the running JDK's own.
- **Automatic module names** (`jar --describe-module`): `guava-33.4.0-jre.jar` is `guava`,
  `commons-lang3-3.17.0.jar` `commons.lang3`, `foo_bar--baz.jar` `foo.bar.baz`, `.x.y..jar`
  `x.y`, `foo-bar-1.0-SNAPSHOT.jar` `foo.bar`; `lib-1a.jar` (`lib.1a`), `1lib-2.0.jar`, and
  `Foo.Bar.JAR` have none. `Automatic-Module-Name` wins over the file name.

## Tests

- **Loading:** every row of the validation table, with the message as a snapshot; unset against
  empty classpaths; origin on each value; relative paths in a model; compile options and the
  processor path, and a version 1 model read without them.
- **Patched modules:** opened apart from the classpath, with their module; a missing jar.
- **Classpath files** in both formats, with Windows separators tested through a parameter rather
  than the platform.
- **Manifest `Class-Path`:** relative entries, chains, cycles, and missing entries, with jars
  written by `ferrion_classfile::testing`; the observations above, as tests.
- **Corpus:** `corpus project-model` for every checked-out corpus project loads without errors
  (warnings only for generated-source roots), and every source file of the lock is in exactly the
  source sets the lock lists. It runs in `cargo test --package corpus` where the corpus is fetched,
  and skips projects that aren't. An ignored test opens every project's type inputs with the JDK
  at `JAVA_HOME`: `cargo test --package corpus -- --ignored type_inputs`. With Temurin 27 they all
  open, with warnings only for manifest entries that don't exist and the two `.pom` files.
