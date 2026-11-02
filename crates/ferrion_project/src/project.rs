//! Loading: flags or the project model become source sets with their roots, releases, JDKs, and
//! classpaths as given. Nothing but the model file and the classpath file is read; jars and JDKs
//! are opened by [`crate::TypeInputs::open`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{InputError, Origin, dedup};
use crate::model::{self, ModelEntry, ModelSourceSet, PlatformKind, ProjectModel};
use crate::paths::{PATH_SEPARATOR, absolute, split_list};
use crate::{Environment, InputFlags};

/// A source set, by its position in the model (0 for the one source set of flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceSetId(u32);

impl SourceSetId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A value with where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given<T> {
    pub value: T,
    pub origin: Origin,
}

/// A classpath entry as given, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassPathItem {
    /// A jar, a directory of classes, or another file, made absolute.
    Path(Given<PathBuf>),
    /// Another source set, read from source.
    SourceSet(Given<SourceSetId>),
}

/// A source set as given: nothing in it has been opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSet {
    pub id: SourceSetId,
    /// The model's `id`; `default` for the source set of flags.
    pub name: String,
    /// Absolute. For the source set of flags, the working directory, though every file the run
    /// covers is in it.
    pub roots: Vec<PathBuf>,
    pub generated_roots: Vec<PathBuf>,
    pub release: Given<u32>,
    /// The JDK's home, made absolute, or `None` if neither the model, `--jdk`, nor `JAVA_HOME`
    /// gives one.
    pub jdk: Option<Given<PathBuf>>,
    /// `None` when no classpath was given, so the type inputs are missing; an empty list when
    /// the code has no dependencies beyond the platform.
    pub classpath: Option<Vec<ClassPathItem>>,
    pub module_path: Vec<Given<PathBuf>>,
    /// How it is compiled, where that changes what its code sees. The default for the source
    /// set of flags.
    pub compile: CompileOptions,
    /// The annotation processor path the build gives javac, or `None` when processors are found
    /// on the classpath. Ferrion runs no processors; it's for tools that compare with javac.
    pub processor_path: Option<Vec<ClassPathItem>>,
}

/// How a source set is compiled: javac's options that change which names its code sees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileOptions {
    /// The release's API alone (`--release`), or the JDK's whole API (`-source` and `-target`).
    pub platform: PlatformKind,
    /// `--enable-preview`.
    pub enable_preview: bool,
    /// `--add-modules`: module names, or `ALL-DEFAULT`, `ALL-SYSTEM`, or `ALL-MODULE-PATH`.
    pub add_modules: Vec<String>,
    /// `--add-exports`, in order.
    pub add_exports: Vec<AddExports>,
    /// `--add-reads`, in order.
    pub add_reads: Vec<AddReads>,
    /// `--patch-module`, by module name.
    pub patch_modules: Vec<PatchModule>,
}

/// `--add-exports <module>/<package>=<target>(,<target>)*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddExports {
    pub module: String,
    /// Dotted, as written.
    pub package: String,
    /// Module names, or `ALL-UNNAMED`.
    pub targets: Vec<String>,
}

impl AddExports {
    /// Whether code in the unnamed module gets the package.
    pub fn to_unnamed(&self) -> bool {
        self.targets.iter().any(|t| t == "ALL-UNNAMED")
    }
}

/// `--add-reads <module>=<target>(,<target>)*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddReads {
    pub module: String,
    pub targets: Vec<String>,
}

/// `--patch-module <module>=<path>(:<path>)*`: jars and directories whose classes are put in the
/// module, before its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchModule {
    pub module: String,
    /// Absolute.
    pub paths: Vec<Given<PathBuf>>,
}

impl SourceSet {
    /// Whether `file` (absolute, without `.` or `..`) is under one of this source set's roots,
    /// generated ones included.
    fn holds(&self, file: &Path) -> bool {
        self.roots
            .iter()
            .chain(&self.generated_roots)
            .any(|root| file.starts_with(root))
    }
}

