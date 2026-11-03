//! Checks the formatter on the corpus, and compares it with Google Java Format (see "Testing" and
//! "Google Java Format comparison" in docs/design/formatter.md).
//!
//! `format-check verify` formats every file at the releases it's meant for, checks the four safety
//! properties on each, and reports throughput and peak memory (see [`verify`]). `format-check gjf`
//! compares Ferrion's output with GJF's fixed point for the same input, per project, against the
//! numbers committed in `gjf-diff.toml` (see [`compare`]). `format-check differences` checks the
//! departures from GJF in `differences/` (see [`differences`]), and `format-check snapshots` that
//! the formatter's snapshots are GJF's output (see [`snapshots`]).
//!
//! The corpus is found as `tools/corpus` finds it, and the GJF jar is cached with the corpus's jars
//! (see [`gjf`]).

mod compare;
mod diff;
mod differences;
mod gjf;
mod snapshots;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::project::Project;
use corpus::sources;
use ferrion_diagnostics::SourceText;
use ferrion_formatter::{FormatOptions, Formatted};

const USAGE: &str = "\
usage: format-check <check> [<option>...] [<target>...]

checks:
  verify       format every file at the releases it's meant for, check the four safety
               properties on the output, without a panic, and report throughput and peak memory
  gjf          compare Ferrion's output with Google Java Format's fixed point for the same input:
               files compared, files identical, and lines that differ, per project, with the
               largest differences and a sample of hunks
  differences  check the departures from GJF in tools/format-check/differences: that GJF and
               Ferrion still give each case's expected outputs (takes only --bless, --jobs, and
               --batch)
  snapshots    check that the expected outputs of the formatter's snapshots are GJF's fixed point,
               except those listed as not; its targets are areas, such as `statements`, or
               snapshots, such as `core/comments` (takes only --jobs and --batch)

options:
  --check        gjf: fail if a project's differing lines are above the number in
                 tools/format-check/gjf-diff.toml, or its files compared below it
  --update       gjf: write the numbers of the projects compared to gjf-diff.toml
  --tracking <f> gjf: the tracking file (default: tools/format-check/gjf-diff.toml)
  --largest <N>  gjf: list the N files with the most differing lines (default: 10)
  --hunks <N>    gjf: show the first hunk of N differing files, spread over them (default: 5)
  --jobs <N>     gjf, differences, snapshots: the processors GJF's JVM may use (default: the
                 number of CPUs)
  --batch <N>    gjf, differences, snapshots: files per JVM start (default: 1000)
  --bless        differences: write GJF's outputs that changed instead of failing
  --limit <N>    show the first N failures, rejected files, and unsettled files (default: 10)
  --verbose      verify: print each file before formatting it, to find one that aborts

Targets default to every corpus project. `jdk-tests` names OpenJDK's javac tests, each formatted
at the releases its jtreg header compiles it at (gjf: at the newest of them), and
`jdk-deep-nesting` only its deep-nesting stress tests; fetch them with `cargo corpus jdk-tests`.
Projects must be checked out (`cargo corpus checkout`). gjf and differences need JAVA_HOME to be
the JDK tools/format-check/gjf.toml names, and download the GJF jar it pins on first use; so do
snapshots.";

/// The stack size of the thread that formats, as in the deep-nesting tests.
const STACK_SIZE: usize = 256 * 1024;

pub struct Options {
    pub check: bool,
    pub update: bool,
    pub tracking: PathBuf,
    pub largest: usize,
    pub hunks: usize,
    pub jobs: usize,
    pub batch: usize,
    pub bless: bool,
    pub limit: usize,
    pub verbose: bool,
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
    let Some(check) = args.next() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    if check == "--help" || check == "-h" {
        println!("{USAGE}");
        return Ok(true);
    }
    let mut options = Options {
        check: false,
        update: false,
        tracking: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("gjf-diff.toml"),
        largest: 10,
        hunks: 5,
        jobs: std::thread::available_parallelism().map_or(1, |n| n.get()),
        batch: 1000,
        bless: false,
        limit: 10,
        verbose: false,
    };
    let mut names = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--check" => options.check = true,
            "--update" => options.update = true,
            "--tracking" => options.tracking = PathBuf::from(value("--tracking")?),
            "--largest" => options.largest = value("--largest")?.parse().context("--largest")?,
            "--hunks" => options.hunks = value("--hunks")?.parse().context("--hunks")?,
            "--jobs" => options.jobs = value("--jobs")?.parse().context("--jobs")?,
            "--batch" => options.batch = value("--batch")?.parse().context("--batch")?,
            "--bless" => options.bless = true,
            "--limit" => options.limit = value("--limit")?.parse().context("--limit")?,
            "--verbose" => options.verbose = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(true);
            }
            _ if arg.starts_with('-') => bail!("unknown option `{arg}`\n\n{USAGE}"),
            _ => names.push(arg),
        }
    }
    if options.check && options.update {
        bail!("--check and --update can't be combined");
    }

    if check == "differences" {
        if !names.is_empty() {
            bail!("differences takes no targets");
        }
        return differences::run(&options);
    }
    if check == "snapshots" {
        return snapshots::run(&names, &options);
    }
    if check != "verify" && check != "gjf" {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }

    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let targets = sources::targets(&projects, &names)?;
    let mut selected = Vec::new();
    for target in &targets {
        let (files, unsupported) = target.sources(&dirs)?;
        if unsupported > 0 {
            eprintln!(
                "{}: {unsupported} files compiled only before Java 8 left out",
                target.name()
            );
        }
        selected.push((target.name().to_owned(), files));
    }
    match check.as_str() {
        "verify" => Ok(verify::run(selected, &options)),
        _ => compare::run(selected, &options),
    }
}

/// Ferrion's output for `source` at `release`: the formatted file, or `None` if the formatter
/// skipped it.
fn ferrion_output(source: &SourceText, release: u32) -> Option<Vec<u8>> {
    match ferrion_formatter::format(source, &FormatOptions { release }) {
        Formatted::Unchanged => Some(source.to_bytes()),
        Formatted::Changed(bytes) => Some(bytes),
        Formatted::Skipped(_) => None,
    }
}

/// Runs `f` on a thread with a [`STACK_SIZE`] stack, and returns its result. A stack overflow
/// aborts the process, as it would for a user.
fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .name("format".to_owned())
        .stack_size(STACK_SIZE)
        .spawn(f)
        .expect("spawning the formatting thread")
        .join()
        .expect("the formatting thread panicked outside a check")
}

/// `part` of `whole` as a percentage, or `n/a` if `whole` is zero.
fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        "n/a".to_owned()
    } else {
        format!("{:.2}%", 100.0 * part as f64 / whole as f64)
    }
}
