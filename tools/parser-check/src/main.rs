//! Checks the parser on the corpus, without javac: the tools/oracle-check comparisons need the
//! oracle, and these don't.
//!
//! `parser-check round-trip` parses every file at the releases it's meant for, checks that the
//! tree reproduces it byte for byte and that `ferrion_parser::invariants` holds, and reports
//! throughput (see [`round_trip`]). `parser-check mutate` is the corpus mutation test of
//! docs/design/parser-error-recovery.md: it breaks files at seeded random tokens and measures how
//! many declarations and statements recovery preserves, against the thresholds in
//! `thresholds.toml` (see [`mutate`]).
//!
//! The corpus is found as `tools/corpus` finds it. Both checks parse on a thread with a small
//! stack, as the deep-nesting tests do, so that recursion without `ensure_stack` overflows here
//! instead of passing on a large stack.

mod mutate;
mod round_trip;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::project::Project;
use corpus::sources::{self, Source};

const USAGE: &str = "\
usage: parser-check <check> [<option>...] [<target>...]

checks:
  round-trip  parse every file at the releases it's meant for; check that the tree reproduces
              it byte for byte and keeps the parser's invariants, without a panic; report
              throughput
  mutate      the corpus mutation test: break files at seeded random tokens, check each broken
              file as round-trip does, and measure the share of declarations and statements
              recovery preserves, failing below the thresholds in tools/parser-check/thresholds.toml

options:
  --seed <N>         mutate: the random seed (default: 1)
  --sample <N>       mutate: how many mutations to make (default: 2000)
  --thresholds <f>   mutate: the thresholds file (default: tools/parser-check/thresholds.toml)
  --limit <N>        show the first N failures (default: 10)
  --verbose          round-trip: print each file before parsing it, to find one that aborts;
                     mutate: also show the first N declarations and statements lost

Targets default to every corpus project. `jdk-tests` names OpenJDK's javac tests, each parsed at
the releases its jtreg header compiles it at, and `jdk-deep-nesting` only its deep-nesting stress
tests; fetch them with `cargo corpus jdk-tests`. Projects must be checked out (`cargo corpus
checkout`).";

/// The stack size of the thread that parses, as in the deep-nesting tests.
const STACK_SIZE: usize = 256 * 1024;

pub struct Options {
    pub seed: u64,
    pub sample: usize,
    pub thresholds: PathBuf,
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
    let mut options = Options {
        seed: 1,
        sample: 2000,
        thresholds: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("thresholds.toml"),
        limit: 10,
        verbose: false,
    };
    let mut names = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--seed" => options.seed = value("--seed")?.parse().context("--seed")?,
            "--sample" => options.sample = value("--sample")?.parse().context("--sample")?,
            "--thresholds" => options.thresholds = PathBuf::from(value("--thresholds")?),
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
        "round-trip" => Ok(round_trip::run(selected, &options)),
        "mutate" => {
            let files: Vec<Source> = selected.into_iter().flat_map(|(_, files)| files).collect();
            mutate::run(files, &options)
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Runs `f` on a thread with a [`STACK_SIZE`] stack, and returns its result. A stack overflow
/// aborts the process, as it would for a user.
fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .name("parse".to_owned())
        .stack_size(STACK_SIZE)
        .spawn(f)
        .expect("spawning the parsing thread")
        .join()
        .expect("the parsing thread panicked outside a check")
}

/// `part` of `whole` as a percentage, or `n/a` if `whole` is zero.
fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        "n/a".to_owned()
    } else {
        format!("{:.2}%", 100.0 * part as f64 / whole as f64)
    }
}
