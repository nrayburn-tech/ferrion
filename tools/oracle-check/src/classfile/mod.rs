//! `oracle classfile`: compares Ferrion's class file reading with the JDK's own tools
//! (docs/design/classfile.md, "Verification").
//!
//! For each JDK named with `--jdk`, and for the corpus projects' dependency jars:
//!
//! - **javap:** every class Ferrion serves (each release's platform API, each jar at the
//!   releases of the source sets that use it) is rendered from Ferrion's model and compared, line
//!   for line, with `javap -v -p` on the same bytes. Each distinct class is compared once.
//! - **containers:** the bytes Ferrion reads from an image must equal what the jrt file system
//!   returns, and from zip files (`ct.sym`, JDK 8's jars, the corpus jars) what `ZipFile`
//!   returns; a jar resolved for a release must have the names and bytes of `JarFile`'s
//!   versioned view.
//! - **visibility:** for each release a JDK 11 or later serves, javac's modules, classes, and
//!   packages visible from the unnamed module, through the javac API on that JDK.

pub mod cases;
mod harness;
pub mod render;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::lock::ClasspathEntry;
use corpus::project::Project;
use ferrion_classfile::class::{Attribute, ClassFile};
use ferrion_classfile::{ArchiveCache, ClassName, Jdk, PackageName, PlatformApi};
use serde::Serialize;

use self::harness::{Harness, JavapOutput, sha256};

pub const USAGE: &str = "\
usage: oracle classfile [--jdk <home>]... [<option>...] [<project>...]

Compares Ferrion's class file reading with javap, the jrt file system, java.util.zip, and
javac: every release each JDK serves, and the dependency jars of the corpus projects (by default
all of them; none when only --jdk is given, unless projects are named).

options:
  --jdk <home>         check this JDK (repeatable)
  --release <N>        only these releases of each JDK (repeatable)
  --no-javap           skip the javap comparison
  --jobs <N>           run up to N javap JVMs at once (default: 2)
  --limit <N>          show the first N failures (default: 20)
  --json <file>        also write the full report as JSON
  --javap-jar <file>   the oracle (default: tools/javap-oracle/target/javap-oracle.jar)

JAVA_HOME must point to the newest JDK, which runs javap for every input. The visibility check
runs on each JDK's own java. javap's output is cached in target/oracle/javap, or under
FERRION_ORACLE_CACHE if it is set.";

pub struct Options {
    pub jdks: Vec<PathBuf>,
    pub releases: Vec<u32>,
    pub projects: Vec<String>,
    pub javap: bool,
    pub jobs: usize,
    pub limit: usize,
    pub json: Option<PathBuf>,
    pub jar: PathBuf,
}

pub fn parse_args(args: impl Iterator<Item = String>, usage: fn(&str) -> !) -> Options {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut options = Options {
        jdks: Vec::new(),
        releases: Vec::new(),
        projects: Vec::new(),
        javap: true,
        jobs: 2,
        limit: 20,
        json: None,
        jar: repository.join("tools/javap-oracle/target/javap-oracle.jar"),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .unwrap_or_else(|| usage(&format!("{name} needs a value")))
        };
        let number = |name: &str, text: String| -> usize {
            text.parse()
                .unwrap_or_else(|_| usage(&format!("{name}: `{text}` isn't a number")))
        };
        match arg.as_str() {
            "--jdk" => options.jdks.push(PathBuf::from(value("--jdk"))),
            "--release" => {
                let text = value("--release");
                options.releases.push(number("--release", text) as u32);
            }
            "--no-javap" => options.javap = false,
            "--jobs" => {
                let text = value("--jobs");
                options.jobs = number("--jobs", text).max(1);
            }
            "--limit" => {
                let text = value("--limit");
                options.limit = number("--limit", text);
            }
            "--json" => options.json = Some(PathBuf::from(value("--json"))),
            "--javap-jar" => options.jar = PathBuf::from(value("--javap-jar")),
            _ if arg.starts_with('-') => usage(&format!("unknown option `{arg}`")),
            _ => options.projects.push(arg),
        }
    }
    options
}

