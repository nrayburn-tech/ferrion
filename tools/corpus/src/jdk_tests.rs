//! OpenJDK's javac tests, used as parser inputs.
//!
//! Their license isn't compatible with Ferrion's, so they're fetched at test time and never
//! committed. `jdk-tests.toml` pins the repository, tag, and commit, and names the directories to check out. Only those are
//! checked out, from a shallow partial clone (`--filter=blob:none`) with a sparse checkout, so git
//! downloads file contents only for them and no implementation source is fetched. The checkout
//! goes to `jdk-tests/` in the corpus directory.
//!
//! [`inputs`] lists every `.java` file in those directories, with the releases each one is compiled
//! at. The releases come from the test's jtreg header (`@compile` options such as `-source 8` or
//! `--release 17`, and `@enablePreview`) and, for javac's diagnostic examples, from their
//! `// options:` line. The headers are read only for that; the tests aren't otherwise interpreted.
//!
//! `jdk-tests-deep-nesting.txt` lists the inputs that are javac's deep-nesting stress tests (see
//! [`deep_nesting`]). Paths are committed; contents are not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use ferrion_java_version::{LATEST_RELEASE, OLDEST_RELEASE};
use serde::Deserialize;

use crate::Dirs;
use crate::lock::{parse_java_version, relative, run};

/// The pinned tests, from `jdk-tests.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JdkTests {
    pub repository: String,
    /// The GA tag of the latest supported release.
    pub tag: String,
    /// The commit `tag` points to.
    pub commit: String,
    /// The directories to check out, relative to the repository root.
    pub paths: Vec<String>,
}

impl JdkTests {
    pub fn load(dirs: &Dirs) -> Result<JdkTests> {
        let path = dirs.tool.join("jdk-tests.toml");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// The patterns given to `git sparse-checkout`: each directory, and nothing else, not even the
    /// files at the repository's root.
    fn sparse_patterns(&self) -> Vec<String> {
        self.paths
            .iter()
            .map(|path| format!("/{}/", path.trim_matches('/')))
            .collect()
    }
}

/// Where the tests are checked out.
pub fn checkout_dir(dirs: &Dirs) -> PathBuf {
    dirs.corpus.join("jdk-tests")
}

/// Checks out the tests at the pinned commit, and returns the checkout's path.
pub fn checkout(tests: &JdkTests, dirs: &Dirs) -> Result<PathBuf> {
    let dir = checkout_dir(dirs);
    if is_checked_out(tests, dirs) {
        return Ok(dir);
    }
    println!(
        "jdk-tests: checking out {} ({}) from {}",
        tests.paths.join(", "),
        tests.tag,
        tests.repository
    );
    std::fs::create_dir_all(&dir)?;
    if !dir.join(".git").exists() {
        run(Command::new("git").arg("init").arg("--quiet").arg(&dir))?;
        run(git(&dir).args(["remote", "add", "origin", &tests.repository]))?;
    } else {
        run(git(&dir).args(["remote", "set-url", "origin", &tests.repository]))?;
    }
    // Set before anything is fetched, so only these paths' contents are ever downloaded.
    run(git(&dir)
        .args(["sparse-checkout", "set", "--no-cone"])
        .args(tests.sparse_patterns()))?;
    run(git(&dir)
        .args([
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "--filter=blob:none",
            "origin",
        ])
        .arg(&tests.commit))?;
    run(git(&dir).args(["checkout", "--quiet", "--force", "--detach", "FETCH_HEAD"]))?;
    if !is_checked_out(tests, dirs) {
        bail!("jdk-tests: the checkout in {} is incomplete", dir.display());
    }
    Ok(dir)
}

/// Whether the tests are checked out at the pinned commit, with every pinned directory present.
pub fn is_checked_out(tests: &JdkTests, dirs: &Dirs) -> bool {
    let dir = checkout_dir(dirs);
    let head = git(&dir).args(["rev-parse", "HEAD"]).output().ok();
    let at_commit = head.is_some_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == tests.commit
    });
    at_commit && tests.paths.iter().all(|path| dir.join(path).is_dir())
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// A test file, and how javac's tests compile it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// The file's path, relative to the checkout, with `/` separators.
    pub path: String,
    pub file: PathBuf,
    /// The supported releases the file is compiled at. Empty when the tests compile it only at
    /// releases before 8 (see `unsupported`).
    pub releases: BTreeSet<u32>,
    /// Releases before 8 that the tests also compile the file at, as written (`7`, `1.4`).
    pub unsupported: Vec<String>,
    /// Whether the tests compile the file with `--enable-preview`. It's compared at the latest
    /// release without preview features, where javac reports them as errors, as Ferrion does.
    pub preview: bool,
    /// Whether the file is one of javac's deep-nesting stress tests.
    pub deep_nesting: bool,
}

