//! `oracle attribute`: B3b's attribution over every file of the corpus, without javac: that
//! nothing panics, and what it costs (docs/design/semantic-attribution.md#results). Each project's
//! engine is the names comparison's ([`Ferrion`]); each file is attributed with a fuel budget
//! far above any body's, so that the fuel each body uses is measured, not cut off.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_semantic::constants::Constant;
use ferrion_semantic::types::{Printed, Type, Why};
use serde::Serialize;

use crate::names::ferrion::Ferrion;
use crate::names::plan;

pub const USAGE: &str = "\
usage: oracle attribute [<option>...] [<project>...]

Attributes every file of each corpus project's source sets (by default all of them) with B3b's
attribution, without javac, and reports panics, the fuel each body used, the time each file took,
and the largest arena. Each file gets fuel far above any body's, so that its use is measured.

options:
  --threads <N>   attribute N files at a time (default: 4)
  --slowest <N>   list the N files that took longest, and the N bodies that used the most fuel
                  (default: 10)
  --json <file>   also write the report as JSON
  --only <text>   attribute only the files whose path contains this
  --repeat <N>    attribute each file N times, for profiling (default: 1)
  --dump <file>   write each file's expressions (byte range, kind, type, constant) as JSON lines

The projects must be checked out with their jars (`cargo corpus fetch`).";

/// Fuel no body is expected to reach: what each body uses is then its own.
const MEASURING_FUEL: u64 = 1 << 40;

pub struct Options {
    projects: Vec<String>,
    threads: usize,
    slowest: usize,
    json: Option<PathBuf>,
    only: Option<String>,
    repeat: usize,
    /// The lines `--dump` writes, and where.
    dump: Option<Mutex<Vec<String>>>,
    dump_path: Option<PathBuf>,
}

pub fn parse_args(args: impl Iterator<Item = String>, usage: fn(&str) -> !) -> Options {
    let mut options = Options {
        projects: Vec::new(),
        threads: 4,
        slowest: 10,
        json: None,
        only: None,
        repeat: 1,
        dump: None,
        dump_path: None,
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
            "--threads" => {
                let text = value("--threads");
                options.threads = number("--threads", text).max(1);
            }
            "--slowest" => {
                let text = value("--slowest");
                options.slowest = number("--slowest", text);
            }
            "--json" => options.json = Some(PathBuf::from(value("--json"))),
            "--only" => options.only = Some(value("--only")),
            "--dump" => {
                options.dump_path = Some(PathBuf::from(value("--dump")));
                options.dump = Some(Mutex::new(Vec::new()));
            }
            "--repeat" => {
                let text = value("--repeat");
                options.repeat = number("--repeat", text).max(1);
            }
            _ if arg.starts_with('-') => usage(&format!("unknown option `{arg}`")),
            _ => options.projects.push(arg),
        }
    }
    options
}

/// What the run found, per project and in all.
#[derive(Default, Serialize, Clone)]
struct Tally {
    files: u64,
    bodies: u64,
    panics: u64,
    expressions: u64,
    /// Expressions by what their type is: `typed`, or the reason it has none.
    kinds: BTreeMap<String, u64>,
    micros: u64,
    largest_arena: u64,
    /// The member tables and constant entries the project's engine held at the end.
    members: u64,
    constants: u64,
}

impl Tally {
    fn merge(&mut self, other: &Tally) {
        self.files += other.files;
        self.bodies += other.bodies;
        self.panics += other.panics;
        self.expressions += other.expressions;
        for (k, v) in &other.kinds {
            *self.kinds.entry(k.clone()).or_default() += v;
        }
        self.micros += other.micros;
        self.largest_arena = self.largest_arena.max(other.largest_arena);
        self.members += other.members;
        self.constants += other.constants;
    }
}