/// The outcome of one check on one input.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub input: String,
    pub check: &'static str,
    /// Items compared: classes, files, or packages.
    pub compared: u64,
    /// Classes javap had already been compared on (identical bytes).
    pub duplicates: u64,
    pub failures: Vec<String>,
    /// Differences that are explained, and other things worth knowing.
    pub notes: Vec<String>,
    pub seconds: f64,
}

impl CheckResult {
    fn new(input: &str, check: &'static str) -> CheckResult {
        CheckResult {
            input: input.to_owned(),
            check,
            ..CheckResult::default()
        }
    }
}

#[derive(Serialize)]
struct Report {
    results: Vec<CheckResult>,
    failures: u64,
}

pub fn run(options: Options) -> Result<bool> {
    let harness = Harness::new(options.jar.clone())?;
    let cache_root = match std::env::var_os("FERRION_ORACLE_CACHE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle"),
    };
    let scratch = cache_root.join("scratch");
    std::fs::create_dir_all(&scratch)?;
    let javap_cache = cache_root.join("javap").join(&harness.key);
    let checker = Checker {
        harness: &harness,
        scratch: &scratch,
        cache: &javap_cache,
        jobs: options.jobs,
        javap: options.javap,
        seen: Mutex::new(HashSet::new()),
    };
    let mut results = Vec::new();
    for home in &options.jdks {
        results.extend(checker.jdk(home, &options.releases)?);
    }
    if options.jdks.is_empty() || !options.projects.is_empty() {
        results.extend(checker.corpus(&options.projects)?);
    }
    let failures: u64 = results.iter().map(|r| r.failures.len() as u64).sum();
    print_report(&results, options.limit);
    if let Some(path) = &options.json {
        let report = Report { results, failures };
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }
    if failures > 0 {
        println!("{failures} failures");
    }
    Ok(failures == 0)
}

fn print_report(results: &[CheckResult], limit: usize) {
    for result in results {
        println!(
            "{:<60} {:<10} {:>7} compared {:>7} duplicates {:>4} failures {:>4} notes {:>7.1}s",
            result.input,
            result.check,
            result.compared,
            result.duplicates,
            result.failures.len(),
            result.notes.len(),
            result.seconds
        );
    }
    let mut shown = 0;
    for result in results {
        for failure in &result.failures {
            if shown == limit {
                break;
            }
            println!("\nFAIL {} {}: {failure}", result.input, result.check);
            shown += 1;
        }
    }
    let mut shown = 0;
    for result in results {
        for note in &result.notes {
            if shown == limit {
                break;
            }
            println!("\nnote {} {}: {note}", result.input, result.check);
            shown += 1;
        }
    }
}

struct Checker<'a> {
    harness: &'a Harness,
    scratch: &'a Path,
    cache: &'a Path,
    jobs: usize,
    javap: bool,
    /// The SHA-256 of every class compared with javap in this run.
    seen: Mutex<HashSet<String>>,
}

/// A class to compare: a label for messages, and its bytes or Ferrion's error reading them.
type Class = (String, ferrion_classfile::Result<Arc<[u8]>>);

