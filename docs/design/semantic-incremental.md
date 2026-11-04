# Incremental computation

How semantic analysis keeps its results between edits, runs across threads, stops when its inputs
change, and bounds its memory. This is the design spike B2 in the [roadmap](../ROADMAP.md) asks for
before any symbol table is built. Part of the [semantic analysis design](semantic.md).

## Decision

**A hand-written, layered engine,** not salsa. The layers follow the shape of Java compilation:

1. **Inputs:** each file's text and each source set's configuration, with a version each.
2. **File summaries:** a file's declarations without bodies or offsets, computed from its tree and
   fingerprinted.
3. **Project symbols:** each source set's packages and types, and each type's resolved header
   (supertypes, member types, member signatures), built from summaries and class files. Each entry
   has a fingerprint, and an entry whose fingerprint didn't change keeps its value.
4. **Body results:** per file, the scopes and resolved names inside bodies, keyed by `NodeId`. Each
   result records the fingerprints of the project entries it read, and is reused while they match.

A change inside a method body changes only that file's tree and body results: its summary comes out
equal, so nothing in layer 3 is touched (measured below). A change to a signature reruns layer 3's
cheap cutoff and only the bodies that read the changed entry.

The main reasons, from the measurements:

- the two prototypes did the same work in the same time, with the same reruns after each edit, so
  speed doesn't decide;
- salsa's LRU drops memoized trees only when the next revision starts, so a cold run held every
  tree (388 MB for lucene, against 26 MB of summaries and results). Avoiding that means keeping
  trees out of salsa, which gives up most of what it would manage;
- setting an input in salsa blocks until every other handle has unwound, so the writer waits for
  the longest step in flight (up to 113 ms measured under load). The hand-written writer returns at
  once, and stale work is discarded when it finishes;
- Ferrion's dependency graph is shallow and fixed by the language (declarations, then bodies),
  which is where a general framework buys least;
- salsa brings 37 other crates, procedural macros, panics as its cancellation mechanism, and ten
  breaking releases between March 2025 and July 2026.

