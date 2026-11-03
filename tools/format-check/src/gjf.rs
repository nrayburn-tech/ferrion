//! Running Google Java Format: the pin in `gjf.toml`, fetching and checking its jar, the JDK it
//! runs on, and running it to a fixed point on many files per JVM start.
//!
//! What GJF does is learned only by running it (see CONTRIBUTING.md's clean-room policy). The
//! behavior this module relies on, and why, is written down in `gjf.toml`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use serde::Deserialize;

const PIN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/gjf.toml");

/// How many times GJF runs on a file's output at most, looking for one that doesn't change it.
pub const MAX_RUNS: usize = 3;

/// The JVM's heap limit, so a run can't take the machine's memory.
const HEAP: &str = "-Xmx2g";

/// Text in a JVM's stderr that means it failed, rather than GJF rejecting a file.
const JVM_FAILURES: [&str; 3] = [
    "Exception in thread",
    "OutOfMemoryError",
    "StackOverflowError",
];

/// The pinned GJF release, from `gjf.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    version: String,
    repository: String,
    path: String,
    sha256: String,
    jdk: u32,
    flags: Vec<String>,
}

/// GJF's result for one file.
pub enum Outcome {
    /// The file's fixed point: GJF's output, which GJF leaves unchanged, after `runs` runs (the
    /// last of which changed nothing).
    Settled { output: Vec<u8>, runs: usize },
    /// GJF rejected the file; the first error it reported.
    Rejected(String),
    /// GJF still changed its own output on the last run, or rejected it.
    Unsettled(String),
}

/// A file to give GJF: its name, which the copy keeps, and its bytes.
pub struct Input<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// The pinned GJF, ready to run.
pub struct Gjf {
    pub version: String,
    java: PathBuf,
    jar: PathBuf,
    flags: Vec<String>,
    /// Passed to the JVM as `-XX:ActiveProcessorCount`, which bounds GJF's threads.
    jobs: usize,
    /// Files per JVM start.
    batch: usize,
}

impl Gjf {
    /// Checks that `JAVA_HOME` is the pinned JDK, and fetches the jar if it isn't cached.
    pub fn new(dirs: &Dirs, jobs: usize, batch: usize) -> Result<Gjf> {
        let text = std::fs::read_to_string(PIN).with_context(|| format!("reading {PIN}"))?;
        let pin: Pin = toml::from_str(&text).with_context(|| format!("parsing {PIN}"))?;
        let java_home = PathBuf::from(
            std::env::var_os("JAVA_HOME")
                .with_context(|| format!("JAVA_HOME must point to JDK {} to run GJF", pin.jdk))?,
        );
        let jdk = corpus::lock::jdk_version(&java_home)?;
        if jdk != pin.jdk {
            bail!(
                "JAVA_HOME is JDK {jdk}, but GJF runs on JDK {} (see {PIN}); with mise, use \
                 `JAVA_HOME=$(mise where java@temurin-{})`",
                pin.jdk,
                pin.jdk
            );
        }
        let jar = dirs.jars().join(&pin.path);
        fetch(&pin, &jar)?;
        Ok(Gjf {
            version: pin.version,
            java: java_home
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" }),
            jar,
            flags: pin.flags,
            jobs: jobs.max(1),
            batch: batch.max(1),
        })
    }

    /// Runs GJF on each input, then on its output, until a run changes nothing, at most
    /// [`MAX_RUNS`] times.
    pub fn fixed_points(&self, inputs: &[Input]) -> Result<Vec<Outcome>> {
        let scratch = std::env::temp_dir().join(format!("format-check-gjf-{}", std::process::id()));
        let result = self.fixed_points_in(inputs, &scratch);
        std::fs::remove_dir_all(&scratch).ok();
        result
    }

