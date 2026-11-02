//! The source sets to compile, from each corpus project's project model
//! (docs/design/project-inputs.md, `corpus::model`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use corpus::Dirs;
use corpus::model::Processors;
use corpus::project::Project;
use ferrion_project::model::{ModelEntry, ModelSourceSet, PlatformKind};

use crate::inventory::java_files;

/// One source set, as the oracle compiles it.
#[derive(Clone, Debug)]
pub struct SourceSet {
    pub project: String,
    /// The model's id: `<module path>/<name>`, or the name alone for the root module.
    pub id: String,
    pub release: u32,
    /// `Jdk` when the build compiles without `--release`: the oracle passes `-source` and
    /// `-target` instead.
    pub platform: PlatformKind,
    /// The build's options for the JDK's modules, as javac takes them, in order.
    pub javac: Vec<String>,
    pub roots: Vec<PathBuf>,
    /// The roots the build's other plugins generated sources into, which exist where the lock's
    /// build ran.
    pub generated_roots: Vec<PathBuf>,
    /// The `.java` files under the roots and generated roots, sorted. A file under two roots is
    /// listed once.
    pub files: Vec<PathBuf>,
    /// Jars and directories, in classpath order.
    pub classpath: Vec<PathBuf>,
    /// The build's module path, for the modular compilation of `module-info.java`; empty when
    /// the build gives none, and then the classpath's jars stand in.
    pub module_path: Vec<PathBuf>,
    /// The build's processor path, or `None` when processors are found on the classpath.
    pub processor_path: Option<Vec<PathBuf>>,
    /// The build's `-processor` and `-A` options.
    pub processors: Processors,
    /// The source sets this one reads from source, in classpath order, by index in the plan.
    pub dependencies: Vec<usize>,
    /// Where annotation processors write sources: the model's generated-source root,
    /// `generated/<project>/<id>` in the corpus directory.
    pub generated: PathBuf,
    /// Classpath entries left out: files that aren't jars (a `.pom` a build lists), source sets
    /// the lock names but doesn't have, and processor path entries that are source sets.
    pub left_out: Vec<String>,
}

impl SourceSet {
    /// The path reports use: the project, then the id.
    pub fn display(&self) -> String {
        format!("{}:{}", self.project, self.id)
    }

    /// Whether `file` is under one of the roots the build's other plugins generate sources into.
    pub fn is_generated(&self, file: &std::path::Path) -> bool {
        self.generated_roots
            .iter()
            .any(|root| file.starts_with(root))
    }

    /// Whether the source set has a module declaration, which the oracle compiles on its own.
    pub fn is_modular(&self) -> bool {
        self.files
            .iter()
            .any(|file| file.file_name().is_some_and(|n| n == "module-info.java"))
    }
}

/// javac's options for a model source set's compile options, in the order javac takes them.
fn javac_options(set: &ModelSourceSet) -> Vec<String> {
    let mut options = Vec::new();
    if set.enable_preview {
        options.push("--enable-preview".to_owned());
    }
    for module in &set.add_modules {
        options.extend(["--add-modules".to_owned(), module.clone()]);
    }
    for export in &set.add_exports {
        options.extend(["--add-exports".to_owned(), export.clone()]);
    }
    for reads in &set.add_reads {
        options.extend(["--add-reads".to_owned(), reads.clone()]);
    }
    for (module, paths) in &set.patch_modules {
        let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        let separator = if cfg!(windows) { ";" } else { ":" };
        options.extend([
            "--patch-module".to_owned(),
            format!("{module}={}", paths.join(separator)),
        ]);
    }
    options
}

