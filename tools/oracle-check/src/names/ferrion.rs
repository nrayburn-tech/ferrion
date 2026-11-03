//! Ferrion's side of the names comparison: what its name resolution gives for a source set, in
//! the comparison's terms.
//!
//! For each file, Ferrion gives a [`ResolvedName`] for every name it resolves
//! (docs/design/semantic-names.md, "Decision"):
//!
//! - **The range** is the byte range of the javac tree for the name: an identifier's own range,
//!   and for a qualified name, from the start of its first qualifier to the end of its last
//!   identifier (`java.util.List` and its qualifiers `java.util` and `java` are three names). These
//!   are the ranges the tree comparison maps Ferrion's nodes to (`ferrion::tree`): a class type's
//!   range ends at its last identifier, before its type arguments.
//! - **The key** is a string in the oracle's form (`Names.Keys` in `tools/javac-oracle`, and
//!   docs/design/semantic-oracle-normalization.md): `package p`, `module m`, `type p/A$B <origin>`
//!   with the origin `platform`, `source <file>`, or `class <classpath entry>`, `type local
//!   <file>:<offset>`, `typevar <owner key> T`, `field <owner>.<name>`, `method
//!   <owner>.<name><descriptor>`, and `local <file>:<offset>`. Files and classpath entries are
//!   relative to the corpus directory, with `/`. Offsets are those of the declaration's name, in
//!   UTF-16 code units with a byte-order mark counted as one, as javac counts them.
//! - **For a method name**, the key is the type whose members are searched for the method (JLS
//!   15.12.1), which is B2's part; the method a call chooses is B3's.
//!
//! Names `ferrion_semantic` leaves to B3 (`Symbol::LeftToB3`) have the key `b3 <why>`: one that
//! javac has in B2's part is a mismatch of its own category, and the others are counted apart
//! from Ferrion's extra names.
//!
//! [`Ferrion`] builds one engine per project from the project's model, with its type inputs
//! opened, and runs it source set by source set: the project symbols once, then each source
//! set's files, whose names are mapped and dropped file by file. The engine is dropped once the
//! last of the project's source sets in the run is done.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use corpus::Dirs;
use corpus::project::Project;
use ferrion_classfile::ArchiveCache;
use ferrion_diagnostics::{LineIndex, SourceText, Span};
use ferrion_project::{Environment, InputError, InputFlags, TypeInputs};
use ferrion_semantic::decl_names::{self, Symbol};
use ferrion_semantic::{Cancelled, Engine, FileId, FileInput, ProjectInput, Snapshot, SourceSetId};
use ferrion_syntax::{SyntaxKind, SyntaxTree};

use super::plan::SourceSet;

/// One name, as Ferrion resolved it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedName {
    /// Bytes in the file's decoded text.
    pub span: Span,
    pub key: String,
}

/// A source set's names, by file: the path relative to the corpus directory, with `/`, as the
/// oracle names files.
pub type SetNames = HashMap<String, Vec<ResolvedName>>;

/// What Ferrion gave for a source set.
#[derive(Debug, Default)]
pub struct Resolved {
    pub names: SetNames,
    /// Ferrion's own time for the source set: loading the project and its symbols if this is the
    /// project's first source set in the run, then the source set's names.
    pub seconds: f64,
}

/// Ferrion's name resolution, as the comparison runs it.
pub trait Resolver: Sync {
    /// The names of every file of `set` (one of `sets`), or `None` when nothing is to be
    /// compared. Files are read from `corpus`; generated sources are already there, under
    /// [`SourceSet::generated`]. An error is reported for the source set, not compared.
    fn resolve(
        &self,
        sets: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
    ) -> Result<Option<Resolved>, String>;
}

/// Nothing to compare: for counting javac's side alone (`--javac-only`).
pub struct StandIn;

impl Resolver for StandIn {
    fn resolve(
        &self,
        _: &[SourceSet],
        _: &SourceSet,
        _: &Path,
    ) -> Result<Option<Resolved>, String> {
        Ok(None)
    }
}

/// `ferrion_semantic` over each corpus project's model, with its jars and the JDK its lock
/// names, the oracle's.
pub struct Ferrion {
    dirs: Dirs,
    projects: Vec<Project>,
    /// Each project's JDK.
    jdks: HashMap<String, PathBuf>,
    /// Where project models are written to be loaded.
    scratch: PathBuf,
    threads: usize,
    /// Each project's engine, while some of its source sets are still to run.
    states: HashMap<String, Mutex<ProjectState>>,
}

struct ProjectState {
    /// The project's source sets in the run still to resolve.
    remaining: usize,
    loaded: Option<Loaded>,
}

struct Loaded {
    engine: Engine,
    /// The engine's source sets by the model's id.
    sets: HashMap<String, SourceSetId>,
}

