//! The engine's differential check (docs/design/semantic-incremental.md#checking-the-engine), for
//! the fuzz target in `fuzz/` and the tests. Not part of the public API.
//!
//! A dependency the engine fails to record would make it reuse a stale entry. So [`check_edits`]
//! applies a sequence of edits to a small project with two source sets, and after each one asks
//! the engine some of its questions and compares every answer with a fresh engine's, built from
//! the same inputs. Which edits, and which questions, come from the input bytes: asking only some
//! questions leaves the other entries unverified across several revisions, which is where a
//! stale entry would hide. Every entry the engine returns is also checked to carry its value's
//! fingerprint.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::packages::{Package, PackageIndex};
use crate::{
    Cancelled, Engine, EngineOptions, FileId, FileInput, Fingerprint, ProjectInput, Snapshot,
    SourceSetId, SourceSetInput,
};

/// Reads `bytes` as an optional file text, up to a first zero byte, then the choices for
/// [`check_edits`]. The text, if any, is added to the project as one more file.
pub fn check_bytes(bytes: &[u8]) -> Result<(), String> {
    let (extra, choices) = match bytes.iter().position(|&b| b == 0) {
        Some(zero) => (
            Some(String::from_utf8_lossy(&bytes[..zero]).into_owned()),
            &bytes[zero + 1..],
        ),
        None => (None, bytes),
    };
    check_edits(extra, choices, false)
}

/// Builds the project, with `extra` as one more file if given, and applies up to
/// [`MAX_EDITS`] edits chosen by `choices`, comparing the engine with a fresh one after each.
///
/// With `concurrent`, a batch starts on another thread at the revision before each edit, and
/// the edit lands while it runs, so that entries computed at a revision already gone are stored
/// and must be verified like any other. Which entries those are depends on timing.
pub fn check_edits(extra: Option<String>, choices: &[u8], concurrent: bool) -> Result<(), String> {
    let mut choices = Choices { bytes: choices };
    let mut model = Model::new(extra);
    let engine = Engine::with_options(
        model.input(),
        EngineOptions {
            // Small enough that trees are evicted and parsed again.
            tree_cache_bytes: 64 << 10,
        },
    );
    model.assign_ids(&engine);
    // Every package name seen, so that entries for packages that have gone are asked for too.
    let mut package_names: BTreeSet<String> = BTreeSet::new();
    compare(
        &engine,
        &model,
        &mut Choices { bytes: &[] },
        &mut package_names,
    )?;
    for step in 0..MAX_EDITS {
        if choices.is_empty() {
            break;
        }
        let before = engine.snapshot();
        let mut batch = None;
        let edit = model.edit(&engine, &mut choices, || {
            if concurrent {
                batch = Some(std::thread::spawn(move || before.run_batch(2, |_| Ok(()))));
                std::thread::yield_now();
            }
        });
        if let Some(batch) = batch {
            // Done or cancelled, either is right; a panic isn't.
            batch
                .join()
                .map_err(|_| format!("a batch panicked during edit {step} ({edit})"))?
                .ok();
        }
        compare(&engine, &model, &mut choices, &mut package_names)
            .map_err(|e| format!("after edit {step} ({edit}): {e}"))?;
    }
    Ok(())
}

/// The most edits one check applies.
pub const MAX_EDITS: usize = 48;

/// Choices made from bytes, until they run out; then every choice is the first option.
struct Choices<'a> {
    bytes: &'a [u8],
}

impl Choices<'_> {
    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// A number below `n`, which must be positive.
    fn below(&mut self, n: usize) -> usize {
        let Some((&first, rest)) = self.bytes.split_first() else {
            return 0;
        };
        self.bytes = rest;
        if n <= 256 {
            first as usize % n
        } else {
            let second = self.below(256);
            (first as usize * 256 + second) % n
        }
    }

    fn pick<'t>(&mut self, options: &[&'t str]) -> &'t str {
        options[self.below(options.len())]
    }

    fn flip(&mut self) -> bool {
        self.below(2) == 1
    }
}

/// The inputs, as the engine should have them: what a fresh engine is built from.
struct Model {
    sets: Vec<SetModel>,
    /// For new files' names.
    next_file: usize,
}

struct SetModel {
    name: &'static str,
    release: u32,
    dependencies: Vec<SourceSetId>,
    files: Vec<FileModel>,
}

struct FileModel {
    /// The incremental engine's id for it.
    id: FileId,
    path: String,
    text: String,
}

