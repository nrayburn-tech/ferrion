//! `oracle names`: compares Ferrion's name resolution with javac's, source set by source set
//! (docs/design/semantic.md, "Verification").
//!
//! For each source set of each corpus project, `javac-oracle names` compiles every file as the
//! build does (its release, `-source` or `--release`, and its options for the JDK's modules),
//! with the source sets it depends on on the source path, the jars on the classpath, and the
//! build's annotation processors (its processor path, or else the classpath) writing to
//! `target/corpus/generated/<project>/<source set>`; each `module-info.java` is compiled on its
//! own, in a modular compilation. Its output is cached by a hash of the inputs ([`oracle`]).
//! Files javac reports language errors in are left out whole, and so are files it never analyzed
//! (counted apart) and Lombok-annotated types; what processors report is no language error, and
//! is counted, not a reason to leave a file out. The names in B2's part ([`compare`]) of the rest
//! are matched with Ferrion's by range and compared by key. Until Ferrion resolves names, [`ferrion`]
//! has a stand-in with no results, and the report counts javac's side: files run, files left out
//! by cause, and names by context and kind of symbol.

mod compare;
pub mod ferrion;
mod oracle;
mod plan;
mod report;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_diagnostics::SourceText;

use self::compare::{Known, Part, javac_key, key_kind};
use self::ferrion::{Offsets, Resolver, SetNames, StandIn};
use self::oracle::{NamesOracle, Ran};
use self::plan::SourceSet;
use self::report::{MismatchRow, Report, SetRow, Tally};

pub const USAGE: &str = "\
usage: oracle names [<option>...] [<project>...]

Compiles each source set of the corpus projects (by default all of them) with javac as its build
does, running the build's annotation processors, and compares what each name in B2's part resolves
to with Ferrion's resolution. Files with javac errors, files javac never analyzed, and
Lombok-annotated types are left out. Until Ferrion resolves names, nothing is compared, and the
report counts javac's side.

options:
  --source-set <id>    only this source set of each project, such as `main` or `common/config/test`
                       (repeatable); the ones it depends on aren't run, so their generated sources
                       are those of an earlier run
  --jobs <N>           run up to N javac JVMs at once (default: 2)
  --heap <size>        each JVM's maximum heap (default: 4g)
  --processor-path <paths>
                       find annotation processors on these jars and directories (separated as
                       on a classpath) for every source set, instead of the build's processor
                       path or its classpath: an experiment
  --limit <N>          show the first N failures (default: 10)
  --json <file>        also write the full report as JSON
  --oracle-jar <file>  the javac oracle (default: tools/javac-oracle/target/javac-oracle.jar)

JAVA_HOME must point to a JDK that supports every source set's release (the newest GA JDK), and
the projects must be checked out with their jars (`cargo corpus fetch`). The oracle's output is
cached in target/oracle/names, or under FERRION_ORACLE_CACHE if it is set; generated sources are
written to target/corpus/generated, or under FERRION_CORPUS_DIR.";

pub struct Options {
    pub projects: Vec<String>,
    pub source_sets: Vec<String>,
    pub jobs: usize,
    pub heap: String,
    pub processor_path: Vec<PathBuf>,
    pub limit: usize,
    pub json: Option<PathBuf>,
    pub jar: PathBuf,
}

pub fn parse_args(args: impl Iterator<Item = String>, usage: fn(&str) -> !) -> Options {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut options = Options {
        projects: Vec::new(),
        source_sets: Vec::new(),
        jobs: 2,
        heap: "4g".to_owned(),
        processor_path: Vec::new(),
        limit: 10,
        json: None,
        jar: repository.join("tools/javac-oracle/target/javac-oracle.jar"),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .unwrap_or_else(|| usage(&format!("{name} needs a value")))
        };
        let number = |name: &str, text: String| -> usize {
            text.parse()
                .unwrap_or_else(|_| usage(&format!("{name}: `{text}` isn't a number")))
        };
        match arg.as_str() {
            "--source-set" => options.source_sets.push(value("--source-set")),
            "--jobs" => {
                let text = value("--jobs");
                options.jobs = number("--jobs", text).max(1);
            }
            "--heap" => options.heap = value("--heap"),
            "--processor-path" => {
                let paths = value("--processor-path");
                options.processor_path = std::env::split_paths(&paths).collect();
            }
            "--limit" => {
                let text = value("--limit");
                options.limit = number("--limit", text);
            }
            "--json" => options.json = Some(PathBuf::from(value("--json"))),
            "--oracle-jar" => options.jar = PathBuf::from(value("--oracle-jar")),
            _ if arg.starts_with('-') => usage(&format!("unknown option `{arg}`")),
            _ => options.projects.push(arg),
        }
    }
    options
}

