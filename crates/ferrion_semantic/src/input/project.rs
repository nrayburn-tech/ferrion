//! The engine's inputs from a `ferrion_project::Project`: each source set's files read from its
//! roots, and with its opened `TypeInputs`, its classpath and platform API, as its compile
//! options change what its code sees of the platform.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_classfile::class::ModuleDescriptor;
use ferrion_classfile::{
    ClassBytes, ClassName, ClassPathEntry, PackageListing, PackageName, PlatformApi,
};
use ferrion_diagnostics::SourceText;
use ferrion_project::{
    ClassesEntry, CompileOptions, PlatformKind, Project, SourceSet, SourceSetTypes, TypeEntry,
    TypeInputs,
};

use super::{
    ClassPathInput, ClassesInput, FileInput, Platform, ProjectInput, SourceSetId, SourceSetInput,
};

/// A file the adapter couldn't read, with why.
#[derive(Clone, Debug)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: String,
}

impl ProjectInput {
    /// Every source set of `project`, with the `.java` files under its source roots and
    /// generated-source roots (those marked generated), in path order. With `types`, each source
    /// set's classpath is its opened entries in order, source sets and jars and directories
    /// interleaved as the model gives them, the module path's after them, and its platform is
    /// its JDK's platform API at its release, or the JDK's whole API for a source set compiled
    /// with `-source` and `-target` (`configured` has how its compile options change what code
    /// sees in it). Without, the classpath has only the source sets, and there is no platform:
    /// what a run that needs no types can resolve.
    ///
    /// Files that can't be read or aren't UTF-8 are left out and listed, as the formatter skips
    /// them.
    pub fn from_project(
        project: &Project,
        types: Option<&TypeInputs>,
    ) -> (ProjectInput, Vec<SkippedFile>) {
        let mut skipped = Vec::new();
        let mut input = ProjectInput::default();
        for set in project.source_sets() {
            let mut files = Vec::new();
            for (roots, generated) in [(&set.roots, false), (&set.generated_roots, true)] {
                for root in roots {
                    for path in java_files(root) {
                        match read(&path) {
                            Ok(text) => files.push(FileInput {
                                path,
                                text,
                                generated,
                            }),
                            Err(reason) => skipped.push(SkippedFile { path, reason }),
                        }
                    }
                }
            }
            let mut source_set = SourceSetInput::new(&set.name, set.release.value);
            source_set.files = files;
            match types {
                Some(types) => {
                    let types = types.source_set(set.id);
                    source_set.platform = Some(configured(set, types));
                    source_set.classpath = types
                        .entries
                        .iter()
                        .map(|entry| match entry {
                            TypeEntry::SourceSet(id) => {
                                ClassPathInput::SourceSet(SourceSetId::from_index(id.value.index()))
                            }
                            TypeEntry::Classes(classes) => ClassPathInput::Classes(ClassesInput {
                                entry: classes.entry.clone(),
                                path: classes.path.clone(),
                                module: classes.module.clone(),
                            }),
                        })
                        .collect();
                }
                None => {
                    source_set.classpath = set
                        .classpath
                        .iter()
                        .flatten()
                        .filter_map(|item| match item {
                            ferrion_project::ClassPathItem::SourceSet(id) => {
                                Some(ClassPathInput::SourceSet(SourceSetId::from_index(
                                    id.value.index(),
                                )))
                            }
                            ferrion_project::ClassPathItem::Path(_) => None,
                        })
                        .collect();
                }
            }
            input.source_sets.push(source_set);
        }
        (input, skipped)
    }
}

