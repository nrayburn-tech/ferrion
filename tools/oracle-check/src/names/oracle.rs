//! Running `javac-oracle names` (or `types`, which compiles alike) on a source set, and caching
//! its output by a hash of its inputs.
//!
//! ```text
//! target/oracle/<command>/<oracle key>/<input hash>.jsonl    the oracle's output for a source set
//! ```
//!
//! The input hash covers the release and how the source set is compiled (`-source` or
//! `--release`, its options for the JDK's modules), the source set's files (names and contents),
//! its classpath and processor path (each entry's path, size, and modification time; jars in the
//! corpus are named by version, so that's enough), its processors' names and options, and every
//! Java file on its source path, which includes the sources its dependencies' processors
//! generated. Paths are hashed relative to the corpus directory, as the
//! oracle writes them, so a cache restored under another corpus directory still applies. An entry
//! is used only if the generated files it lists still exist, since the comparison reads them.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::plan::{self, SourceSet};
use crate::inventory::java_files;
use crate::oracle::{Diagnostic, hex};
use corpus::lock::PlatformKind;

/// Bumped when the oracle's `names` output or the input hash changes meaning.
const FORMAT: &str = "names-2";

/// One file's output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct FileOutput {
    /// Relative to the corpus directory.
    pub file: String,
    /// 1 for a file an annotation processor wrote.
    #[serde(default)]
    pub generated: u8,
    /// 1 for a `module-info.java`, compiled on its own in a modular compilation.
    #[serde(default)]
    pub module: u8,
    /// 1 when javac stopped before analyzing the file: after a processor reported an error, it
    /// doesn't attribute anything, so names have no elements or error ones.
    #[serde(default)]
    pub unanalyzed: u8,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// `[start, end]` of each class that is, or has a member, annotated from `lombok`.
    #[serde(default)]
    pub lombok: Vec<(i64, i64)>,
    #[serde(default)]
    pub names: Vec<Name>,
    /// javac threw on the file, even on its own.
    #[serde(default)]
    pub crash: Option<String>,
    /// The oracle left the file out (`module-info.java`, before module declarations were
    /// compiled).
    #[serde(default)]
    pub skipped: Option<String>,
}

/// The code of an error an annotation processor reports through its `Messager`: not a language
/// error.
pub const PROCESSOR_ERROR: &str = "compiler.err.proc.messager";

impl FileOutput {
    /// javac's language errors in the file: every error but those processors report.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.is_error() && d.code != PROCESSOR_ERROR)
    }

    /// The errors processors reported at the file.
    pub fn processor_errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.is_error() && d.code == PROCESSOR_ERROR)
            .count()
    }
}

/// A name javac resolved: `[start, end, context, qualifier, key]`, and for method names the type
/// searched for the method. See `Names.java` in the oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// UTF-16 offsets; the end is -1 for trees javac made up (implicit names).
    pub start: i64,
    pub end: i64,
    pub context: String,
    pub qualifier: String,
    pub key: String,
    pub searched: Option<String>,
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Name, D::Error> {
        struct NameVisitor;
        impl<'de> Visitor<'de> for NameVisitor {
            type Value = Name;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("[start, end, context, qualifier, key, searched?]")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Name, A::Error> {
                let missing = |i| de::Error::invalid_length(i, &self);
                Ok(Name {
                    start: seq.next_element()?.ok_or_else(|| missing(0))?,
                    end: seq.next_element()?.ok_or_else(|| missing(1))?,
                    context: seq.next_element()?.ok_or_else(|| missing(2))?,
                    qualifier: seq.next_element()?.ok_or_else(|| missing(3))?,
                    key: seq.next_element()?.ok_or_else(|| missing(4))?,
                    searched: seq.next_element()?,
                })
            }
        }
        deserializer.deserialize_seq(NameVisitor)
    }
}

