//! `oracle types`: compares Ferrion's type attribution with javac's, source set by source set
//! (docs/design/types-oracle-normalization.md).
//!
//! Each source set is compiled by `javac-oracle types` exactly as `oracle names` compiles it
//! ([`crate::names`]: the same plan, source path, processors, and cache), and javac's attributed
//! types, chosen methods, constant values, variables' finality, and answers to type relation
//! queries are compared with Ferrion's ([`compare`]). Files with javac's language errors, files
//! javac never analyzed or threw on, and Lombok-annotated types are left out, as in `names`.
//! Ferrion's side answers the relation queries with B3a's relations ([`ferrion`]), and gives the
//! files' entries from B3b's attribution ([`attribution`]). What waits for overload resolution or
//! a later milestone is counted apart.

pub mod attribute;
mod attribution;
pub(crate) mod compare;
mod ferrion;
pub(crate) mod oracle;
mod report;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_diagnostics::SourceText;

use self::compare::{Known, Mismatch, StandIn, Typer, compare, compare_relations, fits_text};
use self::ferrion::FerrionTyper;
use self::oracle::{FORMAT, Output, is_switch_error};
use self::report::{ExtraRow, FileTime, MismatchRow, Report, SetRow, Tally};
use crate::names::ferrion::{Ferrion, Offsets};
use crate::names::oracle::{NamesOracle, OracleCommand, Ran};
use crate::names::plan::{self, SourceSet};
use crate::names::{display, schedule};

pub const USAGE: &str = "\
usage: oracle types [<option>...] [<project>...]

Compiles each source set of the corpus projects (by default all of them) with javac as `oracle
names` does, and compares javac's attributed types, chosen methods, constant values, variables'
finality, and answers to type relation queries about a sample of each source set's declared types
with Ferrion's. Files with javac errors, files javac never analyzed, and Lombok-annotated types are
left out. Fails on a mismatch that isn't a known difference
(tools/oracle-check/differences/types-known.txt), or an entry Ferrion gives where javac has none.
What Ferrion leaves to a later milestone (`left-to-b3c`, `left-to-b3d`), and what waits for
overload resolution, is counted apart.

options:
  --source-set <id>    only this source set of each project (repeatable); the ones it depends on
                       aren't run, so their generated sources are those of an earlier run
  --jobs <N>           run up to N javac JVMs at once (default: 2)
  --heap <size>        each JVM's maximum heap (default: 4g)
  --relation-types <N> ask the relations about N declared types per source set (default: 48)
  --relations          compare the relation queries only (B3a)
  --threads <N>        attribute N files of a source set at a time (default: 4)
  --javac-only         run javac's side alone, and compare nothing
  --slowest <N>        list the N files javac took longest to analyze (default: 20)
  --limit <N>          show the first N failures (default: 10)
  --json <file>        also write the full report as JSON
  --oracle-jar <file>  the javac oracle (default: tools/javac-oracle/target/javac-oracle.jar)

Each project's javac runs on the JDK its lock names, found as `oracle names` finds it. The projects
must be checked out with their jars (`cargo corpus fetch`), and their generated sources made
(`cargo corpus generate`). The oracle's output is cached in target/oracle/types, or under
FERRION_ORACLE_CACHE if it is set.";

pub struct Options {
    pub projects: Vec<String>,
    pub source_sets: Vec<String>,
    pub jobs: usize,
    pub heap: String,
    pub relation_types: usize,
    pub relations_only: bool,
    pub threads: usize,
    pub javac_only: bool,
    pub slowest: usize,
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
        relation_types: 48,
        relations_only: false,
        threads: 4,
        javac_only: false,
        slowest: 20,
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
            "--relation-types" => {
                let text = value("--relation-types");
                options.relation_types = number("--relation-types", text);
            }
            "--relations" => options.relations_only = true,
            "--threads" => {
                let text = value("--threads");
                options.threads = number("--threads", text).max(1);
            }
            "--javac-only" => options.javac_only = true,
            "--slowest" => {
                let text = value("--slowest");
                options.slowest = number("--slowest", text);
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
    let start = Instant::now();
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let selected = corpus::select(&projects, &options.projects)?;
    let cache_root = match std::env::var_os("FERRION_ORACLE_CACHE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle"),
    };
    let scratch = cache_root.join("scratch");
    let known = Known::load()?;

