//! Which benchmarks a change can affect, so that CI runs only those (docs/design/benchmarks.md).
//!
//! A benchmark measures one component's crate, so a change to code that crate isn't built from
//! can't change what it measures. `benchmark affected <base>` compares the working tree with the
//! base commit, and prints the components whose benchmarks to run as filters for `run`:
//!
//! - a file in a package the harness is built from runs the benchmarks of every component built
//!   from that package, or every benchmark when no component is (the harness itself, and
//!   `corpus`, which chooses the inputs). Its `tests`, `benches`, and `examples` directories
//!   don't count, since only test targets are built from them.
//! - a dependency that `Cargo.lock` locks at a version the base didn't, and an entry of the root
//!   manifest's `[workspace.dependencies]` that changed, count as that package.
//! - the toolchain, `.cargo/config.toml`, the bench job, and the rest of the root manifest (such
//!   as its profiles) run every benchmark. The workspace's members and lints don't change what's
//!   built.
//! - anything else runs nothing: documentation, the other tools, and crates the harness isn't
//!   built from.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Each component, as its benchmarks' names begin (`<component>/...`), and the package it
/// measures.
pub const COMPONENTS: &[(&str, &str)] = &[
    ("lexer", "ferrion_lexer"),
    ("parser", "ferrion_parser"),
    ("formatter", "ferrion_formatter"),
    ("classfile", "ferrion_classfile"),
    ("semantic", "ferrion_semantic"),
];

/// Files outside every package that change how every benchmark is built or run.
const EVERYTHING: &[&str] = &[
    "rust-toolchain.toml",
    ".cargo/config.toml",
    ".github/workflows/ci.yml",
];

/// A package's directories that only its test targets are built from.
const NOT_BUILT: &[&str] = &["tests", "benches", "examples"];

/// The root manifest's keys that don't change what's built, besides
/// `[workspace.dependencies]`, which is compared entry by entry.
const IGNORED_WORKSPACE_KEYS: &[&str] =
    &["members", "default-members", "exclude", "lints", "metadata"];

/// Prints the filters that select the benchmarks a change from `base` to the working tree can
/// affect, one per line, and why each is selected on stderr. Prints nothing when none is.
pub fn run(base: &str) -> Result<()> {
    let graph = Graph::load()?;
    let root = &graph.root;
    let mut changes = Vec::new();

    let output = git(
        root,
        &["diff", "--name-only", "--no-renames", "-z", base, "--"],
    )?;
    for path in output.split('\0').filter(|path| !path.is_empty()) {
        if let Some(touched) = graph.touched(path) {
            changes.push((path.to_owned(), touched));
        }
    }

    let base_lock = git(root, &["show", &format!("{base}:Cargo.lock")])
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok());
    match base_lock {
        Some(lock) => changes.extend(graph.dependencies_not_in(&lock)),
        None => changes.push(("Cargo.lock".to_owned(), Touched::Everything)),
    }

    let head_manifest = read_manifest(&std::fs::read_to_string(root.join("Cargo.toml"))?)?;
    let base_manifest = read_manifest(&git(root, &["show", &format!("{base}:Cargo.toml")])?)?;
    changes.extend(graph.manifest_changes(&base_manifest, &head_manifest));

    let mut selected = Vec::new();
    for (what, touched) in &changes {
        let components = graph.affected(touched);
        if !components.is_empty() {
            eprintln!("{what}: {}", components.join(", "));
        }
        selected.extend(components);
    }
    for (component, _) in COMPONENTS {
        if selected.contains(component) {
            println!("{component}/");
        }
    }
    if selected.is_empty() {
        eprintln!("No change affects a benchmark.");
    }
    Ok(())
}

/// What a change touches.
#[derive(Debug, PartialEq)]
enum Touched {
    /// Packages, by their ids in `cargo metadata`.
    Packages(Vec<String>),
    Everything,
}