/// What the oracle reports about the source set as a whole.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetOutput {
    /// The processor classes registered on the processor path.
    #[serde(default)]
    pub processors: Vec<String>,
    /// Errors with no file, or in a file on the source path.
    #[serde(default)]
    pub diagnostics: Vec<OtherDiagnostic>,
    /// Each processor that ran: the files it created, the errors it reported, and what it threw,
    /// after which the oracle left it out.
    #[serde(default)]
    pub processor_runs: Vec<ProcessorRun>,
    /// The modular compilations' errors in files other than their `module-info.java`.
    #[serde(default)]
    pub module_diagnostics: Vec<OtherDiagnostic>,
    /// javac threw on the whole source set, so files were compiled one at a time.
    #[serde(default)]
    pub crash: Option<String>,
    #[serde(default)]
    pub millis: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessorRun {
    pub name: String,
    pub files: u64,
    pub errors: u64,
    /// The first error it reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OtherDiagnostic {
    InFile { file: String, error: Diagnostic },
    NoFile(Diagnostic),
}

#[derive(Deserialize)]
struct Trailer {
    #[serde(rename = "sourceSet")]
    source_set: SetOutput,
}

/// The oracle's output for a source set.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub files: Vec<FileOutput>,
    pub set: SetOutput,
}

/// What a command of the oracle writes for a source set: `names`' [`Output`], or `types`'.
pub trait CommandOutput: Sized {
    fn parse(text: &str) -> Result<Self>;

    /// The files processors generated that the output names, relative to the corpus directory:
    /// a cached output is used only while they exist.
    fn generated(&self) -> Vec<&str>;
}

impl CommandOutput for Output {
    fn parse(text: &str) -> Result<Output> {
        Output::parse(text)
    }

    fn generated(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| f.generated == 1)
            .map(|f| f.file.as_str())
            .collect()
    }
}

/// The trailer line every command ends its output with, if `line` is it.
pub fn parse_trailer(line: &str) -> Option<Result<SetOutput>> {
    line.starts_with("{\"sourceSet\"").then(|| {
        serde_json::from_str::<Trailer>(line)
            .map(|t| t.source_set)
            .context("the source set's summary")
    })
}

impl Output {
    pub fn parse(text: &str) -> Result<Output> {
        let mut output = Output::default();
        let mut trailer = None;
        for (i, line) in text.lines().enumerate() {
            if line.starts_with("{\"sourceSet\"") {
                let parsed: Trailer = serde_json::from_str(line)
                    .with_context(|| format!("line {}: the source set's summary", i + 1))?;
                trailer = Some(parsed.source_set);
            } else {
                let file: FileOutput = serde_json::from_str(line)
                    .with_context(|| format!("line {} of the oracle's output", i + 1))?;
                output.files.push(file);
            }
        }
        output.set =
            trailer.context("the oracle's output has no summary line; it didn't finish")?;
        Ok(output)
    }
}

/// A command of the oracle that compiles a source set: `names` or `types`.
pub struct OracleCommand {
    /// The command, and the cache's directory under the cache root.
    pub name: &'static str,
    /// Bumped when the command's output or the input hash changes meaning.
    pub format: &'static str,
    /// The command's own options, after its name.
    pub args: Vec<String>,
}

/// The jar that runs `names` (or `types`), and the JDKs it runs on: each project's own, the one
/// its lock names.
pub struct NamesOracle {
    command: OracleCommand,
    jar: PathBuf,
    heap: String,
    /// Where every source set's processors are found instead of its own processor path, if
    /// given: an experiment.
    processor_path: Vec<PathBuf>,
    /// The JDKs, by feature version.
    jvms: BTreeMap<u32, Jvm>,
}

/// A JDK the oracle runs on.
pub struct Jvm {
    java: PathBuf,
    /// Identifies the jar, the JDK, and the processor path: the cache's directory.
    pub key: String,
}

/// What running a source set gave.
pub struct Ran<O = Output> {
    pub output: Result<O, String>,
    /// Whether the output came from the cache.
    pub cached: bool,
    pub elapsed: Duration,
    /// The output's size in bytes.
    pub bytes: u64,
}

impl NamesOracle {
    /// The oracle's `names` in `jar`, on each of `jdks`: a feature version and its JDK's home.
    pub fn new(
        jar: PathBuf,
        heap: String,
        processor_path: Vec<PathBuf>,
        jdks: &BTreeMap<u32, PathBuf>,
    ) -> Result<NamesOracle> {
        let command = OracleCommand {
            name: "names",
            format: FORMAT,
            args: Vec::new(),
        };
        NamesOracle::for_command(command, jar, heap, processor_path, jdks)
    }

