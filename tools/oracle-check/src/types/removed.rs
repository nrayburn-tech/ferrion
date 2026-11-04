//! `oracle removed`: the removed-symbols check (docs/design/semantic-types.md#verification). For
//! each project, a seeded choice of classpath jars, and separately of single classes, is removed
//! from Ferrion's inputs, and its answers are compared with javac's on the full classpath, from
//! the cached output of `oracle types`. Ferrion must not crash, and every answer that differs from
//! javac's must be a non-answer whose root cause is a removed symbol: a confident answer that
//! differs fails the run. That is the scope's "never a guess", made testable.
//!
//! This reads the cache and never runs javac: a run of javac rewrites generated sources
//! (docs/design/types-oracle-normalization.md#generated-text-that-changes).
//!
//! - **What is chosen from:** the classes javac's answers name, so that what is removed is used; a
//!   jar with one of them in it. A class is removed from whichever of the jars, the source files,
//!   and the platform it is in. A source file is removed whole, unless a processor generated it.
//! - **A root cause:** an `error <name>` whose name has a segment that is the simple name of a
//!   removed class, or of a member of one; or an `unknown incomplete <class>` with a supertype
//!   that doesn't resolve, for a reason of the same kind. Anything else that differs fails.
//! - **Nothing at javac's range:** Ferrion can have no tree of javac's when it can't tell what a
//!   name is, and no constant where it can't fold one: neither is an answer, and each is counted,
//!   for a file that names something removed, and fails otherwise.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_classfile::class::ModuleDescriptor;
use ferrion_classfile::{ClassBytes, ClassName, ClassPathEntry, PackageListing, PackageName};
use ferrion_diagnostics::SourceText;
use ferrion_semantic::types::TypeContext;
use ferrion_semantic::{ClassPathInput, Platform, ProjectInput, Snapshot, SourceSetId};
use serde::Serialize;

use super::attribution;
use super::compare::{Known, Mismatch, compare, fits_text};
use super::oracle::{FORMAT, Output};
use crate::names::display;
use crate::names::ferrion::{Ferrion, Offsets};
use crate::names::oracle::{NamesOracle, OracleCommand};
use crate::names::plan::{self, SourceSet};

pub const USAGE: &str = "\
usage: oracle removed [<option>...] [<project>...]

For each project (by default all of them), removes a seeded choice of classpath jars, and separately
of single classes, from Ferrion's inputs, and compares its answers with javac's on the full
classpath, from the cached output of `oracle types`, which must have run. Fails on a crash, or on an
answer that differs from javac's and isn't a non-answer whose root cause is a removed symbol.

options:
  --seed <N>          the seed of the first round (default: 1)
  --rounds <N>        how many choices of each kind (default: 2)
  --jars <N>          jars per round: at most this many (default: 3)
  --classes <N>       single classes per round (default: 20)
  --class <name>      remove this class too, in a round of its own (repeatable): java/lang/String
  --threads <N>       attribute N files of a source set at a time (default: 4)
  --limit <N>         show the first N failures (default: 10)
  --json <file>       also write the report as JSON
  --oracle-jar <file> the javac oracle whose cached output to read (default:
                      tools/javac-oracle/target/javac-oracle.jar)";

pub struct Options {
    projects: Vec<String>,
    seed: u64,
    rounds: u64,
    jars: usize,
    classes: usize,
    named: Vec<String>,
    threads: usize,
    limit: usize,
    json: Option<PathBuf>,
    jar: PathBuf,
}