pub fn run(options: Options) -> Result<bool> {
    run_with(options, &StandIn)
}

fn run_with(options: Options, resolver: &dyn Resolver) -> Result<bool> {
    let start = Instant::now();
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let selected = corpus::select(&projects, &options.projects)?;
    let oracle = NamesOracle::new(
        options.jar.clone(),
        options.heap.clone(),
        options.processor_path.clone(),
    )?;
    let cache_root = match std::env::var_os("FERRION_ORACLE_CACHE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle"),
    };
    let cache = cache_root.join("names").join(&oracle.key);
    let scratch = cache_root.join("scratch");
    let known = Known::load()?;

    // Every selected project's source sets, each project's in an order that has dependencies
    // first.
    let mut groups: Vec<Vec<SourceSet>> = Vec::new();
    let mut work: Vec<(usize, usize)> = Vec::new();
    for project in &selected {
        let sets = plan::source_sets(&dirs, project)?;
        if let Some(newest) = sets.iter().map(|s| s.release).max()
            && newest > oracle.jdk
        {
            anyhow::bail!(
                "JAVA_HOME is JDK {}, which can't compile {} for release {newest}",
                oracle.jdk,
                project.name
            );
        }
        let group = groups.len();
        for i in plan::order(&sets) {
            let set = &sets[i];
            let wanted = options.source_sets.is_empty() || options.source_sets.contains(&set.id);
            if wanted && !set.files.is_empty() {
                work.push((group, i));
            }
        }
        groups.push(sets);
    }
    let files: usize = work.iter().map(|&(g, i)| groups[g][i].files.len()).sum();
    eprintln!(
        "running javac on {} source sets ({files} files), {} at a time",
        work.len(),
        options.jobs.min(work.len().max(1))
    );

    let context = Shared {
        groups: &groups,
        oracle: &oracle,
        corpus: &dirs.corpus,
        cache: &cache,
        scratch: &scratch,
        resolver,
        known: &known,
    };
    let results = schedule(&context, &work, options.jobs)?;

    let mut report = Report::default();
    for (&(group, _), result) in work.iter().zip(results) {
        let project = &groups[group][0].project;
        report.total.merge(&result.tally);
        report
            .projects
            .entry(project.clone())
            .or_default()
            .merge(&result.tally);
        report.source_sets.push(result.row);
        report.mismatches.extend(result.mismatches);
        report.crashes.extend(result.crashes);
        report.errors.extend(result.errors);
    }
    report.wall_seconds = start.elapsed().as_secs_f64();
    print!("{}", report.text(options.limit));
    if let Some(path) = &options.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }
    let failures = report.total.failures();
    if failures > 0 {
        println!("{failures} failures");
    }
    Ok(failures == 0)
}

struct Shared<'a> {
    groups: &'a [Vec<SourceSet>],
    oracle: &'a NamesOracle,
    corpus: &'a Path,
    cache: &'a Path,
    scratch: &'a Path,
    resolver: &'a dyn Resolver,
    known: &'a Known,
}

/// What one source set gave.
struct SetResult {
    tally: Tally,
    row: SetRow,
    mismatches: Vec<MismatchRow>,
    crashes: Vec<(String, String)>,
    errors: Vec<(String, String)>,
}

