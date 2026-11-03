//! Benchmarks Ferrion's components, to catch performance regressions (docs/design/benchmarks.md).
//!
//! `benchmark run` runs each benchmark in its own process and reports wall time, user and system
//! time, peak RSS, and heap use per iteration; with `--callgrind`, also the instructions one
//! iteration takes, counted by valgrind. `--out` saves the results as JSON, and `benchmark
//! compare` compares two such files, failing when a metric grows past its threshold in
//! `thresholds.toml`. CI runs the base and head of a pull request in the same job and compares
//! them.
//!
//! The benchmarks are in [`benchmarks`]. Their inputs come from the corpus and a JDK, so nothing
//! they read is committed.

mod affected;
mod alloc;
mod benchmarks;
mod compare;
mod measure;
mod results;

use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result, bail};

use crate::benchmarks::{Benchmark, DEFAULT_PROJECT, DEFAULT_RELEASE, Inputs};
use crate::results::{BenchmarkResult, METRICS, Results};

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

const USAGE: &str = "\
usage: benchmark <command> [<option>...] [<argument>...]

commands:
  list                 list the benchmarks
  run [<filter>...]    run the benchmarks whose names contain a filter (every one if none is
                       given), each in its own process, and print what they measured
  compare <base> <head>
                       compare two results files written by `run --out`, and fail if a metric
                       grows past its threshold
  affected <commit>    print the filters for `run` that select the benchmarks a change from the
                       commit to the working tree can affect, and why on stderr

run options:
  --project <name>     a corpus project to read sources and jars from; repeatable (default:
                       jedis). Check it out and fetch its jars with `cargo corpus`.
  --jdk <path>         the JDK for the class file benchmarks (default: JAVA_HOME)
  --release <N>        the release the ct.sym benchmarks open (default: 21)
  --time <seconds>     how long to time each benchmark for (default: 3)
  --callgrind          also count each benchmark's instructions with valgrind's callgrind; the
                       profiles are kept in target/release/bench-callgrind
  --jobs <N>           how many benchmarks to count instructions for at once, after every one
                       is timed (default: the number of CPUs)
  --out <file>         write the results as JSON
  --no-skip            fail if a benchmark's input isn't available, rather than skipping it

compare options:
  --thresholds <file>  the thresholds (default: tools/bench/thresholds.toml)
  --markdown           print the comparison as Markdown, for a CI job summary";

/// The exit code of a benchmark's process when its input isn't available.
const SKIPPED: i32 = 3;

