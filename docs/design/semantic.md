# Semantic analysis: symbols and name resolution

B2 in the [roadmap](../ROADMAP.md) builds symbols and name resolution across every source set, on
top of B1's class file reading. It starts with a design spike for incremental computation, and
takes in the inputs side of B1 (`ferrion_project`), which source sets depend on. This document is
the overview: what B2 serves, the shape of the crates, how the results are checked against javac,
the order of the work, and the questions for review.

| Topic | Document | In short |
| --- | --- | --- |
| Incremental computation | [Incremental computation](semantic-incremental.md) | The spike: salsa and a hand-written engine measured on lucene. A hand-written layered engine (inputs, file summaries, project symbols, body results) with fingerprint cutoffs, phases over a thread pool, a revision counter for cancellation, and trees kept only in a budgeted cache. |
| Symbols and name resolution | [Names](semantic-names.md) | Stable symbol keys, file summaries without bodies or offsets, source set views (platform, own sources, classpath in order), duplicates ambiguous, JLS scopes with javac's choices, and what B2 resolves versus B3. |
| Inputs | [Project inputs](project-inputs.md) | `ferrion_project`: flags and the environment passed in, a versioned JSON project model, classpath construction with manifest `Class-Path`, and validation messages that name each value's origin. |

## What B2 serves

- **Linter rules that need names only** (C2): unused imports and private members, rules that match
  a specific type or annotation, and rules that need to know what a name is.
- **Type attribution (B3):** symbols for every name, each type's supertypes and members, method
  candidates, and error symbols that spread through dependent code.
- **The language server:** results kept between edits, a change inside a method body costing a few
  milliseconds, cancellation in a few milliseconds, and memory proportional to summaries rather
  than trees.
- **The CLI:** one run over every source set of a build, using every core.
- **Incomplete analysis (B5):** root causes for every name that doesn't resolve, and incomplete
  hierarchies.
- **The formatter (A2, A3):** only the release of each file, from the project model.

## The crates

```
ferrion_project     inputs: flags, the project model, classpath construction, validation
ferrion_semantic    the engine, summaries, views, symbols, and resolution
```

`ferrion_semantic`'s modules:

- `engine`: inputs, revisions, the memo tables, cancellation, the tree cache, and batch phases;
- `summary`: file summaries, from one declaration-only walk of the tree;
- `packages`: each source set's package index and package entries, the first part of the project
  symbols, built in W2 to exercise the engine's verified entries; W4's views read it;
