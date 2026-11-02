//! Source set views: where a source set finds a type by package and simple name, or by binary
//! name, and which packages and modules it can see (docs/design/semantic-names.md#source-set-views).
//!
//! For a type `p.T` in source set S, the first of:
//!
//! 1. the platform API, when one of its modules owns `p` (observed: javac then finds neither a
//!    source type nor a classpath class in `p`), if `p` is visible to S's module;
//! 2. S's own sources, through its package table, a type declared in more than one file being
//!    ambiguous;
//! 3. S's classpath in order: another source set's own sources, read in that set's own view, or a
//!    jar's or directory's class file.
//!
//! Lookups record the package entries they read; class sources don't change, so what is read
//! from them isn't recorded.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use ferrion_classfile::PackageName;
use ferrion_classfile::class::ModuleDescriptor;

use crate::classes::ClassSource;
use crate::engine::derived::{Dep, Reads};
use crate::summary::Directive;
use crate::symbols::{ClassSourceId, TypeKey};
use crate::{Cancelled, FileId, Fingerprint, Interner, Name, Snapshot, SourceSetId};

/// One entry of a source set's classpath, with class sources numbered.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum SetEntry {
    SourceSet(SourceSetId),
    Classes(ClassSourceId),
}

/// A source set's classpath and platform, which don't change while the engine runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct SetConfig {
    pub(crate) classpath: Arc<[SetEntry]>,
    pub(crate) platform: Option<ClassSourceId>,
}

/// What a lookup of a type by name found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Found {
    Type(TypeKey),
    /// Declared in more than one of a source set's own files.
    Ambiguous(Box<[TypeKey]>),
    /// In a platform package the reading module can't see.
    NotVisible,
    /// A class source couldn't list the package.
    Bad(Arc<str>),
    None,
}

/// A source set's module (docs/design/semantic-names.md#modules): unnamed unless one of its files
/// is a `module-info.java`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct ModuleView {
    /// The module's name, `None` for the unnamed module.
    pub name: Option<Name>,
    /// For a named module, the platform modules it reads: `java.base`, those it `requires`, and
    /// transitively those they `requires transitive`. Sorted by id.
    pub reads: Box<[Name]>,
    /// For a named module, its `exports` directives' packages, for `import module` of it.
    pub exports: Box<[Name]>,
}

/// A platform's modules, read once from its descriptors.
pub(crate) struct PlatformModules {
    modules: HashMap<Name, ModuleFacts>,
    /// Each package's module, from the descriptors' `ModulePackages` and exports.
    packages: HashMap<Name, Name>,
}

struct ModuleFacts {
    resolved_by_default: bool,
    requires: Vec<(Name, bool)>,
    /// Each exported package (dotted) and the modules it is exported to, empty for everyone.
    exports: Vec<(Name, Vec<Name>)>,
}

impl PlatformModules {
    pub(crate) fn new(descriptors: &[ModuleDescriptor], names: &Interner) -> PlatformModules {
        let dotted = |internal: &str| names.intern(&internal.replace('/', "."));
        let mut modules = HashMap::new();
        let mut packages = HashMap::new();
        for module in descriptors {
            let name = names.intern(&module.name);
            for package in module.packages.iter().flatten() {
                packages.insert(dotted(package), name);
            }
            for export in &module.exports {
                packages.entry(dotted(&export.package)).or_insert(name);
            }
            modules.insert(
                name,
                ModuleFacts {
                    resolved_by_default: module.is_resolved_by_default()
                        && module.unqualified_exports().next().is_some(),
                    requires: module
                        .requires
                        .iter()
                        .map(|r| {
                            let transitive = ferrion_classfile::class::AccessFlags(r.flags)
                                .contains(ferrion_classfile::class::AccessFlags::TRANSITIVE);
                            (names.intern(&r.module), transitive)
                        })
                        .collect(),
                    exports: module
                        .exports
                        .iter()
                        .map(|e| {
                            let to = e.to.iter().map(|m| names.intern(m)).collect();
                            (dotted(&e.package), to)
                        })
                        .collect(),
                },
            );
        }
        PlatformModules { modules, packages }
    }

    pub(crate) fn has(&self, module: Name) -> bool {
        self.modules.contains_key(&module)
    }