/// The source sets of a run, and how to tell which hold a file.
#[derive(Clone, Debug)]
pub struct Project {
    source_sets: Vec<SourceSet>,
    /// Whether the source sets come from a project model; otherwise there is one, from flags,
    /// and it holds every file.
    from_model: bool,
    working_dir: PathBuf,
    /// `--java-version`, for files under no source set of the model.
    java_version: Option<Given<u32>>,
    warnings: Vec<InputError>,
}

const JAVA_VERSION: &str = "--java-version";
const CLASSPATH: &str = "--classpath";
const CLASSPATH_FILE: &str = "--classpath-file";
const MODULE_PATH: &str = "--module-path";
const JDK: &str = "--jdk";
const PROJECT_MODEL: &str = "--project-model";

impl Project {
    /// Reads the flags, the environment, and the project model if one is given. Errors are usage
    /// errors, returned with any warnings; on success, the warnings are in
    /// [`Project::warnings`].
    pub fn load(flags: &InputFlags, env: &Environment) -> Result<Project, Vec<InputError>> {
        let mut problems = Vec::new();
        let java_version = flags.java_version.map(|value| Given {
            value,
            origin: Origin::Flag(JAVA_VERSION),
        });
        if let Some(release) = &java_version {
            check_release(release, &mut problems);
        }
        let jdk = match (&flags.jdk, &env.java_home) {
            (Some(jdk), _) => Some(Given {
                value: absolute(&env.working_dir, jdk),
                origin: Origin::Flag(JDK),
            }),
            (None, Some(home)) => Some(Given {
                value: absolute(&env.working_dir, home),
                origin: Origin::JavaHome,
            }),
            (None, None) => None,
        };
        let source_sets = match &flags.project_model {
            Some(model) => {
                for (given, flag) in [
                    (flags.classpath.is_some(), CLASSPATH),
                    (flags.classpath_file.is_some(), CLASSPATH_FILE),
                    (flags.module_path.is_some(), MODULE_PATH),
                ] {
                    if given {
                        problems.push(
                            InputError::error(
                                format!("{flag} can't be used with --project-model"),
                                Origin::Flag(flag),
                            )
                            .with_help("the project model gives each source set's classpath and module path"),
                        );
                    }
                }
                load_model(model, env, jdk.as_ref(), &mut problems)
            }
            None => from_flags(flags, env, java_version.clone(), jdk, &mut problems),
        };
        dedup(&mut problems);
        if problems.iter().any(InputError::is_error) {
            return Err(problems);
        }
        Ok(Project {
            source_sets,
            from_model: flags.project_model.is_some(),
            working_dir: env.working_dir.clone(),
            java_version,
            warnings: problems,
        })
    }

    pub fn source_sets(&self) -> &[SourceSet] {
        &self.source_sets
    }

    pub fn source_set(&self, id: SourceSetId) -> &SourceSet {
        &self.source_sets[id.index()]
    }

    /// Whether the source sets come from a project model.
    pub fn is_from_model(&self) -> bool {
        self.from_model
    }

    /// Warnings found while loading, such as source roots that don't exist.
    pub fn warnings(&self) -> &[InputError] {
        &self.warnings
    }

    /// The source sets whose roots hold `file`, in model order. A relative `file` is against the
    /// working directory. Without a project model, every file is in the one source set.
    pub fn source_sets_of(&self, file: &Path) -> Vec<SourceSetId> {
        if !self.from_model {
            return vec![SourceSetId(0)];
        }
        let file = absolute(&self.working_dir, file);
        self.source_sets
            .iter()
            .filter(|set| set.holds(&file))
            .map(|set| set.id)
            .collect()
    }

    /// Whether `file` is under a generated-source root of any source set: it is analyzed, but
    /// never formatted or linted.
    pub fn is_generated(&self, file: &Path) -> bool {
        let file = absolute(&self.working_dir, file);
        self.source_sets.iter().any(|set| {
            set.generated_roots
                .iter()
                .any(|root| file.starts_with(root))
        })
    }