pub fn parse_args(args: impl Iterator<Item = String>, usage: fn(&str) -> !) -> Options {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut options = Options {
        projects: Vec::new(),
        seed: 1,
        rounds: 2,
        jars: 3,
        classes: 20,
        named: Vec::new(),
        threads: 4,
        limit: 10,
        json: None,
        jar: repository.join("tools/javac-oracle/target/javac-oracle.jar"),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .unwrap_or_else(|| usage(&format!("{name} needs a value")))
        };
        let number = |name: &str, text: String| -> u64 {
            text.parse()
                .unwrap_or_else(|_| usage(&format!("{name}: `{text}` isn't a number")))
        };
        match arg.as_str() {
            "--seed" => {
                let text = value("--seed");
                options.seed = number("--seed", text);
            }
            "--rounds" => {
                let text = value("--rounds");
                options.rounds = number("--rounds", text);
            }
            "--jars" => {
                let text = value("--jars");
                options.jars = number("--jars", text) as usize;
            }
            "--classes" => {
                let text = value("--classes");
                options.classes = number("--classes", text) as usize;
            }
            "--class" => options.named.push(value("--class")),
            "--threads" => {
                let text = value("--threads");
                options.threads = (number("--threads", text) as usize).max(1);
            }
            "--limit" => {
                let text = value("--limit");
                options.limit = number("--limit", text) as usize;
            }
            "--json" => options.json = Some(PathBuf::from(value("--json"))),
            "--oracle-jar" => options.jar = PathBuf::from(value("--oracle-jar")),
            _ if arg.starts_with('-') => usage(&format!("unknown option `{arg}`")),
            _ => options.projects.push(arg),
        }
    }
    options
}

/// FNV-1a of a seed and a name: the order a seeded choice takes.
fn order_of(seed: u64, name: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The `count` of `names` first in `seed`'s order.
fn choose(names: impl IntoIterator<Item = String>, seed: u64, count: usize) -> Vec<String> {
    let mut names: Vec<(u64, String)> =
        names.into_iter().map(|n| (order_of(seed, &n), n)).collect();
    names.sort();
    names.truncate(count);
    names.into_iter().map(|(_, name)| name).collect()
}

/// The words of a type string that could be classes: `java/util/List`, `E:p/A`, `(p/B)void`.
fn class_words(type_string: &str, into: &mut BTreeSet<String>) {
    for word in type_string.split(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '$' | '_')))
    {
        if word.contains('/') && !word.ends_with('/') && !word.starts_with('/') {
            into.insert(word.to_owned());
        }
    }
}

/// A platform without some of its classes.
#[derive(Debug)]
struct Hiding {
    inner: Arc<dyn Platform>,
    hidden: Arc<HashSet<String>>,
}

impl Platform for Hiding {
    fn find(&self, name: &ClassName) -> ferrion_classfile::Result<Option<ClassBytes>> {
        if self.hidden.contains(name.as_str()) {
            return Ok(None);
        }
        self.inner.find(name)
    }

    fn package(&self, name: &PackageName) -> ferrion_classfile::Result<Option<PackageListing>> {
        let listing = self.inner.package(name)?;
        Ok(listing.map(|listing| without(listing, name.as_str(), &self.hidden)))
    }

    fn modules(&self) -> &[ModuleDescriptor] {
        self.inner.modules()
    }

    fn is_visible(&self, package: &PackageName) -> bool {
        self.inner.is_visible(package)
    }

    fn module_of(&self, name: &ClassName) -> Option<String> {
        self.inner.module_of(name)
    }

    fn patched_from(&self, name: &ClassName) -> Option<PathBuf> {
        self.inner.patched_from(name)
    }
}

fn without(listing: PackageListing, package: &str, hidden: &HashSet<String>) -> PackageListing {
    let names = listing
        .names()
        .filter(|simple| !hidden.contains(&format!("{package}/{simple}")))
        .map(Into::into)
        .collect();
    PackageListing::new(names)
}

/// One choice of what to remove.
#[derive(Clone, Debug, Default, Serialize)]
struct Removal {
    kind: &'static str,
    seed: u64,
    /// The jars or classes chosen.
    chosen: Vec<String>,
    /// Every class that no longer resolves because of it, by binary name.
    #[serde(skip)]
    classes: Arc<HashSet<String>>,
    /// The names an error's root cause may have: the simple names of those classes, and of their
    /// members.
    #[serde(skip)]
    names: Arc<HashSet<String>>,
    #[serde(skip)]
    jars: Arc<HashSet<PathBuf>>,
    #[serde(skip)]
    files: Arc<HashSet<PathBuf>>,
}