impl Ferrion {
    /// For a run over `work`: each project's name with the number of its source sets the run
    /// resolves, so that its engine is dropped after the last, and with `jdks`, each project's
    /// JDK. Names are resolved on `threads` threads.
    pub fn new(
        work: &[(String, usize)],
        jdks: &HashMap<String, PathBuf>,
        scratch: PathBuf,
        threads: usize,
    ) -> anyhow::Result<Ferrion> {
        let dirs = Dirs::new()?;
        let projects = Project::load_all(&dirs.projects_file())?;
        let states = work
            .iter()
            .map(|(project, remaining)| {
                let state = ProjectState {
                    remaining: *remaining,
                    loaded: None,
                };
                (project.clone(), Mutex::new(state))
            })
            .collect();
        Ok(Ferrion {
            dirs,
            projects,
            jdks: jdks.clone(),
            scratch,
            threads: threads.max(1),
            states,
        })
    }

    /// The project's engine, over its model with its type inputs opened.
    fn load(&self, name: &str) -> Result<Loaded, String> {
        let project = self
            .projects
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("unknown project `{name}`"))?;
        let jdk = self.jdks.get(name).map(PathBuf::as_path);
        let (model, _) =
            corpus::model::project_model(&self.dirs, project, jdk).map_err(|e| format!("{e:#}"))?;
        let dir = self
            .scratch
            .join(format!("names-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(format!("{name}.json"));
        let json = model.to_json().map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("writing {}: {e}", path.display()))?;
        let flags = InputFlags {
            project_model: Some(path.clone()),
            ..InputFlags::default()
        };
        let env = Environment {
            working_dir: dir.clone(),
            java_home: None,
        };
        let loaded = ferrion_project::Project::load(&flags, &env);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
        let loaded = loaded.map_err(errors)?;
        let (types, _warnings) = TypeInputs::open(&loaded, &ArchiveCache::new()).map_err(errors)?;
        let (input, _skipped) = ProjectInput::from_project(&loaded, Some(&types));
        let sets = input
            .source_sets
            .iter()
            .enumerate()
            .map(|(i, set)| (set.name.clone(), SourceSetId::from_index(i)))
            .collect();
        Ok(Loaded {
            engine: Engine::new(input),
            sets,
        })
    }
}

fn errors(errors: Vec<InputError>) -> String {
    let errors: Vec<String> = errors
        .iter()
        .filter(|e| e.is_error())
        .map(ToString::to_string)
        .collect();
    errors.join("; ")
}

fn cancelled(_: Cancelled) -> String {
    "cancelled".to_owned()
}

impl Resolver for Ferrion {
    fn resolve(
        &self,
        _: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
    ) -> Result<Option<Resolved>, String> {
        let Some(state) = self.states.get(&set.project) else {
            return Err(format!("{} isn't one of the run's projects", set.project));
        };
        // One source set of a project at a time: a generated file added for one would cancel
        // the other's snapshot.
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        let start = Instant::now();
        let mut resolve = || -> Result<SetNames, String> {
            if state.loaded.is_none() {
                state.loaded = Some(self.load(&set.project)?);
            }
            let loaded = state.loaded.as_ref().unwrap();
            let &id = loaded
                .sets
                .get(&set.id)
                .ok_or_else(|| format!("the project model has no source set {}", set.id))?;
            sync_generated(&loaded.engine, id, &set.generated)?;
            set_names(&loaded.engine.snapshot(), id, corpus, self.threads).map_err(cancelled)
        };
        let names = resolve();
        state.remaining = state.remaining.saturating_sub(1);
        if state.remaining == 0 {
            state.loaded = None;
        }
        Ok(Some(Resolved {
            names: names?,
            seconds: start.elapsed().as_secs_f64(),
        }))
    }
}

/// Brings the engine's files under `root`, the generated-source root the oracle writes, up to
/// date with what the oracle generated for the source set, which may be after the engine was
/// built.
fn sync_generated(engine: &Engine, set: SourceSetId, root: &Path) -> Result<(), String> {
    let snapshot = engine.snapshot();
    let mut ours: HashMap<PathBuf, FileId> = HashMap::new();
    for &file in snapshot.files(set).map_err(cancelled)?.iter() {
        let info = snapshot.file(file).map_err(cancelled)?;
        if info.path.starts_with(root) {
            ours.insert(info.path.to_path_buf(), file);
        }
    }
    let mut on_disk = HashSet::new();
    for path in java_files(root) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(source) = SourceText::decode(bytes) else {
            continue;
        };
        let text: Arc<str> = Arc::from(source.text());
        match ours.get(&path) {
            Some(&file) => {
                if *snapshot.text(file).map_err(cancelled)? != *text {
                    engine.set_text(file, text);
                }
            }
            None => {
                engine.add_file(
                    set,
                    FileInput {
                        path: path.clone(),
                        text,
                        generated: true,
                    },
                );
            }
        }
        on_disk.insert(path);
    }
    for (path, file) in ours {
        if !on_disk.contains(&path) {
            engine.remove_file(file);
        }
    }
    Ok(())
}