    /// `module` and, transitively, the modules it `requires transitive`.
    fn transitive(&self, module: Name, into: &mut BTreeSet<Name>) {
        let mut work = vec![module];
        while let Some(m) = work.pop() {
            if !into.insert(m) {
                continue;
            }
            if let Some(facts) = self.modules.get(&m) {
                work.extend(facts.requires.iter().filter(|r| r.1).map(|r| r.0));
            }
        }
    }

    /// The modules the unnamed module reads: the default roots and what they require (JEP 261).
    fn default_modules(&self) -> BTreeSet<Name> {
        let mut read = BTreeSet::new();
        let mut work: Vec<Name> = self
            .modules
            .iter()
            .filter(|(_, f)| f.resolved_by_default)
            .map(|(n, _)| *n)
            .collect();
        while let Some(m) = work.pop() {
            if read.insert(m)
                && let Some(facts) = self.modules.get(&m)
            {
                work.extend(facts.requires.iter().map(|r| r.0));
            }
        }
        read
    }

    /// The packages `module` exports to `reader` (`None` for the unnamed module), and those of
    /// the modules it requires transitively: what `import module` imports (JLS 7.5.5).
    fn imported_packages(&self, module: Name, reader: Option<Name>) -> Vec<Name> {
        let mut modules = BTreeSet::new();
        self.transitive(module, &mut modules);
        let mut packages = Vec::new();
        for m in modules {
            if let Some(facts) = self.modules.get(&m) {
                for (package, to) in &facts.exports {
                    if to.is_empty() || reader.is_some_and(|r| to.contains(&r)) {
                        packages.push(*package);
                    }
                }
            }
        }
        packages
    }
}

impl Snapshot {
    /// `set`'s module.
    pub fn module_view(&self, set: SourceSetId) -> Result<Arc<ModuleView>, Cancelled> {
        self.check()?;
        Ok(self.module_view_memo(set)?.value.clone())
    }

    pub(crate) fn module_view_memo(
        &self,
        set: SourceSetId,
    ) -> Result<Arc<crate::engine::derived::Memo<ModuleView>>, Cancelled> {
        self.shared()
            .module_views
            .get(self, &set, |reads| module_view(reads, set))
    }

    pub(crate) fn config(&self, set: SourceSetId) -> &SetConfig {
        &self.shared().configs[set.index()]
    }

    pub(crate) fn platform_modules(&self, platform: ClassSourceId) -> Arc<PlatformModules> {
        let shared = self.shared();
        shared.platform_modules.get_or_insert(platform, || {
            let descriptors = match shared.classes.source(platform) {
                ClassSource::Platform(p) => p.modules(),
                ClassSource::Entry(_) => &[],
            };
            Arc::new(PlatformModules::new(descriptors, &shared.names))
        })
    }

    /// Finds the type a class file names by binary name (`java/util/Map$Entry`) in `view`, as
    /// javac resolves a class file's names against the classpath it compiles with: the platform
    /// for the packages it owns, without checking visibility, then own sources, then the classpath.
    /// A source type is found by its path, trying each way to split the name at `$`.
    pub fn resolve_binary(
        &self,
        view: SourceSetId,
        binary: Name,
    ) -> Result<Option<TypeKey>, Cancelled> {
        // Not recorded: what a class file's name resolves to changes only with the source types
        // it could name, which whoever reads the result records when it reads their entries.
        let mut reads = Reads::untracked(self);
        reads.find_binary(view, binary)
    }
}

fn module_view(reads: &mut Reads<'_>, set: SourceSetId) -> Result<ModuleView, Cancelled> {
    let names = reads.snapshot.names().clone();
    let Some(file) = reads.module_file(set)? else {
        return Ok(ModuleView::default());
    };
    // Before release 9 there are no modules: javac rejects a module declaration (`modules are
    // not supported in -source 8`), and a build that has one compiles it on its own, at a later
    // release, while the rest of the source set is in the unnamed module.
    if reads.release(file)? < 9 {
        return Ok(ModuleView::default());
    }
    let summary = reads.summary(file)?;
    let Some(module) = &summary.module else {
        return Ok(ModuleView::default());
    };
    let mut read = BTreeSet::new();
    let platform = reads.snapshot.config(set).platform;
    let modules = platform.map(|p| reads.snapshot.platform_modules(p));
    let java_base = names.intern("java.base");
    let mut required = vec![java_base];
    let mut exports = Vec::new();
    for directive in &module.directives {
        match directive {
            Directive::Requires { module, .. } => required.push(module.name()),
            Directive::Exports { package, .. } => exports.push(package.name()),
            _ => {}
        }
    }
    for module in required {
        match &modules {
            Some(modules) => modules.transitive(module, &mut read),
            None => {
                read.insert(module);
            }
        }
    }
    Ok(ModuleView {
        name: Some(module.name.name()),
        reads: read.into_iter().collect(),
        exports: exports.into_boxed_slice(),
    })
}

