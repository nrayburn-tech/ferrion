//! Compares Ferrion's results with javac's across the corpus, through the javac oracle
//! (`tools/javac-oracle`).
//!
//! `oracle tokens` checks the lexer: its tokens and comments must match javac's after the
//! normalization in docs/design/lexer-oracle-normalization.md, and must reproduce every file byte
//! for byte. `oracle syntax` and `oracle tree` are the parser's checks (see [`syntax`]); until the
//! parser is plugged in, they run and summarize javac's side only. Later phases add their own
//! checks (see [`check::Check`]). `oracle differences` checks the known differences the corpus
//! doesn't exercise (see [`differences`]), and `oracle parser-cases` the snapshots of javac's
//! parser output (see [`parser_cases`]).
//!
//! Besides the corpus projects, the checks can run on OpenJDK's javac tests, named `jdk-tests`
//! (or `jdk-deep-nesting` for its deep-nesting stress tests only), once `cargo corpus jdk-tests`
//! has fetched them.
//!
//! The corpus is found as `tools/corpus` finds it, and javac's output is cached under
//! `target/oracle`, or `FERRION_ORACLE_CACHE` if it is set (see [`cache`]).

mod allowed;
mod cache;
mod check;
mod differences;
mod inventory;
mod oracle;
mod parser_cases;
mod report;
mod run;
mod syntax;
mod tokens;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_java_version::{LATEST_RELEASE, OLDEST_RELEASE, SUPPORTED_RELEASES};

use crate::cache::Cache;
use crate::check::Check;
use crate::inventory::Releases;
use crate::oracle::Oracle;
use crate::report::Report;

const USAGE: &str = "\
usage: oracle <check> [<option>...] [<project>...]

checks:
  tokens        compare the lexer's tokens and comments with javac's, and check that the
                tokens reproduce each file
  syntax        compare syntax errors with javac's parsing-phase errors; for now, run javac and
                summarize its errors (the parser isn't plugged in yet)
  tree          compare tree positions with javac's; for now, run javac and summarize its trees
  differences   check the known differences in tools/oracle-check/differences against their
                snapshots, at the latest release (takes only --oracle-jar and --bless)
  parser-cases  check the snapshots of javac's parser output in tools/oracle-check/parser-cases
                (takes only --oracle-jar and --bless)

options:
  --release <N>        compare every file at release N (repeatable); by default, each file is
                       compared at its source set's release
  --all-releases       compare every file at every supported release
  --jobs <N>           run up to N JVMs and comparison threads at once (default: the number of CPUs)
  --limit <N>          show the first N failures (default: 10)
  --json <file>        also write the full report as JSON
  --oracle-jar <file>  the javac oracle (default: tools/javac-oracle/target/javac-oracle.jar)
  --file <file>        compare this file instead of the corpus (repeatable); its own release is
                       the latest
  --bless              differences: write the snapshots that changed instead of failing

Projects default to every corpus project. `jdk-tests` names OpenJDK's javac tests, each compared
at the releases its jtreg header compiles it at, and `jdk-deep-nesting` only its deep-nesting
stress tests; fetch them with `cargo corpus jdk-tests`. JAVA_HOME must point to a JDK that
supports the releases compared, and the projects must be checked out (`cargo corpus checkout`).
javac's output is cached in target/oracle, or in FERRION_ORACLE_CACHE if it is set.";

struct Options {
    check: String,
    releases: Releases,
    jobs: usize,
    limit: usize,
    json: Option<PathBuf>,
    oracle_jar: PathBuf,
    projects: Vec<String>,
    files: Vec<PathBuf>,
    bless: bool,
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

fn usage(message: &str) -> ! {
    eprintln!("oracle: {message}\n\n{USAGE}");
    std::process::exit(2);
}

fn parse_args(args: impl Iterator<Item = String>) -> Options {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = args.peekable();
    let check = match args.next() {
        Some(arg) if arg == "-h" || arg == "--help" => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        Some(check) if !check.starts_with('-') => check,
        _ => usage("name a check"),
    };
    let mut options = Options {
        check,
        releases: Releases::Own,
        jobs: std::thread::available_parallelism().map_or(1, |n| n.get()),
        limit: 10,
        json: None,
        oracle_jar: repository.join("tools/javac-oracle/target/javac-oracle.jar"),
        projects: Vec::new(),
        files: Vec::new(),
        bless: false,
    };
    let mut releases = Vec::new();
    let mut all_releases = false;
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .unwrap_or_else(|| usage(&format!("{name} needs a value")))
        };
        let mut number = |name: &str| -> usize {
            let text = value(name);
            text.parse()
                .unwrap_or_else(|_| usage(&format!("{name}: `{text}` isn't a number")))
        };
        match arg.as_str() {
            "--release" => {
                let release = number("--release") as u32;
                if !ferrion_java_version::is_supported(release) {
                    usage(&format!(
                        "--release must be from {OLDEST_RELEASE} to {LATEST_RELEASE}"
                    ));
                }
                releases.push(release);
            }
            "--all-releases" => all_releases = true,
            "--jobs" => options.jobs = number("--jobs").max(1),
            "--limit" => options.limit = number("--limit"),
            "--json" => options.json = Some(PathBuf::from(value("--json"))),
            "--oracle-jar" => options.oracle_jar = PathBuf::from(value("--oracle-jar")),
            "--file" => options.files.push(PathBuf::from(value("--file"))),
            "--bless" => options.bless = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => usage(&format!("unknown option `{arg}`")),
            _ => options.projects.push(arg),
        }
    }
    options.releases = match (all_releases, releases.is_empty()) {
        (true, true) => Releases::These(SUPPORTED_RELEASES.collect()),
        (true, false) => usage("--release and --all-releases can't be combined"),
        (false, true) => Releases::Own,
        (false, false) => {
            releases.sort_unstable();
            releases.dedup();
            Releases::These(releases)
        }
    };
    options
}

