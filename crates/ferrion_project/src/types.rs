//! Opening the type inputs: each source set's JDK and platform API, and its classpath as a
//! `ClassPath` at its release, with manifest `Class-Path` attributes expanded and the module path
//! read as more classpath entries.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_classfile::class::ModuleDescriptor;
use ferrion_classfile::jar::Jar;
use ferrion_classfile::{
    ArchiveCache, ClassFile, ClassPath, ClassPathEntry, ErrorKind, Jdk, PlatformApi,
};

use crate::error::{InputError, Origin, dedup};
use crate::paths::{absolute, has_archive_extension};
use crate::project::{ClassPathItem, Given, Project, SourceSet, SourceSetId};

/// The opened type inputs of every source set, in the project's order.
#[derive(Debug)]
pub struct TypeInputs {
    source_sets: Vec<SourceSetTypes>,
}

/// One source set's opened type inputs.
#[derive(Debug)]
pub struct SourceSetTypes {
    pub jdk: Arc<Jdk>,
    pub platform: Arc<PlatformApi>,
    /// The classpath in order, with each jar's manifest `Class-Path` inserted after it, then the
    /// module path. An entry already on the classpath is left out where it repeats.
    pub entries: Vec<TypeEntry>,
    /// The compiled entries of `entries`, in order. Source sets in `entries` are views over their
    /// sources, which `ferrion_semantic` builds.
    pub class_path: ClassPath,
}

#[derive(Clone, Debug)]
pub enum TypeEntry {
    SourceSet(Given<SourceSetId>),
    Classes(ClassesEntry),
}

/// A jar or a directory of classes.
#[derive(Clone, Debug)]
pub struct ClassesEntry {
    pub entry: ClassPathEntry,
    pub path: PathBuf,
    pub origin: Origin,
    pub on_module_path: bool,
    /// For a module path entry, its module's name: from its `module-info.class`, its manifest's
    /// `Automatic-Module-Name`, or its file name. `None` on the classpath, or when no valid name
    /// can be found.
    pub module: Option<String>,
}

impl TypeInputs {
    /// Opens the JDKs and classpaths. Errors are usage errors; warnings are returned with them.
    /// A problem with a JDK or jar that many source sets share is reported once.
    pub fn open(
        project: &Project,
        cache: &ArchiveCache,
    ) -> Result<(TypeInputs, Vec<InputError>), Vec<InputError>> {
        let mut problems = Vec::new();
        let mut jdks = HashMap::new();
        let mut source_sets = Vec::new();
        for set in project.source_sets() {
            let platform = open_platform(project, set, cache, &mut jdks, &mut problems);
            let mut opener = Opener {
                cache,
                release: set.release.value,
                seen: HashSet::new(),
                entries: Vec::new(),
                problems: &mut problems,
            };
            match &set.classpath {
                Some(items) => opener.class_path(items),
                None => opener.problems.push(
                    InputError::error(
                        "rules that need types are enabled, but no classpath was given",
                        Origin::Flag("--classpath"),
                    )
                    .with_help(
                        "pass --classpath or --classpath-file (`--classpath \"\"` for code with \
                         no dependencies), or --project-model",
                    ),
                ),
            }
            for path in &set.module_path {
                opener.module_path(path);
            }
            let entries = opener.entries;
            if let Some((jdk, platform)) = platform {
                let class_path = ClassPath::new(
                    entries
                        .iter()
                        .filter_map(|entry| match entry {
                            TypeEntry::Classes(classes) => Some(classes.entry.clone()),
                            TypeEntry::SourceSet(_) => None,
                        })
                        .collect(),
                );
                source_sets.push(SourceSetTypes {
                    jdk,
                    platform,
                    entries,
                    class_path,
                });
            }
        }
        dedup(&mut problems);
        if problems.iter().any(InputError::is_error) {
            return Err(problems);
        }
        Ok((TypeInputs { source_sets }, problems))
    }

    pub fn source_set(&self, id: SourceSetId) -> &SourceSetTypes {
        &self.source_sets[id.index()]
    }

    pub fn source_sets(&self) -> &[SourceSetTypes] {
        &self.source_sets
    }
}

type OpenedJdk = Result<Arc<Jdk>, ferrion_classfile::Error>;