    /// The release to format `file` with, and where it came from: the first source set holding
    /// it, else `--java-version`.
    pub fn release_of(&self, file: &Path) -> Option<(u32, Origin)> {
        match self.source_sets_of(file).first() {
            Some(&id) => {
                let release = &self.source_set(id).release;
                Some((release.value, release.origin.clone()))
            }
            None => self
                .java_version
                .as_ref()
                .map(|release| (release.value, release.origin.clone())),
        }
    }

    /// The usage error for a file being formatted or linted that [`Project::release_of`] has no
    /// release for: one under no source set of the model, without `--java-version`.
    pub fn no_release(&self, file: &Path) -> InputError {
        InputError::error(
            format!(
                "`{}` isn't in any source set of the project model, so it has no Java version",
                file.display()
            ),
            Origin::Flag(PROJECT_MODEL),
        )
        .with_help("pass --java-version for files outside the model's source sets")
    }
}

fn check_release(release: &Given<u32>, problems: &mut Vec<InputError>) {
    if !ferrion_java_version::is_supported(release.value) {
        problems.push(InputError::error(
            format!(
                "Java {} isn't supported; Ferrion supports Java {} through {}",
                release.value,
                ferrion_java_version::OLDEST_RELEASE,
                ferrion_java_version::LATEST_RELEASE
            ),
            release.origin.clone(),
        ));
    }
}

fn from_flags(
    flags: &InputFlags,
    env: &Environment,
    java_version: Option<Given<u32>>,
    jdk: Option<Given<PathBuf>>,
    problems: &mut Vec<InputError>,
) -> Vec<SourceSet> {
    let Some(release) = java_version else {
        problems.push(
            InputError::error("no Java version was given", Origin::Flag(JAVA_VERSION))
                .with_help("pass --java-version, or --project-model"),
        );
        return Vec::new();
    };
    let classpath = match (&flags.classpath, &flags.classpath_file) {
        (Some(_), Some(_)) => {
            problems.push(InputError::error(
                "--classpath and --classpath-file can't both be given",
                Origin::Flag(CLASSPATH_FILE),
            ));
            None
        }
        (Some(list), None) => Some(
            split_list(list, PATH_SEPARATOR)
                .map(|entry| {
                    ClassPathItem::Path(Given {
                        value: absolute(&env.working_dir, Path::new(entry)),
                        origin: Origin::Flag(CLASSPATH),
                    })
                })
                .collect(),
        ),
        (None, Some(file)) => match std::fs::read_to_string(absolute(&env.working_dir, file)) {
            Ok(text) => Some(class_path_file(
                &text,
                file,
                PATH_SEPARATOR,
                &env.working_dir,
            )),
            Err(e) => {
                problems.push(InputError::error(
                    format!("can't read the classpath file `{}`: {e}", file.display()),
                    Origin::Flag(CLASSPATH_FILE),
                ));
                None
            }
        },
        (None, None) => None,
    };
    let module_path = flags
        .module_path
        .as_deref()
        .map(|list| {
            split_list(list, PATH_SEPARATOR)
                .map(|entry| Given {
                    value: absolute(&env.working_dir, Path::new(entry)),
                    origin: Origin::Flag(MODULE_PATH),
                })
                .collect()
        })
        .unwrap_or_default();
    vec![SourceSet {
        id: SourceSetId(0),
        name: "default".to_owned(),
        roots: vec![env.working_dir.clone()],
        generated_roots: Vec::new(),
        release,
        jdk,
        classpath,
        module_path,
        compile: CompileOptions::default(),
        processor_path: None,
    }]
}

