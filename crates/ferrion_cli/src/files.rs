//! Which files a run covers (docs/design/cli.md, "Which files a run covers").
//!
//! - With no paths, every `*.java` file under the working directory; a folder, every `*.java` file
//!   under it; a file, that file, whatever its extension.
//! - The walk skips folders whose name starts with `.` (a folder passed explicitly is still
//!   walked), never follows a symbolic link, and includes hidden files. A folder that a global
//!   ignore ending in `/**` matches is skipped without being read.
//! - An explicitly passed file is skipped with a note when a global ignore matches it, its
//!   `format.enabled` is false, it doesn't end in `.java`, or it is outside the working directory.
//!   Walked files that are ignored or disabled are skipped silently.
//! - An explicitly passed symbolic link is used as given.
//! - Paths are normalized lexically (`./`, `..`, repeated separators, and on Windows `\`), matched
//!   and reported relative to the working directory with `/` separators, and each file is listed
//!   once, in byte order of its path.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use ferrion_config::{Config, Entry, IgnoredBy};

/// The files a run covers, and the explicitly passed ones it skips.
#[derive(Debug)]
pub struct Selection<'c> {
    /// The files to format, sorted by path, each once.
    pub files: Vec<SelectedFile>,
    /// Explicitly passed paths that are skipped, sorted by path.
    pub skipped: Vec<Skipped<'c>>,
    /// Folders or files that couldn't be read while walking, sorted by path. The run goes on
    /// without them.
    pub errors: Vec<WalkError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedFile {
    /// The path relative to the working directory, with `/` separators, as configs match it and
    /// reports show it.
    pub path: String,
    /// Where to read and write it: the path as given or walked, under the working directory.
    pub fs_path: PathBuf,
}

/// An explicitly passed path that the run skips, and why.
#[derive(Debug)]
pub struct Skipped<'c> {
    /// The path relative to the working directory, with `/` separators, or for a path outside it,
    /// the path as given, normalized.
    pub path: String,
    pub reason: SkipReason<'c>,
}

#[derive(Debug)]
pub enum SkipReason<'c> {
    /// A global ignore matches it.
    Ignored(IgnoredBy<'c>),
    /// Its effective `format.enabled` is false, from this entry.
    Disabled(&'c Entry),
    /// It doesn't end in `.java`.
    NotJava,
    /// It is outside the working directory, which no glob can match.
    OutsideWorkingDirectory,
}

impl fmt::Display for SkipReason<'_> {
    /// What the file is, after "is": `ignored by configs[0], "**/legacy/**"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::Ignored(by) => write!(f, "{by}"),
            SkipReason::Disabled(entry) => {
                write!(
                    f,
                    "disabled by `format.enabled: false` in {}",
                    entry.label()
                )
            }
            SkipReason::NotJava => f.write_str("not a `.java` file"),
            SkipReason::OutsideWorkingDirectory => f.write_str("outside the working directory"),
        }
    }
}

/// A path that couldn't be read while selecting files.
#[derive(Debug)]
pub struct WalkError {
    /// The path relative to the working directory, with `/` separators; `.` for the working
    /// directory itself.
    pub path: String,
    pub error: io::Error,
}

/// A path given on the command line that doesn't exist: a usage error.
#[derive(Debug, PartialEq, Eq)]
pub struct MissingPath {
    /// The path as given.
    pub path: PathBuf,
}

/// A path normalized lexically against the working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Normalized {
    /// Inside the working directory: the path relative to it, with `/` separators, empty for the
    /// working directory itself.
    Inside(String),
    /// Outside it: the path as given, normalized, with `/` separators.
    Outside(String),
}

/// Normalizes `path`, relative to `cwd` or absolute, without touching the file system: `.`
/// segments and repeated separators are dropped, `..` removes the segment before it, and on
/// Windows `\` is a separator. Symbolic links aren't resolved, so `link/..` is the folder that
/// holds `link`.
pub fn normalize(cwd: &Path, path: &Path) -> Normalized {
    let joined = cwd.join(path);
    let full = lexical(&joined);
    let base = lexical(cwd);
    if full.len() >= base.len() && full.iter().zip(&base).all(|(a, b)| same_component(a, b)) {
        let rest: Vec<String> = full[base.len()..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        Normalized::Inside(rest.join("/"))
    } else {
        Normalized::Outside(display(&lexical(path)))
    }
}

/// The components of `path` with `.` dropped and `..` applied.
fn lexical(path: &Path) -> Vec<Component<'_>> {
    let mut out: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // `..` at the root is the root.
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(component),
            },
            _ => out.push(component),
        }
    }
    out
}