/// How a qualified name's package is written for class sources: `java/util`.
fn internal(names: &Interner, package: Name) -> String {
    names.text(package).replace('.', "/")
}

/// The binary name of `simple` in `package`, interned: `java/util/Map`.
pub(crate) fn binary_name(names: &Interner, package: Name, simple: &str) -> Name {
    if package.is_empty() {
        names.intern(simple)
    } else {
        names.intern(&format!("{}/{simple}", internal(names, package)))
    }
}

impl Reads<'_> {
    /// The `module-info.java` of `set`, if any. Records the set's list of them.
    pub(crate) fn module_file(&mut self, set: SourceSetId) -> Result<Option<FileId>, Cancelled> {
        let index = self.snapshot.package_index_memo(set)?;
        let files = index.value.module_files();
        self.record(Dep::ModuleFiles(set), Fingerprint::of(files));
        Ok(files.first().copied())
    }

    /// `set`'s module. Records it.
    pub(crate) fn module_view(&mut self, set: SourceSetId) -> Result<Arc<ModuleView>, Cancelled> {
        let memo = self.snapshot.module_view_memo(set)?;
        self.record(Dep::ModuleView(set), memo.fingerprint);
        Ok(memo.value.clone())
    }

    /// Whether code in `set` can name classes of the platform package `package` (dotted).
    pub(crate) fn platform_visible(
        &mut self,
        set: SourceSetId,
        platform: ClassSourceId,
        package: Name,
    ) -> Result<bool, Cancelled> {
        let module = self.module_view(set)?;
        let names = self.snapshot.names().clone();
        let Some(name) = module.name else {
            return Ok(match self.snapshot.shared().classes.source(platform) {
                ClassSource::Platform(p) => {
                    p.modules().is_empty()
                        || p.is_visible(&PackageName::new(&internal(&names, package)))
                }
                ClassSource::Entry(_) => true,
            });
        };
        let modules = self.snapshot.platform_modules(platform);
        let Some(&owner) = modules.packages.get(&package) else {
            // Not exported by any module, and not listed by one: an internal package.
            return Ok(false);
        };
        if !module.reads.contains(&owner) {
            return Ok(false);
        }
        Ok(modules.modules.get(&owner).is_some_and(|facts| {
            facts
                .exports
                .iter()
                .any(|(p, to)| *p == package && (to.is_empty() || to.contains(&name)))
        }))
    }

    /// Whether `set` can see the platform package `package`: `None` if the platform doesn't
    /// own it.
    pub(crate) fn platform_package_visible(
        &mut self,
        set: SourceSetId,
        package: Name,
    ) -> Result<Option<bool>, Cancelled> {
        let snapshot = self.snapshot;
        let names = snapshot.names().clone();
        let Some(platform) = snapshot.config(set).platform else {
            return Ok(None);
        };
        match &*snapshot.shared().classes.listing(platform, package, &names) {
            Ok(Some(_)) => Ok(Some(self.platform_visible(set, platform, package)?)),
            _ => Ok(None),
        }
    }

    /// The top-level type `simple` of `package` (dotted) in `set`'s view.
    pub(crate) fn find_top_level(
        &mut self,
        set: SourceSetId,
        package: Name,
        simple: Name,
    ) -> Result<Found, Cancelled> {
        self.check()?;
        if simple.is_empty() {
            return Ok(Found::None);
        }
        let snapshot = self.snapshot;
        let names = snapshot.names().clone();
        let classes = &snapshot.shared().classes;
        let simple_text = names.text(simple);
        let config = snapshot.config(set).clone();
        if let Some(platform) = config.platform {
            match &*classes.listing(platform, package, &names) {
                Ok(Some(listing)) => {
                    if !listing.contains(&simple_text) {
                        return Ok(Found::None);
                    }
                    let binary = binary_name(&names, package, &simple_text);
                    if !top_level_class(snapshot, platform, binary) {
                        return Ok(Found::None);
                    }
                    if !self.platform_visible(set, platform, package)? {
                        return Ok(Found::NotVisible);
                    }
                    return Ok(Found::Type(TypeKey::Class {
                        source: platform,
                        binary,
                    }));
                }
                Ok(None) => {}
                Err(e) => return Ok(Found::Bad(e.clone())),
            }
        }
        if let Some(found) = self.find_in_sources(set, package, simple)? {
            return Ok(found);
        }
        for entry in config.classpath.iter() {
            match *entry {
                SetEntry::SourceSet(dependency) => {
                    if let Some(found) = self.find_in_sources(dependency, package, simple)? {
                        return Ok(found);
                    }
                }
                SetEntry::Classes(source) => {
                    match classes.has(source, package, &simple_text, &names) {
                        Ok(false) => {}
                        Ok(true) => {
                            let binary = binary_name(&names, package, &simple_text);
                            if top_level_class(snapshot, source, binary) {
                                return Ok(Found::Type(TypeKey::Class { source, binary }));
                            }
                        }
                        Err(e) => return Ok(Found::Bad(e)),
                    }
                }
            }
        }
        Ok(Found::None)
    }

    /// A top-level type of `set`'s own sources. Records the package entry.
    fn find_in_sources(
        &mut self,
        set: SourceSetId,
        package: Name,
        simple: Name,
    ) -> Result<Option<Found>, Cancelled> {
        let Some(declarations) = self.package_type(set, package, simple)? else {
            return Ok(None);
        };
        let key = TypeKey::Source {
            set,
            package,
            path: self.snapshot.names().path(None, simple),
        };
        Ok(Some(if declarations.len() > 1 {
            Found::Ambiguous(vec![key; declarations.len()].into_boxed_slice())
        } else {
            Found::Type(key)
        }))
    }

    /// Whether `package` (dotted) exists in `set`'s view: some source declares it, or a class
    /// source has classes in it.
    pub(crate) fn package_exists(
        &mut self,
        set: SourceSetId,
        package: Name,
    ) -> Result<bool, Cancelled> {
        let snapshot = self.snapshot;
        let names = snapshot.names().clone();
        let classes = &snapshot.shared().classes;
        let config = snapshot.config(set).clone();
        let sources = std::iter::once(set).chain(config.classpath.iter().filter_map(|e| match e {
            SetEntry::SourceSet(d) => Some(*d),
            SetEntry::Classes(_) => None,
        }));
        for source_set in sources.collect::<Vec<_>>() {
            if self.package_files(source_set, package)?.is_some() {
                return Ok(true);
            }
        }
        let class_sources = config
            .platform
            .into_iter()
            .chain(config.classpath.iter().filter_map(|e| match e {
                SetEntry::Classes(c) => Some(*c),
                SetEntry::SourceSet(_) => None,
            }));
        Ok(class_sources
            .into_iter()
            .any(|c| matches!(&*classes.listing(c, package, &names), Ok(Some(_)))))
    }

    /// The type a class file names by binary name, in `set`'s view (see
    /// [`Snapshot::resolve_binary`]).
    pub(crate) fn find_binary(
        &mut self,
        set: SourceSetId,
        binary: Name,
    ) -> Result<Option<TypeKey>, Cancelled> {
        let snapshot = self.snapshot;
        let names = snapshot.names().clone();
        let classes = &snapshot.shared().classes;
        let text = names.text(binary);
        let (package_internal, simple) = text.rsplit_once('/').unwrap_or(("", &text));
        let package = names.intern(&package_internal.replace('/', "."));
        let config = snapshot.config(set).clone();
        if let Some(platform) = config.platform
            && let Ok(Some(listing)) = &*classes.listing(platform, package, &names)
        {
            return Ok(listing.contains(simple).then_some(TypeKey::Class {
                source: platform,
                binary,
            }));
        }
        if let Some(key) = self.find_binary_in_sources(set, package, simple)? {
            return Ok(Some(key));
        }
        for entry in config.classpath.iter() {
            match *entry {
                SetEntry::SourceSet(d) => {
                    if let Some(key) = self.find_binary_in_sources(d, package, simple)? {
                        return Ok(Some(key));
                    }
                }
                SetEntry::Classes(source) => {
                    if let Ok(true) = classes.has(source, package, simple, &names) {
                        return Ok(Some(TypeKey::Class { source, binary }));
                    }
                }
            }
        }
        Ok(None)
    }

    /// A source type by the part of a binary name after its package (`Outer$Inner`), trying the
    /// whole as a top-level name first, then each split at a `$` from the left.
    fn find_binary_in_sources(
        &mut self,
        set: SourceSetId,
        package: Name,
        simple: &str,
    ) -> Result<Option<TypeKey>, Cancelled> {
        let names = self.snapshot.names().clone();
        let mut splits = vec![simple.len()];
        splits.extend(simple.match_indices('$').map(|(i, _)| i));
        for split in splits {
            let (top, rest) = simple.split_at(split);
            let Some(top_name) = names.lookup(top) else {
                continue;
            };
            if self
                .package_type(set, package, top_name)?
                .is_none_or(|d| d.len() > 1)
            {
                continue;
            }
            let mut path = names.path(None, top_name);
            if !rest.is_empty() {
                for member in rest[1..].split('$') {
                    path = names.path(Some(path), names.intern(member));
                }
            }
            let key = TypeKey::Source { set, package, path };
            if rest.is_empty() || self.source_type_exists(key)? {
                return Ok(Some(key));
            }
        }
        Ok(None)
    }

    /// Whether a source type's declaring file declares its whole path.
    pub(crate) fn source_type_exists(&mut self, key: TypeKey) -> Result<bool, Cancelled> {
        Ok(self.locate(key)?.is_some())
    }
}