/// The entries of a classpath file: separated by `separator` or by line breaks, with blank lines
/// and empty entries left out, so both `mvn dependency:build-classpath` output and one entry per
/// line work. Relative entries are against the working directory, as they would be on the command
/// line.
pub(crate) fn class_path_file(
    text: &str,
    file: &Path,
    separator: char,
    working_dir: &Path,
) -> Vec<ClassPathItem> {
    let mut items = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for entry in split_list(line, separator) {
            items.push(ClassPathItem::Path(Given {
                value: absolute(working_dir, Path::new(entry)),
                origin: Origin::ClassPathFile {
                    path: file.to_path_buf(),
                    line: index as u32 + 1,
                },
            }));
        }
    }
    items
}

fn load_model(
    given: &Path,
    env: &Environment,
    jdk: Option<&Given<PathBuf>>,
    problems: &mut Vec<InputError>,
) -> Vec<SourceSet> {
    let path = absolute(&env.working_dir, given);
    let whole = Origin::ProjectModel {
        path: given.to_path_buf(),
        source_set: String::new(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            problems.push(InputError::error(
                format!("can't read the project model `{}`: {e}", given.display()),
                Origin::Flag(PROJECT_MODEL),
            ));
            return Vec::new();
        }
    };
    let Some(model) = parse_model(&text, &whole, problems) else {
        return Vec::new();
    };
    let base = path.parent().unwrap_or(Path::new("/"));
    let origin = |set: &str| Origin::ProjectModel {
        path: given.to_path_buf(),
        source_set: set.to_owned(),
    };
    let mut ids: HashMap<&str, SourceSetId> = HashMap::new();
    for (index, set) in model.source_sets.iter().enumerate() {
        if set.id.is_empty() {
            problems.push(InputError::error(
                format!("source set {} has an empty `id`", index + 1),
                whole.clone(),
            ));
        } else if ids.insert(&set.id, SourceSetId(index as u32)).is_some() {
            problems.push(InputError::error(
                format!("two source sets have the id `{}`", set.id),
                origin(&set.id),
            ));
        }
    }
    let mut source_sets = Vec::with_capacity(model.source_sets.len());
    for (index, set) in model.source_sets.iter().enumerate() {
        let here = origin(&set.id);
        let release = Given {
            value: set.java_release,
            origin: here.clone(),
        };
        check_release(&release, problems);
        let roots = |roots: &[PathBuf], kind: &str, problems: &mut Vec<InputError>| {
            roots
                .iter()
                .map(|root| {
                    let root = absolute(base, root);
                    if !root.is_dir() {
                        problems.push(InputError::warning(
                            format!("{kind} `{}` doesn't exist", root.display()),
                            here.clone(),
                        ));
                    }
                    root
                })
                .collect::<Vec<_>>()
        };
        let source_roots = roots(&set.source_roots, "source root", problems);
        let generated_roots = roots(
            &set.generated_source_roots,
            "generated-source root",
            problems,
        );
        let classpath = entries(set, &set.classpath, &ids, base, &here, problems);
        let processor_path = set
            .processor_path
            .as_ref()
            .map(|path| entries(set, path, &ids, base, &here, problems));
        let compile = compile_options(set, base, &here, problems);
        let jdk = match &set.jdk {
            Some(home) => Some(Given {
                value: absolute(base, home),
                origin: here.clone(),
            }),
            None => jdk.cloned(),
        };
        source_sets.push(SourceSet {
            id: SourceSetId(index as u32),
            name: set.id.clone(),
            roots: source_roots,
            generated_roots,
            release,
            jdk,
            classpath: Some(classpath),
            module_path: set
                .module_path
                .iter()
                .map(|path| Given {
                    value: absolute(base, path),
                    origin: here.clone(),
                })
                .collect(),
            compile,
            processor_path,
        });
    }
    for cycle in cycles(&source_sets) {
        let names: Vec<String> = cycle
            .iter()
            .map(|&id| format!("`{}`", source_sets[id.index()].name))
            .collect();
        let message = match names.as_slice() {
            [one] => format!("source set {one} depends on itself"),
            [a, b] => format!("source sets {a} and {b} depend on each other"),
            [rest @ .., last] => format!(
                "source sets {}, and {last} depend on each other",
                rest.join(", ")
            ),
            [] => unreachable!("a cycle has a member"),
        };
        problems.push(InputError::error(message, whole.clone()));
    }
    source_sets
}