    /// The oracle's `command` in `jar`, on each of `jdks`.
    pub fn for_command(
        command: OracleCommand,
        jar: PathBuf,
        heap: String,
        processor_path: Vec<PathBuf>,
        jdks: &BTreeMap<u32, PathBuf>,
    ) -> Result<NamesOracle> {
        if !jar.exists() {
            bail!(
                "{} doesn't exist; build it with `mvn --batch-mode --file tools/javac-oracle/pom.xml \
                 package`",
                jar.display()
            );
        }
        let bytes = std::fs::read(&jar).with_context(|| format!("reading {}", jar.display()))?;
        let mut jvms = BTreeMap::new();
        for (&version, home) in jdks {
            let found = corpus::lock::jdk_version(home)?;
            if found != version {
                bail!("{} is JDK {found}, not JDK {version}", home.display());
            }
            let mut hash = Sha256::new();
            hash.update(command.format);
            hash.update(b"\0");
            for arg in &command.args {
                hash.update(arg);
                hash.update(b"\0");
            }
            hash.update(&bytes);
            hash.update(b"\0");
            hash.update(std::fs::read(home.join("release"))?);
            for entry in &processor_path {
                hash.update(b"\0");
                hash.update(entry.as_os_str().as_encoded_bytes());
            }
            let java = home
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" });
            let key = hex(&hash.finalize())[..16].to_owned();
            jvms.insert(version, Jvm { java, key });
        }
        Ok(NamesOracle {
            command,
            jar,
            heap,
            processor_path,
            jvms,
        })
    }

    /// The JDK of feature version `version`, which [`NamesOracle::new`] was given.
    pub fn jvm(&self, version: u32) -> Result<&Jvm> {
        self.jvms
            .get(&version)
            .with_context(|| format!("the oracle wasn't given JDK {version}"))
    }

