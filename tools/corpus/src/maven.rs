//! Resolving Maven projects.
//!
//! The build runs from `clean` to `test-compile`, so dependencies on other modules resolve to their
//! output directories, with the compiler plugin's debug log on: each execution logs its source
//! roots and the javac options it runs with, which give each source set's release, the options
//! that change what its code sees, its processor path, and the roots other plugins generated
//! sources into (`clean` makes every execution compile, so every one logs them). Then the
//! dependency plugin lists each module's resolved dependencies. A second invocation, which runs
//! no build phases, writes the effective POM, which gives the modules, their output directories,
//! and the releases of source sets with nothing to compile.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use roxmltree::Node;

use crate::lock::{
    ClasspathEntry, CompileOptions, Jars, Module, PlatformKind, SourceSet, compile_options,
    javac_options, jdk_modules, module_path, parse_java_version, relative, release_options, run,
};
use crate::project::Project;
use crate::{Dirs, fetch};

const DEPENDENCY_PLUGIN: &str = "org.apache.maven.plugins:maven-dependency-plugin:3.8.1";
const HELP_PLUGIN: &str = "org.apache.maven.plugins:maven-help-plugin:3.5.1";
const DEPENDENCIES_FILE: &str = "target/ferrion-corpus-dependencies.txt";
const SCOPES: [&str; 5] = ["compile", "provided", "system", "runtime", "test"];
const MAIN_SCOPES: [&str; 3] = ["compile", "provided", "system"];

/// The compiler plugin's logger, whose debug level logs each execution's javac options.
const COMPILER_LOGGER: &str = "org.apache.maven.plugin.compiler";

/// Turns off the build caches of the Develocity Maven extension and of its predecessor.
const BUILD_CACHE_OFF: [&str; 4] = [
    "-Ddevelocity.cache.local.enabled=false",
    "-Ddevelocity.cache.remote.enabled=false",
    "-Dgradle.cache.local.enabled=false",
    "-Dgradle.cache.remote.enabled=false",
];