impl Removal {
    /// What this removal does to a project's inputs.
    fn apply(&self, input: &mut ProjectInput) {
        for set in &mut input.source_sets {
            set.classpath.retain(|entry| match entry {
                ClassPathInput::Classes(classes) => !self.jars.contains(&classes.path),
                ClassPathInput::SourceSet(_) => true,
            });
            if !self.classes.is_empty() && self.jars.is_empty() {
                for entry in &mut set.classpath {
                    if let ClassPathInput::Classes(classes) = entry {
                        classes.entry = classes.entry.clone().hiding(self.classes.clone());
                    }
                }
                if let Some(platform) = set.platform.take() {
                    set.platform = Some(Arc::new(Hiding {
                        inner: platform,
                        hidden: self.classes.clone(),
                    }));
                }
            }
            set.files.retain(|file| !self.files.contains(&file.path));
        }
    }
}

/// What a round found.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    project: String,
    removal: Removal,
    /// Answers the same as javac's, and entries Ferrion leaves to a later milestone.
    same: u64,
    left: u64,
    /// Non-answers where javac has an answer, rooted in something removed by name.
    rooted: BTreeMap<&'static str, u64>,
    /// Those whose error has another name, of a member, say: in a file that names something
    /// removed.
    rooted_in_file: BTreeMap<&'static str, u64>,
    /// javac's answer, but for a descriptor's removed class, which Ferrion writes as `Object`: in
    /// a method's key, or a type variable's owner.
    same_method: u64,
    /// Nothing at javac's range, in a file that names something removed.
    nothing: BTreeMap<&'static str, u64>,
    /// Entries of Ferrion's where javac has none, non-answers rooted in something removed.
    extra_rooted: u64,
    /// A few of those by the file and of nothing at javac's range, for looking at.
    samples: Vec<String>,
    /// Failures: a confident answer that differs, a non-answer with another cause, a crash.
    failures: Vec<String>,
    failure_count: u64,
    confident: u64,
    unrooted: u64,
    panics: u64,
    /// Source files left out, since they were removed.
    files_removed: u64,
    seconds: f64,
}

/// Whether a value is a non-answer, or has one in it.
fn non_answer(value: &str) -> bool {
    value.contains("error ") || value.contains("unknown ")
}

/// The names of what a value's non-answers come from, each `error <name>` and `unknown error
/// <name>`, and the classes of each `unknown incomplete <class>`.
fn roots(value: &str) -> (Vec<&str>, Vec<&str>) {
    let word_after = |rest: &'_ str| -> usize {
        rest.find([',', '>', '<', ')', '(', '[', '&', '|', ' '])
            .unwrap_or(rest.len())
    };
    let (mut errors, mut incomplete) = (Vec::new(), Vec::new());
    let mut rest = value;
    while let Some(at) = rest.find("error ") {
        let after = &rest[at + "error ".len()..];
        let end = word_after(after);
        errors.push(&after[..end]);
        rest = &after[end..];
    }
    let mut rest = value;
    while let Some(at) = rest.find("unknown incomplete ") {
        let after = &rest[at + "unknown incomplete ".len()..];
        let end = word_after(after);
        incomplete.push(&after[..end]);
        rest = &after[end..];
    }
    (errors, incomplete)
}

/// Whether an error's name could be something removed: one of its segments is the simple name of a
/// removed class or a member of one.
fn removed_name(name: &str, names: &HashSet<String>) -> bool {
    name.split(['.', '$', '/'])
        .any(|segment| names.contains(segment))
}

/// Answers whether a value is rooted in something removed.
struct Roots<'s> {
    cx: TypeContext<'s>,
    snapshot: &'s Snapshot,
    names: Arc<HashSet<String>>,
    incomplete: HashMap<String, bool>,
}

