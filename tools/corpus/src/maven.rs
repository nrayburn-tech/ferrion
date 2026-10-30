//! Resolving Maven projects.
//!
//! The build runs to `test-compile`, so dependencies on other modules resolve to their output
//! directories. Then the dependency plugin lists each module's resolved dependencies. A second
//! invocation, which runs no build phases, writes the effective POM, which gives source roots and
//! Java releases.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use roxmltree::Node;

use crate::Dirs;
use crate::lock::{ClasspathEntry, Jars, Module, SourceSet, parse_java_version, relative, run};

const DEPENDENCY_PLUGIN: &str = "org.apache.maven.plugins:maven-dependency-plugin:3.8.1";
const HELP_PLUGIN: &str = "org.apache.maven.plugins:maven-help-plugin:3.5.1";
const DEPENDENCIES_FILE: &str = "target/ferrion-corpus-dependencies.txt";
const SCOPES: [&str; 5] = ["compile", "provided", "system", "runtime", "test"];
const MAIN_SCOPES: [&str; 3] = ["compile", "provided", "system"];

pub fn resolve(checkout: &Path, dirs: &Dirs, jars: &mut Jars) -> Result<Vec<Module>> {
    // A separate local repository keeps every jar's path in it equal to its repository path.
    let repository = dirs.corpus.join("maven-repository");
    let effective_pom = checkout.join("target/ferrion-corpus-effective-pom.xml");
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
        .arg("test-compile")
        .arg(format!("{DEPENDENCY_PLUGIN}:list"))
        .arg("-DoutputAbsoluteArtifactFilename=true")
        .arg(format!("-DoutputFile={DEPENDENCIES_FILE}")))?;
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
    for pom in poms {
        if text_at(pom, &["packaging"]).is_some_and(|packaging| packaging == "pom") {
            continue;
        }
        let output =
            text_at(pom, &["build", "directory"]).context("POM without build directory")?;
        let dir = Path::new(&output)
            .parent()
            .context("bad build directory")?
            .to_owned();
        found.push((pom, dir));
    }
    let module_dirs: Vec<PathBuf> = found.iter().map(|(_, dir)| dir.clone()).collect();

    let mut modules = Vec::new();
    for (pom, dir) in found {
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

        let main = SourceSet {
            name: "main".to_owned(),
            java_release: java_release(pom, false),
            source_roots: source_roots(checkout, pom, "sourceDirectory"),
            classpath: classpath(&MAIN_SCOPES, jars)?,
        };
        let mut test_classpath = vec![ClasspathEntry::SourceSet {
            module: module_path.clone(),
            source_set: "main".to_owned(),
        }];
        test_classpath.extend(classpath(&SCOPES, jars)?);
        let test = SourceSet {
            name: "test".to_owned(),
            java_release: java_release(pom, true),
            source_roots: source_roots(checkout, pom, "testSourceDirectory"),
            classpath: test_classpath,
        };
        modules.push(Module {
            path: module_path,
            source_sets: vec![main, test],
        });
    }
    Ok(modules)
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
