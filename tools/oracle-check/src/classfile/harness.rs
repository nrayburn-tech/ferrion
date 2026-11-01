//! Running the javap oracle (`tools/javap-oracle`) and reading its JSON lines.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The heap each oracle JVM gets: whole-JDK runs stream their output, so this is plenty.
const HEAP: &str = "-Xmx2g";

pub struct Harness {
    /// The `java` of `JAVA_HOME`, which runs javap and the container digests.
    java: PathBuf,
    jar: PathBuf,
    /// Identifies the jar and the JDK running it, for the cache.
    pub key: String,
}

/// javap's normalized output for one class file, or why it failed.
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct JavapOutput {
    pub file: String,
    #[serde(default)]
    pub lines: Option<Vec<String>>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One container's digests: each name's SHA-256, or the error reading it.
#[derive(Debug, Default)]
pub struct Digests {
    pub error: Option<String>,
    pub multi_release: Option<bool>,
    pub entries: BTreeMap<String, Result<String, String>>,
}

#[derive(Deserialize)]
struct DigestLine {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default, rename = "multiRelease")]
    multi_release: Option<bool>,
}

/// What javac sees for a release: each module's classes, and the visible packages (`None` when
/// every package is visible).
#[derive(Debug, Default)]
pub struct Visibility {
    pub modules: BTreeMap<String, Vec<String>>,
    pub visible: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct VisibilityLine {
    #[serde(default)]
    module: Option<String>,
    #[serde(default)]
    classes: Vec<String>,
    #[serde(default)]
    visible: Option<Vec<String>>,
    #[serde(default)]
    all: Option<bool>,
}

pub fn java_of(home: &Path) -> PathBuf {
    home.join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" })
}

impl Harness {
    pub fn new(jar: PathBuf) -> Result<Harness> {
        let home = PathBuf::from(
            std::env::var_os("JAVA_HOME")
                .context("JAVA_HOME must point to the JDK that runs javap")?,
        );
        if !jar.exists() {
            bail!(
                "{} doesn't exist; build it with `mvn --batch-mode --file tools/javap-oracle/pom.xml package`",
                jar.display()
            );
        }
        let mut hash = Sha256::new();
        hash.update(std::fs::read(&jar)?);
        hash.update(b"\0");
        hash.update(std::fs::read(home.join("release")).unwrap_or_default());
        let key = hex(&hash.finalize())[..16].to_owned();
        Ok(Harness {
            java: java_of(&home),
            jar,
            key,
        })
    }

    fn spawn(&self, java: &Path, args: &[&str], stderr: &Path) -> Result<Child> {
        Command::new(java)
            .arg(HEAP)
            .arg("-jar")
            .arg(&self.jar)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(File::create(stderr)?)
            .spawn()
            .with_context(|| format!("running {}", java.display()))
    }

    /// Runs one mode, calling `each` with every output line as it arrives.
    fn stream(
        &self,
        java: &Path,
        args: &[&str],
        scratch: &Path,
        mut each: impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        let stderr = scratch.join(format!("oracle-{}.stderr", std::process::id()));
        let mut child = self.spawn(java, args, &stderr)?;
        let stdout = child.stdout.take().context("no stdout")?;
        let mut result = Ok(());
        for line in BufReader::new(stdout).lines() {
            let line = line.context("reading the oracle's output")?;
            if result.is_ok() {
                result = each(&line);
            }
        }
        let status = child.wait()?;
        result?;
        if !status.success() {
            let message = std::fs::read_to_string(&stderr).unwrap_or_default();
            bail!("the javap oracle failed ({status}): {}", message.trim());
        }
        let _ = std::fs::remove_file(&stderr);
        Ok(())
    }

