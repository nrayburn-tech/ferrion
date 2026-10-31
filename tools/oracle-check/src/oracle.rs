//! Running the javac oracle (`tools/javac-oracle`) and reading its output.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The parts of the oracle's output, which `--sections` selects. The oracle also writes
/// `diagnostics` and `tree`, for the checks to come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Tokens,
    Comments,
    ScanDiagnostics,
}

impl Section {
    pub const fn name(self) -> &'static str {
        match self {
            Section::Tokens => "tokens",
            Section::Comments => "comments",
            Section::ScanDiagnostics => "scanDiagnostics",
        }
    }
}

/// `[kind, start, end]`, with UTF-16 offsets.
pub type Spanned = (String, u32, u32);

/// What the oracle reports for one file, without the file's path and release, so the same output
/// at two releases is stored once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<Spanned>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<Spanned>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_diagnostics: Option<Vec<Diagnostic>>,
    /// The oracle couldn't read the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// javac threw an exception.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash: Option<String>,
}

impl Output {
    /// Why javac produced nothing for the file, if it didn't.
    pub fn failure(&self) -> Option<&str> {
        self.crash.as_deref().or(self.error.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// `ERROR`, `WARNING`, `MANDATORY_WARNING`, `NOTE`, or `OTHER`.
    pub kind: String,
    pub code: String,
    /// UTF-16 offsets, or -1 when javac has no position.
    pub start: i64,
    pub position: i64,
    pub end: i64,
    pub message: String,
}

/// One line of the oracle's output.
#[derive(Deserialize)]
struct Line {
    file: String,
    #[serde(flatten)]
    output: Output,
}

/// The JDK and oracle jar that produce the oracle's output.
pub struct Oracle {
    java: PathBuf,
    jar: PathBuf,
    /// The JDK's feature version, which is the newest release it can compile for.
    pub jdk: u32,
    sections: Vec<Section>,
    /// Identifies everything the output depends on apart from the file and release: the jar, the
    /// JDK, and the sections.
    pub key: String,
}

/// Bumped when the cached form of the output changes.
const OUTPUT_FORMAT: &str = "1";

impl Oracle {
    /// Uses the JDK at `JAVA_HOME`.
    pub fn new(jar: PathBuf, sections: &[Section]) -> Result<Oracle> {
        let java_home = PathBuf::from(
            std::env::var_os("JAVA_HOME")
                .context("JAVA_HOME must point to the JDK to run javac")?,
        );
        let jdk = corpus::lock::jdk_version(&java_home)?;
        let java = java_home
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" });
        if !jar.exists() {
            bail!(
                "{} doesn't exist; build it with `mvn --batch-mode --file tools/javac-oracle/pom.xml \
                 package`",
                jar.display()
            );
        }
        let mut hash = Sha256::new();
        hash.update(OUTPUT_FORMAT);
        hash.update(b"\0");
        hash.update(std::fs::read(&jar).with_context(|| format!("reading {}", jar.display()))?);
        hash.update(b"\0");
        // The release file names the JDK's exact version and build.
        hash.update(std::fs::read(java_home.join("release"))?);
        hash.update(b"\0");
        hash.update(section_list(sections));
        let key = hex(&hash.finalize())[..16].to_owned();
        Ok(Oracle {
            java,
            jar,
            jdk,
            sections: sections.to_vec(),
            key,
        })
    }

    /// Runs one JVM on `files` at `release`, calling `on_output` with each file's index and output
    /// as it arrives. Files the JVM didn't get to, because it exited early, get an error.
    pub fn run(
        &self,
        release: u32,
        files: &[&Path],
        scratch: &Path,
        mut on_output: impl FnMut(usize, Result<Output, String>),
    ) -> Result<()> {
        let list = scratch.with_extension("list");
        let stderr = scratch.with_extension("stderr");
        let mut text = String::new();
        for file in files {
            let path = file
                .to_str()
                .with_context(|| format!("{} is not valid UTF-8", file.display()))?;
            text.push_str(path);
            text.push('\n');
        }
        File::create(&list)?.write_all(text.as_bytes())?;
        let mut child = Command::new(&self.java)
            .arg("-jar")
            .arg(&self.jar)
            .args(["--release", &release.to_string()])
            .args(["--sections", &section_list(&self.sections)])
            .arg(format!("@{}", list.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(File::create(&stderr)?)
            .spawn()
            .with_context(|| format!("running {}", self.java.display()))?;
        let stdout = child.stdout.take().unwrap();
        let mut done = 0;
        for line in BufReader::new(stdout).lines() {
            let line = line.context("reading the oracle's output")?;
            let Some(&file) = files.get(done) else {
                bail!("the oracle wrote more lines than there are files");
            };
            let parsed: Line = serde_json::from_str(&line)
                .with_context(|| format!("parsing the oracle's output for {}", file.display()))?;
            if Path::new(&parsed.file) != file {
                bail!(
                    "the oracle's output is for {}, but {} was expected",
                    parsed.file,
                    file.display()
                );
            }
            on_output(done, Ok(parsed.output));
            done += 1;
        }
        let status = child.wait()?;
        if done < files.len() {
            let stderr = std::fs::read_to_string(&stderr).unwrap_or_default();
            let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            let message = format!(
                "the oracle's JVM exited ({status}) before analyzing the file: {}",
                tail.join(" | ")
            );
            for i in done..files.len() {
                on_output(i, Err(message.clone()));
            }
        }
        std::fs::remove_file(&list).ok();
        std::fs::remove_file(&stderr).ok();
        Ok(())
    }
}

fn section_list(sections: &[Section]) -> String {
    sections
        .iter()
        .map(|section| section.name())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_lines_parse_without_path_and_release() {
        let line = r#"{"file":"/a/A.java","release":"17","tokens":[["CLASS",0,5],["EOF",5,5]],"comments":[],"scanDiagnostics":[{"kind":"ERROR","code":"compiler.err.illegal.char","start":0,"position":0,"end":0,"message":"illegal character: '#'"}]}"#;
        let parsed: Line = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.file, "/a/A.java");
        let output = parsed.output;
        assert_eq!(
            output.tokens,
            Some(vec![("CLASS".into(), 0, 5), ("EOF".into(), 5, 5)])
        );
        assert_eq!(output.scan_diagnostics.as_ref().unwrap()[0].start, 0);
        assert_eq!(output.failure(), None);
        // The stored form leaves out the path, the release, and sections that weren't asked for.
        let stored = serde_json::to_string(&output).unwrap();
        assert!(!stored.contains("release") && !stored.contains("A.java"));
        assert!(!stored.contains("error"));
        assert_eq!(serde_json::from_str::<Output>(&stored).unwrap(), output);
    }

    #[test]
    fn failures() {
        let parsed: Line = serde_json::from_str(r#"{"file":"A.java","crash":"boom"}"#).unwrap();
        assert_eq!(parsed.output.failure(), Some("boom"));
    }
}
