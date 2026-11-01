//! Checks the formatter on the corpus (see "Testing" in docs/design/formatter.md).
//!
//! `format-check verify` formats every file at the releases it's meant for, checks the four safety
//! properties on each, and reports throughput and peak memory (see [`verify`]).
//!
//! The formatter is reached through one seam, [`formatter`], which stands in for
//! `ferrion_formatter` until that crate lands. The corpus is found as `tools/corpus` finds it.

mod formatter;
mod verify;

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::project::Project;
use corpus::sources;

const USAGE: &str = "\
usage: format-check <check> [<option>...] [<target>...]

checks:
  verify       format every file at the releases it's meant for, check the four safety
               properties on the output, without a panic, and report throughput and peak memory

options:
  --limit <N>    show the first N failures (default: 10)
  --verbose      verify: print each file before formatting it, to find one that aborts

Targets default to every corpus project. `jdk-tests` names OpenJDK's javac tests, each formatted
at the releases its jtreg header compiles it at, and `jdk-deep-nesting` only its deep-nesting
stress tests; fetch them with `cargo corpus jdk-tests`. Projects must be checked out (`cargo
corpus checkout`).";

/// The stack size of the thread that formats, as in the deep-nesting tests.
const STACK_SIZE: usize = 256 * 1024;

pub struct Options {
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
        limit: 10,
        verbose: false,
    };
    let mut names = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
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
    if check != "verify" {
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
    Ok(verify::run(selected, &options))
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