fn same_component(a: &Component<'_>, b: &Component<'_>) -> bool {
    match (a, b) {
        // Drive letters are case-insensitive: `c:` is `C:`.
        (Component::Prefix(a), Component::Prefix(b)) => a
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy()),
        _ => a == b,
    }
}

/// Components joined with `/`, after a root or a drive without one.
fn display(components: &[Component<'_>]) -> String {
    let mut out = String::new();
    let mut separate = false;
    for component in components {
        match component {
            Component::Prefix(prefix) => out.push_str(&prefix.as_os_str().to_string_lossy()),
            Component::RootDir => out.push('/'),
            other => {
                if separate {
                    out.push('/');
                }
                out.push_str(&other.as_os_str().to_string_lossy());
                separate = true;
            }
        }
    }
    if out.is_empty() { ".".to_owned() } else { out }
}

/// Selects the files a run covers: every `*.java` file under `cwd` when `paths` is empty, or the
/// files and folders in `paths`, relative to `cwd` or absolute. Fails, before reading any folder,
/// if a path doesn't exist.
pub fn select<'c>(
    cwd: &Path,
    paths: &[PathBuf],
    config: &'c Config,
) -> Result<Selection<'c>, Vec<MissingPath>> {
    let mut selector = Selector {
        config,
        files: BTreeMap::new(),
        skipped: BTreeMap::new(),
        errors: Vec::new(),
    };
    if paths.is_empty() {
        selector.walk(cwd.to_owned(), String::new());
        return Ok(selector.finish());
    }
    let mut existing = Vec::new();
    let mut missing = Vec::new();
    for path in paths {
        let fs_path = cwd.join(path);
        // Follows a symbolic link: one passed explicitly is used as given.
        match std::fs::metadata(&fs_path) {
            Ok(metadata) => existing.push((path, fs_path, Ok(metadata))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(MissingPath { path: path.clone() })
            }
            Err(e) => existing.push((path, fs_path, Err(e))),
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }
    for (path, fs_path, metadata) in existing {
        let normalized = normalize(cwd, path);
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                let path = match normalized {
                    Normalized::Inside(p) | Normalized::Outside(p) => p,
                };
                selector.errors.push(WalkError { path, error });
                continue;
            }
        };
        match normalized {
            Normalized::Outside(display) => {
                selector.skip(display, SkipReason::OutsideWorkingDirectory);
            }
            Normalized::Inside(rel) if metadata.is_dir() => {
                if rel.is_empty() || config.prunes(&rel).is_none() {
                    selector.walk(fs_path, rel);
                }
            }
            Normalized::Inside(rel) => selector.explicit_file(fs_path, rel),
        }
    }
    Ok(selector.finish())
}

struct Selector<'c> {
    config: &'c Config,
    files: BTreeMap<String, SelectedFile>,
    skipped: BTreeMap<String, Skipped<'c>>,
    errors: Vec<WalkError>,
}

