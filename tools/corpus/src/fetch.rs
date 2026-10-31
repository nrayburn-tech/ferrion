//! Checking out projects and downloading their jars.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::Dirs;
use crate::lock::{self, run};
use crate::project::Project;

pub fn fetch(project: &Project, dirs: &Dirs) -> Result<()> {
    checkout(project, dirs)?;
    let Some(lock) = lock::read(dirs, project)? else {
        println!("{}: no lock file, so no jars to fetch", project.name);
        return Ok(());
    };
    if lock.commit != project.commit {
        bail!(
            "{}: the lock file is for a different commit; run `corpus lock`",
            project.name
        );
    }
    let mut downloaded = 0;
    for (path, sha256) in &lock.jars {
        let file = dirs.jars().join(path);
        if file.exists() && &lock::sha256(&file)? == sha256 {
            continue;
        }
        download(project, path, sha256, &file)?;
        downloaded += 1;
    }
    println!(
        "{}: {} jars, {downloaded} downloaded",
        project.name,
        lock.jars.len()
    );
    Ok(())
}

/// Checks out the project at its pinned commit, and returns the checkout's path.
pub fn checkout(project: &Project, dirs: &Dirs) -> Result<PathBuf> {
    let dir = dirs.checkout(project);
    if is_checked_out(project, dirs) {
        return Ok(dir);
    }
    println!("{}: checking out {}", project.name, project.commit);
    std::fs::create_dir_all(&dir)?;
    run(Command::new("git").arg("init").arg("--quiet").arg(&dir))?;
    run(git(&dir)
        .args(["fetch", "--quiet", "--depth", "1"])
        .args([&project.repository, &project.commit]))?;
    run(git(&dir).args(["checkout", "--quiet", "--force", "--detach", "FETCH_HEAD"]))?;
    Ok(dir)
}

/// Whether the project is checked out at its pinned commit.
pub fn is_checked_out(project: &Project, dirs: &Dirs) -> bool {
    head(&dirs.checkout(project)).as_deref() == Some(project.commit.as_str())
}

fn head(dir: &Path) -> Option<String> {
    let output = git(dir).args(["rev-parse", "HEAD"]).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

fn download(project: &Project, path: &str, sha256: &str, file: &Path) -> Result<()> {
    std::fs::create_dir_all(file.parent().unwrap())?;
    let partial = file.with_extension("partial");
    for repository in &project.repositories {
        let url = format!("{}/{path}", repository.trim_end_matches('/'));
        let status = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "3",
                "--output",
            ])
            .arg(&partial)
            .arg(&url)
            .status()
            .context("running curl")?;
        if !status.success() {
            continue;
        }
        let actual = lock::sha256(&partial)?;
        if actual != sha256 {
            std::fs::remove_file(&partial)?;
            bail!("{url}: expected SHA-256 {sha256}, got {actual}");
        }
        std::fs::rename(&partial, file)?;
        return Ok(());
    }
    bail!(
        "{}: {path} is not in any of the project's repositories",
        project.name
    )
}