const STARTING_FILES: &[(usize, &str, &str)] = &[
    (
        0,
        "p/A.java",
        "package p;\n\nimport java.util.List;\nimport p.q.*;\n\npublic class A<T> extends B implements I {\n    static final int LIMIT = 1;\n    int count;\n\n    A() { count = 0; }\n\n    List<T> items() { return null; }\n\n    class Inner { void m() { int x = 1; } }\n}\n",
    ),
    (
        0,
        "p/B.java",
        "package p;\n\nclass B {\n    void m() {}\n}\n\ninterface I {}\n",
    ),
    (
        0,
        "p/q/C.java",
        "package p.q;\n\nimport static p.A.LIMIT;\n\npublic enum C { X, Y { void z() {} } }\n\nrecord R(int x, String... rest) {}\n",
    ),
    (0, "p/package-info.java", "@Deprecated\npackage p;\n"),
    (
        1,
        "p/ATest.java",
        "package p;\n\nimport p.q.C;\n\nclass ATest extends A<String> {\n    void test() { new B().m(); }\n}\n",
    ),
    (
        1,
        "r/D.java",
        "package r;\n\n@Generated\nclass D { @interface Q { int v() default 1; } }\n",
    ),
];

/// Fragments that edits insert, chosen to make and break declarations.
const FRAGMENTS: &[&str] = &[
    "class A {}",
    "class N { int f; }",
    "interface I {}",
    "enum E { X }",
    "record R(int x) {}",
    "@interface Q {}",
    "int f;",
    "final int c = 1;",
    "void m() {}",
    "A() {}",
    "{",
    "}",
    ";",
    "x = 1;",
    "import p.A;",
    "import p.*;",
    "import static p.A.m;",
    "package p;",
    "package p.q;",
    "extends A",
    "implements I",
    "static",
    "final",
    "@Generated",
    "@Deprecated(\"x\")",
    "<T>",
    "(",
    ")",
    "int",
    "String...",
    "\n// comment\n",
    "/* c */",
];

const NAMES: &[&str] = &[
    "A", "B", "C", "I", "E", "R", "p", "q", "r", "m", "f", "x", "T",
];

const PACKAGES: &[&str] = &["p", "p.q", "r", ""];

const RELEASES: &[u32] = &[8, 17, 21, 27];

impl Model {
    fn new(extra: Option<String>) -> Model {
        let mut sets = vec![
            SetModel {
                name: "main",
                release: 21,
                dependencies: Vec::new(),
                files: Vec::new(),
            },
            SetModel {
                name: "test",
                release: 21,
                dependencies: vec![SourceSetId::from_index(0)],
                files: Vec::new(),
            },
        ];
        for &(set, path, text) in STARTING_FILES {
            sets[set].files.push(FileModel {
                id: FileId::from_index(0),
                path: path.to_owned(),
                text: text.to_owned(),
            });
        }
        if let Some(text) = extra {
            sets[0].files.push(FileModel {
                id: FileId::from_index(0),
                path: "p/Extra.java".to_owned(),
                text,
            });
        }
        Model { sets, next_file: 0 }
    }

    fn input(&self) -> ProjectInput {
        ProjectInput {
            source_sets: self
                .sets
                .iter()
                .map(|set| SourceSetInput {
                    name: set.name.to_owned(),
                    release: set.release,
                    dependencies: set.dependencies.clone(),
                    files: set
                        .files
                        .iter()
                        .map(|f| FileInput::new(&f.path, f.text.as_str()))
                        .collect(),
                })
                .collect(),
        }
    }

    /// Records the ids a fresh engine gave the files.
    fn assign_ids(&mut self, engine: &Engine) {
        let snapshot = engine.snapshot();
        for (i, set) in self.sets.iter_mut().enumerate() {
            let ids = snapshot.files(SourceSetId::from_index(i)).unwrap();
            for (file, &id) in set.files.iter_mut().zip(ids.iter()) {
                file.id = id;
            }
        }
    }

