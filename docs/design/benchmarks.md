# Benchmarks

How Ferrion's performance is measured and how regressions are caught: `tools/bench`, its metrics,
and the pull-request check. The roadmap asks for performance regressions to be flagged in CI
(cross-cutting work); this starts with the lexer, the parser, and class file reading, and later
components add their benchmarks to the same harness.

## Decision

1. **One harness, `tools/bench`** (`cargo benchmark`), rather than a benchmark framework. Every
   metric comes from the same benchmark definitions, and nothing needs installing beyond valgrind
   for instruction counts.
2. **Each benchmark runs in its own process,** so its peak RSS and CPU time are its own.
3. **Regressions are judged by counts that repeat exactly:** callgrind's instruction count and the
   heap counts. Times and RSS are measured and reported, but don't fail a check.
4. **CI runs the pull request's base and head in the same job**, on the same inputs, and compares
   them. No baseline is committed. It runs only the benchmarks the change can affect.
5. **Inputs come from the corpus and a JDK**, as the other tools' do. Nothing a benchmark reads is
   committed.

## Benchmarks

`cargo benchmark list` lists them. Names are `<component>/<what>`, and don't change with the input.

| Benchmark | One iteration |
| --- | --- |
| `lexer/corpus` | lex every source file of the corpus projects |
| `parser/corpus` | lex and parse every source file at its source set's release, dropping each tree before the next |
| `formatter/corpus` | format every source file that parses without errors, at its source set's release, from trees parsed in setup |
| `classfile/jdk-open` | open a JDK: its image's index and its `ct.sym`'s central directory |
| `classfile/platform-image` | open the platform API for the JDK's own release, from its image |
| `classfile/platform-ctsym` | open the platform API for an older release (21 by default) from `ct.sym` |
| `classfile/read-image` | find and read every class of the JDK's own release |
| `classfile/read-ctsym` | find and read every class of the older release |
| `classfile/read-jars` | open the corpus projects' dependency jars and read every class in them |
| `semantic/corpus-cold` | a new semantic engine over the corpus projects' source sets (from their lock files), with their jars and the JDK's platform API: parse and summarize every file, build every package table and every type's entry, resolve every file's names in declarations and bodies, and attribute every file, on one thread |
| `semantic/edits` | after a cold run, six edits to a file of the largest package (a statement in a method body, a local variable renamed, a method renamed, a top-level class added, a supertype added, and a `var`'s initializer changed from `1` to `1L`, which changes an expression's type), each followed by asking for every project-wide entry again and the file's names and attribution, and undone the same way |

The semantic benchmarks run on one thread, so that their instruction and heap counts repeat; the
engine's scaling across threads is measured separately (semantic-incremental.md). They grew with
B2: W4 added the project symbols and names in declarations, W5 the names in bodies, and B3b attribution.

"Read a class" is what B2 will need from one: its names, supertypes, members, descriptors (parsed),
and attributes (decoded).

The corpus projects default to jedis, the one CI fetches; `--project` names others. The JDK is
`--jdk` or `JAVA_HOME`. A benchmark whose input isn't there is skipped, or fails the run with
`--no-skip`, which CI passes.

## Metrics

All are per iteration, except peak RSS.

| Metric | How | Repeats | Gate |
| --- | --- | --- | --- |
| instructions | callgrind, one iteration after one to warm up | almost exactly | 2% |
| allocations | a counting global allocator | exactly | 2% |
| heap allocated | bytes asked for by those allocations | exactly | 2% |
| heap peak | the most bytes live at once, beyond those live before the iteration | exactly | 5% |
| wall time | median iteration, and the fastest | a few percent locally; more on shared runners | none |
| user and system time | getrusage around every timed iteration | as wall time; system time much less | none |
| peak RSS | getrusage's maximum resident set, for the whole process, inputs included | a few percent | none |

Timing runs one iteration to warm up, then iterations until `--time` seconds (3 by default) have
passed, at least 5 of them.

