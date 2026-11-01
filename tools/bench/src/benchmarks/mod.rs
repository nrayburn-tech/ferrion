//! The benchmarks. Each prepares its input outside the measurement, then gives a closure that
//! runs one iteration.
//!
//! To add a component, add a module with its benchmarks and list them in [`all`]. Names are
//! `<component>/<what>`, and stay the same when the input changes, so a run can be compared with
//! one on another input (the comparison warns that the inputs differ).

mod classfile;
mod formatter;
mod lexer;
mod parser;
mod semantic;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_diagnostics::SourceText;

pub struct Benchmark {
    pub name: &'static str,
    pub description: &'static str,
    /// Prepares the input. An error means the input isn't available, and the benchmark is
    /// skipped.
    pub setup: fn(&Inputs) -> Result<Prepared>,
}

pub struct Prepared {
    /// What one iteration processes, for the report.
    pub input: String,
    /// Bytes one iteration processes, for throughput, when that means something.
    pub bytes: Option<u64>,
    pub run: Box<dyn FnMut()>,
}

/// Where inputs come from, as given on the command line.
pub struct Inputs {
    /// Corpus projects, for the benchmarks that read source files or jars.
    pub projects: Vec<String>,
    /// The JDK, for the class file benchmarks; `JAVA_HOME` if not given.
    pub jdk: Option<PathBuf>,
    /// The release the `ct.sym` benchmarks open.
    pub release: u32,
}

pub const DEFAULT_PROJECT: &str = "jedis";
pub const DEFAULT_RELEASE: u32 = 21;

pub fn all() -> Vec<Benchmark> {
    let mut all = Vec::new();
    all.extend(lexer::benchmarks());
    all.extend(parser::benchmarks());
    all.extend(formatter::benchmarks());
    all.extend(classfile::benchmarks());
    all.extend(semantic::benchmarks());
    all
}

/// A source file, read and decoded.
struct SourceFile {
    text: Arc<str>,
    /// The first release its source set is compiled for.
    release: u32,
}

/// The source files of the corpus projects in `inputs`, sorted, and a description such as
/// `jedis: 812 files, 8.7 MB`. Files that aren't UTF-8 are left out, as Ferrion skips them.
fn corpus_sources(inputs: &Inputs) -> Result<(String, Vec<SourceFile>)> {
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let mut files = Vec::new();
    for project in corpus::select(&projects, &inputs.projects)? {
        for source in corpus::sources::project_sources(&dirs, project)? {
            let bytes = std::fs::read(&source.path)
                .with_context(|| format!("reading {}", source.path.display()))?;
            if let Ok(decoded) = SourceText::decode(bytes) {
                files.push(SourceFile {
                    text: Arc::from(decoded.text()),
                    release: source.releases[0],
                });
            }
        }
    }
    let bytes: usize = files.iter().map(|file| file.text.len()).sum();
    let input = format!(
        "{}: {} files, {:.1} MB",
        inputs.projects.join(", "),
        files.len(),
        bytes as f64 / 1e6
    );
    Ok((input, files))
}