/// The `.java` files under `dir`, sorted; none if it doesn't exist.
fn java_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "java") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Every name of `set`'s files in the oracle's terms, once the project symbols are up to date.
fn set_names(
    snapshot: &Snapshot,
    set: SourceSetId,
    corpus: &Path,
    threads: usize,
) -> Result<SetNames, Cancelled> {
    snapshot.run_symbols(threads)?;
    let files = snapshot.files(set)?;
    let out = Mutex::new(SetNames::new());
    snapshot.run_names(threads, &files, |file| {
        let (path, names) = file_names(snapshot, file, corpus)?;
        out.lock().unwrap().insert(path, names);
        Ok(())
    })?;
    Ok(out.into_inner().unwrap())
}

/// One file's names, with its path relative to `corpus`.
fn file_names(
    snapshot: &Snapshot,
    file: FileId,
    corpus: &Path,
) -> Result<(String, Vec<ResolvedName>), Cancelled> {
    let info = snapshot.file(file)?;
    let relative = info
        .path
        .strip_prefix(corpus)
        .unwrap_or(&info.path)
        .to_string_lossy()
        .replace('\\', "/");
    let text = snapshot.text(file)?;
    let bom = std::fs::read(&info.path).is_ok_and(|bytes| bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    let offsets = Offsets::from_text(&text, bom);
    let tree = snapshot.tree(file)?;
    let names = snapshot.file_names(file)?;
    let path = info.path.display().to_string();
    // Keys repeat within a file, and a source type's key looks up its file.
    let mut keys: HashMap<&Symbol, String> = HashMap::new();
    let mut out = Vec::with_capacity(names.names.len());
    for name in names.names.iter() {
        let key = match keys.get(&name.symbol) {
            Some(key) => key.clone(),
            None => {
                let key = match &name.symbol {
                    Symbol::Methods { search, .. } => {
                        snapshot.symbol_key(&Symbol::Type(*search))?.0
                    }
                    symbol => snapshot.symbol_key(symbol)?.0,
                };
                let key = oracle_key(&local_offsets(&key, &path, &offsets), corpus);
                keys.insert(&name.symbol, key.clone());
                key
            }
        };
        out.push(ResolvedName {
            span: javac_range(&tree, name),
            key,
        });
    }
    Ok((relative, out))
}

/// The range of javac's tree for a name: a class type's ends at its last identifier, before its
/// type arguments, and one with no qualifier is its identifier alone, after any annotations
/// (`@A String`), as `ferrion::tree` maps class types.
fn javac_range(tree: &SyntaxTree, name: &decl_names::ResolvedName) -> Span {
    let span = Span {
        start: name.start,
        end: name.end,
    };
    let node = tree.node(name.node);
    if node.kind() != SyntaxKind::ClassType {
        // A resource that is a variable (`try (r; ...)`) ends after its `;` in javac's tree.
        let resource = node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::ResourceSpecification);
        if resource
            && name.end == node.span().end
            && let Some(semicolon) = node
                .last_token()
                .next_non_trivia()
                .filter(|t| t.kind() == SyntaxKind::Semi)
        {
            return Span {
                start: span.start,
                end: semicolon.span().end,
            };
        }
        return span;
    }
    let identifier = node
        .direct_tokens()
        .filter(|t| matches!(t.kind(), SyntaxKind::Identifier | SyntaxKind::Underscore))
        .last();
    let Some(identifier) = identifier else {
        return span;
    };
    let qualified = node
        .children()
        .any(|child| child.kind() == SyntaxKind::ClassType);
    // A variable arity parameter's element type, with annotations on its `...` alone
    // (`Object @A ...`), ends after them in javac's tree (parser-oracle-normalization.md).
    let annotated_varargs = node.parent().and_then(|parameter| {
        let varargs = parameter
            .children()
            .find(|c| c.kind() == SyntaxKind::Varargs)?;
        varargs
            .children()
            .any(|c| c.kind() == SyntaxKind::Annotation)
            .then(|| {
                varargs
                    .direct_tokens()
                    .find(|t| t.kind() == SyntaxKind::Ellipsis)
            })?
            .and_then(|ellipsis| ellipsis.prev_non_trivia())
            .map(|before| before.span().end)
    });
    Span {
        start: if qualified {
            span.start
        } else {
            identifier.span().start
        },
        end: annotated_varargs.unwrap_or(identifier.span().end),
    }
}