    /// The cached output for `set`, if there is one for its inputs as they are now. Never runs
    /// javac, so never rewrites generated sources.
    pub fn cached<O: CommandOutput>(
        &self,
        sets: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
        cache_root: &Path,
    ) -> Result<Option<O>> {
        let jvm = self.jvm(set.jdk)?;
        let cache = cache_root.join(self.command.name).join(&jvm.key);
        let sourcepath = plan::source_path(sets, set);
        let classpath = plan::class_path(sets, set);
        let processor_path = if self.processor_path.is_empty() {
            set.processor_path.clone()
        } else {
            Some(self.processor_path.clone())
        };
        let hash = input_hash(
            sets,
            set,
            &classpath,
            processor_path.as_deref(),
            &sourcepath,
            corpus,
        )?;
        let path = cache.join(format!("{hash}.jsonl"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(None);
        };
        Ok(Some(O::parse(&text)?))
    }

    /// The oracle's output for `set`, run on its project's JDK, from the cache under
    /// `cache_root` when its inputs haven't changed. `corpus` is the corpus directory, which
    /// paths in the output are relative to.
    pub fn run<O: CommandOutput>(
        &self,
        sets: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
        cache_root: &Path,
        scratch: &Path,
    ) -> Result<Ran<O>> {
        let start = Instant::now();
        let jvm = self.jvm(set.jdk)?;
        let cache = cache_root.join(self.command.name).join(&jvm.key);
        let sourcepath = plan::source_path(sets, set);
        let classpath = plan::class_path(sets, set);
        let processor_path = if self.processor_path.is_empty() {
            set.processor_path.clone()
        } else {
            Some(self.processor_path.clone())
        };
        let module_sources = if set.is_modular() {
            plan::module_sources(sets, set)
        } else {
            Vec::new()
        };
        let hash = input_hash(
            sets,
            set,
            &classpath,
            processor_path.as_deref(),
            &sourcepath,
            corpus,
        )?;
        let cached = cache.join(format!("{hash}.jsonl"));
        if let Ok(text) = std::fs::read_to_string(&cached)
            && let Ok(output) = O::parse(&text)
            && output
                .generated()
                .iter()
                .all(|file| corpus.join(file).exists())
        {
            return Ok(Ran {
                output: Ok(output),
                cached: true,
                elapsed: start.elapsed(),
                bytes: text.len() as u64,
            });
        }

        // Stale sources from an earlier run would otherwise stay on the source path of the sets
        // that depend on this one.
        if set.generated.exists() {
            std::fs::remove_dir_all(&set.generated)
                .with_context(|| format!("removing {}", set.generated.display()))?;
        }
        std::fs::create_dir_all(scratch)?;
        let name = format!("{hash}-{}", std::process::id());
        let list = scratch.join(format!("{name}.list"));
        let stdout = scratch.join(format!("{name}.out"));
        let stderr = scratch.join(format!("{name}.err"));
        let mut text = String::new();
        for file in &set.files {
            text.push_str(path_text(file)?);
            text.push('\n');
        }
        std::fs::write(&list, text)?;
        let roots: Vec<PathBuf> = set
            .roots
            .iter()
            .chain(&set.generated_roots)
            .cloned()
            .collect();
        let mut command = Command::new(&jvm.java);
        command
            .arg(format!("-Xmx{}", self.heap))
            // javac compiles on one thread; this keeps the JVM's own threads few.
            .arg("-XX:ActiveProcessorCount=2")
            .arg("-jar")
            .arg(&self.jar)
            .arg(self.command.name)
            .args(&self.command.args)
            .arg(match set.platform {
                PlatformKind::Release => "--release",
                PlatformKind::Jdk => "--source",
            })
            .arg(set.release.to_string());
        for option in &set.javac {
            command.arg("--javac").arg(option);
        }
        command
            .arg("--base")
            .arg(corpus)
            .arg("--roots")
            .arg(joined(&roots)?)
            .arg("--classpath")
            .arg(joined(&classpath)?)
            .arg("--sourcepath")
            .arg(joined(&sourcepath)?)
            .arg("--generated")
            .arg(&set.generated);
        if let Some(path) = &processor_path {
            command.arg("--processor-path").arg(joined(path)?);
        }
        if let Some(names) = &set.processors.names {
            command.arg("--processors").arg(names.join(","));
        }
        for option in &set.processors.options {
            command.arg("--processor-option").arg(option);
        }
        if set.is_modular() {
            for roots in &module_sources {
                command.arg("--module-source").arg(joined(roots)?);
            }
            command
                .arg("--module-path")
                .arg(joined(&plan::module_path(sets, set))?);
        }
        command.arg(format!("@{}", list.display()));
        if std::env::var_os("FERRION_ORACLE_SHOW").is_some() {
            eprintln!("{}: {command:?}", set.display());
        }
        let status = command
            .stdin(Stdio::null())
            .stdout(File::create(&stdout)?)
            .stderr(File::create(&stderr)?)
            .status()
            .with_context(|| format!("running {}", jvm.java.display()))?;
        let text = std::fs::read_to_string(&stdout).unwrap_or_default();
        let output = match O::parse(&text) {
            Ok(output) => {
                std::fs::create_dir_all(&cache)?;
                let partial = cached.with_extension(format!("partial-{}", std::process::id()));
                std::fs::write(&partial, &text)?;
                std::fs::rename(&partial, &cached)?;
                Ok(output)
            }
            Err(error) => {
                let stderr = std::fs::read_to_string(&stderr).unwrap_or_default();
                let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                Err(format!(
                    "the oracle's JVM exited ({status}): {error:#}: {}",
                    tail.join(" | ")
                ))
            }
        };
        for file in [&list, &stdout, &stderr] {
            std::fs::remove_file(file).ok();
        }
        Ok(Ran {
            output,
            cached: false,
            elapsed: start.elapsed(),
            bytes: text.len() as u64,
        })
    }
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("{} is not valid UTF-8", path.display()))
}

fn joined(paths: &[PathBuf]) -> Result<String> {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let parts: Vec<&str> = paths.iter().map(|p| path_text(p)).collect::<Result<_>>()?;
    Ok(parts.join(separator))
}