    let mut groups: Vec<Vec<SourceSet>> = Vec::new();
    let mut work: Vec<(usize, usize)> = Vec::new();
    let mut jdks: BTreeMap<u32, PathBuf> = BTreeMap::new();
    let mut project_jdks: std::collections::HashMap<String, PathBuf> =
        std::collections::HashMap::new();
    for project in &selected {
        let sets = plan::source_sets(&dirs, project)?;
        let jdk = corpus::model::lock_jdk(&dirs, project)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = jdks.entry(jdk) {
            entry.insert(
                corpus::jdks::find(jdk)
                    .with_context(|| format!("{}'s lock names JDK {jdk}", project.name))?,
            );
        }
        project_jdks.insert(project.name.clone(), jdks[&jdk].clone());
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
    for (jdk, home) in &jdks {
        eprintln!("JDK {jdk}: {}", home.display());
    }
    let command = OracleCommand {
        name: "types",
        format: FORMAT,
        args: vec![
            "--relation-types".to_owned(),
            options.relation_types.to_string(),
        ],
    };
    let oracle = NamesOracle::for_command(
        command,
        options.jar.clone(),
        options.heap.clone(),
        Vec::new(),
        &jdks,
    )?;
    // Ferrion's side answers the relation queries (B3a) and, unless they are all that is
    // compared, gives the files' entries (B3b).
    let ferrion;
    let typer: &dyn Typer = if options.javac_only {
        &StandIn
    } else {
        let counts: Vec<(String, usize)> = groups
            .iter()
            .enumerate()
            .map(|(g, sets)| {
                let project = sets.first().map(|s| s.project.clone()).unwrap_or_default();
                (project, work.iter().filter(|&&(w, _)| w == g).count())
            })
            .collect();
        let threads = (!options.relations_only).then_some(options.threads);
        ferrion = FerrionTyper::new(
            Ferrion::new(&counts, &project_jdks, scratch.clone(), 1, false)?,
            threads,
        );
        &ferrion
    };
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
        cache_root: &cache_root,
        scratch: &scratch,
        typer,
        known: &known,
        relations_only: options.relations_only,
    };
    let results = schedule(
        &groups,
        &work,
        options.jobs,
        |group, i| run_set(&context, group, i),
        |result: &SetResult| {
            let row = &result.row;
            eprintln!(
                "{}: {} files, {} with errors, {:.1}s{}",
                row.source_set,
                row.files,
                row.error_files,
                row.seconds,
                if row.cached { " (cached)" } else { "" }
            );
        },
    )?;

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
        report.extras.extend(result.extras);
        report.stale_generated.extend(result.stale_generated);
        report.crashes.extend(result.crashes);
        report.errors.extend(result.errors);
        report.times.extend(result.times);
    }
    report
        .times
        .sort_by(|a, b| b.micros.cmp(&a.micros).then(a.file.cmp(&b.file)));
    report.wall_seconds = start.elapsed().as_secs_f64();
    print!("{}", report.text(options.limit, options.slowest));
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
    cache_root: &'a Path,
    scratch: &'a Path,
    typer: &'a dyn Typer,
    known: &'a Known,
    relations_only: bool,
}

/// What one source set gave.
struct SetResult {
    tally: Tally,
    row: SetRow,
    mismatches: Vec<MismatchRow>,
    extras: Vec<ExtraRow>,
    stale_generated: Vec<String>,
    crashes: Vec<(String, String)>,
    errors: Vec<(String, String)>,
    times: Vec<FileTime>,
}