/// The source sets of `project`'s model, in its order.
pub fn source_sets(dirs: &Dirs, project: &Project) -> Result<Vec<SourceSet>> {
    let (model, missing) = corpus::model::project_model(dirs, project, None)?;
    let mut processors = corpus::model::processors(dirs, project)?;
    let index: HashMap<&str, usize> = model
        .source_sets
        .iter()
        .enumerate()
        .map(|(i, set)| (set.id.as_str(), i))
        .collect();
    let mut sets = Vec::new();
    for set in &model.source_sets {
        // The first generated-source root is the oracle's own, for what processors generate.
        let generated = corpus::model::generated_root(dirs, project, &set.id);
        let generated_roots: Vec<PathBuf> = set
            .generated_source_roots
            .iter()
            .filter(|root| **root != generated)
            .cloned()
            .collect();
        let mut files = Vec::new();
        for root in set
            .source_roots
            .iter()
            .chain(&generated_roots)
            .filter(|root| root.is_dir())
        {
            files.extend(java_files(root)?);
        }
        files.sort();
        files.dedup();
        let mut classpath = Vec::new();
        let mut dependencies = Vec::new();
        let mut left_out: Vec<String> = missing
            .missing_source_sets
            .iter()
            .filter(|(of, _)| *of == set.id)
            .map(|(_, other)| format!("source set {other}, which the lock doesn't have"))
            .collect();
        for entry in &set.classpath {
            if let Some(id) = &entry.source_set {
                dependencies.push(index[id.as_str()]);
            } else if let Some(path) = &entry.path {
                let archive = path.extension().is_some_and(|e| e == "jar" || e == "zip");
                if archive || path.is_dir() {
                    classpath.push(path.clone());
                } else {
                    left_out.push(path.display().to_string());
                }
            }
        }
        let processor_path = set.processor_path.as_ref().map(|path| {
            path.iter()
                .filter_map(
                    |entry: &ModelEntry| match (&entry.path, &entry.source_set) {
                        (Some(path), _) => Some(path.clone()),
                        (None, Some(other)) => {
                            left_out.push(format!(
                                "source set {other} on the processor path, which isn't published"
                            ));
                            None
                        }
                        (None, None) => None,
                    },
                )
                .collect()
        });
        sets.push(SourceSet {
            project: project.name.clone(),
            id: set.id.clone(),
            release: set.java_release,
            platform: set.platform,
            javac: javac_options(set),
            roots: set.source_roots.clone(),
            generated_roots,
            files,
            classpath,
            module_path: set.module_path.clone(),
            processor_path,
            processors: processors.remove(&set.id).unwrap_or_default(),
            dependencies,
            generated,
            left_out,
        });
    }
    Ok(sets)
}

/// The order to compile `sets` in: each after the source sets it depends on, so their generated
/// sources exist when it reads them. A cycle, which the model shouldn't have, is broken where it's
/// found.
pub fn order(sets: &[SourceSet]) -> Vec<usize> {
    fn visit(i: usize, sets: &[SourceSet], state: &mut [u8], order: &mut Vec<usize>) {
        // 0: not visited, 1: in progress, 2: done.
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        for &dependency in &sets[i].dependencies {
            visit(dependency, sets, state, order);
        }
        state[i] = 2;
        order.push(i);
    }
    let mut state = vec![0; sets.len()];
    let mut order = Vec::with_capacity(sets.len());
    for i in 0..sets.len() {
        visit(i, sets, &mut state, &mut order);
    }
    order
}

/// The source sets `set` reads from source: its dependencies in classpath order, then theirs that
/// it doesn't list, and so on.
fn closure(sets: &[SourceSet], set: &SourceSet) -> Vec<usize> {
    let mut order = set.dependencies.clone();
    let mut seen: HashSet<usize> = order.iter().copied().collect();
    let mut next = 0;
    while next < order.len() {
        for &dependency in &sets[order[next]].dependencies {
            if seen.insert(dependency) {
                order.push(dependency);
            }
        }
        next += 1;
    }
    order
}

/// The classpath a source set is compiled with: its own, then each entry of the source sets it
/// reads from source that it doesn't have. javac compiles the sources on the source path against
/// this one classpath, while a dependency's own classpath can have entries its dependents' don't
/// (an optional dependency, or one only its processors use); without them, javac reports errors
/// in the dependency's files and, once processors run, attributes nothing. Appended last, they
/// change nothing that resolves in the source set's own classpath.
pub fn class_path(sets: &[SourceSet], set: &SourceSet) -> Vec<PathBuf> {
    let mut path = set.classpath.clone();
    let mut seen: HashSet<PathBuf> = path.iter().cloned().collect();
    for dependency in closure(sets, set) {
        for entry in &sets[dependency].classpath {
            if seen.insert(entry.clone()) {
                path.push(entry.clone());
            }
        }
    }
    path
}