/// Runs every source set of `work` on up to `jobs` threads, each once the source sets it depends
/// on that are also in `work` are done, so their generated sources are on its source path.
/// Returns the results in `work`'s order.
fn schedule(context: &Shared, work: &[(usize, usize)], jobs: usize) -> Result<Vec<SetResult>> {
    struct State {
        started: Vec<bool>,
        done: Vec<bool>,
        results: Vec<Option<SetResult>>,
        error: Option<anyhow::Error>,
    }
    let state = Mutex::new(State {
        started: vec![false; work.len()],
        done: vec![false; work.len()],
        results: (0..work.len()).map(|_| None).collect(),
        error: None,
    });
    let changed = Condvar::new();
    // For each item, the items it waits for.
    let waits: Vec<Vec<usize>> = work
        .iter()
        .map(|&(group, i)| {
            context.groups[group][i]
                .dependencies
                .iter()
                .filter_map(|&d| work.iter().position(|&w| w == (group, d)))
                .collect()
        })
        .collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(work.len()).max(1) {
            scope.spawn(|| {
                loop {
                    let next = {
                        let mut s = state.lock().unwrap();
                        loop {
                            if s.error.is_some() {
                                return;
                            }
                            let ready = (0..work.len())
                                .find(|&i| !s.started[i] && waits[i].iter().all(|&w| s.done[w]));
                            if let Some(i) = ready {
                                s.started[i] = true;
                                break i;
                            }
                            if s.started.iter().all(|&started| started) {
                                return;
                            }
                            s = changed.wait(s).unwrap();
                        }
                    };
                    let (group, i) = work[next];
                    let result = run_set(context, &context.groups[group], i);
                    let mut s = state.lock().unwrap();
                    match result {
                        Ok(result) => {
                            let row = &result.row;
                            eprintln!(
                                "{}: {} files, {} with errors, {:.1}s{}",
                                row.source_set,
                                row.files,
                                row.error_files,
                                row.seconds,
                                if row.cached { " (cached)" } else { "" }
                            );
                            s.results[next] = Some(result);
                        }
                        Err(error) => {
                            s.error.get_or_insert(error);
                        }
                    }
                    s.done[next] = true;
                    changed.notify_all();
                }
            });
        }
    });
    let state = state.into_inner().unwrap();
    if let Some(error) = state.error {
        return Err(error);
    }
    state
        .results
        .into_iter()
        .map(|r| r.ok_or_else(|| anyhow!("a source set wasn't run")))
        .collect()
}