fn open_platform(
    project: &Project,
    set: &SourceSet,
    cache: &ArchiveCache,
    jdks: &mut HashMap<PathBuf, OpenedJdk>,
    problems: &mut Vec<InputError>,
) -> Option<(Arc<Jdk>, Arc<PlatformApi>)> {
    let Some(home) = &set.jdk else {
        let origin = if project.is_from_model() {
            set.release.origin.clone()
        } else {
            Origin::Flag("--jdk")
        };
        problems.push(
            InputError::error(
                "rules that need types are enabled, but no JDK was given",
                origin,
            )
            .with_help("pass --jdk, or set JAVA_HOME"),
        );
        return None;
    };
    let opened = jdks
        .entry(home.value.clone())
        .or_insert_with(|| Jdk::open(&home.value, cache));
    let jdk = match opened {
        Ok(jdk) => jdk.clone(),
        Err(e) => {
            let message = if e.kind() == ErrorKind::Usage {
                e.to_string()
            } else {
                format!("can't read the JDK at `{}`: {e}", home.value.display())
            };
            problems.push(InputError::error(message, home.origin.clone()));
            return None;
        }
    };
    let release = set.release.value;
    if jdk.feature_version() < release {
        let help = match &home.origin {
            Origin::ProjectModel { source_set, .. } => {
                format!("give source set `{source_set}` JDK {release} or later in the build")
            }
            _ => format!("pass --jdk with JDK {release} or later"),
        };
        problems.push(
            InputError::error(
                format!(
                    "release {release} can't be served by the JDK at `{}` (JDK {})",
                    home.value.display(),
                    jdk.feature_version()
                ),
                home.origin.clone(),
            )
            .with_help(help),
        );
        return None;
    }
    match jdk.platform(release) {
        Ok(platform) => Some((jdk, platform)),
        Err(e) => {
            problems.push(InputError::error(e.to_string(), home.origin.clone()));
            None
        }
    }
}

/// Builds one source set's entries.
struct Opener<'a> {
    cache: &'a ArchiveCache,
    release: u32,
    /// The entries so far, by canonical path where they exist, to leave out repeats.
    seen: HashSet<PathBuf>,
    entries: Vec<TypeEntry>,
    problems: &'a mut Vec<InputError>,
}

/// A classpath entry still to open.
enum Pending {
    SourceSet(Given<SourceSetId>),
    /// `listed` is set for an entry from a manifest's `Class-Path`.
    Path {
        path: PathBuf,
        origin: Origin,
        listed: bool,
    },
}

