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
- `view`: a source set's view over its sources, classpath, and platform API;
- `symbols`: symbol keys, type entries for source and class-file types, error symbols;
- `resolve`: name contexts, scopes, and the lookups of [names](semantic-names.md#resolving-names);
- `body`: the walk over bodies, local scopes, and pattern binding scopes.

```rust
/// One per process. Holds the inputs and every memo table.
pub struct Engine;
impl Engine {
    pub fn new(project: &Project, types: Option<TypeInputs>) -> Engine;
    /// Bumps the revision, then replaces the text. Never waits for running work.
    pub fn set_text(&self, file: FileId, text: Arc<str>);
    pub fn add_file(&self, set: SourceSetId, path: PathBuf, text: Arc<str>) -> FileId;
    pub fn remove_file(&self, file: FileId);
    /// A read handle at the current revision; its queries return `Err(Cancelled)` once the
    /// revision moves.
    pub fn snapshot(&self) -> Snapshot;
}

impl Snapshot {
    pub fn summary(&self, file: FileId) -> Result<Arc<Summary>, Cancelled>;
    pub fn type_entry(&self, ty: TypeRef) -> Result<Arc<TypeEntry>, Cancelled>;
    pub fn resolve_type_name(&self, set: SourceSetId, binary_name: &str)
        -> Result<Resolution, Cancelled>;
    /// Every name in the file resolved, keyed by `NodeId` of `tree`.
    pub fn names(&self, file: FileId) -> Result<Arc<FileNames>, Cancelled>;
}

/// Summaries, then symbols, then each file's names, over `threads` workers; `each` runs per file
/// as soon as its names are ready.
pub fn run_batch(
    engine: &Engine,
    threads: usize,
    each: impl Fn(FileId, &FileNames) + Sync,
) -> Result<(), Cancelled>;
```

Dependencies: `ferrion_syntax` and `ferrion_parser` (the engine parses), `ferrion_classfile`,
`ferrion_project`, `ferrion_diagnostics`, and `ferrion_stack` for the few recursive steps (body
walks use the tree's flat pre-order walk with an explicit scope stack, so deep nesting doesn't
recurse). No new external crates; fingerprints use the standard library's `DefaultHasher`, whose
fixed keys make them repeatable within a process. They aren't persisted.

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