    /// Applies one edit to the model and the engine, running `during` just before the engine's
    /// change, and describes it.
    fn edit(
        &mut self,
        engine: &Engine,
        choices: &mut Choices<'_>,
        during: impl FnOnce(),
    ) -> String {
        let s = choices.below(self.sets.len());
        let set_id = SourceSetId::from_index(s);
        let kind = choices.below(10);
        let set = &mut self.sets[s];
        // Every edit but a release change needs a file.
        if set.files.is_empty() && kind < 9 {
            return self.add_file(engine, s, choices, during);
        }
        let f = choices.below(set.files.len().max(1));
        match kind {
            // Most edits are to text: insert a fragment, delete a token, or rename one.
            0..=2 => {
                let file = &mut set.files[f];
                let starts = token_starts(&file.text);
                let at = starts[choices.below(starts.len())];
                let fragment = choices.pick(FRAGMENTS);
                file.text.insert_str(at, &format!(" {fragment} "));
                let (id, text) = (file.id, file.text.clone());
                during();
                engine.set_text(id, Arc::from(text));
                format!("inserted {fragment:?} in {}", file.path)
            }
            3 => {
                let file = &mut set.files[f];
                let tokens = tokens(&file.text);
                if tokens.is_empty() {
                    return "nothing".to_owned();
                }
                let (start, end) = tokens[choices.below(tokens.len())];
                let removed = file.text[start..end].to_owned();
                file.text.replace_range(start..end, "");
                let (id, text) = (file.id, file.text.clone());
                during();
                engine.set_text(id, Arc::from(text));
                format!("deleted {removed:?} in {}", file.path)
            }
            4 => {
                let file = &mut set.files[f];
                let names: Vec<(usize, usize)> = tokens(&file.text)
                    .into_iter()
                    .filter(|&(s, _)| file.text[s..].starts_with(|c: char| c.is_alphabetic()))
                    .collect();
                if names.is_empty() {
                    return "nothing".to_owned();
                }
                let (start, end) = names[choices.below(names.len())];
                let name = choices.pick(NAMES);
                file.text.replace_range(start..end, name);
                let (id, text) = (file.id, file.text.clone());
                during();
                engine.set_text(id, Arc::from(text));
                format!("renamed to {name} in {}", file.path)
            }
            // A statement added to the first method body, which no summary sees.
            5 => {
                let file = &mut set.files[f];
                let Some(at) = file.text.find(") {").map(|i| i + 3) else {
                    return "nothing".to_owned();
                };
                file.text.insert_str(at, " x = x + 1; ");
                let (id, text) = (file.id, file.text.clone());
                during();
                engine.set_text(id, Arc::from(text));
                format!("edited a body in {}", file.path)
            }
            // The same text again: a new revision that changes nothing.
            6 => {
                let file = &set.files[f];
                during();
                engine.set_text(file.id, Arc::from(file.text.as_str()));
                format!("set the same text in {}", file.path)
            }
            7 => self.add_file(engine, s, choices, during),
            8 => {
                let file = set.files.remove(f);
                during();
                engine.remove_file(file.id);
                format!("removed {}", file.path)
            }
            _ => {
                let release = RELEASES[choices.below(RELEASES.len())];
                set.release = release;
                during();
                engine.set_release(set_id, release);
                format!("set {}'s release to {release}", set.name)
            }
        }
    }

    fn add_file(
        &mut self,
        engine: &Engine,
        s: usize,
        choices: &mut Choices<'_>,
        during: impl FnOnce(),
    ) -> String {
        let package = choices.pick(PACKAGES);
        let name = format!("New{}", self.next_file);
        self.next_file += 1;
        let header = if package.is_empty() {
            String::new()
        } else {
            format!("package {package};\n")
        };
        let body = choices.pick(FRAGMENTS);
        let text = format!("{header}\nclass {name} {{ {body} }}\n");
        let path = if package.is_empty() {
            format!("{name}.java")
        } else {
            format!("{}/{name}.java", package.replace('.', "/"))
        };
        during();
        let id = engine.add_file(
            SourceSetId::from_index(s),
            FileInput::new(&path, text.as_str()),
        );
        self.sets[s].files.push(FileModel {
            id,
            path: path.clone(),
            text,
        });
        format!("added {path}")
    }
}

/// Where each non-trivia token starts, and the text's end: where fragments can go.
fn token_starts(text: &str) -> Vec<usize> {
    let mut starts: Vec<usize> = tokens(text).into_iter().map(|(s, _)| s).collect();
    starts.push(text.len());
    starts
}

/// The non-trivia tokens' byte ranges.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    ferrion_lexer::lex(text)
        .tokens
        .iter()
        .filter(|t| !t.kind.is_trivia() && !t.span.is_empty())
        .map(|t| (t.span.start as usize, t.span.end as usize))
        .collect()
}

