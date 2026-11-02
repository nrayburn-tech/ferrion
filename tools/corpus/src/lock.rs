//! Lock files: what `corpus lock` records about a project, and how it's generated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
pub use ferrion_project::model::PlatformKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::project::{Build, Project};
use crate::{Dirs, fetch, gradle, maven};

/// Version 2 added how each source set is compiled, its processor path, and the generated-source
/// roots of the build's other plugins.
const FORMAT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lock {
    pub format_version: u32,
    pub commit: String,
    /// The JDK feature version the build ran with.
    pub jdk: u32,
    pub modules: Vec<Module>,
    /// Every jar on any classpath: its path in a Maven repository, and its SHA-256.
    pub jars: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Module {
    /// The module's directory, relative to the project root (`.` for the root).
    pub path: String,
    pub source_sets: Vec<SourceSet>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSet {
    pub name: String,
    /// The release the source set is compiled for: `--release`, or else `-source`, if the build
    /// gives either.
    pub java_release: Option<u32>,
    /// `jdk` when the build compiles without `--release`, so its code sees the JDK's whole API.
    #[serde(default, skip_serializing_if = "is_default")]
    pub platform: PlatformKind,
    /// The build's javac options that change what code sees, for the JDK's own modules.
    #[serde(default, skip_serializing_if = "is_default")]
    pub options: CompileOptions,
    /// Source roots relative to the project root. Only roots that exist are recorded.
    pub source_roots: Vec<String>,
    /// Roots that the build's other plugins (not its annotation processors) generate sources
    /// into, relative to the project root, as the lock's build left them. They exist only where
    /// that build ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_source_roots: Vec<String>,
    pub classpath: Vec<ClasspathEntry>,
    /// The annotation processor path the build gives javac. `None`: processors are found on the
    /// classpath, as javac finds them without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processor_path: Option<Vec<ClasspathEntry>>,
    /// The build's `-processor`: the processors to run, instead of those the processor path
    /// registers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processors: Option<Vec<String>>,
    /// The build's `-A` options, without the `-A`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processor_options: Vec<String>,
}

/// javac options that change which names code sees. Only those for the JDK's own modules are
/// recorded: the corpus compiles every source set in the unnamed module, where the project's own
/// modules (which a build's options for a modular test compile name) don't exist.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompileOptions {
    #[serde(default, skip_serializing_if = "is_default")]
    pub enable_preview: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_modules: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_exports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_reads: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub patch_modules: BTreeMap<String, Vec<ClasspathEntry>>,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum ClasspathEntry {
    /// A jar, by its path in a Maven repository.
    Jar { jar: String },
    /// Another source set in the same project, read from source.
    SourceSet {
        module: String,
        #[serde(rename = "sourceSet")]
        source_set: String,
    },
    /// A file committed to the project, relative to the project root.
    ProjectFile { file: String },
    /// A file that isn't from a Maven repository or the project, so it can't be fetched.
    Unavailable { unavailable: String },
}

pub fn read(dirs: &Dirs, project: &Project) -> Result<Option<Lock>> {
    let path = dirs.lock_file(project);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let lock: Lock =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if lock.format_version != FORMAT_VERSION {
        bail!(
            "{}: unsupported format version {}",
            path.display(),
            lock.format_version
        );
    }
    Ok(Some(lock))
}

pub fn lock(project: &Project, dirs: &Dirs) -> Result<()> {
    let java_home = PathBuf::from(
        std::env::var_os("JAVA_HOME").context("JAVA_HOME must point to the JDK to build with")?,
    );
    let jdk = jdk_version(&java_home)?;
    if jdk != project.jdk {
        bail!(
            "{} builds with JDK {}, but JAVA_HOME is JDK {jdk}",
            project.name,
            project.jdk
        );
    }
    let checkout = fetch::checkout(project, dirs)?;
    println!("{}: running the {} build", project.name, project.build);
    let mut jars = Jars::default();
    let modules = match project.build {
        Build::Maven => maven::resolve(project, &checkout, dirs, &mut jars)?,
        Build::Gradle => gradle::resolve(&checkout, dirs, &mut jars)?,
    };
    let lock = Lock {
        format_version: FORMAT_VERSION,
        commit: project.commit.clone(),
        jdk,
        modules,
        jars: jars.0,
    };
    let path = dirs.lock_file(project);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(&lock)? + "\n")?;
    let unavailable = lock
        .modules
        .iter()
        .flat_map(|module| &module.source_sets)
        .flat_map(|source_set| &source_set.classpath)
        .filter(|entry| matches!(entry, ClasspathEntry::Unavailable { .. }))
        .count();
    println!(
        "{}: wrote {} ({} modules, {} jars, {unavailable} unavailable classpath entries)",
        project.name,
        path.display(),
        lock.modules.len(),
        lock.jars.len(),
    );
    Ok(())
}

/// The jars found while resolving a project.
#[derive(Default)]
pub struct Jars(BTreeMap<String, String>);

impl Jars {
    /// Records a jar at `file`, whose path in a Maven repository is `repository_path`.
    pub fn add(&mut self, repository_path: String, file: &Path) -> Result<ClasspathEntry> {
        if !self.0.contains_key(&repository_path) {
            self.0.insert(repository_path.clone(), sha256(file)?);
        }
        Ok(ClasspathEntry::Jar {
            jar: repository_path,
        })
    }
}

pub fn sha256(file: &Path) -> Result<String> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Reads the JDK feature version from `$JAVA_HOME/release`.
pub fn jdk_version(java_home: &Path) -> Result<u32> {
    let release = std::fs::read_to_string(java_home.join("release"))
        .with_context(|| format!("{} is not a JDK", java_home.display()))?;
    let version = release
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))
        .context("no JAVA_VERSION in the JDK's release file")?
        .trim_matches('"');
    parse_java_version(version).with_context(|| format!("unrecognized JDK version `{version}`"))
}