fn run() -> Result<bool> {
    let options = parse_args(std::env::args().skip(1));
    match options.check.as_str() {
        "tokens" => run_check(&tokens::Tokens, &options),
        "syntax" => run_check(&syntax::Syntax(syntax::NoParser), &options),
        "tree" => run_check(&syntax::Tree(syntax::NoParser), &options),
        "differences" => differences::run(options.oracle_jar, options.bless),
        "parser-cases" => parser_cases::run(options.oracle_jar, options.bless),
        other => usage(&format!("unknown check `{other}`")),
    }
}

/// Runs a check, prints its report, and returns whether everything matched.
fn run_check<C: Check>(check: &C, options: &Options) -> Result<bool> {
    let files = if options.files.is_empty() {
        let dirs = Dirs::new()?;
        let projects = Project::load_all(&dirs.projects_file())?;
        let (jdk, names): (Vec<&String>, Vec<String>) = {
            let (jdk, names): (Vec<&String>, Vec<&String>) =
                options.projects.iter().partition(|name| {
                    *name == inventory::JDK_TESTS || *name == inventory::JDK_DEEP_NESTING
                });
            (jdk, names.into_iter().cloned().collect())
        };
        let mut files = Vec::new();
        if jdk.is_empty() || !names.is_empty() {
            let selected = corpus::select(&projects, &names)?;
            files = inventory::collect(&dirs, &selected, &options.releases)?;
        }
        for name in jdk {
            let deep = name == inventory::JDK_DEEP_NESTING;
            let (jdk_files, unsupported) =
                inventory::collect_jdk_tests(&dirs, name, deep, &options.releases)?;
            if unsupported > 0 {
                eprintln!("{name}: left out {unsupported} files compiled only before Java 8");
            }
            files.extend(jdk_files);
        }
        files
    } else {
        if !options.projects.is_empty() {
            usage("--file and projects can't be combined");
        }
        inventory::collect_paths(&options.files, &options.releases)?
    };

    let oracle = Oracle::new(options.oracle_jar.clone(), check.sections())?;
    let mut releases: Vec<u32> = files.iter().flat_map(|f| f.releases.clone()).collect();
    releases.sort_unstable();
    releases.dedup();
    if let Some(&newest) = releases.last()
        && newest > oracle.jdk
    {
        bail!(
            "JAVA_HOME is JDK {}, which can't compile for release {newest}; use JDK {newest} or newer",
            oracle.jdk
        );
    }
    let cache_root = match std::env::var_os("FERRION_ORACLE_CACHE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle"),
    };
    let mut cache = Cache::open(&cache_root, &oracle.key, &releases)
        .with_context(|| format!("opening the cache in {}", cache_root.display()))?;

    let (results, timings) = run::run(check, &files, &oracle, &mut cache, options.jobs)?;
    let report = Report::new(check.name(), &files, &results, &timings, options.limit);
    print!("{}", report.text(options.limit));
    println!(
        "\njavac: {} file-releases in {} JVMs, {:.1}s; comparison: {:.1}s",
        timings.javac_files,
        timings.javac_runs,
        timings.javac.as_secs_f64(),
        timings.compare.as_secs_f64()
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &str) -> Options {
        parse_args(args.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn arguments() {
        let options = parse("tokens jedis guava");
        assert_eq!(options.check, "tokens");
        assert_eq!(options.releases, Releases::Own);
        assert_eq!(options.projects, ["jedis", "guava"]);

        let options = parse("tokens --release 17 --release 8 --release 17 --jobs 3 --limit 0");
        assert_eq!(options.releases, Releases::These(vec![8, 17]));
        assert_eq!((options.jobs, options.limit), (3, 0));

        let options = parse("tokens --file A.java --file B.java");
        assert_eq!(
            options.files,
            [PathBuf::from("A.java"), PathBuf::from("B.java")]
        );

        let options = parse("differences --bless");
        assert!(options.bless);

        let options = parse("tokens --all-releases --json out.json");
        let Releases::These(all) = options.releases else {
            panic!("expected every release");
        };
        assert_eq!(all.first(), Some(&OLDEST_RELEASE));
        assert_eq!(all.last(), Some(&LATEST_RELEASE));
        assert_eq!(all.len(), (LATEST_RELEASE - OLDEST_RELEASE + 1) as usize);
        assert_eq!(options.json, Some(PathBuf::from("out.json")));
    }
}