/// Runs the oracle on the source set `i` of `context.groups[group]` and compares its files.
fn run_set(context: &Shared, group: usize, i: usize) -> Result<SetResult> {
    let sets = &context.groups[group];
    let set = &sets[i];
    let Ran {
        output,
        cached,
        elapsed,
        bytes,
    } = context
        .oracle
        .run::<Output>(
            sets,
            set,
            context.corpus,
            context.cache_root,
            context.scratch,
        )
        .with_context(|| set.display())?;
    let mut tally = Tally {
        source_sets: 1,
        cached: u64::from(cached),
        output_bytes: bytes,
        ..Tally::default()
    };
    let mut row = SetRow {
        source_set: set.display(),
        files: 0,
        error_files: 0,
        cached,
        seconds: elapsed.as_secs_f64(),
        failure: None,
    };
    let mut result = SetResult {
        tally: Tally::default(),
        row: row.clone(),
        mismatches: Vec::new(),
        extras: Vec::new(),
        stale_generated: Vec::new(),
        crashes: Vec::new(),
        errors: Vec::new(),
        times: Vec::new(),
    };
    let output = match output {
        Ok(output) => output,
        Err(message) => {
            tally.jvm_failures = 1;
            tally.files = set.files.len() as u64;
            row.files = tally.files;
            row.failure = Some(message);
            result.tally = tally;
            result.row = row;
            return Ok(result);
        }
    };
    tally.javac_seconds = output.set.millis as f64 / 1e3;
    if let Some(crash) = &output.set.crash {
        tally.source_set_crashes = 1;
        row.failure = Some(format!(
            "javac threw, so each file was compiled on its own: {crash}"
        ));
    }
    for query in &output.relations.queries {
        *tally.relations.entry(query.name()).or_default() += 1;
    }
    if let Some(error) = &output.relations.error {
        tally.relation_errors = 1;
        row.failure
            .get_or_insert(format!("javac threw on the relations: {error}"));
    }
    let ferrion = context
        .typer
        .attribute(sets, set, context.corpus, &output.relations);
    if let Ok(Some(attributed)) = &ferrion {
        tally.ferrion_seconds = attributed.seconds;
        tally.relation_fuel = attributed.fuel;
        tally.relation_fuel_query = format!("{}: {}", row.source_set, attributed.fuel_query);
        tally.relation_arena = attributed.arena;
        let compared = compare_relations(&output.relations, &attributed.relations);
        tally.comparison.matched += compared.matched;
        tally.comparison.left += compared.left;
        tally.comparison.unresolved += compared.unresolved;
        for mismatch in compared.mismatches {
            record(
                context,
                &mut tally,
                &mut result.mismatches,
                &row.source_set,
                mismatch,
            );
        }
    } else {
        tally.comparison.not_compared += output.relations.queries.len() as u64;
    }

    for file in &output.files {
        let display = display(&file.file);
        tally.files += 1;
        row.files += 1;
        let generated = file.generated == 1 || set.is_generated(&context.corpus.join(&file.file));
        tally.generated_files += u64::from(generated);
        for diagnostic in file.diagnostics.iter().filter(|d| is_switch_error(&d.code)) {
            *tally
                .switch_errors
                .entry(diagnostic.code.clone())
                .or_default() += 1;
        }
        if file.skipped.is_some() {
            tally.excluded.module_declarations += 1;
            continue;
        }
        if let Some(crash) = &file.crash {
            tally.excluded.crashes += 1;
            result.crashes.push((display, crash.clone()));
            continue;
        }
        if file.errors().next().is_some() {
            tally.excluded.javac_errors += 1;
            tally.files_with_only_switch_errors +=
                u64::from(file.errors().all(|d| is_switch_error(&d.code)));
            row.error_files += 1;
            if let Some(first) = file.errors().next() {
                let message = first.message.lines().next().unwrap_or("");
                result.errors.push((
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
        let bytes = std::fs::read(context.corpus.join(&file.file))
            .with_context(|| format!("reading {}", file.file))?;
        let source = SourceText::decode(bytes).map_err(|e| anyhow!("{}: {e}", file.file))?;
        // A cached run's processors may have written other text than what is there now.
        if generated && !fits_text(file, source.text()) {
            tally.excluded.stale_generated += 1;
            result.stale_generated.push(display);
            continue;
        }
        tally.compared_files += 1;
        tally.lombok_types += file.lombok.len() as u64;
        tally.analyze_micros += file.analyze_micros;
        result.times.push(FileTime {
            file: display.clone(),
            micros: file.analyze_micros,
        });
        let entries = count(&mut tally, file);
        let ours = match &ferrion {
            Ok(Some(attributed)) if !context.relations_only => attributed.files.get(&file.file),
            _ => None,
        };
        let Some(ours) = ours else {
            tally.comparison.not_compared += entries;
            continue;
        };
        let compared = compare(file, ours, &Offsets::new(&source));
        tally.comparison.matched += compared.matched;
        for (aspect, count) in &compared.matched_by {
            *tally
                .comparison
                .matched_by
                .entry(aspect.name())
                .or_default() += count;
        }
        tally.comparison.left += compared.left;
        tally.comparison.implicit_parameters += compared.implicit_parameters;
        tally.comparison.dropped_annotations += compared.dropped_annotations;
        for ((aspect, label), count) in &compared.apart {
            *tally
                .comparison
                .apart
                .entry(format!("{} {label}", aspect.name()))
                .or_default() += count;
        }
        tally.comparison.extra += compared.extras.len() as u64;
        for extra in compared.extras {
            result.extras.push(ExtraRow {
                file: display.clone(),
                aspect: extra.aspect.name(),
                start: extra.start,
                end: extra.end,
                ferrion: extra.ferrion,
            });
        }
        for mismatch in compared.mismatches {
            record(
                context,
                &mut tally,
                &mut result.mismatches,
                &display,
                mismatch,
            );
        }
    }
    if let Err(message) = ferrion {
        row.failure.get_or_insert(format!("Ferrion: {message}"));
    }
    result.tally = tally;
    result.row = row;
    Ok(result)
}

/// Counts a compared file's entries outside Lombok types, and returns how many are compared.
fn count(tally: &mut Tally, file: &oracle::FileOutput) -> u64 {
    let mut entries = 0;
    for e in file.expressions.iter().filter(|e| !file.in_lombok(e.start)) {
        tally.expressions += 1;
        *tally.expression_kinds.entry(e.kind.clone()).or_default() += 1;
        entries += 1;
    }
    for _ in file.constants.iter().filter(|c| !file.in_lombok(c.start)) {
        tally.constants += 1;
        entries += 1;
    }
    for c in file.calls.iter().filter(|c| !file.in_lombok(c.start)) {
        tally.calls += 1;
        tally.implicit_calls += u64::from(c.end < 0);
        entries += 1;
    }
    for v in file.variables.iter().filter(|v| !file.in_lombok(v.offset)) {
        tally.variables += 1;
        *tally.finality.entry(v.finality.clone()).or_default() += 1;
        entries += 1;
    }
    entries
}

/// Counts a mismatch under its known category, or as a failure.
fn record(
    context: &Shared,
    tally: &mut Tally,
    rows: &mut Vec<MismatchRow>,
    file: &str,
    mismatch: Mismatch,
) {
    if let Some(category) = context.known.category(file, &mismatch) {
        *tally
            .comparison
            .known
            .entry(category.to_owned())
            .or_default() += 1;
        return;
    }
    tally.add_mismatch(mismatch.aspect.name(), mismatch.category.name());
    rows.push(MismatchRow {
        file: file.to_owned(),
        aspect: mismatch.aspect.name(),
        start: mismatch.start,
        end: mismatch.end,
        kind: mismatch.kind,
        category: mismatch.category.name(),
        javac: mismatch.javac,
        ferrion: mismatch.ferrion,
    });
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
        let options = parse("--source-set main --jobs 3 --relation-types 8 --relations jedis");
        assert_eq!(options.projects, ["jedis"]);
        assert_eq!(options.source_sets, ["main"]);
        assert_eq!((options.jobs, options.relation_types), (3, 8));
        assert!(options.relations_only && !parse("").relations_only);
        assert_eq!(parse("--threads 2").threads, 2);
        assert_eq!((parse("").jobs, parse("").slowest), (2, 20));
        assert!(parse("--javac-only").javac_only);
    }
}