pub fn resolve(
    project: &Project,
    checkout: &Path,
    dirs: &Dirs,
    jars: &mut Jars,
) -> Result<Vec<Module>> {
    // A separate local repository keeps every jar's path in it equal to its repository path.
    let repository = dirs.corpus.join("maven-repository");
    let effective_pom = checkout.join("target/ferrion-corpus-effective-pom.xml");
    let log = dirs
        .corpus
        .join("logs")
        .join(format!("{}-lock.log", project.name));
    std::fs::create_dir_all(log.parent().unwrap())?;
    let java_home = PathBuf::from(std::env::var_os("JAVA_HOME").context("JAVA_HOME is set")?);
    let modules = jdk_modules(&java_home)?;
    let mvn = || {
        let mut command = Command::new("mvn");
        command
            .current_dir(checkout)
            .args(["--batch-mode", "--no-transfer-progress"])
            .arg(format!("-Dmaven.repo.local={}", repository.display()));
        command
    };
    run(mvn()
        .arg("-DskipTests")
        .arg(format!(
            "-Dorg.slf4j.simpleLogger.log.{COMPILER_LOGGER}=debug"
        ))
        // A build cache (Spring's builds use Develocity's) would skip the compiler, which then
        // logs nothing.
        .args(BUILD_CACHE_OFF)
        .args(["clean", "test-compile"])
        .arg(format!("{DEPENDENCY_PLUGIN}:list"))
        .arg("-DoutputAbsoluteArtifactFilename=true")
        .arg(format!("-DoutputFile={DEPENDENCIES_FILE}"))
        .stdout(File::create(&log)?))
    .with_context(|| format!("the build's log is {}", log.display()))?;
    let executions = read_executions(&std::fs::read_to_string(&log)?);
    // Plugins that run during the build can add properties to the model, and their names aren't
    // always valid XML element names (build-helper's `parse-version` adds `<prefix>.qualifier?`).
    run(mvn()
        .arg(format!("{HELP_PLUGIN}:effective-pom"))
        .arg(format!("-Doutput={}", effective_pom.display())))?;

    // The help plugin writes each POM's own XML declaration after its header comment, which makes
    // the file invalid XML.
    let text = remove_repeated_declarations(&std::fs::read_to_string(&effective_pom)?);
    let document = roxmltree::Document::parse(&text).context("parsing the effective POM")?;
    let root = document.root_element();
    let poms: Vec<Node> = if root.has_tag_name("projects") {
        root.children()
            .filter(|node| node.has_tag_name("project"))
            .collect()
    } else {
        vec![root]
    };

    let mut found = Vec::new();
    // Every module's directory and coordinates, aggregators included, for processor paths.
    let mut coordinates = HashMap::new();
    for pom in poms {
        let output =
            text_at(pom, &["build", "directory"]).context("POM without build directory")?;
        let dir = Path::new(&output)
            .parent()
            .context("bad build directory")?
            .to_owned();
        if let (Some(group), Some(artifact), Some(version)) = (
            text_at(pom, &["groupId"]),
            text_at(pom, &["artifactId"]),
            text_at(pom, &["version"]),
        ) {
            coordinates.insert(dir.clone(), (group, artifact, version));
        }
        if text_at(pom, &["packaging"]).is_some_and(|packaging| packaging == "pom") {
            continue;
        }
        found.push((pom, dir, PathBuf::from(output)));
    }
    let module_dirs: Vec<PathBuf> = found.iter().map(|(_, dir, _)| dir.clone()).collect();
    let context = Build {
        project,
        checkout,
        dirs,
        module_dirs: &module_dirs,
        repository: &repository,
        coordinates: &coordinates,
        modules: &modules,
    };

    let mut modules = Vec::new();
    for (pom, dir, build_dir) in found {
        let module_path = relative(checkout, &dir).with_context(|| {
            format!(
                "module {} is outside the checkout {}",
                dir.display(),
                checkout.display()
            )
        })?;
        let dependencies = read_dependencies(&dir.join(DEPENDENCIES_FILE))?;
        let classpath = |scopes: &[&str], jars: &mut Jars| -> Result<Vec<ClasspathEntry>> {
            let mut entries = Vec::new();
            for (scope, file) in &dependencies {
                if scopes.contains(&scope.as_str()) {
                    entries.push(classify(file, checkout, &module_dirs, &repository, jars)?);
                }
            }
            Ok(entries)
        };

        let main = context.source_set(
            "main",
            pom,
            &build_dir,
            executions_for(&executions, pom, "outputDirectory"),
            classpath(&MAIN_SCOPES, jars)?,
            jars,
        )?;
        let mut test_classpath = vec![ClasspathEntry::SourceSet {
            module: module_path.clone(),
            source_set: "main".to_owned(),
        }];
        test_classpath.extend(classpath(&SCOPES, jars)?);
        let test = context.source_set(
            "test",
            pom,
            &build_dir,
            executions_for(&executions, pom, "testOutputDirectory"),
            test_classpath,
            jars,
        )?;
        modules.push(Module {
            path: module_path,
            source_sets: vec![main, test],
        });
    }
    Ok(modules)
}

/// One compiler plugin execution, from the build's log.
#[derive(Debug, Default, PartialEq, Eq)]
struct Execution {
    /// `compile (default-compile)`, as the log names it.
    name: String,
    /// The source roots it logs, which include roots other plugins added.
    source_roots: Vec<PathBuf>,
    /// The javac options it logs, split at spaces as logged. Empty when it had nothing to
    /// compile.
    options: Vec<String>,
    /// How many files it compiled, from `Compiling N source files`.
    files: usize,
}

impl Execution {
    fn option(&self, names: &[&str]) -> Option<String> {
        javac_options(&self.options)
            .into_iter()
            .rev()
            .find(|(option, _)| names.contains(&option.as_str()))
            .and_then(|(_, value)| value)
    }

    fn has(&self, name: &str) -> bool {
        self.options.iter().any(|option| option == name)
    }
}