struct Options {
    inputs: Inputs,
    seconds: f64,
    callgrind: bool,
    /// How many processes run under callgrind at once.
    jobs: usize,
    out: Option<PathBuf>,
    no_skip: bool,
    thresholds: PathBuf,
    markdown: bool,
    /// For the hidden `child` command: run once for callgrind instead of timing.
    once: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let mut options = Options {
        inputs: Inputs {
            projects: Vec::new(),
            jdk: None,
            release: DEFAULT_RELEASE,
        },
        seconds: 3.0,
        callgrind: false,
        jobs: std::thread::available_parallelism().map_or(1, NonZero::get),
        out: None,
        no_skip: false,
        thresholds: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("thresholds.toml"),
        markdown: false,
        once: false,
    };
    let mut arguments = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--project" => options.inputs.projects.push(value("--project")?),
            "--jdk" => options.inputs.jdk = Some(PathBuf::from(value("--jdk")?)),
            "--release" => {
                options.inputs.release = value("--release")?.parse().context("--release")?
            }
            "--time" => options.seconds = value("--time")?.parse().context("--time")?,
            "--callgrind" => options.callgrind = true,
            "--jobs" => {
                options.jobs = value("--jobs")?
                    .parse::<NonZero<usize>>()
                    .context("--jobs")?
                    .get()
            }
            "--out" => options.out = Some(PathBuf::from(value("--out")?)),
            "--no-skip" => options.no_skip = true,
            "--thresholds" => options.thresholds = PathBuf::from(value("--thresholds")?),
            "--markdown" => options.markdown = true,
            "--once" => options.once = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(true);
            }
            _ if arg.starts_with('-') => bail!("unknown option `{arg}`\n\n{USAGE}"),
            _ => arguments.push(arg),
        }
    }
    if options.inputs.projects.is_empty() {
        options.inputs.projects.push(DEFAULT_PROJECT.to_owned());
    }

    match command.as_str() {
        "list" => {
            for benchmark in benchmarks::all() {
                println!("{:<28} {}", benchmark.name, benchmark.description);
            }
            Ok(true)
        }
        "run" => run_all(&arguments, &options),
        "compare" => {
            let [base, head] = arguments.as_slice() else {
                bail!("compare needs two results files\n\n{USAGE}");
            };
            let base = Results::read(base.as_ref())?;
            let head = Results::read(head.as_ref())?;
            let thresholds = compare::read_thresholds(&options.thresholds)?;
            Ok(compare::run(&base, &head, &thresholds, options.markdown))
        }
        "affected" => {
            let [base] = arguments.as_slice() else {
                bail!("affected needs a commit to compare with\n\n{USAGE}");
            };
            affected::run(base)?;
            Ok(true)
        }
        "child" => {
            let [name] = arguments.as_slice() else {
                bail!("child needs a benchmark name");
            };
            child(name, &options)
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Runs the selected benchmarks, each in a child process, and prints and saves their results.
fn run_all(filters: &[String], options: &Options) -> Result<bool> {
    let selected: Vec<Benchmark> = benchmarks::all()
        .into_iter()
        .filter(|benchmark| {
            filters.is_empty()
                || filters
                    .iter()
                    .any(|filter| benchmark.name.contains(filter.as_str()))
        })
        .collect();
    if selected.is_empty() {
        bail!("no benchmark matches {}", filters.join(", "));
    }
    let exe = std::env::current_exe()?;
    let profiles = exe
        .parent()
        .context("the executable has no directory")?
        .join("bench-callgrind");
    if options.callgrind {
        std::fs::create_dir_all(&profiles)?;
    }

    // Timing runs one benchmark at a time, so that each has the machine to itself. Counting
    // instructions waits until every benchmark is timed, and runs several at once: the counts
    // don't depend on the machine's load.
    let mut results = Vec::new();
    let mut counted = Vec::new();
    let mut ok = true;
    for benchmark in &selected {
        let mut command = Command::new(&exe);
        command.arg("child").arg(benchmark.name);
        for project in &options.inputs.projects {
            command.arg("--project").arg(project);
        }
        if let Some(jdk) = &options.inputs.jdk {
            command.arg("--jdk").arg(jdk);
        }
        command
            .arg("--release")
            .arg(options.inputs.release.to_string());

        let mut timed = Command::new(&exe);
        timed
            .args(command.get_args())
            .arg("--time")
            .arg(options.seconds.to_string());
        let output = timed
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("running {}", benchmark.name))?;
        let result: BenchmarkResult = match output.status.code() {
            Some(0) => serde_json::from_slice(&output.stdout)
                .with_context(|| format!("reading {}'s results", benchmark.name))?,
            Some(SKIPPED) => {
                println!("{}: skipped\n", benchmark.name);
                if options.no_skip {
                    ok = false;
                }
                continue;
            }
            _ => {
                println!("{}: FAILED ({})\n", benchmark.name, output.status);
                ok = false;
                continue;
            }
        };
        if options.callgrind {
            command.arg("--once");
            counted.push(command);
        } else {
            print_result(&result);
        }
        results.push(result);
    }
    if options.callgrind {
        ok &= count_instructions(&counted, &mut results, &profiles, options.jobs);
    }
    if let Some(out) = &options.out {
        Results::new(results).write(out)?;
    }
    Ok(ok)
}

/// Counts the instructions of each benchmark in `results`, whose process `commands` runs in
/// callgrind mode, with up to `jobs` processes under callgrind at once. Prints each result once
/// it and those before it are counted, so they're printed in order. Returns whether every count
/// succeeded.
fn count_instructions(
    commands: &[Command],
    results: &mut [BenchmarkResult],
    profiles: &Path,
    jobs: usize,
) -> bool {
    struct Progress<'a> {
        results: &'a mut [BenchmarkResult],
        counts: Vec<Option<Result<f64>>>,
        printed: usize,
        ok: bool,
    }

    let names: Vec<String> = results.iter().map(|result| result.name.clone()).collect();
    let next = AtomicUsize::new(0);
    let progress = Mutex::new(Progress {
        counts: results.iter().map(|_| None).collect(),
        results,
        printed: 0,
        ok: true,
    });
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(commands.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(command) = commands.get(index) else {
                        break;
                    };
                    let profile = profiles.join(format!("{}.out", names[index].replace('/', "-")));
                    let count = measure::callgrind(command, &profile);

                    let mut progress = progress.lock().unwrap_or_else(PoisonError::into_inner);
                    let progress = &mut *progress;
                    progress.counts[index] = Some(count);
                    while let Some(Some(count)) = progress.counts.get_mut(progress.printed) {
                        let result = &mut progress.results[progress.printed];
                        match count {
                            Ok(count) => {
                                result.metrics.insert("instructions".to_owned(), *count);
                            }
                            Err(error) => {
                                println!(
                                    "{}: counting instructions FAILED: {error:#}",
                                    result.name
                                );
                                progress.ok = false;
                            }
                        }
                        print_result(result);
                        progress.printed += 1;
                    }
                }
            });
        }
    });
    progress
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .ok
}