/// Runs the oracle on one source set and compares its files.
fn run_set(context: &Shared, sets: &[SourceSet], i: usize) -> Result<SetResult> {
    let set = &sets[i];
    let Ran {
        output,
        cached,
        elapsed,
    } = context
        .oracle
        .run(sets, set, context.corpus, context.cache, context.scratch)
        .with_context(|| set.display())?;
    let mut tally = Tally {
        source_sets: 1,
        cached: u64::from(cached),
        javac_seconds: elapsed.as_secs_f64(),
        ..Tally::default()
    };
    let mut row = SetRow {
        source_set: set.display(),
        release: set.release,
        files: 0,
        generated_files: 0,
        error_files: 0,
        cached,
        seconds: elapsed.as_secs_f64(),
        processors: Vec::new(),
        failure: None,
        left_out: set.left_out.clone(),
    };
    let output = match output {
        Ok(output) => output,
        Err(message) => {
            tally.jvm_failures = 1;
            tally.files = set.files.len() as u64;
            row.files = tally.files;
            row.failure = Some(message);
            return Ok(SetResult {
                tally,
                row,
                mismatches: Vec::new(),
                crashes: Vec::new(),
                errors: Vec::new(),
            });
        }
    };
    let mut mismatches = Vec::new();
    let mut crashes = Vec::new();
    let mut errors = Vec::new();
    if let Some(crash) = &output.set.crash {
        tally.source_set_crashes = 1;
        row.failure = Some(format!(
            "javac threw, so each file was compiled on its own: {crash}"
        ));
    }
    tally.processors = output.set.processors.iter().cloned().collect();
    row.processors = output.set.processors.clone();
    for run in &output.set.processor_runs {
        tally.add_processor_run(run);
    }
    let ferrion: Result<Option<SetNames>, String> =
        context.resolver.resolve(sets, set, context.corpus);
    for file in &output.files {
        let display = display(&file.file);
        tally.files += 1;
        row.files += 1;
        let generated = file.generated == 1 || set.is_generated(&context.corpus.join(&file.file));
        if generated {
            tally.generated_files += 1;
            row.generated_files += 1;
        }
        if file.skipped.is_some() {
            tally.excluded.module_declarations += 1;
            continue;
        }
        tally.module_declarations += u64::from(file.module);
        tally.files_with_processor_errors += u64::from(file.processor_errors() > 0);
        if let Some(crash) = &file.crash {
            tally.excluded.crashes += 1;
            crashes.push((display, crash.clone()));
            continue;
        }
        if file.errors().next().is_some() {
            tally.excluded.javac_errors += 1;
            tally.excluded.generated_with_errors += u64::from(generated);
            tally.excluded.module_declarations_with_errors += u64::from(file.module);
            row.error_files += 1;
            if let Some(first) = file.errors().next() {
                let message = first.message.lines().next().unwrap_or("");
                errors.push((
                    display,
                    format!("{} at {}: {message}", first.code, first.position),
                ));
            }
            let codes: BTreeSet<&str> = file.errors().map(|d| d.code.as_str()).collect();
            for code in codes {
                *tally.error_codes.entry(code.to_owned()).or_default() += 1;
            }
            continue;
        }
        if file.unanalyzed == 1 {
            tally.excluded.unanalyzed += 1;
            continue;
        }
        if ferrion.is_err() {
            tally.excluded.ferrion_failures += 1;
            continue;
        }
        tally.compared_files += 1;
        tally.lombok_types += file.lombok.len() as u64;
        let parts = compare::parts(file);
        for (name, &part) in file.names.iter().zip(&parts) {
            tally.add_part(part, &name.context, key_kind(javac_key(name)));
        }
        let b2 = parts.iter().filter(|&&p| p == Part::B2).count() as u64;
        let Ok(Some(names)) = &ferrion else {
            tally.comparison.not_compared += b2;
            continue;
        };
        let bytes = std::fs::read(context.corpus.join(&file.file))
            .with_context(|| format!("reading {}", file.file))?;
        let source = SourceText::decode(bytes).map_err(|e| anyhow!("{}: {e}", file.file))?;
        let offsets = Offsets::new(&source);
        let ours = names.get(&file.file).map_or(&[][..], Vec::as_slice);
        let compared = compare::compare(file, &parts, ours, &offsets);
        tally.comparison.matched += compared.matched;
        tally.comparison.extra += compared.extra;
        for mismatch in compared.mismatches {
            if let Some(category) = context.known.category(&display, &mismatch) {
                *tally
                    .comparison
                    .known
                    .entry(category.to_owned())
                    .or_default() += 1;
                continue;
            }
            tally.add_mismatch(mismatch.category);
            mismatches.push(MismatchRow {
                file: display.clone(),
                start: mismatch.start,
                end: mismatch.end,
                context: mismatch.context,
                category: mismatch.category.name(),
                javac: mismatch.javac,
                ferrion: mismatch.ferrion,
            });
        }
    }
    if let Err(message) = ferrion {
        row.failure.get_or_insert(format!("Ferrion: {message}"));
    }
    Ok(SetResult {
        tally,
        row,
        mismatches,
        crashes,
        errors,
    })
}

/// A file as reports name it: the project, then the path, for a project's own files.
fn display(file: &str) -> String {
    file.strip_prefix("projects/").unwrap_or(file).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &str) -> Options {
        parse_args(args.split_whitespace().map(str::to_owned), |message| {
            panic!("{message}")
        })
    }

    #[test]
    fn arguments() {
        let options = parse("--source-set main --jobs 3 --heap 2g jedis guava");
        assert_eq!(options.projects, ["jedis", "guava"]);
        assert_eq!(options.source_sets, ["main"]);
        assert_eq!((options.jobs, options.heap.as_str()), (3, "2g"));
        assert_eq!(parse("").jobs, 2);
    }

    #[test]
    fn display_names() {
        assert_eq!(display("projects/jedis/src/A.java"), "jedis/src/A.java");
        assert_eq!(
            display("generated/jedis/main/A.java"),
            "generated/jedis/main/A.java"
        );
    }
}