/// The compiler plugin's executions in the build's log, in order. An execution starts at its
/// `--- compiler:<version>:<goal> (<id>) @ <artifact> ---` line; with the plugin's debug log on,
/// it logs `Source roots:` and its roots, one per line, and `Command line options:` followed by
/// a line with the options.
fn read_executions(log: &str) -> Vec<Execution> {
    let mut executions: Vec<Execution> = Vec::new();
    let mut lines = log.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("[INFO] --- ")
            && (rest.starts_with("compiler:") || rest.starts_with("maven-compiler-plugin:"))
        {
            let name = rest.split_once(':').map_or(rest, |(_, r)| r);
            let name = name.split_once(':').map_or(name, |(_, r)| r);
            let name = name.split(" @ ").next().unwrap_or(name);
            executions.push(Execution {
                name: name.to_owned(),
                ..Execution::default()
            });
            continue;
        }
        let Some(execution) = executions.last_mut() else {
            continue;
        };
        if line == "[DEBUG] Source roots:" {
            while let Some(root) = lines.peek().and_then(|l| l.strip_prefix("[DEBUG]  ")) {
                execution.source_roots.push(PathBuf::from(root.trim()));
                lines.next();
            }
        } else if line == "[DEBUG] Command line options:" {
            if let Some(options) = lines.next().and_then(|l| l.strip_prefix("[DEBUG] ")) {
                execution.options = options.split(' ').map(str::to_owned).collect();
            }
        } else if let Some(rest) = line.strip_prefix("[INFO] Compiling ")
            && let Some(count) = rest.split(' ').next().and_then(|n| n.parse().ok())
        {
            execution.files = count;
        }
    }
    executions
}

/// The execution that compiled into the POM's `build/<directory>`, by its `-d`: of several, the
/// one that compiled the most files (Guava compiles `module-info.java` alone into the same
/// directory), then a `default-` one.
fn executions_for<'a>(
    executions: &'a [Execution],
    pom: Node,
    directory: &str,
) -> Option<&'a Execution> {
    let output = PathBuf::from(text_at(pom, &["build", directory])?);
    executions
        .iter()
        .filter(|e| e.option(&["-d"]).is_some_and(|d| Path::new(&d) == output))
        .max_by_key(|e| (e.files, e.name.contains("(default-")))
}

/// What classifying a module's dependencies and its executions needs.
struct Build<'a> {
    project: &'a Project,
    checkout: &'a Path,
    dirs: &'a Dirs,
    module_dirs: &'a [PathBuf],
    repository: &'a Path,
    /// Each module's `groupId`, `artifactId`, and `version`, by its directory.
    coordinates: &'a HashMap<PathBuf, (String, String, String)>,
    /// The JDK's modules.
    modules: &'a BTreeSet<String>,
}