    fn fixed_points_in(&self, inputs: &[Input], scratch: &Path) -> Result<Vec<Outcome>> {
        let mut outcomes: Vec<Option<Outcome>> = inputs.iter().map(|_| None).collect();
        let mut current: Vec<Vec<u8>> = inputs.iter().map(|input| input.bytes.to_vec()).collect();
        let mut pending: Vec<usize> = (0..inputs.len()).collect();
        for run in 1..=MAX_RUNS {
            let files: Vec<Input> = pending
                .iter()
                .map(|&i| Input {
                    name: inputs[i].name,
                    bytes: &current[i],
                })
                .collect();
            let results = self.run(&files, scratch)?;
            let mut next = Vec::new();
            for (i, result) in pending.into_iter().zip(results) {
                match result {
                    Err(error) if run == 1 => outcomes[i] = Some(Outcome::Rejected(error)),
                    Err(error) => {
                        outcomes[i] = Some(Outcome::Unsettled(format!(
                            "GJF rejected its own output on run {run}: {error}"
                        )));
                    }
                    Ok(output) if output == current[i] => {
                        outcomes[i] = Some(Outcome::Settled { output, runs: run });
                    }
                    Ok(output) => {
                        current[i] = output;
                        next.push(i);
                    }
                }
            }
            pending = next;
        }
        for i in pending {
            outcomes[i] = Some(Outcome::Unsettled(format!(
                "GJF still changed its output on run {MAX_RUNS}"
            )));
        }
        Ok(outcomes.into_iter().map(Option::unwrap).collect())
    }

    /// Runs GJF once on each file, [`Gjf::batch`] files per JVM: its output, or the error it
    /// reported.
    fn run(&self, files: &[Input], scratch: &Path) -> Result<Vec<Result<Vec<u8>, String>>> {
        let mut results = Vec::with_capacity(files.len());
        for batch in files.chunks(self.batch) {
            results.extend(self.run_batch(batch, scratch)?);
        }
        Ok(results)
    }

    /// Runs one JVM on copies of `files` in `scratch`. If the JVM fails other than by rejecting
    /// files, as when it runs out of memory, the batch is split and run again, so that one file
    /// can't take the others with it; a single file that fails is rejected.
    fn run_batch(&self, files: &[Input], scratch: &Path) -> Result<Vec<Result<Vec<u8>, String>>> {
        std::fs::create_dir_all(scratch)?;
        // GJF reports the paths it was given, so they're made canonical to be recognized.
        let scratch = scratch.canonicalize()?;
        let dir = scratch.join("files");
        std::fs::remove_dir_all(&dir).ok();
        // Each copy keeps its file name, in a directory of its own.
        let mut list = String::new();
        let mut paths = Vec::with_capacity(files.len());
        for (i, file) in files.iter().enumerate() {
            let path = dir.join(i.to_string()).join(file.name);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, file.bytes)?;
            let listed = path
                .to_str()
                .context("the temporary directory isn't UTF-8")?;
            // The list has one path per line, unquoted, so a path can't hold whitespace.
            if listed.contains(char::is_whitespace) {
                bail!(
                    "{listed}: GJF is given its files' paths unquoted, so they can't contain \
                     whitespace; set TMPDIR (TEMP on Windows) to a directory without any"
                );
            }
            list.push_str(listed);
            list.push('\n');
            paths.push(path);
        }
        let list_file = scratch.join("files.list");
        std::fs::write(&list_file, list)?;
        let output = Command::new(&self.java)
            .arg(HEAP)
            .arg(format!("-XX:ActiveProcessorCount={}", self.jobs))
            .arg("-jar")
            .arg(&self.jar)
            .args(&self.flags)
            .arg("--replace")
            .arg(format!("@{}", list_file.display()))
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running {}", self.java.display()))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let errors = errors_by_file(&stderr, &dir, files.len());
        // Rejected files make GJF exit with 1, and name themselves. A failure that names no file,
        // or that the JVM reports itself, may have stopped GJF before it got to every file, and
        // those it didn't would look already formatted.
        let named = errors.iter().any(Option::is_some);
        let jvm_failed = JVM_FAILURES.iter().any(|text| stderr.contains(text));
        if !output.status.success() && (!named || jvm_failed) {
            if files.len() == 1 {
                return Ok(vec![Err(format!(
                    "GJF failed ({}): {}",
                    output.status,
                    tail(&stderr)
                ))]);
            }
            let (first, second) = files.split_at(files.len() / 2);
            let mut results = self.run_batch(first, &scratch)?;
            results.extend(self.run_batch(second, &scratch)?);
            return Ok(results);
        }
        let mut results = Vec::with_capacity(files.len());
        for (path, error) in paths.iter().zip(errors) {
            results.push(match error {
                Some(error) => Err(error),
                None => Ok(std::fs::read(path)
                    .with_context(|| format!("reading GJF's output {}", path.display()))?),
            });
        }
        std::fs::remove_dir_all(&dir).ok();
        Ok(results)
    }
}