/// For the modular compilation of a source set's `module-info.java`: the roots of each source set
/// it reads from source, each with its generated sources, as one module's (the oracle uses those
/// with a `module-info.java`).
pub fn module_sources(sets: &[SourceSet], set: &SourceSet) -> Vec<Vec<PathBuf>> {
    closure(sets, set)
        .into_iter()
        .map(|dependency| {
            let dependency = &sets[dependency];
            let mut roots: Vec<PathBuf> = dependency
                .roots
                .iter()
                .chain(&dependency.generated_roots)
                .chain([&dependency.generated])
                .filter(|root| root.is_dir())
                .cloned()
                .collect();
            roots.dedup();
            roots
        })
        .filter(|roots| !roots.is_empty())
        .collect()
}

/// The source path a source set is compiled with: the roots and generated sources of the source
/// sets it depends on, then, for the same reason as [`class_path`], those of their dependencies it
/// doesn't list.
pub fn source_path(sets: &[SourceSet], set: &SourceSet) -> Vec<PathBuf> {
    let mut path = Vec::new();
    for dependency in closure(sets, set) {
        let dependency = &sets[dependency];
        path.extend(dependency.roots.iter().cloned());
        path.extend(dependency.generated_roots.iter().cloned());
        path.push(dependency.generated.clone());
    }
    path.retain(|p: &PathBuf| p.is_dir());
    let mut seen = HashSet::new();
    path.retain(|p| seen.insert(p.clone()));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(id: &str, dependencies: Vec<usize>) -> SourceSet {
        SourceSet {
            project: "p".into(),
            id: id.into(),
            release: 21,
            platform: PlatformKind::Release,
            javac: Vec::new(),
            roots: Vec::new(),
            generated_roots: Vec::new(),
            files: Vec::new(),
            classpath: Vec::new(),
            module_path: Vec::new(),
            processor_path: None,
            processors: Processors::default(),
            dependencies,
            generated: PathBuf::new(),
            left_out: Vec::new(),
        }
    }

    #[test]
    fn dependencies_come_first() {
        // test depends on main and b; b depends on main; a cycle between c and d.
        let sets = vec![
            set("test", vec![1, 2]),
            set("b", vec![2]),
            set("main", vec![]),
            set("c", vec![4]),
            set("d", vec![3]),
        ];
        let order = order(&sets);
        assert_eq!(&order[..3], [2, 1, 0]);
        assert_eq!(order.len(), 5);
    }

    #[test]
    fn options_as_javac_takes_them() {
        let set = ModelSourceSet {
            enable_preview: true,
            add_modules: vec!["jdk.incubator.vector".into()],
            add_exports: vec!["java.base/jdk.internal.misc=ALL-UNNAMED".into()],
            patch_modules: [("java.base".to_owned(), vec![PathBuf::from("/a.jar")])]
                .into_iter()
                .collect(),
            ..ModelSourceSet::default()
        };
        assert_eq!(
            javac_options(&set),
            [
                "--enable-preview",
                "--add-modules",
                "jdk.incubator.vector",
                "--add-exports",
                "java.base/jdk.internal.misc=ALL-UNNAMED",
                "--patch-module",
                "java.base=/a.jar"
            ]
        );
    }

    #[test]
    fn dependencies_classpaths_come_last() {
        // test depends on main, which depends on api, which test doesn't list.
        let mut api = set("api", vec![]);
        api.classpath = vec!["api-only.jar".into()];
        let mut main = set("main", vec![0]);
        main.classpath = vec!["a.jar".into(), "optional.jar".into()];
        let mut test = set("test", vec![1]);
        test.classpath = vec!["junit.jar".into(), "a.jar".into()];
        let sets = vec![api, main, test];
        assert_eq!(closure(&sets, &sets[2]), [1, 0]);
        assert_eq!(
            class_path(&sets, &sets[2]),
            [
                PathBuf::from("junit.jar"),
                "a.jar".into(),
                "optional.jar".into(),
                "api-only.jar".into()
            ]
        );
    }
}