    /// javap's normalized output for each of `files`.
    pub fn javap(&self, files: &[PathBuf], scratch: &Path) -> Result<Vec<JavapOutput>> {
        let list = scratch.join(format!(
            "javap-{}-{:?}.list",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut text = String::new();
        for file in files {
            text.push_str(file.to_str().context("a path that isn't UTF-8")?);
            text.push('\n');
        }
        File::create(&list)?.write_all(text.as_bytes())?;
        let mut outputs = Vec::with_capacity(files.len());
        let arg = format!("@{}", list.display());
        self.stream(&self.java, &["javap", &arg], scratch, |line| {
            outputs.push(serde_json::from_str(line).context("parsing javap's output")?);
            Ok(())
        })?;
        let _ = std::fs::remove_file(&list);
        Ok(outputs)
    }

    /// Every file the jrt file system shows for the image of the JDK at `home`.
    pub fn jrt(&self, home: &Path, scratch: &Path) -> Result<BTreeMap<String, String>> {
        let mut digests = BTreeMap::new();
        let home = home.to_str().context("a path that isn't UTF-8")?;
        self.stream(&self.java, &["jrt", home], scratch, |line| {
            let line: DigestLine = serde_json::from_str(line)?;
            if let (Some(path), Some(sha)) = (line.path, line.sha256) {
                digests.insert(path, sha);
            }
            Ok(())
        })?;
        Ok(digests)
    }

    /// Digests of zip files through `ZipFile` (`release` `None`), or of jars through `JarFile`'s
    /// view at `release`; `each` is called with each file's digests as they arrive.
    pub fn containers(
        &self,
        release: Option<u32>,
        files: &[PathBuf],
        scratch: &Path,
        mut each: impl FnMut(&Path, Digests) -> Result<()>,
    ) -> Result<()> {
        let list = scratch.join(format!("containers-{}.list", std::process::id()));
        let mut text = String::new();
        for file in files {
            text.push_str(file.to_str().context("a path that isn't UTF-8")?);
            text.push('\n');
        }
        File::create(&list)?.write_all(text.as_bytes())?;
        let arg = format!("@{}", list.display());
        let release_text = release.map(|r| r.to_string());
        let args: Vec<&str> = match &release_text {
            Some(release) => vec!["jar", release, &arg],
            None => vec!["zip", &arg],
        };
        let mut current: Option<(PathBuf, Digests)> = None;
        self.stream(&self.java, &args, scratch, |line| {
            let line: DigestLine = serde_json::from_str(line)?;
            if let Some(file) = line.file {
                if let Some((path, digests)) = current.take() {
                    each(&path, digests)?;
                }
                current = Some((
                    PathBuf::from(file),
                    Digests {
                        error: line.error,
                        ..Digests::default()
                    },
                ));
                return Ok(());
            }
            let Some((_, digests)) = current.as_mut() else {
                return Ok(());
            };
            if let Some(multi_release) = line.multi_release {
                digests.multi_release = Some(multi_release);
            } else if let Some(name) = line.name {
                let value = match (line.sha256, line.error) {
                    (Some(sha), _) => Ok(sha),
                    (None, error) => Err(error.unwrap_or_default()),
                };
                digests.entries.insert(name, value);
            }
            Ok(())
        })?;
        if let Some((path, digests)) = current.take() {
            each(&path, digests)?;
        }
        let _ = std::fs::remove_file(&list);
        Ok(())
    }

    /// What javac on the JDK at `home` sees with `--release <release>`. `packages` are looked up
    /// for a release without modules.
    pub fn visibility(
        &self,
        home: &Path,
        release: u32,
        packages: &[String],
        scratch: &Path,
    ) -> Result<Visibility> {
        let list = scratch.join(format!("packages-{}.list", std::process::id()));
        File::create(&list)?.write_all((packages.join("\n") + "\n").as_bytes())?;
        let arg = format!("@{}", list.display());
        let release = release.to_string();
        let mut visibility = Visibility::default();
        self.stream(
            &java_of(home),
            &["visibility", &release, &arg],
            scratch,
            |line| {
                let line: VisibilityLine = serde_json::from_str(line)?;
                if let Some(module) = line.module {
                    visibility.modules.insert(module, line.classes);
                } else if line.all == Some(true) {
                    visibility.visible = None;
                } else {
                    visibility.visible = line.visible;
                }
                Ok(())
            },
        )?;
        let _ = std::fs::remove_file(&list);
        Ok(visibility)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