#[derive(Default, Serialize)]
struct Report {
    total: Tally,
    projects: BTreeMap<String, Tally>,
    /// Every body's fuel, sorted, for the distribution.
    #[serde(skip)]
    fuel: Vec<u64>,
    /// Every file's time in microseconds, sorted.
    #[serde(skip)]
    times: Vec<u64>,
    most_fuel: Vec<(u64, String)>,
    slowest: Vec<(u64, String)>,
    largest_arenas: Vec<(u64, String)>,
    panics: Vec<(String, String)>,
    wall_seconds: f64,
}

/// One file's figures.
struct FileRow {
    path: String,
    micros: u64,
    fuel: Vec<(u64, u32)>,
    arena: u64,
    kinds: Vec<(&'static str, u64)>,
    expressions: u64,
    panic: Option<String>,
}

fn kind(t: Type) -> &'static str {
    match t {
        Type::Error(_) => "error",
        Type::Unknown(Why::Calls) => "calls (W3)",
        Type::Unknown(Why::LeftToB3c) => "left-to-b3c",
        Type::Unknown(Why::LeftToB3d) => "left-to-b3d",
        Type::Unknown(Why::Limit) => "limit",
        Type::Unknown(Why::Incomplete(_)) => "incomplete",
        Type::Unknown(Why::Later) => "unknown",
        Type::Unknown(Why::Error(_)) => "error",
        _ => "typed",
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
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
    let mut report = Report::default();
    for project in &selected {
        let sets = plan::source_sets(&dirs, project)?;
        let jdk = corpus::model::lock_jdk(&dirs, project)?;
        let home = corpus::jdks::find(jdk)
            .with_context(|| format!("{}'s lock names JDK {jdk}", project.name))?;
        let order: Vec<usize> = plan::order(&sets)
            .into_iter()
            .filter(|&i| !sets[i].files.is_empty())
            .collect();
        let jdks = std::collections::HashMap::from([(project.name.clone(), home)]);
        let ferrion = Ferrion::new(
            &[(project.name.clone(), order.len())],
            &jdks,
            scratch.clone(),
            options.threads,
            false,
        )?;
        let mut tally = Tally::default();
        for &i in &order {
            let set = &sets[i];
            let (rows, tables) = ferrion
                .with_set(set, |engine, id, _| {
                    let rows = attribute_set(engine, id, &options, &dirs.corpus)?;
                    let sizes = engine.sizes();
                    Ok((rows, (sizes.members, sizes.constants)))
                })
                .map_err(|e| anyhow::anyhow!("{}: {e}", set.display()))?;
            tally.members = tally.members.max(tables.0 as u64);
            tally.constants = tally.constants.max(tables.1 as u64);
            for row in rows {
                tally.files += 1;
                tally.bodies += row.fuel.len() as u64;
                tally.expressions += row.expressions;
                for (k, n) in &row.kinds {
                    *tally.kinds.entry((*k).to_owned()).or_default() += n;
                }
                tally.micros += row.micros;
                tally.largest_arena = tally.largest_arena.max(row.arena);
                if let Some(message) = &row.panic {
                    tally.panics += 1;
                    report.panics.push((row.path.clone(), message.clone()));
                }
                report.times.push(row.micros);
                report.slowest.push((row.micros, row.path.clone()));
                report.largest_arenas.push((row.arena, row.path.clone()));
                for &(fuel, at) in &row.fuel {
                    report.fuel.push(fuel);
                    report.most_fuel.push((fuel, format!("{}:{at}", row.path)));
                }
            }
            let keep = |v: &mut Vec<(u64, String)>| {
                v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                v.truncate(options.slowest);
            };
            keep(&mut report.slowest);
            keep(&mut report.most_fuel);
            keep(&mut report.largest_arenas);
        }
        eprintln!(
            "{}: {} files, {} bodies, {} panics, {:.1}s",
            project.name,
            tally.files,
            tally.bodies,
            tally.panics,
            tally.micros as f64 / 1e6
        );
        report.total.merge(&tally);
        report.projects.insert(project.name.clone(), tally);
    }
    report.fuel.sort_unstable();
    report.times.sort_unstable();
    report.wall_seconds = start.elapsed().as_secs_f64();
    print!("{}", text(&report));
    if let (Some(path), Some(lines)) = (&options.dump_path, &options.dump) {
        let lines = lines.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::write(path, lines.join("\n") + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }
    if let Some(path) = &options.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(report.total.panics == 0)
}

/// Attributes each file of a source set, after the batch's project-wide phases.
fn attribute_set(
    engine: &ferrion_semantic::Engine,
    set: ferrion_semantic::SourceSetId,
    options: &Options,
    corpus: &std::path::Path,
) -> Result<Vec<FileRow>, String> {
    let cancelled = |_| "cancelled with no edit running".to_owned();
    let threads = options.threads;
    let snapshot = engine.snapshot();
    let mut files = snapshot.files(set).map_err(cancelled)?.to_vec();
    if let Some(only) = &options.only {
        files.retain(|&f| {
            snapshot
                .file(f)
                .is_ok_and(|i| i.path.to_string_lossy().contains(only.as_str()))
        });
    }
    snapshot.run_symbols(threads).map_err(cancelled)?;
    let rows = Mutex::new(Vec::new());
    snapshot
        .run_names(threads, &files, |file| {
            let path = snapshot.file(file)?.path;
            let path = path
                .strip_prefix(corpus)
                .unwrap_or(&path)
                .display()
                .to_string();
            let mut result;
            let mut start;
            let mut repeat = options.repeat;
            loop {
                start = Instant::now();
                result = catch_unwind(AssertUnwindSafe(|| {
                    snapshot.attribute_with_fuel(file, MEASURING_FUEL)
                }));
                repeat -= 1;
                if repeat == 0 {
                    break;
                }
            }
            let micros = start.elapsed().as_micros() as u64;
            let row = match result {
                Ok(Ok(types)) => {
                    let tree = snapshot.tree(file)?;
                    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
                    let show = std::env::var("FERRION_ATTRIBUTE_SHOW").ok();
                    for (node, t) in types.types.iter() {
                        let k = kind(types.arena.get(*t));
                        *counts.entry(k).or_default() += 1;
                        // A look at the expressions of one kind, for finding what isn't typed.
                        if show.as_deref() == Some(k) {
                            let syntax = tree.node(*node);
                            let parent = syntax.parent().map(|p| p.kind());
                            let text: String = syntax.text().chars().take(60).collect();
                            eprintln!(
                                "{k}: {path}:{} {:?} in {parent:?}: {}",
                                syntax.span().start,
                                syntax.kind(),
                                text.replace('\n', " ")
                            );
                        }
                    }
                    // Each expression's byte range, kind, type string, and constant, a JSON
                    // line per file, for looking at against javac's by hand.
                    if let Some(dump) = &options.dump {
                        let view = snapshot.file(file)?.set;
                        let mut cx = snapshot.types_in(view, types.arena.clone());
                        let mut exprs = Vec::with_capacity(types.types.len());
                        for (node, t) in types.types.iter() {
                            let span = tree.node(*node).span();
                            let text = cx.type_string(*t, Printed::Captures)?;
                            // A long string's text is left out: every prefix of a long
                            // concatenation is a constant of its own.
                            let constant = types.constant(*node).map(|c| match c {
                                Constant::String(s) if s.len() > 1000 => {
                                    format!("String of {} units", s.len())
                                }
                                c => c.oracle_text(),
                            });
                            exprs.push(serde_json::json!([
                                span.start,
                                span.end,
                                format!("{:?}", tree.node(*node).kind()),
                                text,
                                constant
                            ]));
                        }
                        let mut variables = Vec::with_capacity(types.variables.len());
                        for (node, t) in types.variables.iter() {
                            // The declaring node's name, as `local` keys say where a variable is.
                            let at = tree
                                .node(*node)
                                .direct_tokens()
                                .filter(|t| t.kind() == ferrion_syntax::SyntaxKind::Identifier)
                                .last()
                                .map_or(0, |t| t.span().start);
                            let text = cx.type_string(*t, Printed::Captures)?;
                            variables.push(serde_json::json!([at, text]));
                        }
                        let line = serde_json::json!({
                            "file": path,
                            "expressions": exprs,
                            "variables": variables
                        });
                        dump.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(line.to_string());
                    }
                    FileRow {
                        path,
                        micros,
                        fuel: types
                            .fuel
                            .iter()
                            .map(|&(node, f)| (f, tree.node(node).span().start))
                            .collect(),
                        arena: types.arena.len() as u64,
                        expressions: types.types.len() as u64,
                        kinds: counts.into_iter().collect(),
                        panic: None,
                    }
                }
                Ok(Err(cancelled)) => return Err(cancelled),
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "a panic".to_owned());
                    FileRow {
                        path,
                        micros,
                        fuel: Vec::new(),
                        arena: 0,
                        expressions: 0,
                        kinds: Vec::new(),
                        panic: Some(message),
                    }
                }
            };
            rows.lock().unwrap_or_else(|e| e.into_inner()).push(row);
            Ok(())
        })
        .map_err(cancelled)?;
    Ok(rows.into_inner().unwrap_or_else(|e| e.into_inner()))
}