/// Every `.java` file in the checkout's pinned directories, sorted by path.
pub fn inputs(tests: &JdkTests, dirs: &Dirs) -> Result<Vec<Input>> {
    if !is_checked_out(tests, dirs) {
        bail!(
            "javac's tests aren't checked out at {} ({}); run `cargo corpus jdk-tests`",
            tests.tag,
            tests.commit
        );
    }
    let root = checkout_dir(dirs).canonicalize()?;
    let mut files = Vec::new();
    for path in &tests.paths {
        files.extend(java_files(&root.join(path))?);
    }
    files.sort();
    files.dedup();

    // What each file's own header says, and what other files' headers say about it.
    let mut settings: BTreeMap<PathBuf, Vec<Setting>> = BTreeMap::new();
    for file in &files {
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        let header = Header::parse(&text);
        let dir = file.parent().unwrap();
        for compile in header.compiles {
            let named: Vec<PathBuf> = compile
                .files
                .iter()
                .map(|name| normalize(&dir.join(name)))
                .collect();
            let setting = Setting {
                release: compile.release,
                preview: compile.preview || header.enable_preview,
            };
            if named.is_empty() {
                settings
                    .entry(file.clone())
                    .or_default()
                    .push(setting.clone());
            }
            for path in named {
                settings.entry(path).or_default().push(setting.clone());
            }
        }
        if header.enable_preview {
            settings.entry(file.clone()).or_default().push(Setting {
                release: None,
                preview: true,
            });
        }
    }

    let deep: BTreeSet<String> = deep_nesting(dirs)?.into_iter().collect();
    let mut inputs = Vec::new();
    for file in files {
        let path = relative(&root, &file).unwrap();
        let mut releases = BTreeSet::new();
        let mut unsupported = Vec::new();
        let mut preview = false;
        match settings.get(&file) {
            // jtreg compiles a test it isn't told how to compile with javac's defaults, and so
            // does a test's own `@build`.
            None => {
                releases.insert(LATEST_RELEASE);
            }
            Some(settings) => {
                for setting in settings {
                    preview |= setting.preview;
                    match &setting.release {
                        _ if setting.preview => releases.insert(LATEST_RELEASE),
                        None => releases.insert(LATEST_RELEASE),
                        Some(Release::Supported(release)) => releases.insert(*release),
                        Some(Release::Unsupported(release)) => {
                            if !unsupported.contains(release) {
                                unsupported.push(release.clone());
                            }
                            false
                        }
                    };
                }
            }
        }
        inputs.push(Input {
            deep_nesting: deep.contains(&path),
            path,
            file,
            releases,
            unsupported,
            preview,
        });
    }
    let missing: Vec<&String> = deep
        .iter()
        .filter(|path| !inputs.iter().any(|input| &input.path == *path))
        .collect();
    if !missing.is_empty() {
        bail!(
            "jdk-tests-deep-nesting.txt lists files that aren't in the checkout: {missing:?}; \
             update it for {}",
            tests.tag
        );
    }
    Ok(inputs)
}

