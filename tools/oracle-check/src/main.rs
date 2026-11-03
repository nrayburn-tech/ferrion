//! Compares Ferrion's results with javac's across the corpus, through the javac oracle
//! (`tools/javac-oracle`).
//!
//! `oracle tokens` checks the lexer: its tokens and comments must match javac's after the
//! normalization in docs/design/lexer-oracle-normalization.md, and must reproduce every file byte
//! for byte. `oracle syntax` and `oracle tree` are the parser's checks (see [`syntax`] and
//! [`ferrion`]). Later phases add their own checks (see [`Check`]); `oracle names` compares name
//! resolution source set by source set (see [`names`]), and `oracle types` type attribution (see
//! [`types`]). `oracle differences` checks the known
//! differences the corpus doesn't exercise (see [`differences`]), and `oracle parser-cases` the
//! snapshots of javac's parser output (see [`parser_cases`]).
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
mod classfile;
mod differences;
mod ferrion;
mod inventory;
mod known;
mod names;
mod oracle;
mod parser_cases;
mod report;
mod run;
mod syntax;
mod tokens;
mod types;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_java_version::{LATEST_RELEASE, OLDEST_RELEASE, PRIMARY_RELEASES, SUPPORTED_RELEASES};

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
  syntax        compare the first syntax error with javac's first parsing-phase error, at each
                release
  tree          compare the tree with javac's, node by node, for files javac accepts
  differences   check the known differences in tools/oracle-check/differences against their
                snapshots, at the latest release or the releases a case names (takes only
                --oracle-jar and --bless)
  parser-cases  check the snapshots of javac's parser output in tools/oracle-check/parser-cases
                (takes only --oracle-jar and --bless)
  classfile     compare class file reading with javap, the jrt file system, java.util.zip, and
                javac, on JDKs and the corpus's jars (its own options: `classfile --help`)
  classfile-cases
                check the class files compiled from tools/oracle-check/classfile-cases and
                javap's output for them against their snapshots (takes only --bless)
  names         compare name resolution with javac's, source set by source set (its own
                options: `names --help`)
  types         compare type attribution with javac's, source set by source set (its own
                options: `types --help`)

options:
  --release <N>        compare every file at release N (repeatable); by default, each file is
                       compared at its source set's release
  --primary-releases   compare every file at each primary release: the LTS releases and the
                       latest
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
    /// The arguments of `classfile` and `names`, which have their own options.
    classfile: Vec<String>,
}

impl Options {
    fn classfile(args: Vec<String>) -> Options {
        Options {
            check: String::new(),
            releases: Releases::Own,
            jobs: 1,
            limit: 0,
            json: None,
            oracle_jar: PathBuf::new(),
            projects: Vec::new(),
            files: Vec::new(),
            bless: false,
            classfile: args,
        }
    }
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
    if args
        .peek()
        .is_some_and(|a| a == "classfile" || a == "names" || a == "types")
    {
        let check = args.next().unwrap_or_default();
        return Options {
            check,
            ..Options::classfile(args.collect())
        };
    }
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
        classfile: Vec::new(),
    };
    let mut releases = Vec::new();
    // Set by --primary-releases or --all-releases.
    let mut release_set: Option<Vec<u32>> = None;
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
            "--primary-releases" | "--all-releases" => {
                if release_set.is_some() {
                    usage("--primary-releases and --all-releases can't be combined");
                }
                release_set = Some(if arg == "--all-releases" {
                    SUPPORTED_RELEASES.collect()
                } else {
                    PRIMARY_RELEASES.to_vec()
                });
            }
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
    options.releases = match (release_set, releases.is_empty()) {
        (Some(set), true) => Releases::These(set),
        (Some(_), false) => {
            usage("--release can't be combined with --primary-releases or --all-releases")
        }
        (None, true) => Releases::Own,
        (None, false) => {
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
        "classfile" => {
            let usage = |message: &str| -> ! {
                eprintln!("oracle classfile: {message}\n\n{}", classfile::USAGE);
                std::process::exit(2);
            };
            if options.classfile.iter().any(|a| a == "-h" || a == "--help") {
                println!("{}", classfile::USAGE);
                return Ok(true);
            }
            classfile::run(classfile::parse_args(options.classfile.into_iter(), usage))
        }
        "names" => {
            let usage = |message: &str| -> ! {
                eprintln!("oracle names: {message}\n\n{}", names::USAGE);
                std::process::exit(2);
            };
            if options.classfile.iter().any(|a| a == "-h" || a == "--help") {
                println!("{}", names::USAGE);
                return Ok(true);
            }
            names::run(names::parse_args(options.classfile.into_iter(), usage))
        }
        "types" => {
            let usage = |message: &str| -> ! {
                eprintln!("oracle types: {message}\n\n{}", types::USAGE);
                std::process::exit(2);
            };
            if options.classfile.iter().any(|a| a == "-h" || a == "--help") {
                println!("{}", types::USAGE);
                return Ok(true);
            }
            types::run(types::parse_args(options.classfile.into_iter(), usage))
        }
        "tokens" => run_check(&tokens::Tokens, &options),
        "syntax" => run_check(&syntax::Syntax(ferrion::Ferrion), &options),
        "tree" => run_check(&syntax::Tree(ferrion::Ferrion), &options),
        "differences" => differences::run(options.oracle_jar, options.bless),
        "parser-cases" => parser_cases::run(options.oracle_jar, options.bless),
        "classfile-cases" => {
            let jar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/javap-oracle/target/javap-oracle.jar");
            classfile::cases::run(jar, options.bless)
        }
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

    let (mut results, timings) = run::run(check, &files, &oracle, &mut cache, options.jobs)?;
    known::apply(check.name(), &files, &mut results)?;
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

        let options = parse("tokens --primary-releases");
        assert_eq!(options.releases, Releases::These(PRIMARY_RELEASES.to_vec()));
    }
}