impl Checker<'_> {
    fn jdk(&self, home: &Path, only: &[u32]) -> Result<Vec<CheckResult>> {
        let name = home.file_name().map_or_else(
            || home.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let cache = ArchiveCache::new();
        let jdk = Jdk::open(home, &cache).with_context(|| format!("opening {}", home.display()))?;
        let mut results = Vec::new();
        // Containers.
        if let Some(image) = jdk.image() {
            results.push(self.image(&name, home, image)?);
        }
        let mut zips = Vec::new();
        if jdk.feature_version() == 8 {
            let lib = if home.join("jre/lib").is_dir() {
                home.join("jre/lib")
            } else {
                home.join("lib")
            };
            for entry in std::fs::read_dir(&lib)?.chain(std::fs::read_dir(lib.join("ext"))?) {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "jar") {
                    zips.push(path);
                }
            }
        }
        if home.join("lib/ct.sym").is_file() {
            zips.push(home.join("lib/ct.sym"));
        }
        zips.sort();
        results.push(self.zips(&name, &zips, &cache)?);
        // Each release's platform API.
        let mut releases = jdk.releases()?;
        if !only.is_empty() {
            releases.retain(|r| only.contains(r));
        }
        for release in releases {
            let input = format!("{name} release {release}");
            let platform = jdk.platform(release)?;
            if self.javap {
                let names = platform.class_names()?;
                let classes = names.iter().map(|name| {
                    let bytes = platform.find(&ClassName::new(name)).and_then(|found| {
                        found.map(|c| c.bytes).ok_or_else(|| {
                            ferrion_classfile::Error::new(
                                ferrion_classfile::ErrorKind::Malformed,
                                "listed but not found",
                            )
                        })
                    });
                    (name.clone(), bytes)
                });
                results.push(self.compare_classes(&input, classes)?);
            }
            if jdk.feature_version() >= 11 {
                results.push(self.visibility(&input, home, release, &platform)?);
            }
        }
        Ok(results)
    }

    /// The image's resources against the jrt file system's files.
    fn image(
        &self,
        name: &str,
        home: &Path,
        image: &ferrion_classfile::jimage::Image,
    ) -> Result<CheckResult> {
        let start = Instant::now();
        let mut result = CheckResult::new(name, "jrt");
        let jrt = self.harness.jrt(home, self.scratch)?;
        let mut ours = BTreeMap::new();
        for resource in image.resources() {
            let name = resource.name();
            match resource.read() {
                Ok(bytes) => {
                    ours.insert(name, sha256(&bytes));
                }
                Err(error) => result
                    .failures
                    .push(format!("{name}: Ferrion can't read it: {error}")),
            }
        }
        for problem in image.problems() {
            result
                .failures
                .push(format!("Ferrion left a resource out: {problem}"));
        }
        compare_digests(
            &mut result,
            &ours,
            &jrt.into_iter().map(|(k, v)| (k, Ok(v))).collect(),
        );
        result.notes.push(format!(
            "image version {:?}, {}, {} resources{}",
            image.version(),
            if image.is_big_endian() {
                "big-endian"
            } else {
                "little-endian"
            },
            ours.len(),
            if image.resources().any(|r| r.is_compressed()) {
                ", compressed"
            } else {
                ""
            }
        ));
        result.seconds = start.elapsed().as_secs_f64();
        Ok(result)
    }

    /// Zip files' entries against `ZipFile`'s.
    fn zips(&self, input: &str, files: &[PathBuf], cache: &ArchiveCache) -> Result<CheckResult> {
        let start = Instant::now();
        let mut result = CheckResult::new(input, "zip");
        if files.is_empty() {
            return Ok(result);
        }
        self.harness
            .containers(None, files, self.scratch, |path, digests| {
                let label = path.display().to_string();
                if let Some(error) = digests.error {
                    let ours = cache.zip(path).err().map(|e| e.to_string());
                    match ours {
                        Some(ours) => result.notes.push(format!(
                            "{label}: both reject it: ZipFile: {error}; Ferrion: {ours}"
                        )),
                        None => result.failures.push(format!(
                            "{label}: ZipFile rejects it ({error}), Ferrion reads it"
                        )),
                    }
                    return Ok(());
                }
                let archive = match cache.zip(path) {
                    Ok(archive) => archive,
                    Err(error) => {
                        result
                            .failures
                            .push(format!("{label}: Ferrion can't read it: {error}"));
                        return Ok(());
                    }
                };
                let mut ours = BTreeMap::new();
                for entry in archive.unique_entries() {
                    match archive.read(entry, u64::MAX) {
                        Ok(bytes) => {
                            ours.insert(entry.name().to_owned(), sha256(&bytes));
                        }
                        Err(error) => {
                            result.failures.push(format!(
                                "{label}({}): Ferrion can't read it: {error}",
                                entry.name()
                            ));
                        }
                    }
                }
                for problem in archive.problems() {
                    result
                        .notes
                        .push(format!("{label}: Ferrion left an entry out: {problem}"));
                }
                let mut file_result = CheckResult::default();
                compare_digests(&mut file_result, &ours, &digests.entries);
                result.compared += file_result.compared;
                result.failures.extend(
                    file_result
                        .failures
                        .into_iter()
                        .map(|f| format!("{label}: {f}")),
                );
                Ok(())
            })?;
        result.seconds = start.elapsed().as_secs_f64();
        Ok(result)
    }

    /// Compares each class with javap, once per distinct class.
    fn compare_classes(
        &self,
        input: &str,
        classes: impl Iterator<Item = Class>,
    ) -> Result<CheckResult> {
        const BATCH: usize = 1000;
        let start = Instant::now();
        let result = Mutex::new(CheckResult::new(input, "javap"));
        let dir = self.scratch.join(format!("classes-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let (sender, receiver) = mpsc::sync_channel::<Vec<(String, String, PathBuf)>>(self.jobs);
        let receiver = Mutex::new(receiver);
        std::thread::scope(|scope| -> Result<()> {
            let workers: Vec<_> = (0..self.jobs)
                .map(|_| {
                    scope.spawn(|| -> Result<()> {
                        loop {
                            let batch = match receiver.lock().unwrap().recv() {
                                Ok(batch) => batch,
                                Err(_) => return Ok(()),
                            };
                            self.javap_batch(&batch, &result)?;
                        }
                    })
                })
                .collect();
            let mut batch = Vec::new();
            for (label, bytes) in classes {
                let bytes = match bytes {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        result
                            .lock()
                            .unwrap()
                            .failures
                            .push(format!("{label}: Ferrion can't read it: {error}"));
                        continue;
                    }
                };
                let hash = sha256(&bytes);
                if !self.seen.lock().unwrap().insert(hash.clone()) {
                    result.lock().unwrap().duplicates += 1;
                    continue;
                }
                let path = dir.join(format!("{hash}.class"));
                std::fs::write(&path, &bytes)?;
                batch.push((label, hash, path));
                if batch.len() == BATCH && sender.send(std::mem::take(&mut batch)).is_err() {
                    break;
                }
            }
            if !batch.is_empty() {
                let _ = sender.send(batch);
            }
            drop(sender);
            for worker in workers {
                worker.join().expect("a worker panicked")?;
            }
            Ok(())
        })?;
        let _ = std::fs::remove_dir_all(&dir);
        let mut result = result.into_inner().unwrap();
        result.seconds = start.elapsed().as_secs_f64();
        Ok(result)
    }

    fn cached(&self, hash: &str) -> Option<JavapOutput> {
        let text =
            std::fs::read_to_string(self.cache.join(&hash[..2]).join(format!("{hash}.json")))
                .ok()?;
        serde_json::from_str(&text).ok()
    }

    fn javap_batch(
        &self,
        batch: &[(String, String, PathBuf)],
        result: &Mutex<CheckResult>,
    ) -> Result<()> {
        let mut outputs: Vec<Option<JavapOutput>> =
            batch.iter().map(|(_, hash, _)| self.cached(hash)).collect();
        let missing: Vec<PathBuf> = batch
            .iter()
            .zip(&outputs)
            .filter(|(_, o)| o.is_none())
            .map(|((_, _, path), _)| path.clone())
            .collect();
        if !missing.is_empty() {
            let mut fresh = self.harness.javap(&missing, self.scratch)?.into_iter();
            for ((_, hash, _), output) in batch.iter().zip(outputs.iter_mut()) {
                if output.is_none() {
                    let found = fresh
                        .next()
                        .context("javap gave fewer results than classes")?;
                    let dir = self.cache.join(&hash[..2]);
                    std::fs::create_dir_all(&dir)?;
                    std::fs::write(
                        dir.join(format!("{hash}.json")),
                        serde_json::to_string(&found)?,
                    )?;
                    *output = Some(found);
                }
            }
        }
        for ((label, _, path), output) in batch.iter().zip(outputs) {
            let output = output.context("no javap output")?;
            let bytes = std::fs::read(path)?;
            let _ = std::fs::remove_file(path);
            let outcome = compare_class(&bytes, &output);
            let mut result = result.lock().unwrap();
            result.compared += 1;
            match outcome {
                Outcome::Match => {}
                Outcome::Note(note) => result.notes.push(format!("{label}: {note}")),
                Outcome::Failure(failure) => result.failures.push(format!("{label}: {failure}")),
            }
        }
        Ok(())
    }

    fn visibility(
        &self,
        input: &str,
        home: &Path,
        release: u32,
        platform: &PlatformApi,
    ) -> Result<CheckResult> {
        let start = Instant::now();
        let mut result = CheckResult::new(input, "visibility");
        let names = platform.class_names()?;
        let packages: BTreeSet<String> = names
            .iter()
            .map(|n| n.rsplit_once('/').map_or("", |(p, _)| p).to_owned())
            .collect();
        let packages: Vec<String> = packages.into_iter().collect();
        let javac = self
            .harness
            .visibility(home, release, &packages, self.scratch)?;
        // Visible packages.
        let ours = platform
            .visible_packages()
            .map(|v| v.into_iter().map(str::to_owned).collect::<Vec<_>>());
        match (&javac.visible, &ours) {
            (None, None) => {}
            (Some(theirs), Some(ours)) => {
                let theirs: BTreeSet<&String> = theirs.iter().collect();
                let ours: BTreeSet<&String> = ours.iter().collect();
                result.compared += theirs.len() as u64;
                for missing in theirs.difference(&ours) {
                    result
                        .failures
                        .push(format!("javac sees package {missing}; Ferrion doesn't"));
                }
                for extra in ours.difference(&theirs) {
                    result
                        .failures
                        .push(format!("Ferrion sees package {extra}; javac doesn't"));
                }
            }
            (theirs, ours) => result.failures.push(format!(
                "javac sees {} packages; Ferrion sees {}",
                theirs
                    .as_ref()
                    .map_or("every".to_owned(), |v| v.len().to_string()),
                ours.as_ref()
                    .map_or("every".to_owned(), |v| v.len().to_string()),
            )),
        }
        // Classes. javac's model and Ferrion can name different modules for a class (in JDK 11's
        // flat ct.sym, javac puts a package's classes in a module that has a subpackage of it,
        // such as java.awt's in java.datatransfer), so the sets are compared across modules, and
        // the modules only explain classes javac doesn't list.
        let mut theirs: BTreeMap<&str, &str> = BTreeMap::new();
        for (module, classes) in &javac.modules {
            for class in classes {
                theirs.insert(class, module);
            }
        }
        let ours: BTreeMap<&str, Option<String>> = names
            .iter()
            .map(|n| (n.as_str(), platform.module_of(&ClassName::new(n))))
            .collect();
        result.compared += theirs.len() as u64;
        let mut attributed = 0;
        for (class, module) in &theirs {
            match ours.get(class) {
                None => result.failures.push(format!(
                    "javac has class {class} in module `{module}`; Ferrion doesn't"
                )),
                Some(Some(our)) if !module.is_empty() && our != module => attributed += 1,
                Some(_) => {}
            }
        }
        let (mut unresolved, mut unexported, mut not_members) = (0, 0, 0);
        for (class, module) in &ours {
            if theirs.contains_key(class) {
                continue;
            }
            let package = class.rsplit_once('/').map_or("", |(p, _)| p);
            if module
                .as_ref()
                .is_some_and(|m| !javac.modules.contains_key(m))
            {
                unresolved += 1;
            } else if !platform.is_visible(&PackageName::new(package)) {
                unexported += 1;
            } else if explain_extra(platform, class, 0).is_some() {
                not_members += 1;
            } else {
                result.failures.push(format!(
                    "Ferrion has class {class} in module `{}`; javac doesn't",
                    module.as_deref().unwrap_or("")
                ));
            }
        }
        for (count, what) in [
            (
                unresolved,
                "are in modules javac doesn't resolve by default, so its model doesn't list them",
            ),
            (
                unexported,
                "are in packages their module doesn't export, which javac's model doesn't list; \
                          they are there as supertypes of exported classes",
            ),
            (
                not_members,
                "aren't members javac's model lists: local, anonymous, and synthetic classes, \
                           their members, module-info, and package-info",
            ),
            (
                attributed,
                "javac's model puts in a different module than Ferrion (JDK 11's flat ct.sym)",
            ),
        ] {
            if count > 0 {
                result
                    .notes
                    .push(format!("{count} classes Ferrion serves {what}"));
            }
        }
        result.seconds = start.elapsed().as_secs_f64();
        Ok(result)
    }

    /// The corpus projects' jars: as zips, as jars at each release that uses them, and each class
    /// with javap.
    fn corpus(&self, names: &[String]) -> Result<Vec<CheckResult>> {
        let dirs = Dirs::new()?;
        let projects = Project::load_all(&dirs.projects_file())?;
        let selected = corpus::select(&projects, names)?;
        let mut results = Vec::new();
        for project in selected {
            let Some(lock) = corpus::lock::read(&dirs, project)? else {
                continue;
            };
            let mut jars: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
            for module in &lock.modules {
                for source_set in &module.source_sets {
                    let release = source_set.java_release.unwrap_or(lock.jdk);
                    for entry in &source_set.classpath {
                        if let ClasspathEntry::Jar { jar } = entry {
                            jars.entry(dirs.jars().join(jar))
                                .or_default()
                                .insert(release);
                        }
                    }
                }
            }
            let missing = jars.keys().filter(|j| !j.is_file()).count();
            if missing > 0 {
                bail!(
                    "{}: {missing} jars aren't fetched; run `cargo corpus fetch {}`",
                    project.name,
                    project.name
                );
            }
            let cache = ArchiveCache::new();
            let input = format!("{} jars", project.name);
            let files: Vec<PathBuf> = jars.keys().cloned().collect();
            results.push(self.zips(&input, &files, &cache)?);
            let mut releases: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
            for (jar, used) in &jars {
                for &release in used {
                    releases.entry(release).or_default().push(jar.clone());
                }
            }
            for (release, files) in releases {
                let input = format!("{} jars at {release}", project.name);
                results.push(self.jars(&input, release, &files, &cache)?);
                if self.javap {
                    let mut classes: Vec<Class> = Vec::new();
                    let mut results_for_release = CheckResult::new(&input, "javap");
                    for file in &files {
                        let jar = match cache.jar(file, release) {
                            Ok(jar) => jar,
                            Err(error) => {
                                // Not a zip file (a `.pom` a build lists as a dependency):
                                // reported by the zip check, where ZipFile rejects it too.
                                results_for_release
                                    .notes
                                    .push(format!("{}: {error}", file.display()));
                                continue;
                            }
                        };
                        for (name, entry) in jar.entries() {
                            if name.ends_with(".class") {
                                let label = format!(
                                    "{}({name})",
                                    file.file_name().unwrap_or_default().to_string_lossy()
                                );
                                classes.push((label, entry.read().map(Arc::from)));
                            }
                        }
                        // Compare a few thousand classes at a time, to bound memory.
                        if classes.len() >= 4000 {
                            let compared = self.compare_classes(
                                &input,
                                std::mem::take(&mut classes).into_iter(),
                            )?;
                            merge(&mut results_for_release, compared);
                        }
                    }
                    let compared = self.compare_classes(&input, classes.into_iter())?;
                    merge(&mut results_for_release, compared);
                    results.push(results_for_release);
                }
            }
        }
        Ok(results)
    }

    /// Jars resolved for `release` against `JarFile`'s versioned view.
    fn jars(
        &self,
        input: &str,
        release: u32,
        files: &[PathBuf],
        cache: &ArchiveCache,
    ) -> Result<CheckResult> {
        let start = Instant::now();
        let mut result = CheckResult::new(input, "jar");
        self.harness
            .containers(Some(release), files, self.scratch, |path, digests| {
                let label = path.display().to_string();
                if let Some(error) = &digests.error {
                    result
                        .notes
                        .push(format!("{label}: JarFile rejects it: {error}"));
                    return Ok(());
                }
                let jar = match cache.jar(path, release) {
                    Ok(jar) => jar,
                    Err(error) => {
                        result
                            .failures
                            .push(format!("{label}: Ferrion can't read it: {error}"));
                        return Ok(());
                    }
                };
                if digests.multi_release != Some(jar.manifest().multi_release) {
                    result.failures.push(format!(
                        "{label}: JarFile says multi-release {:?}, Ferrion {}",
                        digests.multi_release,
                        jar.manifest().multi_release
                    ));
                }
                if let Some(problem) = jar.manifest_problem() {
                    result.notes.push(format!("{label}: {problem}"));
                }
                let mut ours = BTreeMap::new();
                for (name, entry) in jar.entries() {
                    match jar.archive().read(entry, u64::MAX) {
                        Ok(bytes) => {
                            ours.insert(name.to_owned(), sha256(&bytes));
                        }
                        Err(error) => result
                            .failures
                            .push(format!("{label}({name}): Ferrion can't read it: {error}")),
                    }
                }
                let mut file_result = CheckResult::default();
                compare_digests(&mut file_result, &ours, &digests.entries);
                result.compared += file_result.compared;
                result.failures.extend(
                    file_result
                        .failures
                        .into_iter()
                        .map(|f| format!("{label}: {f}")),
                );
                Ok(())
            })?;
        result.seconds = start.elapsed().as_secs_f64();
        Ok(result)
    }
}