impl Build<'_> {
    /// The source set `name` of a module, from the execution that compiled it, or from the POM
    /// alone when nothing was compiled.
    fn source_set(
        &self,
        name: &str,
        pom: Node,
        build_dir: &Path,
        execution: Option<&Execution>,
        classpath: Vec<ClasspathEntry>,
        jars: &mut Jars,
    ) -> Result<SourceSet> {
        let test = name == "test";
        let pom_roots = source_roots(
            self.checkout,
            pom,
            if test {
                "testSourceDirectory"
            } else {
                "sourceDirectory"
            },
        );
        let Some(execution) = execution.filter(|e| !e.options.is_empty()) else {
            return Ok(SourceSet {
                name: name.to_owned(),
                java_release: java_release(pom, test),
                platform: PlatformKind::default(),
                options: CompileOptions::default(),
                source_roots: pom_roots,
                generated_source_roots: Vec::new(),
                classpath,
                processor_path: None,
                processors: None,
                processor_options: Vec::new(),
                module_path: Vec::new(),
            });
        };
        let (release, platform) = release_options(&execution.options);
        // Roots in the build directory are generated; the processors' own (`-s`) isn't one, since
        // the oracle runs the processors.
        let processor_output = execution.option(&["-s"]).map(PathBuf::from);
        let mut source_roots = Vec::new();
        let mut generated_source_roots = Vec::new();
        for root in &execution.source_roots {
            if !root.is_dir() || Some(root) == processor_output.as_ref() {
                continue;
            }
            let Some(path) = relative(self.checkout, root) else {
                continue;
            };
            if root.starts_with(build_dir) {
                generated_source_roots.push(path);
            } else {
                source_roots.push(path);
            }
        }
        if source_roots.is_empty() && generated_source_roots.is_empty() {
            source_roots = pom_roots;
        }
        let processor_path = match execution.option(&["-processorpath", "--processor-path"]) {
            Some(path) => Some(
                std::env::split_paths(&path)
                    .filter(|entry| !entry.as_os_str().is_empty())
                    .map(|entry| self.processor_entry(&entry, jars))
                    .collect::<Result<_>>()?,
            ),
            None if execution.has("-proc:none") => Some(Vec::new()),
            None => None,
        };
        let processors = execution
            .option(&["-processor"])
            .map(|names| names.split(',').map(str::to_owned).collect());
        let processor_options = javac_options(&execution.options)
            .into_iter()
            .filter(|(option, _)| option == "-A")
            .filter_map(|(_, value)| value)
            .collect();
        let options = compile_options(&execution.options, self.modules, |path| {
            self.classify(path, jars)
        })?;
        let module_path = module_path(&execution.options, |path| {
            self.classify(path, jars).map(Some)
        })?;
        Ok(SourceSet {
            name: name.to_owned(),
            java_release: release.or_else(|| java_release(pom, test)),
            platform,
            options,
            source_roots,
            generated_source_roots,
            classpath,
            processor_path,
            processors,
            processor_options,
            module_path,
        })
    }

    fn classify(&self, file: &Path, jars: &mut Jars) -> Result<ClasspathEntry> {
        classify(file, self.checkout, self.module_dirs, self.repository, jars)
    }

    /// An entry of a processor path. Another module's output is that module's published jar,
    /// downloaded from the project's repositories, so that fetching the corpus gives the oracle
    /// the processors without building the project: the jar of a release is built from the
    /// same commit. When it isn't published, it's the module's source set, which the oracle
    /// can't run.
    fn processor_entry(&self, file: &Path, jars: &mut Jars) -> Result<ClasspathEntry> {
        let entry = self.classify(file, jars)?;
        let ClasspathEntry::SourceSet { module, .. } = &entry else {
            return Ok(entry);
        };
        let Some((group, artifact, version)) = self.coordinates.get(&self.checkout.join(module))
        else {
            return Ok(entry);
        };
        let path = format!(
            "{}/{artifact}/{version}/{artifact}-{version}.jar",
            group.replace('.', "/")
        );
        let file = self.dirs.jars().join(&path);
        if !file.exists() && fetch::download_any(self.project, &path, &file).is_err() {
            println!(
                "{}: {group}:{artifact}:{version} isn't published; its processors are left out",
                self.project.name
            );
            return Ok(entry);
        }
        jars.add(path, &file)
    }
}