fn text(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let t = &report.total;
    let _ = writeln!(
        out,
        "\n| Project | Files | Bodies | Panics | Expressions | Typed | Calls (W3) | B3c | B3d | Error | Incomplete | Limit | Other | Time | Largest arena | Member tables | Constants |"
    );
    let _ = writeln!(
        out,
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
    );
    let row = |out: &mut String, name: &str, t: &Tally| {
        let k = |name: &str| t.kinds.get(name).copied().unwrap_or(0);
        let _ = writeln!(
            out,
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.1} s | {} | {} | {} |",
            t.files,
            t.bodies,
            t.panics,
            t.expressions,
            k("typed"),
            k("calls (W3)"),
            k("left-to-b3c"),
            k("left-to-b3d"),
            k("error"),
            k("incomplete"),
            k("limit"),
            k("unknown"),
            t.micros as f64 / 1e6,
            t.largest_arena,
            t.members,
            t.constants
        );
    };
    for (name, tally) in &report.projects {
        row(&mut out, name, tally);
    }
    row(&mut out, "total", t);
    let f = &report.fuel;
    let _ = writeln!(
        out,
        "\nFuel per body: median {}, p90 {}, p99 {}, p99.9 {}, p99.99 {}, max {}; {} of {} bodies above 10,000",
        percentile(f, 0.5),
        percentile(f, 0.9),
        percentile(f, 0.99),
        percentile(f, 0.999),
        percentile(f, 0.9999),
        f.last().copied().unwrap_or(0),
        f.iter().filter(|&&x| x > 10_000).count(),
        f.len()
    );
    let ms = |us: u64| us as f64 / 1000.0;
    let tm = &report.times;
    let _ = writeln!(
        out,
        "Time per file: median {:.2} ms, p90 {:.2} ms, p99 {:.2} ms, p99.9 {:.2} ms, max {:.2} ms",
        ms(percentile(tm, 0.5)),
        ms(percentile(tm, 0.9)),
        ms(percentile(tm, 0.99)),
        ms(percentile(tm, 0.999)),
        ms(tm.last().copied().unwrap_or(0))
    );
    let _ = writeln!(out, "\nMost fuel:");
    for (fuel, at) in &report.most_fuel {
        let _ = writeln!(out, "  {fuel} {at}");
    }
    let _ = writeln!(out, "Slowest:");
    for (micros, path) in &report.slowest {
        let _ = writeln!(out, "  {:.1} ms {path}", ms(*micros));
    }
    let _ = writeln!(out, "Largest arenas:");
    for (n, path) in &report.largest_arenas {
        let _ = writeln!(out, "  {n} {path}");
    }
    for (path, message) in &report.panics {
        let _ = writeln!(out, "panic: {path}: {message}");
    }
    let _ = writeln!(out, "\n{:.1} s in all", report.wall_seconds);
    out
}