/// Whether the class file `binary` of `source` is a top-level class, not a member, local, or
/// anonymous one a listing also shows (`Map$Entry`). A class that can't be read counts as top
/// level, so that its error is reported where it is used.
fn top_level_class(snapshot: &Snapshot, source: ClassSourceId, binary: Name) -> bool {
    let shared = snapshot.shared();
    match shared.classes.entry(source, binary, &shared.names) {
        Some(entry) => entry.outer.is_none() && !entry.local,
        None => false,
    }
}

impl crate::packages::PackageIndex {
    /// The set's `module-info.java` files, in order.
    pub(crate) fn module_files(&self) -> &[FileId] {
        &self.module_files
    }
}

/// For `import module M` (JLS 7.5.5): the packages M exports to `set`'s module, and those of the
/// modules M requires transitively, or why there are none.
pub(crate) enum ModuleImport {
    Packages(Vec<Name>),
    NotRead,
    NoModule,
}

impl Reads<'_> {
    pub(crate) fn module_import(
        &mut self,
        set: SourceSetId,
        module: Name,
    ) -> Result<ModuleImport, Cancelled> {
        let snapshot = self.snapshot;
        let names = snapshot.names().clone();
        let view = self.module_view(set)?;
        let config = snapshot.config(set).clone();
        if let Some(platform) = config.platform {
            let modules = snapshot.platform_modules(platform);
            if modules.has(module) {
                let read = match view.name {
                    Some(_) => view.reads.contains(&module),
                    None => modules.default_modules().contains(&module),
                };
                if !read {
                    return Ok(ModuleImport::NotRead);
                }
                return Ok(ModuleImport::Packages(
                    modules.imported_packages(module, view.name),
                ));
            }
        }
        // A source set's own module, or a dependency's, by its `module-info.java`.
        let mut sets = vec![set];
        sets.extend(config.classpath.iter().filter_map(|e| match e {
            SetEntry::SourceSet(d) => Some(*d),
            SetEntry::Classes(_) => None,
        }));
        for source_set in sets {
            let other = self.module_view(source_set)?;
            if other.name == Some(module) {
                return Ok(ModuleImport::Packages(other.exports.to_vec()));
            }
        }
        // A module path jar, read leniently: every package it has.
        for entry in config.classpath.iter() {
            if let SetEntry::Classes(source) = *entry
                && let ClassSource::Entry(input) = snapshot.shared().classes.source(source)
                && input.module.as_deref() == Some(&*names.text(module))
            {
                let packages = input
                    .entry
                    .as_jar()
                    .map(|jar| {
                        jar.packages()
                            .map(|p| names.intern(&p.replace('/', ".")))
                            .collect()
                    })
                    .unwrap_or_default();
                return Ok(ModuleImport::Packages(packages));
            }
        }
        Ok(ModuleImport::NoModule)
    }
}
