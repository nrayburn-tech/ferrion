use std::collections::{BTreeMap, HashMap};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::lock::{self, ClasspathEntry};
use corpus::project::Project;
use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, SyntaxKind, parse};
use ferrion_semantic::{
    Engine, FileId, FileInput, ProjectInput, Snapshot, SourceSetId, SourceSetInput,
};

use super::{Benchmark, Inputs, Prepared};

pub fn benchmarks() -> Vec<Benchmark> {
    vec![
        Benchmark {
            name: "semantic/corpus-cold",
            description: "a new engine over the corpus projects' source sets: parse and summarize \
                          every file, then build every package table, on one thread",
            setup: corpus_cold,
        },
        Benchmark {
            name: "semantic/edits",
            description: "after a cold run, a statement added to a method body, a method renamed, \
                          and a top-level class added, in a file of the largest package, each \
                          followed by every summary and package entry asked for again, and \
                          undone the same way",
            setup: edits,
        },
    ]
}

fn corpus_cold(inputs: &Inputs) -> Result<Prepared> {
    let (input, description, bytes) = project_input(inputs)?;
    Ok(Prepared {
        input: description,
        bytes: Some(bytes),
        run: Box::new(move || {
            let engine = Engine::new(input.clone());
            ask_everything(&engine.snapshot());
            black_box(engine);
        }),
    })
}

fn edits(inputs: &Inputs) -> Result<Prepared> {
    let (input, description, _) = project_input(inputs)?;
    let engine = Engine::new(input);
    let snapshot = engine.snapshot();
    ask_everything(&snapshot);
    let (target, original) = edit_target(&snapshot)?;
    let path = snapshot.file(target).unwrap().path;
    let projects = Dirs::new()?.corpus.join("projects");
    let path = path.strip_prefix(&projects).unwrap_or(&path).to_path_buf();
    let edited: Vec<Arc<str>> = [Edit::Body, Edit::Signature, Edit::NewType]
        .into_iter()
        .map(|edit| Arc::from(apply(&original, edit).unwrap()))
        .collect();
    drop(snapshot);
    Ok(Prepared {
        input: format!("{description}; edits to {}", path.display()),
        bytes: None,
        run: Box::new(move || {
            for text in &edited {
                for text in [text, &original] {
                    engine.set_text(target, text.clone());
                    ask_everything(&engine.snapshot());
                }
            }
        }),
    })
}

/// Every summary, then every package table: the batch's first two phases.
fn ask_everything(snapshot: &Snapshot) {
    snapshot
        .run_batch(1, |_| Ok(()))
        .expect("nothing changes the inputs during a benchmark");
}

/// The corpus projects' source sets, as the lock files record them, with a description and the
/// text's size. Files that aren't UTF-8 are left out, as Ferrion skips them.
fn project_input(inputs: &Inputs) -> Result<(ProjectInput, String, u64)> {
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let mut input = ProjectInput::default();
    for project in corpus::select(&projects, &inputs.projects)? {
        add_project(&dirs, project, &mut input)?;
    }
    let files: usize = input.source_sets.iter().map(|s| s.files.len()).sum();
    let bytes: u64 = input
        .source_sets
        .iter()
        .flat_map(|s| &s.files)
        .map(|f| f.text.len() as u64)
        .sum();
    if files == 0 {
        bail!("the corpus projects have no source files");
    }
    let description = format!(
        "{}: {} source sets, {files} files, {:.1} MB",
        inputs.projects.join(", "),
        input.source_sets.len(),
        bytes as f64 / 1e6
    );
    Ok((input, description, bytes))
}

fn add_project(dirs: &Dirs, project: &Project, input: &mut ProjectInput) -> Result<()> {
    let lock =
        lock::read(dirs, project)?.with_context(|| format!("{} has no lock file", project.name))?;
    let checkout = dirs.checkout(project);
    if lock.commit != project.commit || !corpus::fetch::is_checked_out(project, dirs) {
        bail!("{} isn't checked out at its locked commit", project.name);
    }
    let first = input.source_sets.len();
    let mut ids = HashMap::new();
    for module in &lock.modules {
        for set in &module.source_sets {
            ids.insert((module.path.clone(), set.name.clone()), ids.len() + first);
        }
    }
    for module in &lock.modules {
        for set in &module.source_sets {
            let mut files = Vec::new();
            for root in &set.source_roots {
                for path in java_files(&checkout.join(root))? {
                    let bytes = std::fs::read(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    if let Ok(source) = SourceText::decode(bytes) {
                        files.push(FileInput::new(path, source.text()));
                    }
                }
            }
            let dependencies = set
                .classpath
                .iter()
                .filter_map(|entry| match entry {
                    ClasspathEntry::SourceSet { module, source_set } => ids
                        .get(&(module.clone(), source_set.clone()))
                        .map(|&i| SourceSetId::from_index(i)),
                    _ => None,
                })
                .collect();
            input.source_sets.push(SourceSetInput {
                name: format!("{}:{}/{}", project.name, module.path, set.name),
                release: set.java_release.unwrap_or(lock.jdk),
                dependencies,
                files,
            });
        }
    }
    Ok(())
}

/// The `.java` files under `dir`, sorted; none if it doesn't exist.
fn java_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "java") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The spike's edit target: the first file, in the directory with the most files, that has a
/// method with a body. Its edits reach the most other files' packages.
fn edit_target(snapshot: &Snapshot) -> Result<(FileId, Arc<str>)> {
    let mut by_dir: BTreeMap<PathBuf, Vec<FileId>> = BTreeMap::new();
    for set in snapshot.source_sets().unwrap() {
        for &file in snapshot.files(set).unwrap().iter() {
            let path = snapshot.file(file).unwrap().path;
            let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            by_dir.entry(dir).or_default().push(file);
        }
    }
    let mut dirs: Vec<_> = by_dir.into_values().collect();
    dirs.sort_by_key(|files| std::cmp::Reverse(files.len()));
    for files in dirs {
        for file in files {
            let text = snapshot.text(file).unwrap();
            if apply(&text, Edit::Body).is_some() {
                return Ok((file, text));
            }
        }
    }
    bail!("no file has a method with a body")
}

#[derive(Clone, Copy)]
enum Edit {
    /// A statement at the start of the first method body.
    Body,
    /// The first method with a body renamed.
    Signature,
    /// A top-level class at the end.
    NewType,
}

fn apply(text: &str, edit: Edit) -> Option<String> {
    let tree = parse(Arc::from(text), ParseOptions { release: 27 }).tree;
    let (offset, insert) = match edit {
        Edit::NewType => (text.len(), "\nclass FerrionBenchNew {}\n"),
        Edit::Body | Edit::Signature => {
            let method = tree.nodes().find(|n| {
                n.kind() == SyntaxKind::MethodDecl
                    && n.children().any(|c| c.kind() == SyntaxKind::Block)
            })?;
            match edit {
                Edit::Body => {
                    let block = method.children().find(|c| c.kind() == SyntaxKind::Block)?;
                    (
                        block.first_token().span().end as usize,
                        " int ferrionBenchX = 0; ",
                    )
                }
                _ => {
                    let name = method
                        .direct_tokens()
                        .find(|t| t.kind() == SyntaxKind::Identifier)?;
                    (name.span().end as usize, "Bench")
                }
            }
        }
    };
    let mut out = text.to_owned();
    out.insert_str(offset, insert);
    Some(out)
}
