//! Fetches the corpus of open-source Java projects used in tests (see the library's docs).

use std::process::ExitCode;

use anyhow::{Result, bail};
use corpus::project::Project;
use corpus::{Dirs, fetch, jdk_tests, lock, select};

const USAGE: &str = "\
usage: corpus <command> [<project>...]

commands:
  list      list the corpus projects
  fetch     check out projects and download their dependency jars (all projects by default)
  checkout  check out projects without downloading jars (all projects by default)
  lock      run each project's build to regenerate its lock file (needs JAVA_HOME and network access)
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
    let names: Vec<String> = args.collect();
    let dirs = Dirs::new()?;
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
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
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