- `view`: a source set's view over its sources, classpath, and platform API;
- `symbols`: symbol keys, type entries for source and class-file types, error symbols;
- `resolve`: name contexts, scopes, and the lookups of [names](semantic-names.md#resolving-names);
- `body`: the walk over bodies, local scopes, and pattern binding scopes.

As built in W4, beside `engine`, `summary`, and `packages`: `names` (the interner, and types'
paths), `input` (the inputs, and `ProjectInput::from_project`), `classes` (class sources, their
listings, and class files read as class entries), `view` (lookups by name, packages, modules),
`symbols` (keys, resolved types, error symbols, and the entries' values), `resolve` (file scopes,
the lookups, and the supertypes, member types, and type entries), `decl_names` (each file's names
in declarations, and their keys for the javac comparison), and `testing` (a stand-in platform and
resolution cases, for tests and the fuzz target). `body`, built in W5, has each file's names in
bodies, and the local and anonymous classes they declare, keyed `TypeKey::Local`.

```rust
/// One per process. Holds the inputs and every memo table.
pub struct Engine;
impl Engine {
    /// `ProjectInput` is the engine's own input struct (source sets: name, release, classpath in
    /// order of other source sets and opened jars and directories, platform API, files with text
    /// and a generated flag), built from `ferrion_project::Project` and its `TypeInputs` by
    /// `ProjectInput::from_project`.
    pub fn new(input: ProjectInput) -> Engine;
    /// Bumps the revision and replaces the text. Never waits for running work.
    pub fn set_text(&self, file: FileId, text: Arc<str>);
    pub fn add_file(&self, set: SourceSetId, file: FileInput) -> FileId;
    pub fn remove_file(&self, file: FileId);
    pub fn set_release(&self, set: SourceSetId, release: u32);
    /// A read handle at the current revision; its queries return `Err(Cancelled)` once the
    /// revision moves.
    pub fn snapshot(&self) -> Snapshot;
}

impl Snapshot {
    // Built in W2:
    pub fn tree(&self, file: FileId) -> Result<Arc<SyntaxTree>, Cancelled>;
    pub fn summary(&self, file: FileId) -> Result<Arc<Summary>, Cancelled>;
    pub fn package_index(&self, set: SourceSetId) -> Result<Arc<PackageIndex>, Cancelled>;
    pub fn package(&self, set: SourceSetId, name: &str)
        -> Result<Option<Arc<Package>>, Cancelled>;
    /// Summaries, then package tables (then, from W4 and W5, symbols and each file's names),
    /// over `threads` workers; `each` runs per file in the last phase, last file first.
    pub fn run_batch(
        &self,
        threads: usize,
        each: impl Fn(FileId) -> Result<(), Cancelled> + Sync,
    ) -> Result<(), Cancelled>;

    // Built in W4:
    pub fn run_symbols(&self, threads: usize) -> Result<Vec<FileId>, Cancelled>;
    pub fn type_key(&self, file: FileId, index: usize) -> Result<TypeKey, Cancelled>;
    pub fn supertypes(&self, ty: TypeKey) -> Result<Option<Arc<Supertypes>>, Cancelled>;
    pub fn member_types(&self, view: SourceSetId, ty: TypeKey)
        -> Result<Arc<MemberTypes>, Cancelled>;
    /// A source type's resolved header; a class file's is its `class_entry`.
    pub fn type_entry(&self, ty: TypeKey) -> Result<Option<Arc<TypeEntry>>, Cancelled>;
    pub fn class_entry(&self, ty: TypeKey) -> Option<Arc<ClassEntry>>;
    pub fn resolve_type_name(&self, set: SourceSetId, canonical: &str)
        -> Result<TypeResolution, Cancelled>;
    pub fn resolve_binary(&self, view: SourceSetId, binary: Name)
        -> Result<Option<TypeKey>, Cancelled>;
    pub fn module_view(&self, set: SourceSetId) -> Result<Arc<ModuleView>, Cancelled>;
    /// Every name in the file's declarations resolved, keyed by `NodeId` of its tree.
    pub fn decl_names(&self, file: FileId) -> Result<Arc<FileNames>, Cancelled>;
    pub fn symbol_key(&self, symbol: &Symbol) -> Result<SymbolKey, Cancelled>;

    // Built in W5 (`names` as planned is `file_names`, since `Snapshot::names` gives the
    // interner):
    /// Every name in the file's bodies resolved, keyed by `NodeId` of its tree, with the local
    /// and anonymous classes they declare.
    pub fn body_names(&self, file: FileId) -> Result<Arc<BodyNames>, Cancelled>;
    /// Every name in the file, in declarations and bodies, keyed by `NodeId` of its tree.
    pub fn file_names(&self, file: FileId) -> Result<Arc<FileNames>, Cancelled>;
}
```

Dependencies: `ferrion_syntax` and `ferrion_parser` (the engine parses), `ferrion_lexer` (names
with Unicode escapes translated), `ferrion_classfile` and `ferrion_diagnostics` (from W4), and
`ferrion_stack` for the few recursive steps (body walks use the tree's flat pre-order walk with an
explicit scope stack, so deep nesting doesn't recurse). The engine takes its own small input
struct, so it doesn't depend on how a project is loaded; as built, the adapter that makes one from
a `ferrion_project::Project` lives in `ferrion_semantic::input` too, which makes
`ferrion_project` a dependency (a deviation from the plan, which put the adapter in the CLI and
language server; one adapter beside the input it builds is simpler, and `ferrion_project` brings
only `ferrion_classfile`, `ferrion_java_version`, and serde, which the workspace has already). No
new external crates; fingerprints use the standard library's `DefaultHasher`, whose fixed keys
make them repeatable within a process. They aren't persisted.

## Verification

### The javac oracle

`tools/javac-oracle` gains a `names` command, using javac's public API (`JavacTask`, `Trees`,
`javax.lang.model`). For one source set from a project model:

- **The compilation:** every file of the source set on the command line (observation 2 in
  [names](semantic-names.md#observations) is why), dependency source sets on `-sourcepath` with
  `-implicit:none`, the jars on `-classpath`, and `--release`. Annotation processors found on the
  classpath run, as in the build, with `-s` writing generated sources to
  `target/corpus/generated/<project>/<source set>`. `analyze()` attributes everything; nothing is
  written but the generated sources.
- **Errors:** each file's error diagnostics, so the comparison can leave those files out.
- **Names:** for every identifier and member select (in imports, annotations, types, and
  expressions), the start and end, and a key for the element `Trees.getElement` gives:

  | Element | Key |
  | --- | --- |
  | Package | `package java.util` |
  | Module | `module java.base` |
  | Type | `type java/util/Map$Entry`, plus `source <path>` or `class <classpath entry>` for where it came from |
  | Local or anonymous class | `type local <file>:<offset of declaration>` |
  | Type variable | `typevar <owner key> T` |
  | Field, enum constant | `field <owner binary name>.<name>` |
  | Method, constructor | `method <owner binary name>.<name><erased descriptor>` |
  | Local, parameter, binding | `local <file>:<offset of declaration>` |

- **Crashes:** one javac per source set. If javac crashes, the source set is run again one file at
  a time with the others on `-sourcepath`, so a crash loses one file, as the existing oracle's
  per-class runs do.

A normalization document (`semantic-oracle-normalization.md`, written with the comparison, like the
parser's) records how javac's ranges and keys are matched with Ferrion's: UTF-16 against byte
offsets, javac trees without a Ferrion node, and implicit names (an enum's `values`, a record's
accessors) that javac reports at the declaration.

### The comparison

`tools/oracle-check` gains a `names` check: for each corpus project, its project model, the oracle's
output (cached by input hash, like the parser's), and Ferrion's `run_batch`. Each javac name in
B2's part is matched with Ferrion's resolution of the same range, and each mismatch is counted
under a category. Known differences go in a file with a category and an explanation for each, as
the parser's do. The check runs weekly with the other oracle jobs, and on pull requests for one
small project.

### What the exit criterion means

"Names in the corpus resolve to the same symbols as in javac, except for a documented, categorized
list of disagreements. Code where javac reports errors is excluded." Under this design:

- **Names** are the names in B2's part ([names](semantic-names.md#decision)) in every source file
  of every corpus source set, at the release its lock gives.
- **The same symbol** is an equal key, including where a type came from (source or which classpath
  entry), since shadowing is part of B2.
- **Code where javac reports errors** is every file with an error diagnostic, left out whole.
  Lombok-annotated types are left out until B4, and the count left out is reported by cause.
- **Generated sources** come from running the build's annotation processors in the oracle, written
  to `target/corpus` and listed as generated-source roots in the corpus model, so the code that uses
  them resolves (question 3).

### Tests and fuzzing

- **Resolution tests:** small inputs for each rule in [names](semantic-names.md), each checked
  against javac when written (the observation becomes the test), with expected resolutions as
  snapshots.
- **The engine's differential test and fuzz target:** random edit sequences, comparing the engine's
  results with a fresh engine's ([incremental](semantic-incremental.md#checking-the-engine)).
- **A resolution fuzz target:** any text, parsed and resolved in a small project with a fixed
  classpath, never panics; deep nesting from the parser's stress patterns doesn't overflow.
- **Benchmarks:** `semantic/corpus-cold` (summaries, symbols, and names for a project) and
  `semantic/edits` (the spike's three edits), with peak memory, in `tools/bench`.

## Work order

After review, on `b2/integration`:

1. **W1, inputs** (`ferrion_project`), and `corpus project-model`. Independent of the rest.
2. **W2, the engine and summaries:** revisions, memo tables, cancellation, the tree cache, phases,
   the differential test harness, and `summary` with its tests. Independent of W1 behind a
   stand-in for `Project`.
3. **W3, the oracle side:** `javac-oracle names` and the oracle-check `names` check, reading
   Ferrion's results through a stand-in until W4 and W5 land. Independent of W1 and W2 except for
   the model format, which W1 fixes first.
4. **W4, project symbols:** views, packages, imports, type entries for source and class files,
   inheritance, cycles, modules. Needs W1 and W2.
5. **W5, body names:** local scopes, pattern bindings, expression and method names. Needs W4.
6. **Corpus run:** the comparison on every project, fixes, and the categorized list.

W1, W2, and W3 run as three worktree agents in parallel, while the formatter's agents finish, with
the heavy-job lock shared between them. W4 and W5 follow in turn; W5's areas (statements,
expressions, patterns) can split across agents once W4's lookups exist. A review stop after W2,
before W4, checks the engine against this design while it is still small.

## What was consulted

- **Specifications:** the Java Language Specification, Java SE 27 edition: chapter 6 (names,
  scopes, shadowing and obscuring), 7 (packages, modules, imports, compilation units), 8 and 9
  (members and inheritance), 13.1 (binary names), and 15.12.1 (the type to search for a method);
  JEP 511 (module imports), JEP 512 (compact source files); the JVM Specification for class files
  (as in B1).
- **Observation:** javac (Temurin 27.0.1) on small inputs written for this design, recorded in
  [names](semantic-names.md#observations).
- **Not consulted:** javac's source, or any other Java compiler's or tool's name resolution.
- **Crates:** salsa 0.28.5, as a candidate (see [incremental](semantic-incremental.md)).

## Questions for review

1. **Engine:** the hand-written layered engine, as recommended, or salsa (with trees kept out of
   it, a budgeted tree cache beside it, and a parser that polls for cancellation, to fix what the
   spike found)?
2. **B2's part of names:** packages, types, annotations, simple expression names, names qualified by
   a package or type, and the type searched for a method; names qualified by an expression's type,
   and chosen overloads, wait for B3. Is that the right line for the exit criterion?
3. **Generated sources in the corpus:** the corpus locks have no generated-source roots. The
   recommendation is that the oracle runs the build's annotation processors and writes their output
   to `target/corpus`, which the corpus model lists as generated-source roots. The alternative is
   to leave out every file javac can't compile without them: Helidon has 320 builder blueprints,
   and about 2,800 of its 6,723 files name a type they generate.
4. **`@Generated`:** any annotation whose simple name is `Generated` marks a type as generated,
   resolved or not (recommended), or only a fixed list of qualified names?
5. **A single source set's sources:** every file of the run's default file set, even when only some
   files are passed (recommended, so that names resolve in a pre-commit hook), or only the files
   passed?
6. **The project model format** in [inputs](project-inputs.md#the-project-model), with
   source sets as classpath entries. The Maven plugin (A4) and IntelliJ (A5) write it, and the
   formatter reads releases from it.
7. **Excluding code with javac errors:** whole files (recommended for B2), or only the members that
   contain an error?

## Decisions made in review

The user reviewed this design and accepted every recommendation:

1. **Engine:** the hand-written layered engine of
   [incremental computation](semantic-incremental.md), not salsa.
2. **B2's part of names:** packages, types, annotations, simple expression names, names qualified
   by a package or type, and the type searched for a method. Names qualified by an expression's
   type, and chosen overloads, are compared from B3.
3. **Generated sources in the corpus:** the oracle runs the build's annotation processors and
   writes their output to `target/corpus/generated/`, which the corpus model lists as
   generated-source roots.
4. **`@Generated`:** any annotation whose simple name is `Generated` marks a type as generated,
   resolved or not.
5. **A single source set's sources** are every file of the run's default file set, even when only
   some files are passed.
6. **The project model format** is as in [inputs](project-inputs.md#the-project-model), with source
   sets as classpath entries.
7. **Excluding code with javac errors:** whole files, for B2.

The work order stands: W1, W2, and W3 in parallel, with a review stop after W2 before W4.

### The engine review

After W2, the user reviewed the engine and summaries as built, accepted them, and settled the
questions W2 raised:

1. **Trees shrink their arrays to fit** when built, so the tree cache's budget counts what they
   hold.
2. **Names are interned and the "as written" type parts made smaller** at the start of W4, before
   the project symbols are keyed by them: summaries took 51 MB for lucene's 50 MB of source.
3. **Lombok** is recognized from imports and `lombok.`-qualified names until W4 resolves
   annotations, then by the resolved annotation.
4. **A record's `equals`, `hashCode`, and `toString`** are listed with its implicit members
   (JLS 8.10.3).

### The project symbols review

After W4, the user accepted the project symbols as built and settled the questions W4 raised:

1. **A dependency source set's types stay resolved in its own view**, once, and shared by every
   source set that reads them. Where javac resolves a name differently because a dependent source
   set redeclares the type (observation 5 in [names](semantic-names.md#observations)), the
   comparison lists it as a known difference, and the corpus run counts how often it happens. If
   the count isn't negligible, a dependency's references are resolved again by binary name in the
   reader's view, as a class file's are.
2. **A qualified name of more than 1,024 segments** is resolved up to the first 1,024 and is an
   error after them (`Cause::TooLong`), so the parser's stress patterns stay linear.

### The oracle review

After W3, the user accepted the oracle as built and settled the questions W3 raised, all for the
corpus run (W6):

1. **The lock and the project model record each build's annotation processor path** (Maven's
   `annotationProcessorPaths`, Gradle's `annotationProcessor` configurations) **and the generated
   source roots of other plugins**, such as Helidon's templates, so the oracle runs the build's
   processors and the code that uses generated sources resolves.
2. **The model records how a source set is compiled:** `--release` or `-source` and `-target`, and
   flags that change what it sees (`--enable-preview`, `--add-modules`, `--add-exports`). Ferrion
   uses them too, to choose the platform API a source set sees.
3. **Only language errors leave a file out.** A file javac never analyzed is counted on its own,
   apart from files with errors, and processors that only check code and generate nothing (such
   as Helidon's API stability checks) aren't run.
4. **Module declarations are compared**, through a separate modular compile of each source set
   with a `module-info.java`.

As built in W6a, the corpus's inputs side of the run (details in
[normalization](semantic-oracle-normalization.md#the-compilation) and
[inputs](project-inputs.md#the-project-model)):

1. The lock (format 2) reads each build's processor path, release, options for the JDK's modules,
   module path, and other plugins' generated-source roots from what the build ran: the Maven
   compiler plugin's debug log of each execution's javac options and source roots, and each
   Gradle compile task's options. A processor path entry that is one of the build's own modules
   is its published jar, so fetching gives the oracle the processors.
2. The project model (format 2) records `platform` (`release`, or `jdk` for `-source` and
   `-target`), `enablePreview`, `addModules`, `addExports`, `addReads`, `patchModules`, and
   `processorPath`; `ferrion_project` validates and exposes them, and `ferrion_semantic`'s input
   adapter gives a `jdk` source set its JDK's whole API (every package before release 9) and
   applies `--add-modules`, `--add-exports`, and `--patch-module` to what its code sees.
3. What a processor reports isn't a language error and leaves no file out, and
   `--should-stop=ifError=FLOW` keeps it from stopping javac before attribution. Processors that
   only check code aren't recognized in advance, since nothing tells them apart before they run;
   one that created no file is reported as such, and running it changes nothing the comparison
   sees (a deviation from "aren't run", which the user accepted; see below).
4. Each `module-info.java` is compiled on its own with its module's and its dependencies' roots
   on the module source path, and its build's module path with those of its dependencies.

### The corpus inputs review

After W6a, the user accepted the corpus inputs as built and settled the questions W6a raised, as a
follow-up once W6b's comparison has landed:

1. **Each project's oracle runs on the JDK its lock names**, so that Lombok runs where its version
   supports the JDK: hertzbeat runs on JDK 25. Its test files that use the generated members of
   its main source set's types still fail, since javac reads those types from source, where
   Lombok doesn't run; they wait for B4.
2. **Processors that only check code run**, and nothing they report counts (decision 3 above, as
   built), rather than being recognized and skipped, which would take a list of processor names
   per project.
3. **A `corpus generate` command** runs each build's source generation after `corpus fetch` (Maven's
   `generate-sources`, or the Gradle generator tasks), so the generated-source roots of plugins
   other than processors, and processor jars a build makes for itself (caffeine's), exist on CI as
   they did when locking. The weekly job runs it.