/// The packages the harness is built from, and which of them each component is.
struct Graph {
    /// The workspace's root directory.
    root: PathBuf,
    /// By id.
    packages: HashMap<String, Package>,
    /// Each component, and the ids of the packages its crate is built from, its own included.
    components: Vec<(&'static str, HashSet<String>)>,
}

struct Package {
    name: String,
    version: String,
    /// A workspace member's directory, relative to the root, with `/` between components; `None`
    /// for a dependency.
    dir: Option<String>,
}

impl Graph {
    /// Reads the packages the harness is built from from `cargo tree`, which, unlike `cargo
    /// metadata`, resolves features as building only the harness does.
    fn load() -> Result<Graph> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .context("the harness isn't two directories below the workspace's root")?
            .to_owned();
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args(["tree", "--package", env!("CARGO_PKG_NAME")])
            .args([
                "--edges",
                "normal,build",
                "--prefix",
                "depth",
                "--format",
                "{p}",
            ])
            .current_dir(&root)
            .output()
            .context("running cargo tree")?;
        if !output.status.success() {
            bail!(
                "cargo tree failed ({}):\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let tree =
            String::from_utf8(output.stdout).context("cargo tree printed text that isn't UTF-8")?;
        let (packages, edges) = read_tree(&tree, &root)?;

        let closure = |start: &str| {
            let mut seen = HashSet::from([start.to_owned()]);
            let mut stack = vec![start.to_owned()];
            while let Some(id) = stack.pop() {
                for dep in edges.get(&id).into_iter().flatten() {
                    if seen.insert(dep.clone()) {
                        stack.push(dep.clone());
                    }
                }
            }
            seen
        };
        let mut components = Vec::new();
        for &(component, name) in COMPONENTS {
            let (id, _) = packages
                .iter()
                .find(|(_, package)| package.name == name && package.dir.is_some())
                .with_context(|| format!("the harness isn't built from `{name}`"))?;
            components.push((component, closure(id)));
        }
        Ok(Graph {
            root,
            packages,
            components,
        })
    }

    /// What a change to the file at `path` (relative to the root) touches, if it can affect a
    /// benchmark. `Cargo.lock` and the root manifest are compared separately.
    fn touched(&self, path: &str) -> Option<Touched> {
        if EVERYTHING.contains(&path) {
            return Some(Touched::Everything);
        }
        let (id, _) = self.packages.iter().find(|(_, package)| {
            package
                .dir
                .as_deref()
                .and_then(|dir| path.strip_prefix(dir)?.strip_prefix('/'))
                .is_some_and(|rest| {
                    !NOT_BUILT
                        .iter()
                        .any(|dir| rest.split('/').next() == Some(dir) && rest.contains('/'))
                })
        })?;
        Some(Touched::Packages(vec![id.clone()]))
    }

    /// The dependencies the harness is built from that the base's `Cargo.lock` doesn't lock at
    /// the same version.
    fn dependencies_not_in(&self, base_lock: &toml::Table) -> Vec<(String, Touched)> {
        let locked: HashSet<(&str, &str)> = base_lock
            .get("package")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|package| {
                let field = |name| package.get(name)?.as_str();
                Some((field("name")?, field("version")?))
            })
            .collect();
        let mut changes: Vec<(String, Touched)> = self
            .packages
            .iter()
            .filter_map(|(id, package)| {
                if package.dir.is_some() || locked.contains(&(&package.name, &package.version)) {
                    return None;
                }
                Some((
                    format!("Cargo.lock: {} {}", package.name, package.version),
                    Touched::Packages(vec![id.clone()]),
                ))
            })
            .collect();
        changes.sort_by(|a, b| a.0.cmp(&b.0));
        changes
    }