/// A classpath (or processor path) of the model as items, reporting entries with both or neither
/// of `path` and `sourceSet`, and references to source sets the model doesn't have.
fn entries(
    set: &ModelSourceSet,
    given: &[ModelEntry],
    ids: &HashMap<&str, SourceSetId>,
    base: &Path,
    here: &Origin,
    problems: &mut Vec<InputError>,
) -> Vec<ClassPathItem> {
    let mut items = Vec::with_capacity(given.len());
    for entry in given {
        match (&entry.path, &entry.source_set) {
            (Some(path), None) => items.push(ClassPathItem::Path(Given {
                value: absolute(base, path),
                origin: here.clone(),
            })),
            (None, Some(other)) => match ids.get(other.as_str()) {
                Some(&id) => items.push(ClassPathItem::SourceSet(Given {
                    value: id,
                    origin: here.clone(),
                })),
                None => problems.push(InputError::error(
                    format!(
                        "source set `{}` depends on `{other}`, which isn't in the project model",
                        set.id
                    ),
                    here.clone(),
                )),
            },
            _ => problems.push(InputError::error(
                format!(
                    "a classpath entry of source set `{}` has both or neither of `path` and \
                     `sourceSet`",
                    set.id
                ),
                here.clone(),
            )),
        }
    }
    items
}

/// A source set's compile options, checked: module names, and the forms of `addExports` and
/// `addReads`. Paths are made absolute against `base`; whether they exist is checked when type
/// inputs are opened.
fn compile_options(
    set: &ModelSourceSet,
    base: &Path,
    here: &Origin,
    problems: &mut Vec<InputError>,
) -> CompileOptions {
    let mut bad = |field: &str, flag: &str, value: &str, form: &str| {
        problems.push(
            InputError::error(
                format!(
                    "`{field}` entry `{value}` of source set `{}` isn't {form}",
                    set.id
                ),
                here.clone(),
            )
            .with_help(format!("write it as javac's {flag} takes it")),
        );
    };
    let mut add_modules = Vec::new();
    for module in &set.add_modules {
        if is_module_name(module) || ALL_MODULES.contains(&module.as_str()) {
            add_modules.push(module.clone());
        } else {
            bad("addModules", "--add-modules", module, "a module name");
        }
    }
    let mut add_exports = Vec::new();
    for export in &set.add_exports {
        match parse_add_exports(export) {
            Some(parsed) => add_exports.push(parsed),
            None => bad(
                "addExports",
                "--add-exports",
                export,
                "`<module>/<package>=<target>(,<target>)*`",
            ),
        }
    }
    let mut add_reads = Vec::new();
    for reads in &set.add_reads {
        match parse_targets(reads) {
            Some((module, targets)) if is_module_name(module) => add_reads.push(AddReads {
                module: module.to_owned(),
                targets,
            }),
            _ => bad(
                "addReads",
                "--add-reads",
                reads,
                "`<module>=<target>(,<target>)*`",
            ),
        }
    }
    let mut patch_modules = Vec::new();
    for (module, paths) in &set.patch_modules {
        if !is_module_name(module) {
            bad("patchModules", "--patch-module", module, "a module name");
            continue;
        }
        patch_modules.push(PatchModule {
            module: module.clone(),
            paths: paths
                .iter()
                .map(|path| Given {
                    value: absolute(base, path),
                    origin: here.clone(),
                })
                .collect(),
        });
    }
    CompileOptions {
        platform: set.platform,
        enable_preview: set.enable_preview,
        add_modules,
        add_exports,
        add_reads,
        patch_modules,
    }
}

