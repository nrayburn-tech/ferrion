//! Fetches the corpus of open-source Java projects used in tests (see the library's docs).

use std::process::ExitCode;

use anyhow::{Result, bail};
use corpus::project::Project;
use corpus::{Dirs, fetch, lock, select};

const USAGE: &str = "\
usage: corpus <command> [<project>...]

commands:
  list      list the corpus projects
  fetch     check out projects and download their dependency jars (all projects by default)
  checkout  check out projects without downloading jars (all projects by default)
  lock      run each project's build to regenerate its lock file (needs JAVA_HOME and network access)";

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
