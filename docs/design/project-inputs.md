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
  source sets, in order.

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
  entry per line work. Giving both `--classpath` and `--classpath-file` is a usage error.
- **Unset and empty** differ: no classpath flag means the type inputs are missing; `--classpath ""`
  means the code has no dependencies beyond the platform.

## The project model

Written by integrations (the Maven plugin, IntelliJ), never by hand, never committed (SCOPE.md).
Paths are absolute, or relative to the model file's directory.

```json
{
  "formatVersion": 1,
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
      ]
    }
  ]
}
```

- **`id`** is any unique string; the Maven plugin uses `<module path>/<main|test>`.
- **Dependencies on source sets** are classpath entries, so their order relative to jars is kept,
  as javac's first-match rule needs. Integrations write a `sourceSet` entry where the build has
  that source set's output directory (`target/classes`), since Ferrion reads it from source.
- **`jdk`** may be left out: then `--jdk`, then `JAVA_HOME`, with the origin recorded.
- **`javaRelease`** is required.
- **Unknown fields** are a usage error, and a `formatVersion` newer than this Ferrion knows says to
  upgrade Ferrion, since the Maven plugin may be newer than the binary.
- **A file under the roots of more than one source set** is analyzed in each and its findings
  merged; it is formatted with the release of the first source set listing it. A file under no
  source set is formatted with `--java-version`, and is a usage error without it.

The corpus lock's shape (modules, source sets, classpath entries pointing at source sets or jars)
is the same idea, so `corpus project-model <project>` writes a model for a fetched corpus project,
with the jars' paths in `target/corpus` and the JDK from the matrix. The oracle comparison and the
semantic benchmark use it.

## Classpath construction

When type inputs are opened, each source set's classpath becomes a `ferrion_classfile::ClassPath`
at its release:

- **Jars** are opened through the shared `ArchiveCache`, resolved for the release when
  multi-release.
- **`Class-Path` manifest attributes** are expanded, as javac does (observed in B1): each listed
  path, relative to the jar's directory, is inserted right after the jar, recursively, skipping
  entries already on the classpath and ones that don't exist (with a warning naming the jar whose
  manifest listed it).
- **Directories** of class files are read as they are; one that doesn't exist is a warning, since
  it may be build output not produced yet (SCOPE.md).
- **Source sets** become views over their summaries (see
  [names](semantic-names.md#source-set-views)).
- **Other files** (a `.pom` a build lists, as one corpus project does) are skipped with a warning,
  since they aren't zip files.
- **The module path** is read as more classpath entries, after the classpath (SCOPE.md: leniently),
  with each jar's module name recorded.
- **The JDK** is opened with `Jdk::open` and asked for `platform(release)`; B1's usage errors (a
  JDK 9 or 10, a release the JDK can't serve) are passed through with the JDK's origin added.

## Validation

| Condition | Result |
| --- | --- |
| No release for a file being formatted or linted | Usage error |
| Release outside the supported range | Usage error |
| JDK path isn't a JDK, is older than the release, or can't serve it | Usage error, when types are needed |
| A classpath or module path jar doesn't exist | Usage error, when types are needed |
| A classpath directory doesn't exist | Warning |
| Type inputs missing and an enabled rule needs them | Usage error |
| Project model: unreadable, malformed, unknown field, newer format version | Usage error |
| Project model: duplicate `id`, unknown `sourceSet` reference, a cycle of source set dependencies | Usage error |
| Project model: a source root that doesn't exist | Warning |
| A manifest `Class-Path` entry that doesn't exist | Warning |

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
```

The wording is checked with the rest of the CLI's messages in A3 and C1.

## The crate

```rust
/// Flag values as the CLI parsed them. `None` is "not given".
pub struct InputFlags {
    pub java_version: Option<u32>,
    pub classpath: Option<String>,
    pub classpath_file: Option<PathBuf>,
    pub module_path: Option<String>,
    pub jdk: Option<PathBuf>,
    pub project_model: Option<PathBuf>,
}

/// What loading reads from the process, passed in.
pub struct Environment {
    pub working_dir: PathBuf,
    pub java_home: Option<PathBuf>,
}

/// Where a value came from, for messages.
pub enum Origin {
    Flag(&'static str),
    ClassPathFile { path: PathBuf, line: u32 },
    ProjectModel { path: PathBuf, source_set: String },
    Manifest { jar: PathBuf },
    JavaHome,
}

pub struct Project; // the source sets
impl Project {
    pub fn load(flags: &InputFlags, env: &Environment) -> Result<Project, Vec<InputError>>;
    pub fn source_sets(&self) -> &[SourceSet];
    /// The source sets whose roots hold `file`, in model order.
    pub fn source_sets_of(&self, file: &Path) -> Vec<SourceSetId>;
    /// The release to format `file` with, and where it came from.
    pub fn release_of(&self, file: &Path) -> Option<(u32, Origin)>;
}

pub struct SourceSet; // id, roots, generated roots, release, classpath and JDK as given
pub struct TypeInputs; // per source set: the opened ClassPath and PlatformApi

impl TypeInputs {
    /// Opens the JDKs and classpaths. Errors are usage errors; warnings are returned with them.
    pub fn open(project: &Project, cache: &ArchiveCache)
        -> Result<(TypeInputs, Vec<InputError>), Vec<InputError>>;
}

pub struct InputError {
    pub severity: Severity, // Error or Warning
    pub message: String,
    pub origin: Origin,
    pub help: Option<String>,
}
```

It depends on `ferrion_classfile`, `ferrion_java_version`, `serde`, and `serde_json`.

## Tests

- **Loading:** every row of the validation table, with the message as a snapshot; unset against
  empty classpaths; origin on each value; relative paths in a model.
- **Classpath files** in both formats, with Windows separators tested through a parameter rather
  than the platform.
- **Manifest `Class-Path`:** relative entries, chains, cycles, and missing entries, with jars
  written by `ferrion_classfile::testing`.
- **Corpus:** `corpus project-model` for every corpus project loads without errors, and every
  source file of the lock is in exactly the source sets the lock lists.