    /// What changed between the base's root manifest and the head's.
    fn manifest_changes(&self, base: &toml::Table, head: &toml::Table) -> Vec<(String, Touched)> {
        let mut changes = Vec::new();
        let (base_dependencies, base_rest) = split_manifest(base);
        let (head_dependencies, head_rest) = split_manifest(head);
        if base_rest != head_rest {
            changes.push(("Cargo.toml".to_owned(), Touched::Everything));
        }
        let mut names: Vec<&String> = base_dependencies
            .keys()
            .chain(head_dependencies.keys())
            .collect();
        names.sort();
        names.dedup();
        for name in names {
            let (before, after) = (base_dependencies.get(name), head_dependencies.get(name));
            if before == after {
                continue;
            }
            // An entry may name the package differently, with `package = "..."`.
            let package = [after, before]
                .into_iter()
                .flatten()
                .find_map(|entry| entry.get("package")?.as_str())
                .unwrap_or(name);
            let ids: Vec<String> = self
                .packages
                .iter()
                .filter(|(_, candidate)| candidate.name == package)
                .map(|(id, _)| id.clone())
                .collect();
            if !ids.is_empty() {
                changes.push((
                    format!("Cargo.toml: workspace dependency {name}"),
                    Touched::Packages(ids),
                ));
            }
        }
        changes
    }

    /// The components whose benchmarks a change to `touched` can affect.
    fn affected(&self, touched: &Touched) -> Vec<&'static str> {
        let all = || self.components.iter().map(|(name, _)| *name).collect();
        let Touched::Packages(ids) = touched else {
            return all();
        };
        let ids: Vec<&String> = ids
            .iter()
            .filter(|id| self.packages.contains_key(*id))
            .collect();
        // A package no component is built from is the harness's own: every benchmark runs on it.
        if ids.iter().any(|id| {
            !self
                .components
                .iter()
                .any(|(_, packages)| packages.contains(*id))
        }) {
            return all();
        }
        self.components
            .iter()
            .filter(|(_, packages)| ids.iter().any(|id| packages.contains(*id)))
            .map(|(name, _)| *name)
            .collect()
    }
}

type Edges = HashMap<String, Vec<String>>;

/// The packages in the output of `cargo tree --prefix depth --format {p}`, by id (`<name>
/// <version>`), and the dependencies of each. A package's dependencies are listed under its first
/// line only; later ones end in `(*)`.
fn read_tree(tree: &str, root: &Path) -> Result<(HashMap<String, Package>, Edges)> {
    let mut packages = HashMap::new();
    let mut edges: Edges = HashMap::new();
    // The ids of the packages the current line is below, by depth.
    let mut parents: Vec<String> = Vec::new();
    for line in tree.lines().filter(|line| !line.is_empty()) {
        let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let depth: usize = line[..digits]
            .parse()
            .with_context(|| format!("cargo tree printed `{line}`"))?;
        let mut words = line[digits..].splitn(3, ' ');
        let (Some(name), Some(version)) = (words.next(), words.next()) else {
            bail!("cargo tree printed `{line}`");
        };
        let version = version.strip_prefix('v').unwrap_or(version);
        let rest = words.next().unwrap_or("");
        // `(<path>)` for a path dependency, then `(proc-macro)` and `(*)` as they apply.
        let dir = rest
            .split(" (")
            .filter_map(|part| part.trim_start_matches('(').strip_suffix(')'))
            .map(Path::new)
            .find(|path| path.is_absolute())
            .map(|path| {
                relative_dir(root, path)
                    .with_context(|| format!("{} isn't in the workspace", path.display()))
            })
            .transpose()?;
        let id = format!("{name} {version}");
        parents.truncate(depth);
        if let Some(parent) = parents.last() {
            edges.entry(parent.clone()).or_default().push(id.clone());
        }
        parents.push(id.clone());
        packages.entry(id).or_insert_with(|| Package {
            name: name.to_owned(),
            version: version.to_owned(),
            dir,
        });
    }
    Ok((packages, edges))
}