impl<'c> Selector<'c> {
    fn finish(self) -> Selection<'c> {
        let mut errors = self.errors;
        errors.sort_by(|a, b| a.path.cmp(&b.path));
        Selection {
            files: self.files.into_values().collect(),
            skipped: self.skipped.into_values().collect(),
            errors,
        }
    }

    fn add(&mut self, fs_path: PathBuf, path: String) {
        self.files
            .entry(path.clone())
            .or_insert(SelectedFile { path, fs_path });
    }

    fn skip(&mut self, path: String, reason: SkipReason<'c>) {
        self.skipped
            .entry(path.clone())
            .or_insert(Skipped { path, reason });
    }

    /// Why a file at `path` is left out of the run, if it is: a global ignore, or
    /// `format.enabled: false`. Whether it ends in `.java` is checked apart.
    fn excluded(&self, path: &str) -> Option<SkipReason<'c>> {
        let resolution = self.config.resolve(path);
        if let Some(ignored) = resolution.ignored {
            return Some(SkipReason::Ignored(ignored));
        }
        match resolution.format.entry {
            Some(entry) if !resolution.format.value.enabled => {
                Some(SkipReason::Disabled(&self.config.entries[entry]))
            }
            _ => None,
        }
    }

    fn explicit_file(&mut self, fs_path: PathBuf, path: String) {
        let reason = match self.config.ignored_by(&path) {
            Some(ignored) => Some(SkipReason::Ignored(ignored)),
            None if !path.ends_with(".java") => Some(SkipReason::NotJava),
            None => self.excluded(&path),
        };
        match reason {
            Some(reason) => self.skip(path, reason),
            None => self.add(fs_path, path),
        }
    }

    /// Adds the `*.java` files under the folder at `fs_path`, whose normalized path is `rel`.
    fn walk(&mut self, fs_path: PathBuf, rel: String) {
        let mut stack = vec![(fs_path, rel)];
        while let Some((dir, rel)) = stack.pop() {
            let child = |name: &str| {
                if rel.is_empty() {
                    name.to_owned()
                } else {
                    format!("{rel}/{name}")
                }
            };
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) => {
                    self.error(&rel, error);
                    continue;
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        self.error(&rel, error);
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = child(&name);
                // Doesn't follow symbolic links, so a link reports itself.
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(error) => {
                        self.error(&path, error);
                        continue;
                    }
                };
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if !name.starts_with('.') && self.config.prunes(&path).is_none() {
                        stack.push((entry.path(), path));
                    }
                } else if name.ends_with(".java") && self.excluded(&path).is_none() {
                    self.add(entry.path(), path);
                }
            }
        }
    }

    fn error(&mut self, path: &str, error: io::Error) {
        let path = if path.is_empty() { "." } else { path };
        self.errors.push(WalkError {
            path: path.to_owned(),
            error,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// A scratch folder for one test, removed when it ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ferrion-cli-files-{}-{name}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            // Canonical, as the working directory is (macOS's temporary folder is behind a
            // symbolic link), without the `\\?\` prefix Windows adds.
            let dir = dir.canonicalize().unwrap();
            let dir = match dir.to_str().and_then(|d| d.strip_prefix(r"\\?\")) {
                Some(plain) => PathBuf::from(plain),
                None => dir,
            };
            Scratch(dir)
        }

        /// Creates files (and their folders), given with `/`.
        fn files(&self, names: &[&str]) -> &Scratch {
            for name in names {
                let path = self.0.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, "class A {}\n").unwrap();
            }
            self
        }

        fn dir(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn config(text: &str) -> Config {
        Config::from_text(text).unwrap()
    }

    fn select_paths<'c>(cwd: &Path, paths: &[&str], config: &'c Config) -> Selection<'c> {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        select(cwd, &paths, config).unwrap()
    }

    fn paths(selection: &Selection<'_>) -> Vec<String> {
        selection.files.iter().map(|f| f.path.clone()).collect()
    }

    fn skipped(selection: &Selection<'_>) -> Vec<(String, String)> {
        selection
            .skipped
            .iter()
            .map(|s| (s.path.clone(), s.reason.to_string()))
            .collect()
    }

    #[test]
    fn the_working_directory_is_walked() {
        let scratch = Scratch::new("walk");
        scratch.files(&[
            "A.java",
            "src/main/java/app/Main.java",
            "src/main/resources/app.properties",
            "src/.Hidden.java",
            ".git/objects/X.java",
            "src/.generated/Gen.java",
            "README.md",
            "Upper.JAVA",
        ]);
        let none = Config::default();
        let selection = select_paths(&scratch.0, &[], &none);
        assert_eq!(
            paths(&selection),
            ["A.java", "src/.Hidden.java", "src/main/java/app/Main.java"]
        );
        assert!(selection.skipped.is_empty() && selection.errors.is_empty());
        let file = &selection.files[2];
        assert_eq!(file.fs_path, scratch.0.join("src/main/java/app/Main.java"));
        assert!(file.fs_path.is_file());
        // `.` is the same.
        assert_eq!(
            paths(&select_paths(&scratch.0, &["."], &none)),
            paths(&selection)
        );
    }

    #[test]
    fn explicit_folders_and_files() {
        let scratch = Scratch::new("explicit");
        scratch.files(&[
            "src/A.java",
            "src/b/B.java",
            "src/.hidden/H.java",
            ".config/C.java",
            ".config/.deeper/D.java",
            ".hidden/E.java",
            "notes.txt",
            "lib/L.java",
        ]);
        let none = Config::default();
        let selection = select_paths(
            &scratch.0,
            &["src", ".config", ".hidden/E.java", "notes.txt"],
            &none,
        );
        // A hidden folder passed explicitly is walked, but hidden folders in it aren't; a file in a
        // hidden folder passed explicitly is used.
        assert_eq!(
            paths(&selection),
            [
                ".config/C.java",
                ".hidden/E.java",
                "src/A.java",
                "src/b/B.java"
            ]
        );
        assert_eq!(
            skipped(&selection),
            [("notes.txt".to_owned(), "not a `.java` file".to_owned())]
        );
    }

    #[test]
    fn missing_paths_are_errors() {
        let scratch = Scratch::new("missing");
        scratch.files(&["A.java"]);
        let paths: Vec<PathBuf> = ["A.java", "B.java", "gone/"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let missing = select(&scratch.0, &paths, &Config::default()).unwrap_err();
        assert_eq!(
            missing,
            [
                MissingPath {
                    path: "B.java".into()
                },
                MissingPath {
                    path: "gone/".into()
                }
            ]
        );
    }

    #[test]
    fn ignored_and_disabled_files() {
        let scratch = Scratch::new("config");
        scratch.files(&[
            "src/main/A.java",
            "src/legacy/Old.java",
            "src/test/ATest.java",
            "src/test/fixtures/F.java",
            "build/Gen.java",
        ]);
        let config = config(
            r#"{ "configs": [
                { "ignores": ["**/legacy/**", "build/"] },
                { "name": "tests", "files": ["src/test/**"], "format": { "enabled": false } },
                { "files": ["src/test/fixtures/**"], "format": {} }
            ] }"#,
        );
        // Walked files that are ignored or disabled are skipped silently.
        let walked = select_paths(&scratch.0, &[], &config);
        assert_eq!(
            paths(&walked),
            ["src/main/A.java", "src/test/fixtures/F.java"]
        );
        assert!(walked.skipped.is_empty());
        // Explicitly passed ones are skipped with a note.
        let explicit = select_paths(
            &scratch.0,
            &[
                "src/legacy/Old.java",
                "src/test/ATest.java",
                "src/test/fixtures/F.java",
                "build/Gen.java",
            ],
            &config,
        );
        assert_eq!(paths(&explicit), ["src/test/fixtures/F.java"]);
        assert_eq!(
            skipped(&explicit),
            [
                (
                    "build/Gen.java".to_owned(),
                    r#"ignored by configs[0], "build/""#.to_owned()
                ),
                (
                    "src/legacy/Old.java".to_owned(),
                    r#"ignored by configs[0], "**/legacy/**""#.to_owned()
                ),
                (
                    "src/test/ATest.java".to_owned(),
                    r#"disabled by `format.enabled: false` in configs[1] "tests""#.to_owned()
                ),
            ]
        );
        // An ignored file that isn't `.java` is reported as ignored.
        scratch.files(&["src/legacy/notes.txt"]);
        let selection = select_paths(&scratch.0, &["src/legacy/notes.txt"], &config);
        assert!(skipped(&selection)[0].1.starts_with("ignored by"));
    }

    #[cfg(unix)]
    #[test]
    fn ignored_folders_are_not_read() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("prune");
        scratch.files(&["A.java", "legacy/Old.java", "src/locked/L.java"]);
        let locked = scratch.0.join("legacy");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // A folder that can't be read is an error, unless a global ignore prunes it.
        let unreadable = std::fs::read_dir(&locked).is_err();
        let none = Config::default();
        let selection = select_paths(&scratch.0, &[], &none);
        let config = config(r#"{ "configs": [{ "ignores": ["legacy/**"] }] }"#);
        let pruned = select_paths(&scratch.0, &[], &config);
        let explicit = select_paths(&scratch.0, &["legacy"], &config);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root can read the folder anyway.
        if unreadable {
            assert_eq!(selection.errors.len(), 1);
            assert_eq!(selection.errors[0].path, "legacy");
        }
        assert_eq!(paths(&pruned), ["A.java", "src/locked/L.java"]);
        assert!(pruned.errors.is_empty());
        assert!(explicit.files.is_empty() && explicit.errors.is_empty());
    }

    #[test]
    fn outside_the_working_directory() {
        let scratch = Scratch::new("outside");
        scratch.files(&["proj/A.java", "other/B.java", "other/C.java"]);
        let cwd = scratch.dir("proj");
        let none = Config::default();
        let absolute = scratch.0.join("other/C.java");
        let selection = select_paths(
            &cwd,
            &[
                "../other/B.java",
                absolute.to_str().unwrap(),
                "../proj/A.java",
                "../other",
            ],
            &none,
        );
        // A path that leaves the working directory and comes back is inside it.
        assert_eq!(paths(&selection), ["A.java"]);
        let reasons = skipped(&selection);
        let outside: Vec<&str> = reasons.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(outside.len(), 3);
        assert!(outside.contains(&"../other/B.java"));
        assert!(outside.contains(&"../other"));
        assert!(
            reasons
                .iter()
                .all(|(_, r)| r == "outside the working directory")
        );
    }

    #[test]
    fn paths_are_normalized_and_deduplicated() {
        let scratch = Scratch::new("normalize");
        scratch.files(&["src/a/A.java", "src/b/B.java"]);
        let none = Config::default();
        let absolute = scratch.0.join("src/b/B.java");
        let selection = select_paths(
            &scratch.0,
            &[
                "./src//a/A.java",
                "src/b/../a/A.java",
                "src",
                absolute.to_str().unwrap(),
                "src/a/",
            ],
            &none,
        );
        assert_eq!(paths(&selection), ["src/a/A.java", "src/b/B.java"]);
        assert!(selection.skipped.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn backslashes_are_separators_on_windows() {
        let scratch = Scratch::new("windows");
        scratch.files(&["src/a/A.java"]);
        let none = Config::default();
        let selection = select_paths(&scratch.0, &[r"src\a\A.java", r".\src\a"], &none);
        assert_eq!(paths(&selection), ["src/a/A.java"]);
        let cwd = scratch.0.to_str().unwrap();
        let lower = format!("{}{}", &cwd[..1].to_lowercase(), &cwd[1..]);
        assert_eq!(
            normalize(&scratch.0, Path::new(&format!(r"{lower}\src\x.java"))),
            Normalized::Inside("src/x.java".into())
        );
    }

    #[test]
    fn normalizing() {
        let cwd = std::env::temp_dir().join("proj");
        let inside = |p: &str| Normalized::Inside(p.to_owned());
        assert_eq!(normalize(&cwd, Path::new("a/./b//c")), inside("a/b/c"));
        assert_eq!(normalize(&cwd, Path::new("a/../../proj/b")), inside("b"));
        assert_eq!(normalize(&cwd, Path::new(".")), inside(""));
        assert_eq!(normalize(&cwd, Path::new("a/..")), inside(""));
        assert_eq!(normalize(&cwd, &cwd.join("x/y")), inside("x/y"));
        assert_eq!(
            normalize(&cwd, Path::new("../other/./x")),
            Normalized::Outside("../other/x".into())
        );
        assert_eq!(
            normalize(&cwd, Path::new("../projects/x")),
            Normalized::Outside("../projects/x".into())
        );
    }

    #[test]
    fn sorted_by_bytes() {
        let scratch = Scratch::new("order");
        scratch.files(&[
            "b.java",
            "C.java",
            "a/b.java",
            "a.b.java",
            "a-b/x.java",
            "é.java",
        ]);
        let none = Config::default();
        let selection = select_paths(&scratch.0, &[], &none);
        assert_eq!(
            paths(&selection),
            [
                "C.java",
                "a-b/x.java",
                "a.b.java",
                "a/b.java",
                "b.java",
                "é.java"
            ]
        );
    }

    /// Creates a symbolic link, or returns false where the test can't (Windows without the
    /// privilege).
    fn symlink(target: &Path, link: &Path, dir: bool) -> bool {
        #[cfg(unix)]
        {
            let _ = dir;
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            if dir {
                std::os::windows::fs::symlink_dir(target, link).is_ok()
            } else {
                std::os::windows::fs::symlink_file(target, link).is_ok()
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link, dir);
            false
        }
    }

    #[test]
    fn symbolic_links() {
        let scratch = Scratch::new("links");
        scratch.files(&["real/A.java", "outside/B.java"]);
        let cwd = scratch.dir("proj");
        std::fs::write(cwd.join("Own.java"), "class Own {}\n").unwrap();
        if !symlink(
            &scratch.0.join("real/A.java"),
            &cwd.join("Link.java"),
            false,
        ) || !symlink(&scratch.0.join("outside"), &cwd.join("linked"), true)
            || !symlink(&cwd, &cwd.join("loop"), true)
        {
            eprintln!("skipped: can't create symbolic links here");
            return;
        }
        let none = Config::default();
        // The walk follows no link, to a file or a folder, and so can't loop.
        let walked = select_paths(&cwd, &[], &none);
        assert_eq!(paths(&walked), ["Own.java"]);
        // An explicitly passed link is used as given: matched and reported by its own path, and
        // read through it.
        let config = config(r#"{ "configs": [{ "ignores": ["linked/**"] }] }"#);
        let explicit = select_paths(&cwd, &["Link.java", "linked/B.java", "linked"], &config);
        assert_eq!(paths(&explicit), ["Link.java"]);
        assert_eq!(explicit.files[0].fs_path, cwd.join("Link.java"));
        assert_eq!(
            std::fs::read_to_string(&explicit.files[0].fs_path).unwrap(),
            "class A {}\n"
        );
        assert_eq!(
            skipped(&explicit),
            [(
                "linked/B.java".to_owned(),
                r#"ignored by configs[0], "linked/**""#.to_owned()
            )]
        );
        // A linked folder passed explicitly is walked, by the link's path.
        let folder = select_paths(&cwd, &["linked"], &none);
        assert_eq!(paths(&folder), ["linked/B.java"]);
    }
}