impl Opener<'_> {
    /// Whether `path` is new to this source set's entries, recording it.
    fn first_time(&mut self, path: &Path) -> bool {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.seen.insert(key)
    }

    /// The classpath, with each jar's `Class-Path` inserted right after it, depth first, as
    /// javac does: with `a.jar` listing `b.jar c.jar` and `b.jar` listing `d.jar`, the order is
    /// `a b d c`. A stack of what is left keeps chains and cycles from recursing.
    fn class_path(&mut self, items: &[ClassPathItem]) {
        let mut pending: Vec<Pending> = items
            .iter()
            .rev()
            .map(|item| match item {
                ClassPathItem::SourceSet(id) => Pending::SourceSet(id.clone()),
                ClassPathItem::Path(path) => Pending::Path {
                    path: path.value.clone(),
                    origin: path.origin.clone(),
                    listed: false,
                },
            })
            .collect();
        let mut seen_sets = HashSet::new();
        while let Some(next) = pending.pop() {
            let (path, origin, listed) = match next {
                Pending::SourceSet(id) => {
                    if seen_sets.insert(id.value) {
                        self.entries.push(TypeEntry::SourceSet(id));
                    }
                    continue;
                }
                Pending::Path {
                    path,
                    origin,
                    listed,
                } => (path, origin, listed),
            };
            if !self.first_time(&path) {
                continue;
            }
            if !path.exists() {
                if listed {
                    self.problems.push(InputError::warning(
                        format!("classpath entry `{}` doesn't exist", path.display()),
                        origin,
                    ));
                } else {
                    self.missing(&path, origin, "classpath");
                }
                continue;
            }
            if path.is_dir() {
                self.push(ClassPathEntry::directory(&path), path, origin, false, None);
                continue;
            }
            let Some(jar) = self.jar(&path, &origin, listed, "classpath") else {
                continue;
            };
            let listed_here = self.manifest_class_path(&jar, &path);
            pending.extend(listed_here.into_iter().rev());
            self.push(ClassPathEntry::jar(jar), path, origin, false, None);
        }
    }

    /// A missing entry: an error for a jar, and a warning for a directory, which may be build
    /// output not produced yet. It is told apart by its extension, and a directory is kept, since
    /// it may appear later.
    fn missing(&mut self, path: &Path, origin: Origin, list: &str) {
        if has_archive_extension(path) {
            self.problems.push(InputError::error(
                format!("{list} entry `{}` doesn't exist", path.display()),
                origin,
            ));
        } else {
            self.problems.push(InputError::warning(
                format!("{list} directory `{}` doesn't exist", path.display()),
                origin.clone(),
            ));
            self.push(
                ClassPathEntry::directory(path),
                path.to_path_buf(),
                origin,
                list == "module path",
                None,
            );
        }
    }

    /// Opens a jar at the source set's release. A file that isn't a zip is an error if it is
    /// named like an archive, and is skipped with a warning otherwise (a `.pom` a build lists),
    /// as javac does.
    fn jar(&mut self, path: &Path, origin: &Origin, listed: bool, list: &str) -> Option<Arc<Jar>> {
        match self.cache.jar(path, self.release) {
            Ok(jar) => Some(jar),
            Err(e) if has_archive_extension(path) || listed => {
                self.problems.push(InputError::error(
                    format!(
                        "can't read the {list} jar `{}`: {}",
                        path.display(),
                        e.message()
                    ),
                    origin.clone(),
                ));
                None
            }
            Err(_) => {
                self.problems.push(InputError::warning(
                    format!(
                        "{list} entry `{}` isn't a jar or a directory, so it's skipped",
                        path.display()
                    ),
                    origin.clone(),
                ));
                None
            }
        }
    }

    /// The entries of `jar`'s `Class-Path`, resolved against its directory, in order.
    fn manifest_class_path(&mut self, jar: &Jar, path: &Path) -> Vec<Pending> {
        if let Some(problem) = jar.manifest_problem() {
            self.problems.push(InputError::warning(
                format!(
                    "the manifest of `{}` can't be read, so its Class-Path is ignored: {}",
                    path.display(),
                    problem.message()
                ),
                Origin::Manifest {
                    jar: path.to_path_buf(),
                },
            ));
        }
        let dir = path.parent().unwrap_or(Path::new("/"));
        let mut listed = Vec::new();
        for url in &jar.manifest().class_path {
            let origin = Origin::Manifest {
                jar: path.to_path_buf(),
            };
            match manifest_entry(dir, url) {
                Some(entry) => listed.push(Pending::Path {
                    path: entry,
                    origin,
                    listed: true,
                }),
                None => self.problems.push(InputError::warning(
                    format!("Class-Path entry `{url}` isn't a local file, so it's skipped"),
                    origin,
                )),
            }
        }
        listed
    }

    /// A module path entry: a jar, an exploded module (a directory with `module-info.class`), or
    /// a directory of them, read in name order. Manifests' `Class-Path` aren't followed there.
    fn module_path(&mut self, given: &Given<PathBuf>) {
        let path = &given.value;
        if !path.exists() {
            if self.first_time(path) {
                self.missing(path, given.origin.clone(), "module path");
            }
            return;
        }
        if !path.is_dir() || path.join("module-info.class").is_file() {
            self.module(path, &given.origin);
            return;
        }
        let mut members: Vec<PathBuf> = match std::fs::read_dir(path) {
            Ok(entries) => entries.flatten().map(|entry| entry.path()).collect(),
            Err(e) => {
                self.problems.push(InputError::error(
                    format!(
                        "can't read the module path directory `{}`: {e}",
                        path.display()
                    ),
                    given.origin.clone(),
                ));
                return;
            }
        };
        members.sort();
        for member in members {
            let is_module = if member.is_dir() {
                member.join("module-info.class").is_file()
            } else {
                member
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("jar"))
            };
            if is_module {
                self.module(&member, &given.origin);
            }
        }
    }

    fn module(&mut self, path: &Path, origin: &Origin) {
        if !self.first_time(path) {
            return;
        }
        if path.is_dir() {
            let bytes = std::fs::read(path.join("module-info.class"));
            let name = match bytes {
                Ok(bytes) => self.descriptor_name(&bytes, path),
                Err(_) => None,
            };
            self.push(
                ClassPathEntry::directory(path),
                path.to_path_buf(),
                origin.clone(),
                true,
                name,
            );
            return;
        }
        let Some(jar) = self.jar(path, origin, false, "module path") else {
            return;
        };
        let name = match jar.find("module-info.class").map(|entry| entry.read()) {
            Some(Ok(bytes)) => self.descriptor_name(&bytes, path),
            Some(Err(e)) => {
                self.module_problem(path, e.message());
                None
            }
            None => jar
                .manifest()
                .automatic_module_name
                .clone()
                .or_else(|| automatic_module_name(path)),
        };
        self.push(
            ClassPathEntry::jar(jar),
            path.to_path_buf(),
            origin.clone(),
            true,
            name,
        );
    }

    fn descriptor_name(&mut self, bytes: &[u8], path: &Path) -> Option<String> {
        let descriptor =
            ClassFile::parse(bytes.into()).and_then(|class| ModuleDescriptor::from_class(&class));
        match descriptor {
            Ok(descriptor) => Some(descriptor.name),
            Err(e) => {
                self.module_problem(path, e.message());
                None
            }
        }
    }

    fn module_problem(&mut self, path: &Path, detail: &str) {
        self.problems.push(InputError::warning(
            format!(
                "can't read the module descriptor of `{}`, so its module has no name: {detail}",
                path.display()
            ),
            Origin::Flag("--module-path"),
        ));
    }

    fn push(
        &mut self,
        entry: ClassPathEntry,
        path: PathBuf,
        origin: Origin,
        on_module_path: bool,
        module: Option<String>,
    ) {
        self.entries.push(TypeEntry::Classes(ClassesEntry {
            entry,
            path,
            origin,
            on_module_path,
            module,
        }));
    }
}