fn merge(into: &mut CheckResult, from: CheckResult) {
    into.compared += from.compared;
    into.duplicates += from.duplicates;
    into.failures.extend(from.failures);
    into.notes.extend(from.notes);
    into.seconds += from.seconds;
}

/// Why a class Ferrion serves isn't in javac's list of member classes, or `None` when it should
/// be: `module-info` and `package-info` aren't types, and local, anonymous, and synthetic
/// classes, and the members of such classes, aren't members javac's model lists.
fn explain_extra(platform: &PlatformApi, name: &str, depth: usize) -> Option<&'static str> {
    let simple = name.rsplit_once('/').map_or(name, |(_, s)| s);
    if simple == "module-info" || simple == "package-info" {
        return Some("not a type");
    }
    if depth > 16 {
        return None;
    }
    let bytes = platform.find(&ClassName::new(name)).ok()??.bytes;
    let class = ClassFile::parse(bytes).ok()?;
    if class
        .access()
        .contains(ferrion_classfile::class::AccessFlags::SYNTHETIC)
    {
        return Some("synthetic");
    }
    for info in class.attributes() {
        match info.decode() {
            Ok(Attribute::EnclosingMethod(_)) => return Some("local or anonymous"),
            Ok(Attribute::InnerClasses(entries)) => {
                for entry in entries.flatten() {
                    if !entry.inner.eq_str(name) {
                        continue;
                    }
                    if entry.outer.is_none() || entry.name.is_none() {
                        return Some("local or anonymous");
                    }
                    let outer = entry.outer.and_then(|o| o.to_str().ok())?;
                    return explain_extra(platform, &outer, depth + 1).map(|_| "a member of one");
                }
            }
            _ => {}
        }
    }
    None
}