impl Roots<'_> {
    fn rooted(&mut self, value: &str) -> bool {
        let (errors, incomplete) = roots(value);
        errors.iter().any(|name| removed_name(name, &self.names))
            || incomplete.iter().any(|class| self.incomplete(class))
    }

    /// Whether a class's hierarchy has a supertype that doesn't resolve for a removed name.
    fn incomplete(&mut self, class: &str) -> bool {
        if let Some(&known) = self.incomplete.get(class) {
            return known;
        }
        // A local class's key isn't a binary name: its hierarchy is the file's, and the file
        // names what it inherits from.
        let found = class.starts_with("local") || self.walk(class).unwrap_or(false);
        self.incomplete.insert(class.to_owned(), found);
        found
    }

    fn walk(&mut self, class: &str) -> Result<bool, ferrion_semantic::Cancelled> {
        let Some(key) = self.cx.lang(class)? else {
            return Ok(removed_name(class, &self.names));
        };
        let mut pending = vec![key];
        let mut seen = HashSet::new();
        while let Some(key) = pending.pop() {
            if !seen.insert(key) {
                continue;
            }
            let Some(supers) = self.snapshot.supertypes(key)? else {
                continue;
            };
            let all = supers.superclass.iter().chain(supers.interfaces.iter());
            for supertype in all {
                match supertype {
                    // A supertype named that can't be read: every class's `Object`, say.
                    Ok(key) if self.cx.kind(*key)?.is_none() => return Ok(true),
                    Ok(key) => pending.push(*key),
                    Err(error) => {
                        let name = self.snapshot.names().text(error.name);
                        if removed_name(&name, &self.names) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}

pub fn run(options: Options) -> Result<bool> {
    let start = Instant::now();
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let selected = corpus::select(&projects, &options.projects)?;
    let cache_root = match std::env::var_os("FERRION_ORACLE_CACHE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle"),
    };
    let scratch = cache_root.join("scratch");
    let known = Known::load()?;
    let mut rows = Vec::new();
    for project in selected {
        rows.extend(run_project(
            project,
            &dirs,
            &cache_root,
            &scratch,
            &known,
            &options,
        )?);
    }
    print!("{}", text(&rows, options.limit));
    println!("{:.1} s in all", start.elapsed().as_secs_f64());
    if let Some(path) = &options.json {
        std::fs::write(path, serde_json::to_string_pretty(&rows)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(rows.iter().all(|r| r.failure_count == 0))
}

fn run_project(
    project: &Project,
    dirs: &Dirs,
    cache_root: &Path,
    scratch: &Path,
    known: &Known,
    options: &Options,
) -> Result<Vec<Row>> {
    let sets = plan::source_sets(dirs, project)?;
    let jdk = corpus::model::lock_jdk(dirs, project)?;
    let home = corpus::jdks::find(jdk)
        .with_context(|| format!("{}'s lock names JDK {jdk}", project.name))?;
    let order: Vec<usize> = plan::order(&sets)
        .into_iter()
        .filter(|&i| !sets[i].files.is_empty())
        .collect();
    let jdks = BTreeMap::from([(jdk, home.clone())]);
    let oracle = NamesOracle::for_command(
        OracleCommand {
            name: "types",
            format: FORMAT,
            args: vec!["--relation-types".to_owned(), "48".to_owned()],
        },
        options.jar.clone(),
        "4g".to_owned(),
        Vec::new(),
        &jdks,
    )?;
    let cached = |set: &SourceSet| -> Result<Output> {
        oracle
            .cached::<Output>(&sets, set, &dirs.corpus, cache_root)?
            .with_context(|| {
                format!(
                    "{}: no cached output of `oracle types`, which this check reads and doesn't run",
                    set.display()
                )
            })
    };

    // What javac's answers name, and what the project's inputs have of them.
    let mut used = BTreeSet::new();
    for &i in &order {
        for file in cached(&sets[i])?.files {
            for ty in &file.types {
                class_words(ty, &mut used);
            }
        }
    }
    let project_jdks = HashMap::from([(project.name.clone(), home)]);
    let full = Ferrion::new(
        &[(project.name.clone(), usize::MAX)],
        &project_jdks,
        scratch.to_path_buf(),
        options.threads,
        false,
    )?;
    let input = full.input(&project.name).map_err(|e| anyhow!(e))?;
    let origins = Origins::of(&input, &used);

    let mut removals = Vec::new();
    for round in 0..options.rounds {
        let seed = options.seed + round;
        let jars = choose(origins.jars.keys().cloned(), seed, options.jars);
        if !jars.is_empty() {
            removals.push(origins.of_jars(jars, seed));
        }
        let classes = choose(origins.classes(), seed, options.classes);
        if !classes.is_empty() {
            removals.push(origins.of_classes(classes, seed));
        }
    }
    for class in &options.named {
        removals.push(origins.of_classes(vec![class.clone()], 0));
    }

    let mut rows = Vec::new();
    for removal in removals {
        let begin = Instant::now();
        let mut row = Row {
            project: project.name.clone(),
            removal: removal.clone(),
            ..Row::default()
        };
        let edit = removal.clone();
        let ferrion = Ferrion::new(
            &[(project.name.clone(), order.len())],
            &project_jdks,
            scratch.to_path_buf(),
            options.threads,
            false,
        )?
        .with_edit(Arc::new(move |input| edit.apply(input)));
        for &i in &order {
            let set = &sets[i];
            let output = cached(set)?;
            let done = catch_unwind(AssertUnwindSafe(|| {
                ferrion.with_set(set, |engine, id, _| {
                    compare_set(
                        engine.snapshot(),
                        id,
                        &output,
                        set,
                        &dirs.corpus,
                        known,
                        &removal,
                        options,
                        &mut row,
                    )
                })
            }));
            match done {
                Ok(Ok(())) => {}
                Ok(Err(message)) => bail!("{}: {message}", set.display()),
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "a panic".to_owned());
                    row.panics += 1;
                    row.failure_count += 1;
                    row.failures
                        .push(format!("panic in {}: {message}", set.display()));
                }
            }
        }
        row.seconds = begin.elapsed().as_secs_f64();
        eprintln!(
            "{}: without {} ({}): {} same, {} failures, {:.1} s",
            project.name,
            removal.kind,
            removal.chosen.len(),
            row.same,
            row.failure_count,
            row.seconds
        );
        rows.push(row);
    }
    Ok(rows)
}

/// What the project's inputs have of the classes javac's answers name.
struct Origins {
    /// Each jar with some of them: its path, and those classes.
    jars: BTreeMap<String, ClassPathEntry>,
    /// Those that are on a classpath or the platform.
    classes: BTreeSet<String>,
    /// By a top-level class's binary name, the source files that declare it: not generated.
    sources: HashMap<String, Vec<PathBuf>>,
}

impl Origins {
    fn of(input: &ProjectInput, used: &BTreeSet<String>) -> Origins {
        let mut origins = Origins {
            jars: BTreeMap::new(),
            classes: BTreeSet::new(),
            sources: HashMap::new(),
        };
        let wanted: HashSet<&str> = used.iter().map(String::as_str).collect();
        let mut platforms = HashSet::new();
        for set in &input.source_sets {
            for entry in &set.classpath {
                let ClassPathInput::Classes(classes) = entry else {
                    continue;
                };
                let key = classes.path.display().to_string();
                if origins.jars.contains_key(&key) {
                    continue;
                }
                let mut holds_some = false;
                for class in &wanted {
                    let found = classes.entry.find(&ClassName::new(class));
                    if found.ok().flatten().is_some() {
                        origins.classes.insert((*class).to_owned());
                        holds_some = true;
                    }
                }
                if holds_some {
                    origins.jars.insert(key, classes.entry.clone());
                }
            }
            if let Some(platform) = &set.platform {
                let id = Arc::as_ptr(platform).cast::<()>();
                if platforms.insert(id) {
                    for class in &wanted {
                        let found = platform.find(&ClassName::new(class));
                        if found.ok().flatten().is_some() {
                            origins.classes.insert((*class).to_owned());
                        }
                    }
                }
            }
            for file in &set.files {
                if file.generated {
                    continue;
                }
                let text = file.path.to_string_lossy();
                let Some(stem) = text.strip_suffix(".java") else {
                    continue;
                };
                // Every suffix of the path that could be the class's binary name.
                let mut at = 0;
                while let Some(slash) = stem[at..].find('/') {
                    at += slash + 1;
                    let candidate = &stem[at..];
                    if wanted.contains(candidate) {
                        origins
                            .sources
                            .entry(candidate.to_owned())
                            .or_default()
                            .push(file.path.clone());
                    }
                }
            }
        }
        for class in origins.sources.keys() {
            origins.classes.insert(class.clone());
        }
        origins
    }

    fn classes(&self) -> impl Iterator<Item = String> + '_ {
        self.classes.iter().cloned()
    }

    fn of_jars(&self, jars: Vec<String>, seed: u64) -> Removal {
        let mut classes = HashSet::new();
        let mut names = HashSet::new();
        for jar in &jars {
            let Some(entry) = self.jars.get(jar).and_then(ClassPathEntry::as_jar) else {
                continue;
            };
            for (name, _) in entry.entries() {
                if let Some(class) = name.strip_suffix(".class") {
                    add_class(class, &mut classes, &mut names);
                }
            }
        }
        Removal {
            kind: "jars",
            seed,
            jars: Arc::new(jars.iter().map(PathBuf::from).collect()),
            chosen: jars,
            classes: Arc::new(classes),
            names: Arc::new(names),
            files: Arc::new(HashSet::new()),
        }
    }

    fn of_classes(&self, chosen: Vec<String>, seed: u64) -> Removal {
        let mut classes = HashSet::new();
        let mut names = HashSet::new();
        let mut files = HashSet::new();
        for class in &chosen {
            add_class(class, &mut classes, &mut names);
            // Every file imports `java.lang` without naming it.
            if class
                .rsplit_once('/')
                .is_some_and(|(p, _)| p == "java/lang")
            {
                names.insert(ANY_MEMBER.to_owned());
            }
            let top = class.split('$').next().unwrap_or(class);
            if let Some(paths) = self.sources.get(top) {
                files.extend(paths.iter().cloned());
                // A file removed takes everything in it: its members' names come back as errors,
                // which its classes' don't list.
                names.insert(ANY_MEMBER.to_owned());
            }
        }
        Removal {
            kind: "classes",
            seed,
            chosen,
            classes: Arc::new(classes),
            names: Arc::new(names),
            jars: Arc::new(HashSet::new()),
            files: Arc::new(files),
        }
    }
}

/// In a removal's names: a source file went, whose members aren't listed.
const ANY_MEMBER: &str = "*";

/// A class that no longer resolves, and the names that may come of it: its simple names.
fn add_class(class: &str, classes: &mut HashSet<String>, names: &mut HashSet<String>) {
    classes.insert(class.to_owned());
    names.extend(
        class
            .rsplit('/')
            .next()
            .unwrap_or(class)
            .split('$')
            .map(str::to_owned),
    );
}

/// Compares one source set's files with javac's, counting into `row`.
#[allow(clippy::too_many_arguments)]
fn compare_set(
    snapshot: Snapshot,
    view: SourceSetId,
    output: &Output,
    set: &SourceSet,
    corpus: &Path,
    known: &Known,
    removal: &Removal,
    options: &Options,
    row: &mut Row,
) -> Result<(), String> {
    let cancelled = |_| "cancelled with no edit running".to_owned();
    let files =
        attribution::set_files(&snapshot, view, corpus, options.threads).map_err(cancelled)?;
    let mut roots = Roots {
        cx: snapshot.types(view),
        snapshot: &snapshot,
        names: removal.names.clone(),
        incomplete: HashMap::new(),
    };
    for file in &output.files {
        let generated = file.generated == 1 || set.is_generated(&corpus.join(&file.file));
        if file.skipped.is_some()
            || file.crash.is_some()
            || file.errors().next().is_some()
            || file.unanalyzed == 1
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(corpus.join(&file.file)) else {
            continue;
        };
        let Ok(source) = SourceText::decode(bytes) else {
            continue;
        };
        if generated && !fits_text(file, source.text()) {
            continue;
        }
        let Some(ours) = files.get(&file.file) else {
            row.files_removed += 1;
            continue;
        };
        let name = display(&file.file);
        let compared = compare(file, ours, &Offsets::new(&source));
        row.same += compared.matched;
        row.left += compared.left;
        let removed_here = names_something_removed(source.text(), &removal.names);
        for mismatch in compared.mismatches {
            if known.category(&name, &mismatch).is_some() {
                continue;
            }
            judge(&name, mismatch, removed_here, &mut roots, row);
        }
        for extra in compared.extras {
            if non_answer(&extra.ferrion) && (roots.rooted(&extra.ferrion) || removed_here) {
                row.extra_rooted += 1;
                continue;
            }
            row.failure_count += 1;
            if extra.ferrion.contains("unknown ") || extra.ferrion.contains("error ") {
                row.unrooted += 1;
            } else {
                row.confident += 1;
            }
            if row.failures.len() < options.limit.max(50) {
                row.failures.push(format!(
                    "{name} {} {}-{}: javac nothing, Ferrion {}",
                    extra.aspect.name(),
                    extra.start,
                    extra.end,
                    extra.ferrion
                ));
            }
        }
    }
    Ok(())
}

/// Whether a file's text has a word that names something removed.
fn names_something_removed(text: &str, names: &HashSet<String>) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .any(|word| names.contains(word))
}

fn judge(name: &str, mismatch: Mismatch, removed_here: bool, roots: &mut Roots<'_>, row: &mut Row) {
    let aspect = mismatch.aspect.name();
    let fail = |row: &mut Row, kind: &str| {
        row.failure_count += 1;
        if row.failures.len() < 200 {
            row.failures.push(format!(
                "{kind} {name} {aspect} {}-{} ({}, {}): javac {}, Ferrion {}",
                mismatch.start,
                mismatch.end,
                mismatch.kind,
                mismatch.category.name(),
                mismatch.javac,
                mismatch.ferrion.as_deref().unwrap_or("nothing"),
            ));
        }
    };
    let sample = |row: &mut Row| {
        let every = row.nothing.values().sum::<u64>() + row.rooted_in_file.values().sum::<u64>();
        if every.is_multiple_of(37) && row.samples.len() < 40 {
            row.samples.push(format!(
                "{name} {aspect} {}-{} ({}): javac {}, Ferrion {}",
                mismatch.start,
                mismatch.end,
                mismatch.kind,
                mismatch.javac,
                mismatch.ferrion.as_deref().unwrap_or("nothing"),
            ));
        }
    };
    match &mismatch.ferrion {
        None => {
            if removed_here || roots.names.contains(ANY_MEMBER) {
                sample(row);
                *row.nothing.entry(aspect).or_default() += 1;
            } else {
                row.unrooted += 1;
                fail(row, "NOTHING");
            }
        }
        Some(value) if non_answer(value) => {
            if roots.rooted(value) {
                *row.rooted.entry(aspect).or_default() += 1;
            } else if removed_here || roots.names.contains(ANY_MEMBER) {
                sample(row);
                *row.rooted_in_file.entry(aspect).or_default() += 1;
            } else {
                row.unrooted += 1;
                fail(row, "UNROOTED");
            }
        }
        Some(value) if same_but_descriptors(&mismatch.javac, value, &roots.names) => {
            row.same_method += 1;
        }
        // javac's own type has an error in it, one of its compilation lacks.
        Some(_) if mismatch.javac.contains("error ") => {}
        Some(_) => {
            row.confident += 1;
            fail(row, "CONFIDENT");
        }
    }
}

/// Whether two values say the same, but for classes in a descriptor that Ferrion writes as `Object`
/// since they don't resolve, in a method's key or a type variable's owner. For a method's key, the
/// class must be one removed, since an overload of `Object` would be a guess; in a variable's
/// owner, the variable is the same whichever: a module whose jar went takes its neighbors with it.
fn same_but_descriptors(javac: &str, ferrion: &str, names: &HashSet<String>) -> bool {
    const OBJECT: &str = "Ljava/lang/Object;";
    let method = javac.starts_with("method ");
    let (mut theirs, mut ours) = (javac, ferrion);
    loop {
        let common = theirs
            .bytes()
            .zip(ours.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if common == theirs.len() && common == ours.len() {
            return true;
        }
        // The `L` that starts the class they differ in: here, or back in what they share.
        let here = theirs[common..].starts_with('L');
        let found = if here {
            Some(common)
        } else {
            theirs[..common].rfind('L')
        };
        let Some(at) = found else {
            return false;
        };
        // `Object` itself removed, there's nothing to erase to.
        let after_ours = ours[at..]
            .strip_prefix(OBJECT)
            .or_else(|| ours[at..].strip_prefix('?'));
        let class = &theirs[at + 1..];
        let (Some(end), Some(after_ours)) = (class.find(';'), after_ours) else {
            return false;
        };
        let class = &class[..end];
        if !class.contains('/') || (method && !removed_name(class, names)) {
            return false;
        }
        theirs = &theirs[at + 1 + end + 1..];
        ours = after_ours;
    }
}

fn text(rows: &[Row], limit: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::from(
        "\n| Project | Removed | Seed | Chosen | Same | Same, a removed class in a descriptor | Left to later | Non-answers rooted by name | By the file | Nothing at javac's range | Extra, rooted | Files removed | Confident | Unrooted | Panics | Time |\n\
         | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for row in rows {
        let rooted: u64 = row.rooted.values().sum();
        let by_file: u64 = row.rooted_in_file.values().sum();
        let nothing: u64 = row.nothing.values().sum();
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {rooted} | {by_file} | {nothing} | {} | {} | {} | {} | {} | {:.1} s |",
            row.project,
            row.removal.kind,
            row.removal.seed,
            row.removal.chosen.len(),
            row.same,
            row.same_method,
            row.left,
            row.extra_rooted,
            row.files_removed,
            row.confident,
            row.unrooted,
            row.panics,
            row.seconds,
        );
    }
    for row in rows {
        for failure in row.failures.iter().take(limit) {
            let _ = writeln!(
                out,
                "\nFAIL {} ({}): {failure}",
                row.project, row.removal.kind
            );
        }
    }
    let failures: u64 = rows.iter().map(|r| r.failure_count).sum();
    let _ = writeln!(out, "\n{failures} failures");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> HashSet<String> {
        list.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn choices_repeat_and_differ_by_seed() {
        let all: Vec<String> = (0..50).map(|i| format!("p/C{i}")).collect();
        let one = choose(all.clone(), 1, 5);
        assert_eq!(one, choose(all.clone(), 1, 5));
        assert_eq!(one.len(), 5);
        assert_ne!(one, choose(all, 2, 5));
    }

    #[test]
    fn classes_in_type_strings() {
        let mut found = BTreeSet::new();
        class_words(
            "(java/util/List<E:p/A$B>,int)capture#1 extends q/C[]",
            &mut found,
        );
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            ["java/util/List", "p/A$B", "q/C"]
        );
    }

    #[test]
    fn root_causes() {
        let (errors, incomplete) = roots("java/util/List<error Foo.Bar>");
        assert_eq!((errors, incomplete), (vec!["Foo.Bar"], vec![]));
        let (errors, incomplete) = roots("unknown incomplete p/A");
        assert_eq!((errors, incomplete), (vec![], vec!["p/A"]));
        assert!(removed_name("Foo.Bar", &names(&["Bar"])));
        assert!(!removed_name("Foobar", &names(&["Bar"])));
    }

    #[test]
    fn descriptors_that_erase_a_removed_class() {
        let removed = names(&["Gone"]);
        let same = |a, b| same_but_descriptors(a, b, &removed);
        // The method javac chose, a removed class written as `Object`.
        assert!(same(
            "method p/A.m(Lq/Gone;I)Lq/Gone;",
            "method p/A.m(Ljava/lang/Object;I)Ljava/lang/Object;"
        ));
        assert!(same(
            "method p/A.m([Lq/Gone;)V",
            "method p/A.m([Ljava/lang/Object;)V"
        ));
        // A class that isn't removed, in a method's key, is another overload: a guess.
        assert!(!same(
            "method p/A.m(Lq/Other;)V",
            "method p/A.m(Ljava/lang/Object;)V"
        ));
        assert!(!same(
            "method p/A.n(Lq/Gone;)V",
            "method p/A.m(Ljava/lang/Object;)V"
        ));
        // In a type variable's owner, the variable is the same whichever class it is.
        assert!(same("T:p/A.m(Lq/Other;)V", "T:p/A.m(Ljava/lang/Object;)V"));
        assert!(!same("java/util/List", "java/lang/Object"));
        assert!(same("method p/A.m(Lq/Gone;)V", "method p/A.m(?)V"));
    }
}