/// A manifest `Class-Path` entry as a path: a URL relative to the jar's directory, or a `file:`
/// URL, with `%` escapes decoded (javac finds `sp%20ace/c.jar` in `sp ace`). `None` for another
/// scheme.
fn manifest_entry(dir: &Path, url: &str) -> Option<PathBuf> {
    let scheme_end = url.find(':').filter(|&end| {
        end >= 2
            && url[..end]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    });
    let path = match scheme_end {
        Some(end) if url[..end].eq_ignore_ascii_case("file") => {
            let rest = &url[end + 1..];
            // `file:///x` and `file:/x`: the authority, if there is one, is empty.
            rest.strip_prefix("//").unwrap_or(rest)
        }
        Some(_) => return None,
        None => url,
    };
    Some(absolute(dir, Path::new(&percent_decode(path))))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The name of the automatic module a jar without `module-info.class` or
/// `Automatic-Module-Name` is, from its file name, as `ModuleFinder`'s documentation describes it
/// and the jar tool's `--describe-module` shows: `.jar` removed; a version (from a `-` followed by
/// a digit, then a `.` or the end) removed; every other character than ASCII letters and digits
/// replaced with `.`; repeated dots made one; leading and trailing dots removed. `None` when a
/// part of the result isn't a Java identifier (`lib-1a.jar` gives `lib.1a`).
pub(crate) fn automatic_module_name(path: &Path) -> Option<String> {
    let file = path.file_name()?.to_str()?;
    let stem = file.strip_suffix(".jar")?;
    let bytes = stem.as_bytes();
    let mut end = stem.len();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'-' {
            continue;
        }
        let digits = bytes[i + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let after = bytes.get(i + 1 + digits);
        if digits > 0 && matches!(after, None | Some(b'.')) {
            end = i;
            break;
        }
    }
    let mut name = String::with_capacity(end);
    for c in stem[..end].chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '.' };
        if !(c == '.' && (name.is_empty() || name.ends_with('.'))) {
            name.push(c);
        }
    }
    while name.ends_with('.') {
        name.pop();
    }
    let valid = !name.is_empty()
        && name
            .split('.')
            .all(|part| part.chars().next().is_some_and(|c| !c.is_ascii_digit()));
    valid.then_some(name)
}

#[cfg(test)]
mod tests;