/// Compares Ferrion's digests with the JDK's, by name.
fn compare_digests(
    result: &mut CheckResult,
    ours: &BTreeMap<String, String>,
    theirs: &BTreeMap<String, Result<String, String>>,
) {
    for (name, their) in theirs {
        result.compared += 1;
        match (ours.get(name), their) {
            (Some(our), Ok(their)) if our == their => {}
            (Some(_), Ok(_)) => result.failures.push(format!("{name}: different bytes")),
            (None, Ok(_)) => result
                .failures
                .push(format!("{name}: the JDK has it; Ferrion doesn't")),
            (Some(_), Err(error)) => result.failures.push(format!(
                "{name}: the JDK can't read it ({error}); Ferrion can"
            )),
            (None, Err(_)) => {}
        }
    }
    for name in ours.keys() {
        if !theirs.contains_key(name) {
            result
                .failures
                .push(format!("{name}: Ferrion has it; the JDK doesn't"));
        }
    }
}

enum Outcome {
    Match,
    Note(String),
    Failure(String),
}

/// What javap 27 prints, among a class's lines, for `ACC_STRICT` on a method of a class file of
/// version 61 or later, where JVMS 4.6 no longer defines the flag. `ct.sym`'s `.sig` files for
/// releases 8 to 16 carry the JDK's own version and keep the flag, so javap fails on them; the
/// rest of its output is still compared, and the renderer prints the flags as javap does.
const STRICT_ERROR: &str = "Error: Access Flags: Unmatched bit position 0x800 for location METHOD";