/// The values of `--add-modules` that aren't module names.
const ALL_MODULES: [&str; 3] = ["ALL-DEFAULT", "ALL-SYSTEM", "ALL-MODULE-PATH"];

/// Whether `name` is a module or package name as javac takes one: identifiers separated by dots
/// (JLS 7.7). Keywords aren't checked.
fn is_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
                && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        })
}

/// `<module>=<target>(,<target>)*`, the targets module names or `ALL-UNNAMED`.
fn parse_targets(text: &str) -> Option<(&str, Vec<String>)> {
    let (module, targets) = text.split_once('=')?;
    let targets: Vec<String> = targets.split(',').map(str::to_owned).collect();
    targets
        .iter()
        .all(|t| t == "ALL-UNNAMED" || is_module_name(t))
        .then_some((module, targets))
}

fn parse_add_exports(text: &str) -> Option<AddExports> {
    let (source, targets) = parse_targets(text)?;
    let (module, package) = source.split_once('/')?;
    (is_module_name(module) && is_module_name(package)).then(|| AddExports {
        module: module.to_owned(),
        package: package.to_owned(),
        targets,
    })
}

/// Parses the model, checking the format version before the fields, since a newer format may
/// have fields this Ferrion doesn't know.
fn parse_model(text: &str, whole: &Origin, problems: &mut Vec<InputError>) -> Option<ProjectModel> {
    let malformed = |detail: String| {
        InputError::error(
            format!("the project model isn't valid: {detail}"),
            whole.clone(),
        )
    };
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => {
            problems.push(malformed(e.to_string()));
            return None;
        }
    };
    match value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
    {
        None => {
            problems.push(malformed("no `formatVersion`".to_owned()));
            return None;
        }
        Some(version) if version > u64::from(model::FORMAT_VERSION) => {
            problems.push(
                InputError::error(
                    format!(
                        "the project model has format version {version}, and this Ferrion reads \
                         versions up to {}",
                        model::FORMAT_VERSION
                    ),
                    whole.clone(),
                )
                .with_help(
                    "upgrade Ferrion to the version of the integration that wrote the model",
                ),
            );
            return None;
        }
        Some(0) => {
            problems.push(malformed("format version 0 doesn't exist".to_owned()));
            return None;
        }
        Some(_) => {}
    }
    match serde_json::from_str(text) {
        Ok(model) => Some(model),
        Err(e) => {
            problems.push(malformed(e.to_string()));
            None
        }
    }
}

/// The cycles among source sets' dependencies on each other: each strongly connected component
/// with more than one member, or with a member that depends on itself, its members in model
/// order. Tarjan's algorithm, with an explicit stack.
fn cycles(source_sets: &[SourceSet]) -> Vec<Vec<SourceSetId>> {
    let edges: Vec<Vec<usize>> = source_sets
        .iter()
        .map(|set| {
            set.classpath
                .iter()
                .flatten()
                .filter_map(|item| match item {
                    ClassPathItem::SourceSet(id) => Some(id.value.index()),
                    ClassPathItem::Path(_) => None,
                })
                .collect()
        })
        .collect();
    let n = edges.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut found = Vec::new();
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        // (node, position in its edges).
        let mut work = vec![(start, 0)];
        index[start] = next;
        low[start] = next;
        next += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&(node, edge)) = work.last() {
            if let Some(&to) = edges[node].get(edge) {
                work.last_mut().expect("just looked at it").1 += 1;
                if index[to] == usize::MAX {
                    index[to] = next;
                    low[to] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    work.push((to, 0));
                } else if on_stack[to] {
                    low[node] = low[node].min(index[to]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                loop {
                    let member = stack.pop().expect("the node is on the stack");
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                if component.len() > 1 || edges[node].contains(&node) {
                    component.sort_unstable();
                    found.push(
                        component
                            .into_iter()
                            .map(|i| SourceSetId(i as u32))
                            .collect(),
                    );
                }
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests;