/// The paths of javac's deep-nesting stress tests, relative to the checkout, from
/// `jdk-tests-deep-nesting.txt`.
pub fn deep_nesting(dirs: &Dirs) -> Result<Vec<String>> {
    let path = dirs.tool.join("jdk-tests-deep-nesting.txt");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(text
        .lines()
        .map(|line| line.split('#').next().unwrap().trim())
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// How a file is compiled: at a release, or javac's default (the latest) when `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Setting {
    release: Option<Release>,
    preview: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Release {
    Supported(u32),
    /// A release before 8, as written.
    Unsupported(String),
}

impl Release {
    /// Reads the value of `-source` or `--release`. `None` if it isn't a release, as when the
    /// header names a variable other than `${jdk.version}`.
    fn parse(value: &str) -> Option<Release> {
        if value == "${jdk.version}" {
            return Some(Release::Supported(LATEST_RELEASE));
        }
        let release = parse_java_version(value)?;
        // `-source 6` and `-source 1.6` are both releases; `-source 1` isn't.
        if !value.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
        Some(if release >= OLDEST_RELEASE {
            Release::Supported(release)
        } else {
            Release::Unsupported(value.to_owned())
        })
    }
}

/// What a file's jtreg test descriptions, or its diagnostic example options, say about compiling.
#[derive(Debug, Default, PartialEq, Eq)]
struct Header {
    /// `@enablePreview`.
    enable_preview: bool,
    compiles: Vec<Compile>,
}

/// One `@compile` tag, or a diagnostic example's `// options:` line.
#[derive(Debug, Default, PartialEq, Eq)]
struct Compile {
    /// The `.java` files it names, relative to the test's directory. None means the file itself.
    files: Vec<String>,
    release: Option<Release>,
    preview: bool,
}

impl Header {
    fn parse(text: &str) -> Header {
        let mut header = Header::default();
        // jtreg test descriptions are block comments with an `@test` tag; a file can have several.
        let mut rest = text;
        while let Some(start) = rest.find("/*") {
            let after = &rest[start + 2..];
            let end = after.find("*/").unwrap_or(after.len());
            let comment = &after[..end];
            rest = &after[end..];
            let tags = tags(comment);
            if !tags.iter().any(|(tag, _)| *tag == "test") {
                continue;
            }
            for (tag, args) in tags {
                let name = tag.split('/').next().unwrap();
                if name == "enablePreview" {
                    // `@enablePreview false` turns it off.
                    header.enable_preview = args.first() != Some(&"false");
                } else if name == "compile" {
                    header.compiles.push(Compile::from_args(&args));
                }
            }
        }
        // javac's diagnostic examples give their options in a line comment.
        for line in text.lines() {
            if let Some(options) = line.trim_start().strip_prefix("// options:") {
                let args: Vec<&str> = options.split_whitespace().collect();
                header.compiles.push(Compile::from_args(&args));
            }
        }
        header
    }
}

impl Compile {
    fn from_args(args: &[&str]) -> Compile {
        let mut compile = Compile::default();
        let mut args = args.iter();
        while let Some(&arg) = args.next() {
            let value = match arg {
                "-source" | "--source" | "--release" | "-release" => args.next().copied(),
                _ => arg
                    .strip_prefix("--source=")
                    .or_else(|| arg.strip_prefix("--release=")),
            };
            if let Some(value) = value {
                if let Some(release) = Release::parse(value) {
                    compile.release = Some(release);
                }
            } else if arg == "--enable-preview" {
                compile.preview = true;
            } else if arg.ends_with(".java") && !arg.starts_with('-') {
                compile.files.push(arg.to_owned());
            }
        }
        compile
    }
}

/// The tags in a comment, each with its arguments: `@compile -source 8 A.java` is
/// `("compile", ["-source", "8", "A.java"])`. A tag runs until the next one.
fn tags(comment: &str) -> Vec<(&str, Vec<&str>)> {
    let mut tags: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in comment.lines() {
        let line = line.trim_start();
        let line = line.strip_prefix('*').unwrap_or(line);
        for word in line.split_whitespace() {
            match word.strip_prefix('@') {
                Some(tag) if tag.starts_with(|c: char| c.is_ascii_alphabetic()) => {
                    tags.push((tag, Vec::new()));
                }
                _ => {
                    if let Some((_, args)) = tags.last_mut() {
                        args.push(word);
                    }
                }
            }
        }
    }
    tags
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}

/// The `.java` files under `dir`. Symbolic links aren't followed.
fn java_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "java") {
                files.push(path);
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(files: &[&str], release: Option<Release>, preview: bool) -> Compile {
        Compile {
            files: files.iter().map(|&f| f.to_owned()).collect(),
            release,
            preview,
        }
    }

    #[test]
    fn pinned_to_the_latest_release() {
        let dirs = Dirs {
            tool: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            corpus: PathBuf::new(),
        };
        let tests = JdkTests::load(&dirs).unwrap();
        assert_eq!(tests.tag, format!("jdk-{LATEST_RELEASE}-ga"));
        assert_eq!(tests.commit.len(), 40);
        assert!(tests.paths.iter().all(|path| path.starts_with("test/")));
        assert_eq!(tests.sparse_patterns(), ["/test/langtools/tools/javac/"]);
        let deep = deep_nesting(&dirs).unwrap();
        assert!(deep.iter().all(|path| path.ends_with(".java")));
    }

    #[test]
    fn releases() {
        use Release::*;
        assert_eq!(Release::parse("8"), Some(Supported(8)));
        assert_eq!(Release::parse("1.8"), Some(Supported(8)));
        assert_eq!(Release::parse("17"), Some(Supported(17)));
        assert_eq!(
            Release::parse("${jdk.version}"),
            Some(Supported(LATEST_RELEASE))
        );
        assert_eq!(Release::parse("7"), Some(Unsupported("7".into())));
        assert_eq!(Release::parse("1.4"), Some(Unsupported("1.4".into())));
        assert_eq!(Release::parse("%s"), None);
        assert_eq!(Release::parse("${other}"), None);
    }

    #[test]
    fn jtreg_headers() {
        let text = "\
/*
 * Copyright notice.
 */

/*
 * @test /nodynamiccopyright/
 * @bug 1234567
 * @summary Something about @Deprecated
 * @compile/fail/ref=T.out -XDrawDiagnostics -source 8 T.java
 * @compile --release 17
 *     T.java Other.java
 * @compile --enable-preview -source ${jdk.version} T.java
 * @run main T
 */
class T {}
";
        let header = Header::parse(text);
        assert!(!header.enable_preview);
        assert_eq!(
            header.compiles,
            [
                compile(&["T.java"], Some(Release::Supported(8)), false),
                compile(
                    &["T.java", "Other.java"],
                    Some(Release::Supported(17)),
                    false
                ),
                compile(&["T.java"], Some(Release::Supported(LATEST_RELEASE)), true),
            ]
        );

        let header = Header::parse("/* @test\n * @enablePreview\n * @run main T */");
        assert!(header.enable_preview);
        assert!(header.compiles.is_empty());

        let header = Header::parse("/* @summary not a test\n * @compile -source 8 A.java */");
        assert_eq!(header, Header::default());
    }

    #[test]
    fn diagnostic_examples() {
        let text = "// key: compiler.err.feature.not.supported.in.source.plural\n\
                    // options: --release 8\n\nrecord R() {}\n";
        let header = Header::parse(text);
        assert_eq!(
            header.compiles,
            [compile(&[], Some(Release::Supported(8)), false)]
        );
    }

    #[test]
    fn paths() {
        assert_eq!(
            normalize(Path::new("/a/b/./c/../d.java")),
            PathBuf::from("/a/b/d.java")
        );
    }
}