/// Asks `engine` the questions `choices` picks (every question when they have run out) and
/// compares each answer with a fresh engine's.
fn compare(
    engine: &Engine,
    model: &Model,
    choices: &mut Choices<'_>,
    package_names: &mut BTreeSet<String>,
) -> Result<(), String> {
    let fresh = Engine::new(model.input());
    let (old, new) = (engine.snapshot(), fresh.snapshot());
    let cancelled = |_: Cancelled| "cancelled with no edit running".to_owned();
    let every = choices.is_empty();
    for (s, set) in model.sets.iter().enumerate() {
        let set_id = SourceSetId::from_index(s);
        let new_files = new.files(set_id).map_err(cancelled)?;
        let old_files = old.files(set_id).map_err(cancelled)?;
        let old_ids: Vec<FileId> = set.files.iter().map(|f| f.id).collect();
        if *old_files != *old_ids {
            return Err(format!(
                "{}'s files are {old_files:?}, not {old_ids:?}",
                set.name
            ));
        }
        let paths = |snapshot: &Snapshot, ids: &[FileId]| -> Vec<String> {
            ids.iter()
                .map(|&id| {
                    let path = snapshot.file(id).unwrap().path;
                    path.to_string_lossy().into_owned()
                })
                .collect()
        };
        for (file, &new_id) in set.files.iter().zip(new_files.iter()) {
            if !every && !choices.flip() {
                continue;
            }
            let memo = old.summary_memo(file.id).map_err(cancelled)?.unwrap();
            if memo.fingerprint != Fingerprint::of(&*memo.value) {
                return Err(format!("{}'s summary has a stale fingerprint", file.path));
            }
            let expected = new.summary(new_id).map_err(cancelled)?;
            let actual = memo.value.summary.debug_dump(old.names());
            let expected = expected.debug_dump(new.names());
            if actual != expected {
                return Err(format!(
                    "{}'s summary differs from a fresh engine's:\n{actual}\nexpected:\n{expected}",
                    file.path,
                ));
            }
        }
        if every || choices.flip() {
            let memo = old.package_index_memo(set_id).map_err(cancelled)?;
            if memo.fingerprint != Fingerprint::of(&*memo.value) {
                return Err(format!(
                    "{}'s package index has a stale fingerprint",
                    set.name
                ));
            }
            let expected = new.package_index(set_id).map_err(cancelled)?;
            let describe = |snapshot: &Snapshot, index: &PackageIndex| -> Vec<String> {
                let mut described: Vec<String> = index
                    .names()
                    .map(|name| {
                        let files = paths(snapshot, index.files(name).unwrap());
                        format!("{:?}: {files:?}", snapshot.names().text(name))
                    })
                    .collect();
                described.sort();
                described
            };
            let (actual, expected) = (describe(&old, &memo.value), describe(&new, &expected));
            if actual != expected {
                return Err(format!(
                    "{}'s package index is {actual:?}, not {expected:?}",
                    set.name
                ));
            }
            package_names.extend(memo.value.names().map(|n| old.names().text(n).to_string()));
        }
        for text in package_names.iter() {
            if !every && !choices.flip() {
                continue;
            }
            let name = old.names().intern(text);
            let memo = old.package_memo(set_id, name).map_err(cancelled)?;
            if memo.fingerprint != Fingerprint::of(&*memo.value) {
                return Err(format!(
                    "{}'s package {text:?} has a stale fingerprint",
                    set.name
                ));
            }
            let describe = |snapshot: &Snapshot, package: Option<&Package>| -> String {
                let Some(package) = package.filter(|p| !p.files().is_empty()) else {
                    return "none".to_owned();
                };
                let mut types: Vec<String> = package
                    .types()
                    .iter()
                    .map(|t| {
                        let name = snapshot.names().text(t.name);
                        format!("{name} in {:?}", paths(snapshot, &t.declarations))
                    })
                    .collect();
                types.sort();
                format!(
                    "files {:?}, types {types:?}",
                    paths(snapshot, package.files())
                )
            };
            let expected = new
                .package(set_id, new.names().intern(text))
                .map_err(cancelled)?;
            let (actual, expected) = (
                describe(&old, Some(&memo.value)),
                describe(&new, expected.as_deref()),
            );
            if actual != expected {
                return Err(format!(
                    "{}'s package {text:?} is {actual}, not {expected}",
                    set.name
                ));
            }
        }
    }
    Ok(())
}