/// A source set's platform as its compile options make it. Code in the unnamed module, as every
/// source set's is, sees the packages the platform's module graph exports to it: from the default
/// root modules and `--add-modules`'s, with `--add-exports`'s to `ALL-UNNAMED` added. Compiled
/// with `-source` and `-target` before release 9, which has no modules, it sees every package of
/// the JDK (observed with javac 27: `sun.security.jca` and `jdk.internal.misc` resolve with
/// `-source 8`, and aren't visible with `-source 11`). A `--patch-module`'s classes are found
/// before the module's own. With none of these, it's the platform API as it is.
fn configured(set: &SourceSet, types: &SourceSetTypes) -> Arc<dyn Platform> {
    let options = &set.compile;
    let all_visible = options.platform == PlatformKind::Jdk && set.release.value < 9;
    if !all_visible
        && options.add_modules.is_empty()
        && options.add_exports.is_empty()
        && types.patches.iter().all(|(_, entries)| entries.is_empty())
    {
        return types.platform.clone();
    }
    let api = types.platform.clone();
    let visible = if all_visible {
        None
    } else {
        visible(&api, options)
    };
    let patches = types
        .patches
        .iter()
        .map(|(module, entries)| (module.clone(), entries.clone()))
        .collect();
    Arc::new(Configured {
        api,
        visible,
        patches,
    })
}

/// The packages the unnamed module sees in `api` with `options`, in internal form, or `None` when
/// the platform has no modules.
fn visible(api: &PlatformApi, options: &CompileOptions) -> Option<HashSet<Box<str>>> {
    let roots: Vec<&str> = options.add_modules.iter().map(String::as_str).collect();
    let mut visible = api.visible_with_roots(&roots)?;
    for export in options.add_exports.iter().filter(|e| e.to_unnamed()) {
        visible.insert(export.package.replace('.', "/").into());
    }
    Some(visible)
}

/// A platform API with what a source set's compile options change.
#[derive(Debug)]
struct Configured {
    api: Arc<PlatformApi>,
    /// In internal form; `None`: every package.
    visible: Option<HashSet<Box<str>>>,
    /// `--patch-module`'s entries, by module, searched before the platform.
    patches: Vec<(String, Vec<ClassesEntry>)>,
}

impl Configured {
    fn patch_entries(&self) -> impl Iterator<Item = (&str, &ClassPathEntry)> {
        self.patches.iter().flat_map(|(module, entries)| {
            entries
                .iter()
                .map(move |entry| (module.as_str(), &entry.entry))
        })
    }
}

impl Platform for Configured {
    fn find(&self, name: &ClassName) -> ferrion_classfile::Result<Option<ClassBytes>> {
        for (_, entry) in self.patch_entries() {
            if let Some(found) = entry.find(name)? {
                return Ok(Some(found));
            }
        }
        self.api.find(name)
    }

    fn package(&self, name: &PackageName) -> ferrion_classfile::Result<Option<PackageListing>> {
        let mut listing = self.api.package(name)?;
        for (_, entry) in self.patch_entries() {
            if let Some(found) = entry.package(name)? {
                match &mut listing {
                    Some(listing) => listing.merge(found),
                    None => listing = Some(found),
                }
            }
        }
        Ok(listing)
    }

    fn modules(&self) -> &[ModuleDescriptor] {
        self.api.modules()
    }

    fn is_visible(&self, package: &PackageName) -> bool {
        self.visible
            .as_ref()
            .is_none_or(|visible| visible.contains(package.as_str()))
    }

    fn module_of(&self, name: &ClassName) -> Option<String> {
        for (module, entry) in self.patch_entries() {
            if entry.find(name).ok().flatten().is_some() {
                return Some(module.to_owned());
            }
        }
        self.api.module_of(name)
    }
}

fn read(path: &Path) -> Result<Arc<str>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let source = SourceText::decode(bytes).map_err(|e| format!("{e:?}"))?;
    Ok(Arc::from(source.text()))
}

/// The `.java` files under `dir`, sorted; none if it doesn't exist. A stack of directories, so
/// a deep tree doesn't recurse; symbolic links to directories aren't followed, so a link cycle
/// can't loop.
fn java_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "java") && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
