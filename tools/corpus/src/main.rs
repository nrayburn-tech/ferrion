//! Fetches the corpus of open-source Java projects used in tests (see the library's docs).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use corpus::project::Project;
use corpus::{Dirs, fetch, jdk_tests, lock, model, select};

const USAGE: &str = "\
usage: corpus <command> [<project>...]

commands:
  list      list the corpus projects
  fetch     check out projects and download their dependency jars (all projects by default)
  checkout  check out projects without downloading jars (all projects by default)
  lock      run each project's build to regenerate its lock file (needs JAVA_HOME and network access)
  project-model [--jdk <path>]
            write each fetched project's project model to target/corpus/models/<project>.json
            (all projects by default), with the JDK (default: JAVA_HOME) for every source set
  jdk-tests [list]
            check out OpenJDK's javac tests at the tag in jdk-tests.toml (test directories only);
            `list` prints each input with the releases it's compiled at";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let mut names: Vec<String> = args.collect();
    let dirs = Dirs::new()?;
    let jdk = if command == "project-model" {
        model_jdk(&mut names)?
    } else {
        None
    };
    if command == "jdk-tests" {
        return run_jdk_tests(&dirs, &names);
    }
    let projects = Project::load_all(&dirs.projects_file())?;
    let selected = select(&projects, &names)?;

    match command.as_str() {
        "list" => {
            for project in selected {
                println!(
                    "{:<20} {:<24} {:<7} {}",
                    project.name, project.tag, project.build, project.notes
                );
            }
        }
        "fetch" => {
            for project in selected {
                fetch::fetch(project, &dirs)?;
            }
        }
        "checkout" => {
            for project in selected {
                fetch::checkout(project, &dirs)?;
            }
        }
        "lock" => {
            if names.is_empty() {
                bail!("name the projects to lock");
            }
            for project in selected {
                lock::lock(project, &dirs)?;
            }
        }
        "project-model" => {
            for project in selected {
                model::write(&dirs, project, jdk.as_deref())?;
            }
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

/// Takes `--jdk <path>` out of `args`, falling back to `JAVA_HOME`, made absolute. Without
/// either, the model leaves the JDK out, and its readers use their own `--jdk` or `JAVA_HOME`.
fn model_jdk(args: &mut Vec<String>) -> Result<Option<PathBuf>> {
    let jdk = match args.iter().position(|arg| arg == "--jdk") {
        Some(at) => {
            if at + 1 >= args.len() {
                bail!("--jdk needs a path");
            }
            let jdk = args.remove(at + 1);
            args.remove(at);
            Some(PathBuf::from(jdk))
        }
        None => std::env::var_os("JAVA_HOME").map(PathBuf::from),
    };
    match jdk {
        Some(jdk) => Ok(Some(std::env::current_dir()?.join(jdk))),
        None => {
            eprintln!("note: no --jdk or JAVA_HOME, so the models give no JDK");
            Ok(None)
        }
    }
}

fn run_jdk_tests(dirs: &Dirs, args: &[String]) -> Result<()> {
    let tests = jdk_tests::JdkTests::load(dirs)?;
    let list = match args {
        [] => false,
        [arg] if arg == "list" => true,
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let dir = jdk_tests::checkout(&tests, dirs)?;
    let inputs = jdk_tests::inputs(&tests, dirs)?;
    if list {
        for input in &inputs {
            let releases: Vec<String> = input.releases.iter().map(u32::to_string).collect();
            let mut notes = Vec::new();
            if input.preview {
                notes.push("preview".to_owned());
            }
            if input.deep_nesting {
                notes.push("deep-nesting".to_owned());
            }
            if !input.unsupported.is_empty() {
                notes.push(format!("also {}", input.unsupported.join(",")));
            }
            println!(
                "{}\t{}\t{}",
                input.path,
                if releases.is_empty() {
                    "-".to_owned()
                } else {
                    releases.join(",")
                },
                notes.join(" ")
            );
        }
    }
    let unsupported = inputs.iter().filter(|i| i.releases.is_empty()).count();
    let preview = inputs.iter().filter(|i| i.preview).count();
    let deep = inputs.iter().filter(|i| i.deep_nesting).count();
    eprintln!(
        "jdk-tests: {} files at {} in {}; {unsupported} compiled only before Java 8, \
         {preview} with preview features, {deep} deep-nesting",
        inputs.len(),
        tests.tag,
        dir.display()
    );
    Ok(())
}