Instructions are counted with `--callgrind`: the benchmark's process runs under valgrind with
collection off except inside the one function that runs the measured iteration
(`--toggle-collect`), so setup and warm-up aren't counted. Counting starts once every benchmark is
timed, and runs as many benchmarks under valgrind at once as there are CPUs (`--jobs`): timing needs
the machine to itself, but the counts don't depend on its load. Valgrind runs a process many times
slower, setup and warm-up included, so counting takes most of a run; in parallel, it takes about
as long as the slowest benchmark's count. The profiles are kept in
`target/release/bench-callgrind/`, for `callgrind_annotate` to show where a change went. Valgrind
runs on Linux (and on Intel Macs, but not Apple silicon), so locally this needs Linux or a Linux
container.

On Windows, CPU time and peak RSS aren't measured yet.

### Why instruction counts

Wall time on a shared CI runner moves by more than the regressions worth catching, from one run to
the next and between the base and head of the same job. Instruction counts don't depend on the
machine's load or clock, and two runs of the same code agree almost exactly, so a threshold of a
few percent catches real changes without false alarms. They miss what instructions don't show
(cache misses, system calls, I/O), which is why times are still measured and shown next to them,
and why the heap counts are gated as well.

The counting allocator is the only unsafe code in the workspace: implementing `GlobalAlloc` needs
it. It forwards every call to the system allocator. It's in the tool, not in anything shipped.

### Why not a framework

Criterion and divan time well but don't count instructions or memory; iai-callgrind (now Gungraun)
counts instructions but needs its runner installed at the library's exact version, and runs only
where valgrind does. Either would mean defining each benchmark twice to get every metric. The
harness is small, and the measurement it needs is a timing loop, two counters, getrusage, and one
valgrind invocation.

## In CI

The `bench` job in `ci.yml` runs on pull requests:

1. runs `cargo benchmark affected` with the base commit, which prints the components whose
   benchmarks the change can affect (below); when there are none, the job ends there and says so
   in its summary;
2. fetches and checks out jedis, and installs JDK 27 and valgrind;
3. runs `cargo benchmark run --callgrind --no-skip` on the head (the merge commit), for those
   components;
4. checks out the base commit in a worktree and runs the base's harness the same way, in the same
   target directory, for those of the components it has benchmarks for;
5. runs the head's `cargo benchmark compare --markdown` and adds the table to the job summary.

A metric that grows past its threshold in `tools/bench/thresholds.toml` fails the job. A
regression that's intended (a feature that costs time) is accepted in review: say why in the pull
request. Thresholds come from the head, so a change to them shows in the diff.

When the base has no harness yet, the comparison is skipped.

### Which benchmarks run

A benchmark measures one component's crate, so a change to code that crate isn't built from can't
change what it measures. `benchmark affected` compares the working tree with a commit, and reads
which packages the harness is built from out of `cargo tree` (normal and build dependencies, with
features resolved as building only the harness resolves them). Then:

- a file in one of those packages runs the benchmarks of every component built from it, so a change
  to `ferrion_diagnostics` runs every component's but class file reading's. A package no component
  is built from (the harness itself, and `corpus`, which chooses the inputs) runs every benchmark.
  Files under a package's `tests`, `benches`, and `examples` don't count: only test targets are
  built from them. Test modules in `src` do count, since nothing tells them apart from the rest.
- a dependency that the base's `Cargo.lock` doesn't lock at the same version, and an entry of the
  root manifest's `[workspace.dependencies]` that changed, count as that package. Comparing the
  dependencies the harness is built from, rather than the files, keeps a lock file change for a
  crate the harness doesn't use from running anything.
- `rust-toolchain.toml`, `.cargo/config.toml`, `ci.yml` (where the job is defined), and the rest
  of the root manifest, such as its profiles, run every benchmark. The workspace's members and
  lints don't change what's built.
- anything else runs nothing: documentation, the other tools, and the crates the harness isn't built
  from, such as the CLI's and the configuration's.

Each component's crate is listed in `affected.rs`, and a test checks that every benchmark's
component is there.

## Adding a component

Add a module to `tools/bench/src/benchmarks/` with its benchmarks and list them in `all()`, and
add the component and the crate it measures to `COMPONENTS` in `affected.rs`. A
benchmark's setup reads and prepares its input, outside the measurement, and returns a closure that
runs one iteration; results go through `black_box`. Keep an iteration between a millisecond and a
second, so timing gets enough iterations and callgrind finishes in a reasonable time. A new
benchmark shows as new in the first comparison and is gated from the next.