/// The directory `dir`, relative to `root`, with `/` between components.
fn relative_dir(root: &Path, dir: &Path) -> Option<String> {
    let dir = dir.strip_prefix(root).ok()?;
    let parts: Vec<&str> = dir
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

fn read_manifest(text: &str) -> Result<toml::Table> {
    toml::from_str(text).context("parsing the root Cargo.toml")
}

/// The root manifest's `[workspace.dependencies]`, and the rest of it, without the keys that
/// don't change what's built.
fn split_manifest(manifest: &toml::Table) -> (toml::Table, toml::Table) {
    let mut rest = manifest.clone();
    let mut dependencies = toml::Table::new();
    if let Some(toml::Value::Table(workspace)) = rest.get_mut("workspace") {
        if let Some(toml::Value::Table(entries)) = workspace.remove("dependencies") {
            dependencies = entries;
        }
        for key in IGNORED_WORKSPACE_KEYS {
            workspace.remove(*key);
        }
    }
    (dependencies, rest)
}

/// Runs git in `dir`, and returns what it printed.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} failed ({}):\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("git printed text that isn't UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace like Ferrion's: the lexer and class file reading share nothing but the
    /// harness, which also reads the corpus.
    fn graph() -> Graph {
        let package = |name: &str, dir: Option<&str>| {
            let package = Package {
                name: name.to_owned(),
                version: "1.0.0".to_owned(),
                dir: dir.map(str::to_owned),
            };
            (name.to_owned(), package)
        };
        let ids = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        Graph {
            root: PathBuf::new(),
            packages: HashMap::from([
                package("bench", Some("tools/bench")),
                package("corpus", Some("tools/corpus")),
                package("ferrion_lexer", Some("crates/ferrion_lexer")),
                package("ferrion_diagnostics", Some("crates/ferrion_diagnostics")),
                package("ferrion_classfile", Some("crates/ferrion_classfile")),
                package("miniz_oxide", None),
                package("serde_json", None),
            ]),
            components: vec![
                ("lexer", ids(&["ferrion_lexer", "ferrion_diagnostics"])),
                ("classfile", ids(&["ferrion_classfile", "miniz_oxide"])),
            ],
        }
    }

    fn affected(path: &str) -> Vec<&'static str> {
        let graph = graph();
        graph
            .touched(path)
            .map_or_else(Vec::new, |touched| graph.affected(&touched))
    }

    #[test]
    fn a_file_affects_the_components_built_from_its_package() {
        assert_eq!(affected("crates/ferrion_lexer/src/lib.rs"), ["lexer"]);
        assert_eq!(affected("crates/ferrion_lexer/Cargo.toml"), ["lexer"]);
        assert_eq!(affected("crates/ferrion_diagnostics/src/lib.rs"), ["lexer"]);
        assert_eq!(
            affected("crates/ferrion_classfile/src/lib.rs"),
            ["classfile"]
        );
    }

    #[test]
    fn the_harness_and_its_inputs_affect_every_component() {
        assert_eq!(affected("tools/bench/src/main.rs"), ["lexer", "classfile"]);
        assert_eq!(
            affected("tools/corpus/projects.toml"),
            ["lexer", "classfile"]
        );
        assert_eq!(affected("rust-toolchain.toml"), ["lexer", "classfile"]);
        assert_eq!(affected(".github/workflows/ci.yml"), ["lexer", "classfile"]);
    }

    #[test]
    fn tests_and_other_packages_affect_nothing() {
        assert!(affected("crates/ferrion_lexer/tests/lexer.rs").is_empty());
        assert!(affected("crates/ferrion_lexer/benches/lex.rs").is_empty());
        assert!(affected("crates/ferrion_cli/src/main.rs").is_empty());
        assert!(affected("crates/ferrion_lexer_extra/src/lib.rs").is_empty());
        assert!(affected("docs/design/benchmarks.md").is_empty());
        assert!(affected(".github/workflows/fuzz.yml").is_empty());
        // A file named like one of those directories is built.
        assert_eq!(affected("crates/ferrion_lexer/tests"), ["lexer"]);
    }

    #[test]
    fn dependencies_locked_at_another_version_are_changes() {
        let graph = graph();
        let lock: toml::Table = toml::from_str(
            r#"
            [[package]]
            name = "miniz_oxide"
            version = "0.9.0"
            source = "registry+https://github.com/rust-lang/crates.io-index"

            [[package]]
            name = "serde_json"
            version = "1.0.0"
            source = "registry+https://github.com/rust-lang/crates.io-index"

            [[package]]
            name = "ferrion_lexer"
            version = "0.0.0"
            "#,
        )
        .unwrap();
        let changes = graph.dependencies_not_in(&lock);
        assert_eq!(
            changes,
            [(
                "Cargo.lock: miniz_oxide 1.0.0".to_owned(),
                Touched::Packages(vec!["miniz_oxide".to_owned()])
            )]
        );
        assert_eq!(graph.affected(&changes[0].1), ["classfile"]);
    }

    #[test]
    fn only_the_root_manifest_entries_that_change_what_is_built_are_changes() {
        let graph = graph();
        let manifest = |text: &str| -> toml::Table { toml::from_str(text).unwrap() };
        let base = manifest(
            r#"
            [workspace]
            members = ["crates/*"]
            [workspace.dependencies]
            miniz_oxide = "1"
            serde_json = "1"
            pulldown-cmark = "0.13"
            [workspace.lints.rust]
            unsafe_code = "deny"
            "#,
        );
        let head = manifest(
            r#"
            [workspace]
            members = ["crates/*", "tools/codegen"]
            [workspace.dependencies]
            miniz_oxide = { version = "1", features = ["simd"] }
            serde_json = "1"
            pulldown-cmark = "0.14"
            roxmltree = "0.21"
            [workspace.lints.rust]
            unsafe_code = "forbid"
            "#,
        );
        assert_eq!(
            graph.manifest_changes(&base, &head),
            [(
                "Cargo.toml: workspace dependency miniz_oxide".to_owned(),
                Touched::Packages(vec!["miniz_oxide".to_owned()])
            )]
        );

        let mut profiled = head.clone();
        profiled.insert(
            "profile".to_owned(),
            toml::Value::Table(manifest("release = { lto = true }")),
        );
        assert_eq!(
            graph.manifest_changes(&head, &profiled),
            [("Cargo.toml".to_owned(), Touched::Everything)]
        );
    }

    #[test]
    fn the_tree_gives_each_package_its_dependencies_and_directory() {
        let root = std::env::temp_dir().join("ferrion");
        let path = |dir: &str| root.join(dir).display().to_string();
        let tree = format!(
            "0bench v0.0.0 ({})\n\
             1ferrion_lexer v0.0.0 ({})\n\
             2memchr v2.8.3\n\
             1serde_derive v1.0.229 (proc-macro)\n\
             2proc-macro2 v1.0.107\n\
             1ferrion_lexer v0.0.0 ({}) (*)\n",
            path("tools/bench"),
            path("crates/ferrion_lexer"),
            path("crates/ferrion_lexer"),
        );
        let (packages, edges) = read_tree(&tree, &root).unwrap();
        assert_eq!(packages.len(), 5);
        let lexer = &packages["ferrion_lexer 0.0.0"];
        assert_eq!(lexer.dir.as_deref(), Some("crates/ferrion_lexer"));
        assert_eq!(packages["memchr 2.8.3"].dir, None);
        assert_eq!(packages["serde_derive 1.0.229"].dir, None);
        assert_eq!(
            edges["bench 0.0.0"],
            [
                "ferrion_lexer 0.0.0",
                "serde_derive 1.0.229",
                "ferrion_lexer 0.0.0"
            ]
        );
        assert_eq!(edges["ferrion_lexer 0.0.0"], ["memchr 2.8.3"]);
        assert_eq!(edges["serde_derive 1.0.229"], ["proc-macro2 1.0.107"]);
    }

    #[test]
    fn every_benchmark_belongs_to_a_component() {
        for benchmark in crate::benchmarks::all() {
            let (component, _) = benchmark.name.split_once('/').unwrap();
            assert!(
                COMPONENTS.iter().any(|(name, _)| *name == component),
                "{} isn't in COMPONENTS",
                benchmark.name
            );
        }
    }

    #[test]
    fn the_workspace_graph_loads() {
        let graph = Graph::load().unwrap();
        assert_eq!(graph.components.len(), COMPONENTS.len());
        assert_eq!(
            graph.affected(
                &graph
                    .touched("crates/ferrion_formatter/src/lib.rs")
                    .unwrap()
            ),
            ["formatter"]
        );
        assert!(graph.touched("crates/ferrion_cli/src/main.rs").is_none());
        assert!(graph.touched("tools/bench/src/main.rs").is_some());
    }
}