/// What javap prints, among a class's lines, for any other flag a class file's version leaves
/// undefined at its location (such as `ACC_PRIVATE` on the JDK 11 image's generated `Holder`
/// classes). javap names no such bit; the renderer doesn't either, and the hex value is compared.
const FLAG_ERROR: &str = "Error: Access Flags: Unmatched bit position";

/// Renders Ferrion's model of `bytes` and compares it with javap's output.
fn compare_class(bytes: &[u8], javap: &JavapOutput) -> Outcome {
    let ours = ClassFile::parse(Arc::from(bytes))
        .map_err(anyhow::Error::from)
        .and_then(|class| render::render(&class));
    if let (Ok(ours), Some(theirs)) = (&ours, &javap.lines) {
        let strict = theirs.iter().any(|l| l.starts_with(STRICT_ERROR));
        let filtered: Vec<String> = theirs
            .iter()
            .filter(|l| !l.starts_with(FLAG_ERROR))
            .cloned()
            .collect();
        let quirk = filtered.len() != theirs.len();
        if let Some(diff) = first_difference(ours, &filtered) {
            return Outcome::Failure(diff);
        }
        return match (&javap.error, quirk) {
            (_, true) if strict => Outcome::Note(
                "javap 27 reports ACC_STRICT on methods of a version 61+ class file as an error; \
                 the rest matches"
                    .to_owned(),
            ),
            (_, true) => Outcome::Note(
                "javap reports a flag the class file's version doesn't define as an error; the rest \
                 matches"
                    .to_owned(),
            ),
            (Some(error), false) => {
                Outcome::Failure(format!("javap failed after printing the class: {error}"))
            }
            (None, false) => Outcome::Match,
        };
    }
    match (ours, &javap.lines, &javap.error) {
        (Ok(_), Some(_), _) => unreachable!("compared above"),
        (Err(ours), None, Some(theirs)) => Outcome::Note(format!(
            "both reject it: javap: {}; Ferrion: {ours:#}",
            theirs.lines().next().unwrap_or("")
        )),
        (Err(ours), Some(theirs), _)
            if theirs
                .iter()
                .any(|l| l.starts_with("Error: ") && !l.starts_with(FLAG_ERROR)) =>
        {
            let error = theirs
                .iter()
                .find(|l| l.starts_with("Error: "))
                .cloned()
                .unwrap_or_default();
            Outcome::Note(format!(
                "javap and Ferrion both report a malformed part: javap: {error}; Ferrion: {ours:#}"
            ))
        }
        (Err(ours), _, _) => Outcome::Failure(format!("javap reads it; Ferrion: {ours:#}")),
        (Ok(_), _, error) => Outcome::Failure(format!(
            "javap rejects it ({}); Ferrion reads it",
            error.as_deref().unwrap_or("?")
        )),
    }
}

/// The first line that differs, with the lines around it.
fn first_difference(ours: &[String], theirs: &[String]) -> Option<String> {
    let at = ours
        .iter()
        .zip(theirs)
        .position(|(a, b)| a != b)
        .or_else(|| (ours.len() != theirs.len()).then_some(ours.len().min(theirs.len())))?;
    let context = |lines: &[String]| -> String {
        lines[at.saturating_sub(2)..(at + 3).min(lines.len())]
            .iter()
            .map(|l| format!("\n    | {l}"))
            .collect()
    };
    Some(format!(
        "line {}\n  javap:{}\n  Ferrion:{}",
        at + 1,
        context(theirs),
        context(ours)
    ))
}