/// Parses a Java version as written in builds and JDKs: `17`, `1.8`, or `21.0.4`.
pub fn parse_java_version(version: &str) -> Option<u32> {
    let mut parts = version.trim().split(['.', '_', '+', '-']);
    match parts.next()?.parse().ok()? {
        1 => parts.next()?.parse().ok(),
        major => Some(major),
    }
}

/// `path` relative to `root`, with `/` separators, or `None` if it isn't under `root`.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    Some(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

/// The JDK's modules, from `java --list-modules`, by name.
pub fn jdk_modules(java_home: &Path) -> Result<std::collections::BTreeSet<String>> {
    let output = Command::new(java_home.join("bin").join("java"))
        .arg("--list-modules")
        .output()
        .with_context(|| format!("running java in {}", java_home.display()))?;
    if !output.status.success() {
        bail!("java --list-modules failed with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split('@').next())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect())
}

/// Options for the JDK's modules from a build's javac options, and the rest as given: the
/// options [`CompileOptions`] records, from `args` (each option and its value as separate
/// arguments, or joined by `=`), keeping only those whose module is one of `modules` (the
/// `ALL-` values of `--add-modules` too). `path` classifies each path of `--patch-module`.
pub fn compile_options(
    args: &[String],
    modules: &std::collections::BTreeSet<String>,
    mut path: impl FnMut(&Path) -> Result<ClasspathEntry>,
) -> Result<CompileOptions> {
    let mut options = CompileOptions::default();
    let platform = |module: &str| modules.contains(module);
    for (option, value) in javac_options(args) {
        let value = value.unwrap_or_default();
        match option.as_str() {
            "--enable-preview" => options.enable_preview = true,
            "--add-modules" => options.add_modules.extend(
                value
                    .split(',')
                    .filter(|m| platform(m) || m.starts_with("ALL-"))
                    .map(str::to_owned),
            ),
            "--add-exports" => {
                if value.split('/').next().is_some_and(platform) {
                    options.add_exports.push(value);
                }
            }
            "--add-reads" => {
                if value.split('=').next().is_some_and(platform) {
                    options.add_reads.push(value);
                }
            }
            "--patch-module" => {
                if let Some((module, paths)) = value.split_once('=')
                    && platform(module)
                {
                    let separator = if cfg!(windows) { ';' } else { ':' };
                    let entries = options.patch_modules.entry(module.to_owned()).or_default();
                    for part in paths.split(separator).filter(|p| !p.is_empty()) {
                        entries.push(path(Path::new(part))?);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(options)
}

/// The javac options that take a value, which [`javac_options`] pairs with it.
const WITH_VALUES: [&str; 20] = [
    "--release",
    "-source",
    "--source",
    "-target",
    "--target",
    "--add-modules",
    "--add-exports",
    "--add-reads",
    "--patch-module",
    "-processorpath",
    "--processor-path",
    "--processor-module-path",
    "-processor",
    "-classpath",
    "-cp",
    "--class-path",
    "-sourcepath",
    "--source-path",
    "-d",
    "-s",
];

/// javac's options in `args`, each with its value: the next argument, or what follows `=` for a
/// `--` option written `--add-reads=a=b`. Options javac doesn't take a value for, or this tool
/// doesn't know, have none. `-Akey=value` is `("-A", "key=value")`.
pub fn javac_options(args: &[String]) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(option) = arg.strip_prefix("-A") {
            found.push(("-A".to_owned(), Some(option.to_owned())));
            continue;
        }
        if arg.starts_with("--")
            && let Some((option, value)) = arg.split_once('=')
            && WITH_VALUES.contains(&option)
        {
            found.push((option.to_owned(), Some(value.to_owned())));
            continue;
        }
        if WITH_VALUES.contains(&arg.as_str()) {
            found.push((arg.clone(), args.next().cloned()));
        } else {
            found.push((arg.clone(), None));
        }
    }
    found
}

/// What a build's javac options say about the release: `--release`'s, or else `-source`'s with
/// [`PlatformKind::Jdk`], or neither.
pub fn release_options(args: &[String]) -> (Option<u32>, PlatformKind) {
    let options = javac_options(args);
    let value = |names: &[&str]| {
        options
            .iter()
            .rev()
            .find(|(option, _)| names.contains(&option.as_str()))
            .and_then(|(_, value)| value.as_deref())
            .and_then(parse_java_version)
    };
    match value(&["--release"]) {
        Some(release) => (Some(release), PlatformKind::Release),
        None => (value(&["-source", "--source"]), PlatformKind::Jdk),
    }
}

/// Runs `command`, failing if it exits unsuccessfully.
pub fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {command:?}"))?;
    if !status.success() {
        bail!("{command:?} failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_versions() {
        assert_eq!(parse_java_version("1.8"), Some(8));
        assert_eq!(parse_java_version("1.8.0_452"), Some(8));
        assert_eq!(parse_java_version("17"), Some(17));
        assert_eq!(parse_java_version("21.0.4"), Some(21));
        assert_eq!(parse_java_version("27+10"), Some(27));
        assert_eq!(parse_java_version("VERSION_17"), None);
    }

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn javac_options_with_values() {
        let options = javac_options(&args(
            "-d out --release 21 --add-reads=a.b=ALL-UNNAMED -Akey=v -g -parameters",
        ));
        let pairs: Vec<(&str, Option<&str>)> = options
            .iter()
            .map(|(o, v)| (o.as_str(), v.as_deref()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("-d", Some("out")),
                ("--release", Some("21")),
                ("--add-reads", Some("a.b=ALL-UNNAMED")),
                ("-A", Some("key=v")),
                ("-g", None),
                ("-parameters", None),
            ]
        );
        assert_eq!(
            release_options(&args("-source 1.8 -target 1.8")),
            (Some(8), PlatformKind::Jdk)
        );
        assert_eq!(
            release_options(&args("--release 17 -g")),
            (Some(17), PlatformKind::Release)
        );
        assert_eq!(release_options(&args("-g")), (None, PlatformKind::Jdk));
    }

    #[test]
    fn options_for_the_jdks_modules() {
        let modules = ["java.base", "jdk.incubator.vector"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let options = compile_options(
            &args(
                "--add-modules jdk.incubator.vector,my.module --add-modules ALL-SYSTEM \
                 --add-exports java.base/java.lang.foreign=ALL-UNNAMED \
                 --add-reads my.module=ALL-UNNAMED --patch-module my.module=src/test/java \
                 --patch-module java.base=api.jar --enable-preview",
            ),
            &modules,
            |path| {
                Ok(ClasspathEntry::ProjectFile {
                    file: path.display().to_string(),
                })
            },
        )
        .unwrap();
        assert!(options.enable_preview);
        assert_eq!(options.add_modules, ["jdk.incubator.vector", "ALL-SYSTEM"]);
        assert_eq!(
            options.add_exports,
            ["java.base/java.lang.foreign=ALL-UNNAMED"]
        );
        assert!(options.add_reads.is_empty());
        assert_eq!(
            options.patch_modules.keys().collect::<Vec<_>>(),
            ["java.base"]
        );
    }

    #[test]
    fn relative_paths() {
        let root = Path::new("/corpus/project");
        assert_eq!(
            relative(root, Path::new("/corpus/project")).as_deref(),
            Some(".")
        );
        assert_eq!(
            relative(root, Path::new("/corpus/project/a/b")).as_deref(),
            Some("a/b")
        );
        assert_eq!(relative(root, Path::new("/elsewhere")), None);
    }
}