/// `key` with the byte offsets of locals and local classes declared in the file at `path`
/// (`local <path>:<offset>`) in UTF-16 code units, as javac counts them.
fn local_offsets(key: &str, path: &str, offsets: &Offsets) -> String {
    let marker = format!("local {path}:");
    if !key.contains(&marker) {
        return key.to_owned();
    }
    let mut out = String::with_capacity(key.len());
    let mut rest = key;
    while let Some(at) = rest.find(&marker) {
        let after = at + marker.len();
        out.push_str(&rest[..after]);
        rest = &rest[after..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        match rest[..digits].parse::<u32>() {
            Ok(byte) => out.push_str(&offsets.utf16(byte).to_string()),
            Err(_) => out.push_str(&rest[..digits]),
        }
        rest = &rest[digits..];
    }
    out.push_str(rest);
    out
}

/// A `ferrion_semantic` symbol key in the oracle's form: each path under `corpus` relative to it,
/// with `/` (in `source <path>`, `class <path>`, and `local <path>:<offset>`), and `class
/// platform` as `platform`. Keys nest (a type variable's owner, a descriptor's local class), so
/// every occurrence is converted.
pub fn oracle_key(key: &str, corpus: &Path) -> String {
    let prefix = format!("{}/", corpus.display());
    let mut out = String::with_capacity(key.len());
    let mut rest = key;
    while let Some(at) = rest.find(&prefix) {
        out.push_str(&rest[..at]);
        rest = &rest[at + prefix.len()..];
        // The path runs to the next space, or for a local to its offset.
        let end = rest.find(' ').unwrap_or(rest.len());
        out.push_str(&rest[..end].replace('\\', "/"));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.replace("class platform", "platform")
}

/// Converts a file's byte offsets to javac's: UTF-16 code units, with a byte-order mark as one.
pub struct Offsets {
    index: LineIndex,
    bom: bool,
}

impl Offsets {
    pub fn new(source: &SourceText) -> Offsets {
        Offsets::from_text(source.text(), source.has_bom())
    }

    /// For a file's decoded text, which had a byte-order mark before it if `bom`.
    pub fn from_text(text: &str, bom: bool) -> Offsets {
        Offsets {
            index: LineIndex::new(text),
            bom,
        }
    }

    pub fn utf16(&self, byte: u32) -> i64 {
        i64::from(self.index.utf16_offset(byte) + u32::from(self.bom))
    }

    /// A span as javac's `[start, end]`.
    pub fn range(&self, span: Span) -> (i64, i64) {
        (self.utf16(span.start), self.utf16(span.end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_keys_in_the_oracles_form() {
        let corpus = Path::new("/c");
        assert_eq!(
            oracle_key("type java/lang/String class platform", corpus),
            "type java/lang/String platform"
        );
        assert_eq!(
            oracle_key(
                "typevar type p/A source /c/projects/x/src/p/A.java T",
                corpus
            ),
            "typevar type p/A source projects/x/src/p/A.java T"
        );
        assert_eq!(
            oracle_key("type q/B class /c/jars/q/b.jar", corpus),
            "type q/B class jars/q/b.jar"
        );
        assert_eq!(oracle_key("field p/A.f", corpus), "field p/A.f");
        assert_eq!(
            oracle_key("field local /c/projects/x/A.java:12.f", corpus),
            "field local projects/x/A.java:12.f"
        );
        assert_eq!(
            oracle_key(
                "typevar method local /c/p/A.java:3.m(Llocal /c/p/A.java:9;)V T",
                corpus
            ),
            "typevar method local p/A.java:3.m(Llocal p/A.java:9;)V T"
        );
    }

    #[test]
    fn local_offsets_in_utf16() {
        let text = "/* é */ class A { void m() { int x; } }";
        let offsets = Offsets::from_text(text, false);
        let x = text.find("x;").unwrap();
        // é is two bytes and one unit. Another file's offsets are left alone.
        let key = format!("field local /p/A.java:{x}.f and local /q/B.java:{x}");
        assert_eq!(
            local_offsets(&key, "/p/A.java", &offsets),
            format!("field local /p/A.java:{}.f and local /q/B.java:{x}", x - 1)
        );
    }

    #[test]
    fn offsets_count_utf16_and_the_byte_order_mark() {
        let source =
            SourceText::decode("\u{feff}/* é😀 */ class A {}".as_bytes().to_vec()).unwrap();
        let offsets = Offsets::new(&source);
        let class = source.text().find("class").unwrap() as u32;
        // `/* `, then é (one unit), 😀 (two), ` */ `, after the mark.
        assert_eq!(offsets.utf16(class), 1 + 3 + 1 + 2 + 4);
        assert_eq!(
            offsets.range(Span {
                start: class,
                end: class + 5
            }),
            (11, 16)
        );
    }
}