The risk of hand-written dependency tracking is a stale result: a body result reused when a project
entry it read has changed. A differential test guards against it (see
[Checking the engine](#checking-the-engine)).

## The conditions

B2's conditions, and how the design meets each:

| Condition | How |
| --- | --- |
| Cancellation in a few milliseconds | A revision counter, bumped before an input changes. Work polls it between files and every 1,024 tokens inside a body walk; setting an input never waits. |
| Scaling across threads | Batch runs go in phases over a work-stealing pool: summaries, then project symbols, then bodies. Within a phase, files are independent. |
| A change inside a method body doesn't invalidate project-wide results | Summaries hold no bodies and no offsets, and an unchanged summary fingerprint stops the change at layer 2. |
| Per-file summaries, not whole trees, kept for project-wide queries | Trees are dropped after each phase, except in a cache with a byte budget. Project-wide layers read summaries and class files only. |

## Options considered

### (a) salsa

[salsa](https://github.com/salsa-rs/salsa) 0.28.5, MIT OR Apache-2.0, used by rust-analyzer. Queries
are functions memoized on their arguments, and salsa records which queries each one called. After an
input changes, a memoized result is reused if every query it read still gives an equal value
("backdating"), so equal summaries stop a body edit the same way as in the hand-written design.

- **Dependencies:** recorded automatically, at any granularity, which is its main strength: there is
  no dependency to forget.
- **Threads:** each thread holds a clone of the database. Queries are demand-driven: a thread that
  needs a result another thread is computing waits for it.
- **Cancellation:** setting an input sets a flag that the other handles see at their next query
  call, or when code calls `unwind_if_revision_cancelled`. They unwind with a panic carrying
  `salsa::Cancelled`, and the setter blocks until every other handle has been dropped. A step that
  doesn't poll, such as parsing one file, delays the writer by its whole length.
- **Memory:** a query can have an LRU capacity, but entries are only evicted when a new revision
  starts. Within a revision, every memoized value stays.
- **Code:** inputs, tracked functions, and interned values are declared with macros, and most types
  carry a `'db` lifetime. The prototype's queries took 220 lines.
- **Cost:** 37 crates besides salsa (including `syn`, `rayon`, `parking_lot`,
  `crossbeam`, `hashbrown`, `inventory`, and `tracing`), and `unsafe` code inside salsa. Breaking
  releases: 0.19 in March 2025 through 0.28 in July 2026. Cancellation needs `panic = "unwind"`.

### (b) A hand-written layered engine

Versioned inputs, memo tables per layer validated by version and by fingerprints of what they read,
explicit phases for batch runs, and a revision counter for cancellation.

- **Dependencies:** recorded by hand, at the granularity the layers define: a body result lists the
  package and type entries it read, with their fingerprints. Forgetting one is a stale-result bug.
- **Threads:** phases run over a pool; within a phase, each file is independent. A layer-3 entry
  computed on demand by two threads at once is computed twice and stored once, which is cheap
  because entries are small.
- **Cancellation:** the writer bumps the revision and returns. Running work sees the new revision at
  its next poll, stops, and any result it finishes is stored only if the file's version still
  matches.
- **Memory:** under the engine's control. Trees are dropped after use or kept in a byte-budget
  cache, evicted at any time.
- **Code:** the prototype's engine took 270 lines. The real one adds layer 3's lazily verified
  entries (see [Project symbols](#project-symbols)).

### (c) Recompute everything on change

No memoization beyond the inputs: every change reruns summaries, symbols, and the changed file's
bodies. Simple, but a body edit would cost a project's summaries (about 0.5 s for lucene on four
threads) instead of a few milliseconds, which fails the cancellation and language server goals for
any large project.

## Measurements

A throwaway prototype, outside the repository, implemented the same computation both ways on
Ferrion's parser:

- **summary:** a declaration-only walk of the tree, keeping the package, the imports, and for each
  type its path, header, and member headers as token text, skipping bodies by jumping to their
  `subtree_end`;
- **package index:** for each package, the names of the top-level types its files declare;
- **body resolution:** every capitalized identifier in a member body looked up in the file's own
  types, its single-type imports, its package, and its on-demand imports, through the package
  index. A crude stand-in for name resolution that reads project-wide data the way the real one
  will.

salsa ran with a `parse` query (LRU 64, or unbounded) or without one (summary and body resolution
parse for themselves), and either demand-driven (the bodies query pulls in everything) or phased
(summaries in parallel first). The hand-written engine dropped trees after use or kept all of them.

Input: lucene's 5,981 files (50.4 MB). Release build, Apple M1 Pro (10 cores, 32 GB). Memory is
counted by a global allocator: what stays allocated after the run, beyond the texts, and the peak
during it. The machine was shared with the formatter's work, with load averages from 7 to 20, so
the times are noisy and the comparison between the options matters more than the absolute values.
For scale, parsing every file on one thread took 0.71 s, and the largest file (a 1.8 MB generated
tokenizer) took 62 ms.

### Cold runs

| Engine | 1 thread | 2 | 4 | 8 | Kept after | Peak |
| --- | --- | --- | --- | --- | --- | --- |
| salsa, demand-driven, parse LRU 64 | 921 ms | 894 ms | 860 ms | 865 ms | 389 MB | 389 MB |
| salsa, phased, parse LRU 64 | 902 ms | 489 ms | 320 ms | 364 ms | 389 MB | 389 MB |
| salsa, phased, every tree kept | 889 ms | 486 ms | 317 ms | 373 ms | 389 MB | 389 MB |
| salsa, phased, no parse query | 1,607 ms | 872 ms | 525 ms | 539 ms | 27 MB | 38 MB |
| hand-written, trees dropped | 1,580 ms | 842 ms | 523 ms | 528 ms | 26 MB | 37 MB |
| hand-written, every tree kept | 892 ms | 493 ms | 317 ms | 352 ms | 388 MB | 388 MB |

- **Demand-driven doesn't scale:** the first body query on any thread needs the package index,
  which needs every summary, and they are computed on that one thread while the others wait. Both
  options need explicit phases for batch runs.
- **The LRU didn't bound memory:** with a capacity of 64, salsa still held every tree at the end of
  the run, because eviction happens when the next revision starts. Its "parse LRU 64" and "every
  tree kept" rows are the same.
- **Trees are 15 times the summaries:** 388 MB of trees against 26 MB of summaries and results.
  Dropping them costs a second parse of each file in the body phase (1.6 s against 0.9 s on one
  thread), so the engine keeps trees in a cache with a byte budget and runs the body phase in the
  reverse of the summary phase's order, so the trees still cached are used before they're
  evicted.
- **8 threads were slower than 4** on the loaded machine; the shared machine, not the engines, set
  that limit.

### Edits

After a cold run on four threads, the target was `lucene/search/AbstractDocIdSetIterator.java`,
in the directory with the most files (222). Every file's body query was then asked for again.
Executions counted per query:

| Edit | salsa reruns | salsa time | hand-written reruns | hand-written time |
| --- | --- | --- | --- | --- |
| None | none | 0.7 to 1.1 ms | none | 2.4 to 2.7 ms |
| Statement added to a method body | parse, summary, bodies: 1 file | 2.7 to 3.4 ms | the same | 2.7 to 2.8 ms |
| Method renamed | the same, and the package index (equal result, cut off) | 4.3 to 4.9 ms | the same | 4.7 to 4.9 ms |
| Top-level class added | the same, 347 package entries checked, bodies of 359 files | 10.5 to 21.7 ms | bodies of 359 files | 8.8 to 18.7 ms |

The two engines did exactly the same work after each edit; the small differences in time are the
cost of checking: salsa verifies a memo by walking its dependencies, the prototype by comparing
fingerprints. Most of the time after a new class is re-resolving the 359 files whose packages
changed, and parsing them again when trees were dropped.

### Cancellation

Nineteen trials per engine, each starting a cold run on four threads and changing one file at
5%, 10%, ... 95% of the cold run's length. For salsa, the time is how long the setter blocked; for
the hand-written engine, the setter's time and how long until the old run's threads had stopped.

| Engine | Setter, median | Setter, slowest three | Old work stopped, median | Slowest three |
| --- | --- | --- | --- | --- |
| salsa, phased, parse LRU 64 | 2.7 ms | 47, 63, 68 ms | (same as setter) | |
| salsa, phased, every tree kept | 15.5 ms | 54, 65, 113 ms | (same as setter) | |
| salsa, phased, no parse query | 2.0 ms | 36, 39, 92 ms | (same as setter) | |
| hand-written, trees dropped | under 0.1 ms | under 0.1 ms | 1.3 ms | 15, 21, 59 ms |
| hand-written, every tree kept | under 0.1 ms | under 0.1 ms | 8.0 ms | 37, 37, 42 ms |

The slow cases are steps that don't poll: a parse in flight (62 ms for the largest file, longer
when its thread is descheduled on a loaded machine). In salsa they hold up the writer; in the
hand-written engine they only use a thread until they finish, and their results are discarded. The
parser could poll a cancellation flag too (a `ParseOptions` field checked every few thousand
tokens), which would bound both; that's left until profiling of the language server asks for it,
since the hand-written writer doesn't wait.

## Layers

### Inputs

- **Files:** per source set, every `.java` file under its source roots and generated-source roots,
  with its text, a version, and whether it is generated. The CLI reads them once; the language
  server replaces a file's text on each change, and rescans a root when the client says files were
  added or removed.
- **Source set configuration:** release, classpath, module path, JDK, and dependencies, from
  `ferrion_project` (see [inputs](project-inputs.md)). Changing it is rare and drops every derived
  value of that source set and the source sets that depend on it.
- **Class path entries and JDKs:** opened once and shared through `ferrion_classfile`'s
  `ArchiveCache`. A jar rebuilt on disk is noticed (`Changed`) and treated as a configuration
  change of the source sets using it.

`Engine::set_text(file, text)` bumps the revision, then replaces the text and drops the file's
tree. It doesn't wait for running work. Both happen under the inputs lock, which readers hold only
to copy an input out, so no snapshot sees the new revision without the new text. The file's
summary and body results aren't dropped: their recorded version no longer matches, so their next
use computes them again. `add_file`, `remove_file`, and `set_release` work the same way; a release
change gives each of the source set's files a new version.

The engine is given its inputs as `ferrion_semantic::ProjectInput`: source sets with a name, a
release, a classpath in order (other source sets, read from source, and opened jars and
directories, each with its path and, for the module path, its module name), a platform API, and
files with their text and whether they are generated. `ProjectInput::from_project` builds it from a
`ferrion_project::Project` and its opened `TypeInputs`, reading every `.java` file under each
source set's roots; tests build it directly, with a stand-in platform
(`ferrion_semantic::testing`). The platform is anything that implements the engine's `Platform`
trait, which B1's `PlatformApi` does.

As built, the classpath entries and platforms of every source set are numbered across the engine
as class sources when it is made (a jar or platform that several source sets share is one), and
don't change while it runs: a changed classpath, or a jar rebuilt on disk, makes a new engine.
What is read from a class source (a package's listing, and each class file read as a class entry)
is cached without dependencies and never verified.

### File summaries

A summary is a pure function of one tree: the declarations a file makes and the imports it has,
with types and annotations as written (unresolved), and no offsets, so an edit above a declaration
doesn't change it. Its content is in [names](semantic-names.md#file-summaries). Each summary has a
fingerprint (a 64-bit hash of its content), and is kept for every file of every source set for as
long as the file exists. Positions for diagnostics are found again from the current tree by
declaration path when needed.

### Project symbols

Per source set:

- **The package table:** for each package, the source types declared in it (from summaries, with
  duplicates across roots marked ambiguous), merged lazily with the class path's and platform's
  listings. It is two kinds of derived entry, using the same verification as type entries below:
  a package index per source set (each package and the files declaring it), which reads the set's
  file list and each file's package and top-level type names, the only part of a summary it
  depends on, so it is rebuilt by a cheap pass over the summaries only when a file is added or
  removed or that part changes; and an entry per package (its top-level types, each with the files
  declaring it), which reads only its own package's files in the index. A new type recomputes its
  own package's entry, and the others are verified without computing.
- **Type entries:** for each type, its resolved header (supertypes, type parameters' bounds, member
  types, fields and method signatures with their types resolved), computed on demand and memoized.
  An entry records what it read: the summaries of the files it resolved names in, and the package
  and type entries it looked up, each with the fingerprint seen. After a change, an entry is
  verified on first use by checking those fingerprints (verifying the type entries it read first,
  recursively); if any differ, it is recomputed, and if the new value has the old fingerprint, the
  entries and bodies that read it stay valid.

This is the same early-cutoff rule salsa applies, for the kinds of derived entry that need it.
Class-file types are entries of the same kind, whose only input is their class file, so they never
change unless a jar does.

In the engine, every derived entry works this way, summaries included (a summary reads its file's
input, whose fingerprint is its version). A computation reads only through a recorder that notes
each dependency and the fingerprint it had, so a dependency can't be read without being recorded;
each kind of dependency is a variant of one enum, and one function gives each one's current
fingerprint. An entry stores the latest revision at which it was known valid: a use at that
revision returns it at once, and a use at a later one checks its dependencies in the order they
were read, stopping at the first that differs. A result finished at a revision that has since
moved is still stored, with what it read, and is verified like any other at the next revision, so
work that the change didn't affect isn't lost. Verification recurses through entries that read
entries, on `ferrion_stack::ensure_stack`, so a long chain of type entries can't overflow the
stack.

The project symbols as built in W4, each a kind of derived entry:

- **A file scope** per file: its summary, each of its types' keys, and indexes by key, top-level
  name, and outer type, made in one pass over the summary, so that a scope inside a type nested
  ten thousand deep costs nothing more than one at the top. Its fingerprint is the summary's.
- **A module** per source set, from its `module-info.java`, which the package index lists.
- **Supertypes** per source type: the classes its `extends` and `implements` clauses name,
  erased, and whether it is in a cycle.
- **Member types** per type and view (a source type's own set): declared and inherited.
- **A type entry** per source type: its header resolved, with the root causes of its errors.
- **Names in declarations** per file, keyed by `NodeId` of the tree they were resolved on.

A lookup by package and simple name reads one type of a package entry (`Dep::PackageType`), not
the whole entry, so a type added to a package reaches only the lookups of its name: with whole
package entries, lucene's "top-level class added" recomputed 1,916 type entries (all cut off),
and with one type, 1 or 2. A computation that resolves many names reads the same entries again and
again, and keeps each dependency once.

**Cycles.** The kinds of entry that can read themselves (supertypes, which read the member types
of the types their clauses qualify, which read their supertypes) are tracked while verified or
computed, on a stack per thread. A request for one already on the stack is a cycle: it reads as
`Fingerprint::CYCLE`, and every entry on the stack from that one up learns the cycle's entries.
Each such supertypes entry gives error supertypes and records the source types of its cycle; a
type whose source supertype's cycle names it is in that cycle too, so the result is the same
whichever type of the cycle was asked for first, or verified first after an edit (the
differential check found the case where it wasn't, before the cycle's types were recorded). A
member types entry in a cycle leaves out what it can't read and is incomplete. A cycle found is
one record shared by every entry on the stack in it, and its types one shared list, so a cycle of
n types costs O(n) memory; each entry's fingerprint still hashes the list, so its time is
quadratic, which the tests keep to 2,000 types. An entry that saw
a cycle has recorded `CYCLE`, which no entry has, so it is computed again at every revision;
cycles are rare and small. An entry computed in a cycle keeps the cycle, and is verified again
when asked for with an entry of the cycle on the stack, even at a revision it is already valid at:
verifying a type's supertypes can compute its member types again, with the supertypes on the
stack, before computing the supertypes again, which then read those member types and must find
the cycle through them (the differential check found that they didn't). Tracking is per thread, so threads in the same cycle never wait for
each other.

### Body results

Per file, the scopes, local symbols, and resolved names inside its bodies, keyed by `NodeId` of the
file's current tree (B3 adds attributed types). A body result is valid while the file's version is
unchanged and every package and type entry it read still has the fingerprint it saw. The language
server computes body results for open files and files with diagnostics requested; the CLI computes
them for every file it lints, in the last phase.

Body results are the largest derived data after trees, so the language server keeps them for open
files and drops the rest after use; the CLI drops each file's after its rules have run.

Names in declarations (W4) are kept the same way: `run_batch` drops each file's after its callback
has run. Kept for every lucene file, they took 57 MB, more than the summaries.

As built in W5, a file's body result is a derived entry like its names in declarations
(`ferrion_semantic::body`, `Snapshot::body_names`): its scopes are the walk's and aren't kept, so
the result is the resolved names, by `NodeId` of the tree they were resolved on with their
ranges, and the local and anonymous classes they declare, each with a type entry, supertypes,
and member types. It reads the file's tree (its input, by version) and its file scope, and
project symbols only through `Reads`, as type entries do: the package entries of the names it
looks up by name (`Dep::PackageType`, so a new type reaches only the bodies that look its name
up), and the supertypes, member types, and type entries of the types whose members it looks up.
A body edit gives a new tree and summary, the summary comes out equal, and only that file's body
result is computed again. `run_batch` computes each file's after its names in declarations, and
drops both after its callback; it holds the file's tree meanwhile, so that with the tree cache
too small to keep it, the file is parsed once for both.

A lookup in the walk is a map read, whatever the nesting: each scope's declarations are bound in
maps by name, innermost last, and unbound when the walk leaves the node that opened it.
A type's fields and methods are looked up by name, its supertypes' first, by a loop, and kept for
the rest of the file's walk, as are the simple type names resolved in each class around a scope.

### Trees

Trees are an input-derived cache, not a layer: computed from a file's text, used by the summary and
body phases, and kept in a cache with a byte budget (by default 256 MB, a setting for the language
server), least recently used evicted first. A tree evicted while in use stays alive through its
`Arc` until its user is done. The budget counts each tree's node and token arrays by their
lengths, which is what they hold, since `SyntaxTree::new` shrinks them to fit. Before it did,
their spare capacity made trees about half as large again as counted (lucene: 243 MB counted,
about 360 MB allocated).

## Threads

- **Batch (CLI):** three phases over a pool of worker threads (one per core by default): parse and
  summarize every file; build each source set's package table; then, per file, verify or compute
  the type entries its bodies need and resolve the bodies. Files are handed out by an atomic
  counter, as in the prototype, to scoped threads started for each phase; with one thread, a phase
  runs on the calling thread. Source sets are processed together, so a dependent source set's
  summaries exist before its dependents' bodies run. `Snapshot::run_batch` runs the summary,
  package table, and type entry phases (`Snapshot::run_symbols`, which a language server runs
  after an edit), then, per file in reverse order, its names in declarations and in bodies and a
  callback.
- **Language server:** a writer (the message loop) applies edits and bumps the revision; requests
  run on a pool and hold `Arc`s to what they read. A request started at revision N that sees N+1
  stops and is retried or answered with "content modified", as LSP allows.
- **Shared state:** memo tables are sharded maps behind `RwLock`s, held only to read or store an
  entry, never while computing one. Values are immutable and shared through `Arc`.

## Cancellation

`Engine::revision()` is read at the start of a unit of work; `check(revision)` returns
`Err(Cancelled)` once it has moved. It is called:

- between files in each phase;
- every 1,024 tokens in a body walk, and in every loop over members or supertypes in layer 3;
- at every read of an input, and between the dependencies checked when verifying an entry.

A result finished after the revision moved is stored with what it read, as above, and the query
that computed it still returns `Err(Cancelled)`. Summaries don't poll: a summary is a fast pure
function of a tree, and its parse doesn't poll either. `Cancelled` is an ordinary error value
returned with `?`, never a panic.

## Memory

From the prototype, for lucene: summaries and body results 26 MB, trees 388 MB, against 50 MB of
text. The budget the design holds to:

- summaries, the package tables, and type entries for every file and every class file used, in
  every source set;
- trees in the byte-budget cache only;
- body results only where needed, as above;
- class files are read on demand and not cached by B1; type entries keep what B3 needs from them.

`tools/bench` has `semantic/corpus-cold` and `semantic/edits`, measuring the cold run and the edit
scenarios above on a corpus project on one thread, with peak memory, so regressions show up in
pull requests (benchmarks.md).

### The engine as built

Measured after W2 on lucene through its lock file: 79 source sets, 5,968 files, 50.3 MB, with the
summaries of [names](semantic-names.md#file-summaries) and the package tables (no bodies yet).
Release build, Apple M1 Pro, on the shared machine (load averages 8 to 10).

| Cold run | 1 thread | 2 | 4 | 8 | Kept after | Peak |
| --- | --- | --- | --- | --- | --- | --- |
| tree cache 256 MB (every tree fits) | 922 ms | 520 ms | 331 ms | 419 ms | 412 MB | 412 MB |
| trees dropped (cache of 0 bytes) | 885 ms | 502 ms | 334 ms | 431 ms | 51 MB | 51 to 56 MB |

A last phase reading every tree again took 2 to 8 ms with the cache, and 248 to 743 ms without
it, parsing every file again. The summaries and tables take 51 MB, against the prototype's 26 MB
for its cruder summaries and results: types as written were 40 bytes a part, and names weren't
interned. At the start of W4, interning every name and making a part 12 bytes took the 51 MB to
36 MB (49 MB to 36 MB measured the same way, a cold run on four threads with trees dropped).

After a cold run on four threads, `AbstractDocIdSetIterator.java` was edited and everything asked
for again (`run_batch`), five times each:

| Edit | Time | Work |
| --- | --- | --- |
| The same text again | 4.1 to 4.9 ms | 1 parse and summary (cut off); 5,967 summaries, 79 indexes, 594 packages verified |
| Statement added to a method body | 3.8 to 4.5 ms | the same |
| Method renamed | 3.6 to 4.7 ms | 1 summary changed; no package table entry computed |
| Top-level class added | 3.7 to 5.5 ms | also 1 index computed (cut off) and 1 package entry computed; 593 verified |

Cancellation, at 5%, 10%, ... 95% of a cold run on four threads: the setter returned in 1 to
14 µs, and the run stopped in a median of 0.5 ms (0.7 ms with trees dropped); the slowest three
took 18, 39, and 54 ms, each a parse in flight of one of lucene's largest files.

### With project symbols (W4)

Measured after W4 on lucene through its lock file, with its jars and JDK 27's platform API at each
source set's release: 79 source sets, 5,968 files, 50.3 MB, 8,688 source types, 788 class files
read. A cold run is `run_batch`: summaries, package tables, every type's supertypes, member types,
and entry, and every file's names in declarations. Release build, on the shared machine.

| Cold run | 1 thread | 4 | 8 | Kept after | Peak |
| --- | --- | --- | --- | --- | --- |
| trees dropped (cache of 0 bytes) | 2.27 s | 1.00 s | 1.36 s | 79 MB | 91 MB |
| tree cache 256 MB | | 0.75 s | | 323 MB | 324 MB |

With trees dropped, every file is parsed twice (11,936 parses), the second time for its names in
declarations. The 79 MB kept are the summaries and package tables' 36 MB, and 43 MB of file
scopes, supertypes, member types, type entries, the class entries of the 788 class files read,
and 60,761 distinct names. Before each computation kept each dependency once and the batch dropped
each file's names after its callback, the same run kept 144 MB.

After a cold run on four threads, `AbstractDocIdSetIterator.java` was edited, everything
project-wide brought up to date (`run_symbols` on four threads), and the file's names resolved
again, then undone the same way, three times each:

| Edit | Time | Work |
| --- | --- | --- |
| The same text again | 16 to 20 ms | 1 parse and summary (cut off); everything else verified |
| Statement added to a method body | 17 to 21 ms | the same |
| Method renamed | 15 to 20 ms | 1 summary changed; the file's scope, and its types' supertypes and member types, computed and cut off; 1 type entry changed |
| Top-level class added | 18 to 20 ms | also 1 index computed (cut off), 1 package entry computed, and the new type's entries |
| `implements java.lang.Cloneable` added to a class | 16 to 25 ms | the file's 16 supertypes entries computed, 1 changed; its member types computed and cut off; 1 type entry changed; nothing in other files |

Nearly all of each edit's time is verifying what didn't change: 5,968 summaries, about 3,700
package entries, 8,800 supertypes, 9,400 member types, and 8,700 type entries.

### With body names (W5)

Measured after W5 on lucene through its lock file, as W4 was: 79 source sets, 5,968 files,
50.3 MB, with its jars and JDK 27's platform API. A cold run is `run_batch`, now resolving every
file's names in bodies too: 1,354,817 names, and 2,900 local and anonymous classes. Release
build, Apple M1 Pro, on the shared machine (load averages 5 to 9).

| Cold run | 1 thread | 4 | 8 | Kept after | Peak |
| --- | --- | --- | --- | --- | --- |
| trees dropped (cache of 0 bytes) | 3.4 s | 1.9 s | 2.4 s | 83 MB | 95 MB |
| tree cache 256 MB | | 1.7 s | | 327 MB | 329 MB |

The body phase alone, on one thread with every tree cached, takes 1.16 s (0.9 µs a name), and
0.9 s on four threads: it scales worse than the phases before it, on the shared machine, and is
mostly hashing and allocation, the parse of the summary phase aside. Before the walk looked
members up by name and its resolver kept the type names it had resolved, it took 1.5 s; before
the batch held each file's tree for both kinds of names, a cold run with trees dropped parsed
every file three times (17,904 parses, 4.4 s on one thread). The 83 MB kept are W4's 79 MB and
the names bodies interned.

Body results kept for every file would be large: with the project symbols' 78 MB, every file's
names in declarations and in bodies took 214 MB, 137 MB of names, against 50 MB of text. So the
batch drops each file's after its callback, and a language server keeps them for open files
only, as planned.

After a cold run on four threads, with every file's body names then asked for and kept, the edit
target was `lucene/search/AbstractKnnVectorQuery.java` (the first file of the largest directory
that has a local variable to rename; W4's target has none). Each edit was made, everything
project-wide brought up to date (`run_symbols` on four threads), and the file's names resolved
(`file_names`), then undone the same way, three times each. Then every file's body names were
asked for again, to see which the edit reached:

| Edit | Time | Work | Every file's body names then |
| --- | --- | --- | --- |
| The same text again | 24 to 28 ms | 1 parse and summary (cut off); its body names computed and cut off; the rest verified | 15 to 21 ms, none other computed |
| Statement added to a method body | 20 to 27 ms | the same, its body names changed | 17 to 21 ms, none other computed |
| Local variable renamed | 25 to 29 ms | the same | 17 to 21 ms, none other computed |
| Method renamed | 25 to 28 ms | 1 summary changed; 3 type entries computed, 1 changed | 18 to 22 ms, 12 other files' computed, 8 of them cut off |
| Top-level class added | 25 to 27 ms | also the new type's entries | 16 to 21 ms, none other computed |
| `implements java.lang.Cloneable` added | 21 to 27 ms | 7 supertypes computed, 1 changed; 1 type entry changed | 20 to 23 ms, 12 other files' computed, 11 of them cut off |

A new top-level type reaches no other file's body names, since none looks its name up. A renamed
method or a new supertype reaches the bodies that read the type's entry: those of its file's
users and subclasses. Nearly all of each edit's time is still verifying what didn't change, as
after W4.

Cancellation, at 5%, 10%, ... 95% of a cold run on four threads, with the body phase from about
half way: the setter returned in 1 to 5 µs, and the run stopped in a median of 0.10 ms (0.18 ms
with trees dropped); the slowest three took 0.2, 1.6, and 3.1 ms (1.6, 2.8, and 60 ms with trees
dropped, the last a parse in flight in the body phase, which doesn't poll). The walk polls every
1,024 tokens.

`ProjectInput::from_project` with real `TypeInputs` (lucene's project model, with the JDKs it
names): `Project::load` took 6 ms, `TypeInputs::open` 52 ms, with one warning (a manifest
`Class-Path` entry of an opennlp jar that doesn't exist), and `from_project` 193 ms, reading
5,968 files into 79 source sets with 490 classpath entries, none skipped. A cold `run_batch` of
that input on four threads took 1.7 s and kept 326 MB with the default tree cache.

## Checking the engine

A dependency the engine fails to record would make it reuse a stale result. The check is
differential:

- **Edit sequences:** a test applies random edits to a small multi-file project (insert or delete a
  token in a body, rename a member, add or remove a type or import, change a supertype) and, after
  each, compares every body result and type entry with a fresh engine's. A fuzz target does the
  same on corpus-derived inputs.

  As built, `ferrion_semantic::invariants::check_edits` does this for summaries, package
  tables, and, from W4, each file's type entries (with their supertypes and member types) and
  names in declarations, and from W5 its names in bodies and the local classes they declare,
  compared as text with a fresh engine's, on a project of two source sets on the stand-in
  platform, with edits chosen from bytes: a fragment inserted at a token (supertypes, member
  types, imports, locals, pattern bindings, and local and anonymous classes among them), a token
  deleted or renamed, a statement added to a body, the same text set again, a file added or
  removed, and a release changed. After each edit it asks only some questions, chosen from the
  bytes, so that entries go unverified across several revisions, and checks each entry's stored
  fingerprint against its value. The tests run it from seeds, with a batch at the old revision
  running while each edit lands, and with a deeply nested file; the `semantic_edits` fuzz target
  runs it on fuzzer-chosen bytes, from seeds of small files of each kind rather than corpus
  files, whose licenses vary. Leaving out one recorded dependency (a file's top-level part, read
  by the package table) makes each of the three tests fail, and so does leaving out the one a
  lookup by name records (W4). Leaving out a body's read of a type's entry for its methods (W5)
  makes the engine test of a renamed method fail, but not the seeded edits, which rarely rename
  a member that another file's body calls; the fuzz target ran 28,883 inputs in five minutes
  without a failure.
- **Corpus:** `oracle-check` runs the comparison with javac twice, once from a cold engine and once
  after replaying each file as an edit, and the results must be identical.

  As built, `cargo oracle names --replay` gives each project a second engine. That engine has every
  file's text empty and runs a whole batch, then is given each file's text as an edit. For each
  source set, its names must equal the cold engine's, or the source set fails. Across the corpus,
  every source set's names were equal: 19.9 s on Ferrion's side, against 11.1 s without, with a
  peak of 1.8 GB. Skipping one file's edit makes the check fail. CI runs it on jedis for each pull
  request and on every project weekly.

## What was consulted

- salsa 0.28.5's documentation, tests, and source, as a dependency candidate under MIT OR
  Apache-2.0; the prototype used it through its public API.
- The incremental model as generally known from build systems and language servers (versioned
  inputs, memoization with early cutoff); nothing from javac or any Java tool.