fn print_result(result: &BenchmarkResult) {
    println!("{}  {}", result.name, result.input);
    let value = |name: &str| result.metrics.get(name).copied();
    let shown = |names: &[&str]| -> Vec<String> {
        names
            .iter()
            .filter_map(|name| {
                let metric = METRICS.iter().find(|metric| metric.name == *name)?;
                Some(format!(
                    "{} {}",
                    metric.label,
                    results::format(value(name)?, metric.unit)
                ))
            })
            .collect()
    };
    let mut wall = shown(&["wall_ns"]);
    if let (Some(median), Some(fastest)) = (wall.first_mut(), value("wall_min_ns")) {
        median.push_str(&format!(
            " (fastest {})",
            results::format(fastest, results::Unit::Nanoseconds)
        ));
    }
    if let (Some(bytes), Some(nanos)) = (result.bytes, value("wall_ns")) {
        wall.push(format!("{:.1} MB/s", bytes as f64 / nanos * 1e3));
    }
    wall.push(format!("over {} iterations", result.iterations));
    for line in [
        wall,
        shown(&["user_ns", "system_ns", "peak_rss_bytes"]),
        shown(&["heap_peak_bytes", "heap_allocated_bytes", "allocations"]),
        shown(&["instructions"]),
    ] {
        if !line.is_empty() {
            println!("  {}", line.join(", "));
        }
    }
    println!();
}

/// Measures one benchmark in this process, and prints its results as JSON. Exits with
/// [`SKIPPED`] if its input isn't available.
fn child(name: &str, options: &Options) -> Result<bool> {
    let benchmark = benchmarks::all()
        .into_iter()
        .find(|benchmark| benchmark.name == name)
        .with_context(|| format!("no benchmark is named `{name}`"))?;
    let prepared = match (benchmark.setup)(&options.inputs) {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("{name}: {error:#}");
            std::process::exit(SKIPPED);
        }
    };
    if options.once {
        measure::once(prepared);
    } else {
        let result = measure::time(name, prepared, options.seconds);
        println!("{}", serde_json::to_string(&result)?);
    }
    Ok(true)
}