/// Removes every XML declaration after the first.
fn remove_repeated_declarations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut seen = false;
    while let Some(start) = rest.find("<?xml ") {
        let Some(length) = rest[start..].find("?>") else {
            break;
        };
        let end = start + length + 2;
        out.push_str(&rest[..start]);
        if !seen {
            out.push_str(&rest[start..end]);
            seen = true;
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Reads the dependency plugin's output: `(scope, file)` for each dependency, in classpath order.
fn read_dependencies(path: &Path) -> Result<Vec<(String, PathBuf)>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut dependencies = Vec::new();
    for line in text.lines() {
        // `group:artifact:type[:classifier]:version:scope:/path/to/file[ (optional)][ -- module name]`
        let Some((index, scope)) = SCOPES
            .iter()
            .filter_map(|scope| {
                line.find(&format!(":{scope}:"))
                    .map(|index| (index, *scope))
            })
            .min()
        else {
            continue;
        };
        let file = &line[index + scope.len() + 2..];
        let file = file.split(" -- ").next().unwrap().trim();
        let file = file.strip_suffix(" (optional)").unwrap_or(file);
        dependencies.push((scope.to_owned(), PathBuf::from(file)));
    }
    Ok(dependencies)
}

fn classify(
    file: &Path,
    checkout: &Path,
    module_dirs: &[PathBuf],
    repository: &Path,
    jars: &mut Jars,
) -> Result<ClasspathEntry> {
    // Another module's build output is recorded as that module's source set.
    if let Some(dir) = module_dirs
        .iter()
        .filter(|dir| file.starts_with(dir))
        .max_by_key(|dir| dir.components().count())
        && let Ok(output) = file.strip_prefix(dir.join("target"))
    {
        let name = output.to_string_lossy();
        let source_set = match name.as_ref() {
            "classes" => Some("main"),
            "test-classes" => Some("test"),
            _ if name.ends_with("-tests.jar") => Some("test"),
            _ if name.ends_with(".jar") && !name.contains('/') => Some("main"),
            _ => None,
        };
        if let Some(source_set) = source_set {
            return Ok(ClasspathEntry::SourceSet {
                module: relative(checkout, dir).unwrap(),
                source_set: source_set.to_owned(),
            });
        }
    }
    if let Some(path) = relative(checkout, file) {
        return Ok(ClasspathEntry::ProjectFile { file: path });
    }
    match relative(repository, file) {
        Some(path) => jars.add(path, file),
        None => Ok(ClasspathEntry::Unavailable {
            unavailable: file.display().to_string(),
        }),
    }
}

fn source_roots(checkout: &Path, pom: Node, element: &str) -> Vec<String> {
    text_at(pom, &["build", element])
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .and_then(|dir| relative(checkout, &dir))
        .into_iter()
        .collect()
}

/// The Java release, from the compiler plugin's configuration or the properties it reads.
fn java_release(pom: Node, test: bool) -> Option<u32> {
    let (execution, keys): (&str, &[&str]) = if test {
        (
            "default-testCompile",
            &["testRelease", "release", "testSource", "source"],
        )
    } else {
        ("default-compile", &["release", "source"])
    };
    let plugin = children(pom, &["build", "plugins"])
        .into_iter()
        .find(|plugin| {
            text_at(*plugin, &["artifactId"]).as_deref() == Some("maven-compiler-plugin")
        });
    let mut configurations = Vec::new();
    if let Some(plugin) = plugin {
        for candidate in children(plugin, &["executions"]) {
            if text_at(candidate, &["id"]).as_deref() == Some(execution) {
                configurations.extend(child(candidate, "configuration"));
            }
        }
        configurations.extend(child(plugin, "configuration"));
    }
    for key in keys {
        for configuration in &configurations {
            if let Some(release) =
                text_at(*configuration, &[key]).and_then(|v| parse_java_version(&v))
            {
                return Some(release);
            }
        }
    }
    keys.iter().find_map(|key| {
        text_at(pom, &["properties", &format!("maven.compiler.{key}")])
            .and_then(|value| parse_java_version(&value))
    })
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(name))
}

fn children<'a, 'input>(node: Node<'a, 'input>, path: &[&str]) -> Vec<Node<'a, 'input>> {
    let mut node = Some(node);
    for name in path {
        node = node.and_then(|node| child(node, name));
    }
    node.map(|node| node.children().filter(Node::is_element).collect())
        .unwrap_or_default()
}

fn text_at(node: Node, path: &[&str]) -> Option<String> {
    let mut node = node;
    for name in path {
        node = child(node, name)?;
    }
    node.text().map(|text| text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executions_in_the_log() {
        let log = "\
[INFO] --- compiler:3.14.0:compile (default-compile) @ a ---
[DEBUG] Source roots:
[DEBUG]  /p/a/src/main/java
[DEBUG]  /p/a/target/generated-sources/templates
[DEBUG] Command line options:
[DEBUG] -d /p/a/target/classes -s /p/a/target/generated-sources/annotations --release 21
[INFO] Compiling 12 source files with javac [debug release 21] to target/classes
[INFO] --- compiler:3.14.0:testCompile (default-testCompile) @ a ---
[INFO] No sources to compile
";
        let executions = read_executions(log);
        assert_eq!(executions.len(), 2);
        let main = &executions[0];
        assert_eq!(main.name, "compile (default-compile)");
        assert_eq!(
            main.source_roots,
            [
                PathBuf::from("/p/a/src/main/java"),
                PathBuf::from("/p/a/target/generated-sources/templates")
            ]
        );
        assert_eq!(main.option(&["-d"]).as_deref(), Some("/p/a/target/classes"));
        assert_eq!(main.files, 12);
        assert!(executions[1].options.is_empty());
    }

    #[test]
    fn repeated_declarations() {
        let text =
            "<?xml version=\"1.0\"?>\n<!-- header -->\n<?xml version=\"1.0\"?>\n<project/>\n";
        let fixed = remove_repeated_declarations(text);
        assert_eq!(
            fixed,
            "<?xml version=\"1.0\"?>\n<!-- header -->\n\n<project/>\n"
        );
        assert!(roxmltree::Document::parse(&fixed).is_ok());
        assert_eq!(remove_repeated_declarations("<project/>"), "<project/>");
    }
}