/// The first error GJF reported for each of `count` files copied into `dir` as `dir/<i>/<name>`,
/// from its stderr, where each error starts a line with the file's path.
fn errors_by_file(stderr: &str, dir: &Path, count: usize) -> Vec<Option<String>> {
    let mut errors = vec![None; count];
    let prefix = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let Some((index, rest)) = rest.split_once(std::path::MAIN_SEPARATOR) else {
            continue;
        };
        let Some(error) = index.parse::<usize>().ok().and_then(|i| errors.get_mut(i)) else {
            continue;
        };
        if error.is_none() {
            // `<name>:<line>:<column>: error: <message>`, reported without the directory.
            *error = Some(rest.to_owned());
        }
    }
    errors
}

/// The last few lines of a JVM's stderr, on one line.
fn tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().rev().take(5).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

/// Downloads the jar to `jar` unless it's already there, and checks its SHA-256 either way.
fn fetch(pin: &Pin, jar: &Path) -> Result<()> {
    if jar.exists() {
        let actual = corpus::lock::sha256(jar)?;
        if actual == pin.sha256 {
            return Ok(());
        }
        bail!(
            "{} has SHA-256 {actual}, but {PIN} pins {}; delete it to download it again",
            jar.display(),
            pin.sha256
        );
    }
    let url = format!("{}/{}", pin.repository.trim_end_matches('/'), pin.path);
    eprintln!("downloading GJF {} from {url}", pin.version);
    std::fs::create_dir_all(jar.parent().unwrap())?;
    let partial = jar.with_extension("partial");
    corpus::lock::run(
        Command::new("curl")
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
            .arg(&url),
    )?;
    let actual = corpus::lock::sha256(&partial)?;
    if actual != pin.sha256 {
        std::fs::remove_file(&partial)?;
        bail!(
            "{url}: expected SHA-256 {}, got {actual}. The pin in {PIN} is the reference; don't \
             change it to match a download",
            pin.sha256
        );
    }
    std::fs::rename(&partial, jar)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_parses() {
        let pin: Pin = toml::from_str(&std::fs::read_to_string(PIN).unwrap()).unwrap();
        assert_eq!(pin.version, "2.0.0");
        assert!(
            pin.path
                .ends_with(&format!("-{}-all-deps.jar", pin.version))
        );
        assert_eq!(pin.sha256.len(), 64);
        assert!(pin.flags.iter().any(|flag| flag == "--cleanup=false"));
        // Running in place is the tool's job, not the pin's.
        assert!(
            !pin.flags
                .iter()
                .any(|flag| flag == "-i" || flag == "--replace")
        );
    }

    #[test]
    fn errors_are_found_by_path() {
        let dir = Path::new("/tmp/x/files");
        let sep = std::path::MAIN_SEPARATOR;
        let stderr = format!(
            "{d}{sep}1{sep}A.java:1:21: error: illegal start of type\n\
             class A {{ void f( {{ }}\n\
             \x20                   ^\n\
             {d}{sep}1{sep}A.java:1:24: error: reached end of file while parsing\n\
             {d}{sep}3{sep}B.java:2:1: error: illegal character\n\
             {d}{sep}9{sep}C.java:2:1: error: out of range\n",
            d = dir.display()
        );
        let errors = errors_by_file(&stderr, dir, 4);
        assert_eq!(
            errors,
            [
                None,
                Some("A.java:1:21: error: illegal start of type".to_owned()),
                None,
                Some("B.java:2:1: error: illegal character".to_owned()),
            ]
        );
    }
}