fn input_hash(
    sets: &[SourceSet],
    set: &SourceSet,
    classpath: &[PathBuf],
    processor_path: Option<&[PathBuf]>,
    sourcepath: &[PathBuf],
    corpus: &Path,
) -> Result<String> {
    let relative = |path: &Path| {
        corpus::lock::relative(corpus, path).unwrap_or_else(|| path.display().to_string())
    };
    let mut hash = Sha256::new();
    let mut field = |label: &str, value: &[u8]| {
        hash.update(label);
        hash.update(b"\0");
        hash.update(value);
        hash.update(b"\0");
    };
    field("release", set.release.to_string().as_bytes());
    field("platform", format!("{:?}", set.platform).as_bytes());
    for option in &set.javac {
        field("javac", relative(Path::new(option)).as_bytes());
    }
    if let Some(names) = &set.processors.names {
        field("processors", names.join(",").as_bytes());
    }
    for option in &set.processors.options {
        field("processor option", option.as_bytes());
    }
    field("generated", relative(&set.generated).as_bytes());
    for root in &set.roots {
        field("root", relative(root).as_bytes());
    }
    for file in &set.files {
        field("file", relative(file).as_bytes());
        field(
            "content",
            &std::fs::read(file).with_context(|| format!("reading {}", file.display()))?,
        );
    }
    let processor_path = processor_path.map(|path| path.iter().map(|e| ("processor path", e)));
    let entries = classpath
        .iter()
        .map(|e| ("classpath", e))
        .chain(processor_path.into_iter().flatten());
    if set.processor_path.as_ref().is_some_and(Vec::is_empty) {
        field("processor path", b"none");
    }
    for (label, entry) in entries {
        field(label, relative(entry).as_bytes());
        // A missing entry is hashed as missing; javac reports it.
        if let Ok(metadata) = std::fs::metadata(entry) {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            field("size", metadata.len().to_string().as_bytes());
            field("modified", modified.to_string().as_bytes());
        }
    }
    if set.is_modular() {
        for entry in plan::module_path(sets, set) {
            field("module path", relative(&entry).as_bytes());
        }
    }
    for dir in sourcepath {
        field("sourcepath", relative(dir).as_bytes());
        for file in java_files(dir)? {
            field("source", relative(&file).as_bytes());
            field("content", &std::fs::read(&file)?);
        }
    }
    Ok(hex(&hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_parses() {
        let text = concat!(
            r#"{"file":"projects/p/src/A.java","diagnostics":[],"lombok":[[0,10]],"names":"#,
            r#"[[8,9,"package","-","package p"],[20,21,"method","-","method p/A.m()V","type p/A source projects/p/src/A.java"]]}"#,
            "\n",
            r#"{"file":"projects/p/src/module-info.java","module":1,"diagnostics":[],"lombok":[],"names":[[7,10,"module","e","none"]]}"#,
            "\n",
            r#"{"file":"generated/p/main/G.java","generated":1,"diagnostics":[{"kind":"ERROR","code":"compiler.err.cant.resolve","start":1,"position":1,"end":2,"args":[],"message":"cannot find symbol"}],"lombok":[],"names":[]}"#,
            "\n",
            r#"{"sourceSet":{"processors":["a.P"],"diagnostics":[{"file":"x/B.java","error":{"kind":"ERROR","code":"c","start":0,"position":0,"end":0,"args":[],"message":"m"}}],"processorRuns":[{"name":"a.P","files":0,"errors":2,"first":"A: missing"}],"moduleDiagnostics":[],"millis":12}}"#,
            "\n"
        );
        let output = Output::parse(text).unwrap();
        assert_eq!(output.files.len(), 3);
        let a = &output.files[0];
        assert_eq!(a.lombok, [(0, 10)]);
        assert_eq!(a.names[0].key, "package p");
        assert_eq!(a.names[0].searched, None);
        assert_eq!(
            a.names[1].searched.as_deref(),
            Some("type p/A source projects/p/src/A.java")
        );
        assert_eq!(output.files[1].module, 1);
        assert_eq!(output.set.processor_runs[0].errors, 2);
        assert_eq!(output.files[2].errors().count(), 1);
        assert_eq!(output.set.processors, ["a.P"]);
        assert!(matches!(
            output.set.diagnostics[0],
            OtherDiagnostic::InFile { .. }
        ));
        // Without the summary line, the run didn't finish.
        assert!(Output::parse(text.lines().next().unwrap()).is_err());
    }
}
