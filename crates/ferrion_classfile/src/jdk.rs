//! A JDK on disk and its platform API for each release (docs/design/classfile.md, "The platform
//! API for JDK 9 and later", and docs/design/classfile-containers.md, "JDK 8").
//!
//! Which source serves a release is decided by what `ct.sym` holds, never by a version table:
//!
//! - a release with class files in `ct.sym` is served from `ct.sym`;
//! - the JDK's own release, when `ct.sym` has only its `system-modules` list, is served from the
//!   image, filtered to those modules;
//! - JDK 8 serves only release 8, from javac 8's boot class path, with `rt.jar`'s classes taken
//!   from `lib/ct.sym` when it exists.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use crate::cache::ArchiveCache;
use crate::class::{ClassFile, ModuleDescriptor};
use crate::classpath::{ClassBytes, ClassPathEntry, PackageListing};
use crate::ctsym::{CtSym, CtSymRelease, ReleaseContent};
use crate::error::{Error, ErrorKind, Origin, Result};
use crate::jimage::Image;
use crate::names::{ClassName, PackageName};
use crate::zip::ZipArchive;

/// The boot class path jars of JDK 8 in `jre/lib`, in javac 8's order (observed with `javac
/// -verbose`). `jre/classes` follows them, then the jars in `jre/lib/ext`.
const JDK8_BOOT_JARS: [&str; 7] = [
    "resources.jar",
    "rt.jar",
    "sunrsasign.jar",
    "jsse.jar",
    "jce.jar",
    "charsets.jar",
    "jfr.jar",
];

/// Where JDK 8's `lib/ct.sym` keeps `rt.jar`'s classes.
const RT_CTSYM_PREFIX: &str = "META-INF/sym/rt.jar/";

fn usage(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Usage, message)
}

/// One entry of JDK 8's boot class path.
#[derive(Clone, Debug)]
enum BootEntry {
    Entry(ClassPathEntry),
    /// `rt.jar`'s classes through `lib/ct.sym`.
    RtCtSym(Arc<ZipArchive>),
}

impl BootEntry {
    fn find(&self, name: &ClassName) -> Result<Option<ClassBytes>> {
        match self {
            BootEntry::Entry(entry) => entry.find(name),
            BootEntry::RtCtSym(archive) => {
                let Some(entry) =
                    archive.find(&format!("{RT_CTSYM_PREFIX}{}.class", name.as_str()))
                else {
                    return Ok(None);
                };
                let origin = archive.source().entry_origin(entry.name());
                Ok(Some(ClassBytes {
                    bytes: entry.read()?.into(),
                    origin,
                }))
            }
        }
    }

    fn package(&self, name: &PackageName) -> Result<Option<PackageListing>> {
        match self {
            BootEntry::Entry(entry) => entry.package(name),
            BootEntry::RtCtSym(archive) => {
                let prefix = if name.as_str().is_empty() {
                    RT_CTSYM_PREFIX.to_owned()
                } else {
                    format!("{RT_CTSYM_PREFIX}{}/", name.as_str())
                };
                let names: Vec<Box<str>> = archive
                    .with_prefix(&prefix)
                    .filter_map(|e| e.name()[prefix.len()..].strip_suffix(".class"))
                    .filter(|simple| !simple.contains('/'))
                    .map(Into::into)
                    .collect();
                Ok((!names.is_empty()).then(|| PackageListing::new(names)))
            }
        }
    }
}

#[derive(Debug)]
enum Layout {
    Jdk8 {
        boot: Vec<BootEntry>,
    },
    Modular {
        image: Arc<Image>,
        ctsym: Option<Arc<CtSym>>,
    },
}

/// A JDK: its layout (JDK 8 or 9+), feature version from the `release` file, and archives.
pub struct Jdk {
    home: PathBuf,
    version: u32,
    layout: Layout,
    platforms: Mutex<HashMap<u32, Weak<PlatformApi>>>,
    /// [`Jdk::image_platform`]'s, shared like `platforms`.
    image_platform: Mutex<Weak<PlatformApi>>,
}

impl std::fmt::Debug for Jdk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jdk")
            .field("home", &self.home)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The feature version in a `release` file's `JAVA_VERSION` (`"1.8.0_502"` is 8, `"21.0.11"` is
/// 21).
fn feature_version(release_file: &str) -> Option<u32> {
    let value = release_file
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))?;
    let value = value.trim().trim_matches('"');
    let value = value.strip_prefix("1.").unwrap_or(value);
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    value[..end].parse().ok()
}

impl Jdk {
    /// Opens the JDK at `home`, sharing it and its archives through `cache`.
    pub fn open(home: &Path, cache: &ArchiveCache) -> Result<Arc<Jdk>> {
        cache.jdk(home)
    }

    pub(crate) fn open_uncached(home: &Path, cache: &ArchiveCache) -> Result<Jdk> {
        let release = home.join("release");
        let version = match std::fs::read_to_string(&release) {
            Ok(text) => feature_version(&text).ok_or_else(|| {
                usage(format!(
                    "{} doesn't name a Java version (JAVA_VERSION)",
                    release.display()
                ))
            })?,
            Err(_) if Jdk::jdk8_lib(home).join("rt.jar").is_file() => 8,
            Err(e) => {
                return Err(usage(format!(
                    "{} isn't a JDK: {e} reading its release file",
                    home.display()
                )));
            }
        };
        let layout = match version {
            0..=7 => {
                return Err(usage(format!(
                    "{} is JDK {version}; Ferrion needs JDK 8 or later",
                    home.display()
                )));
            }
            8 => Layout::Jdk8 {
                boot: Jdk::jdk8_boot(home, cache)?,
            },
            9 | 10 => {
                return Err(usage(format!(
                    "{} is JDK {version}, which isn't supported; use JDK 11 or later with --release \
                     {version}",
                    home.display()
                )));
            }
            _ => {
                let image = cache.image(&home.join("lib/modules"))?;
                let ctsym_path = home.join("lib/ct.sym");
                let ctsym = if ctsym_path.is_file() {
                    Some(cache.ctsym(&ctsym_path)?)
                } else {
                    None
                };
                Layout::Modular { image, ctsym }
            }
        };
        Ok(Jdk {
            home: home.to_path_buf(),
            version,
            layout,
            platforms: Mutex::new(HashMap::new()),
            image_platform: Mutex::new(Weak::new()),
        })
    }

    /// `jre/lib` in a JDK, or `lib` in a JRE.
    fn jdk8_lib(home: &Path) -> PathBuf {
        let jre = home.join("jre/lib");
        if jre.is_dir() { jre } else { home.join("lib") }
    }

    fn jdk8_boot(home: &Path, cache: &ArchiveCache) -> Result<Vec<BootEntry>> {
        let lib = Jdk::jdk8_lib(home);
        let ctsym = home.join("lib/ct.sym");
        let mut boot = Vec::new();
        for name in JDK8_BOOT_JARS {
            let path = lib.join(name);
            if !path.is_file() {
                continue;
            }
            if name == "rt.jar" && ctsym.is_file() {
                boot.push(BootEntry::RtCtSym(cache.zip(&ctsym)?));
            } else {
                boot.push(BootEntry::Entry(ClassPathEntry::jar(cache.jar(&path, 8)?)));
            }
        }
        let classes = lib.parent().map(|jre| jre.join("classes"));
        if let Some(classes) = classes.filter(|c| c.is_dir()) {
            boot.push(BootEntry::Entry(ClassPathEntry::directory(&classes)));
        }
        // The ext jars in the order the directory lists them, as javac 8 does.
        if let Ok(entries) = std::fs::read_dir(lib.join("ext")) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_archive = path.extension().is_some_and(|e| e == "jar" || e == "zip");
                if is_archive && path.is_file() {
                    boot.push(BootEntry::Entry(ClassPathEntry::jar(cache.jar(&path, 8)?)));
                }
            }
        }
        Ok(boot)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The feature version: 8, 11, 17, ...
    pub fn feature_version(&self) -> u32 {
        self.version
    }

    /// The module image, for JDK 9 and later.
    pub fn image(&self) -> Option<&Arc<Image>> {
        match &self.layout {
            Layout::Modular { image, .. } => Some(image),
            Layout::Jdk8 { .. } => None,
        }
    }

    /// `lib/ct.sym`, for JDK 9 and later.
    pub fn ctsym(&self) -> Option<&Arc<CtSym>> {
        match &self.layout {
            Layout::Modular { ctsym, .. } => ctsym.as_ref(),
            Layout::Jdk8 { .. } => None,
        }
    }

    /// The releases this JDK serves, oldest first.
    pub fn releases(&self) -> Result<Vec<u32>> {
        match &self.layout {
            Layout::Jdk8 { .. } => Ok(vec![8]),
            Layout::Modular { ctsym, .. } => {
                let mut releases = ctsym.as_ref().map(|c| c.releases()).unwrap_or_default();
                releases
                    .retain(|&r| r >= ferrion_java_version::OLDEST_RELEASE && r <= self.version);
                if !releases.contains(&self.version) && self.platform_source(self.version).is_ok() {
                    releases.push(self.version);
                }
                Ok(releases)
            }
        }
    }

    /// Decides the source of `release`, without building its index.
    fn platform_source(&self, release: u32) -> Result<Choice> {
        let home = self.home.display();
        if release < ferrion_java_version::OLDEST_RELEASE {
            return Err(usage(format!(
                "release {release} isn't supported; the oldest is {}",
                ferrion_java_version::OLDEST_RELEASE
            )));
        }
        if release > self.version {
            return Err(usage(format!(
                "JDK {} at {home} can't serve release {release}; use JDK {release} or later",
                self.version
            )));
        }
        match &self.layout {
            Layout::Jdk8 { .. } => Ok(Choice::Boot),
            Layout::Modular { ctsym: None, .. } if release == self.version => {
                Ok(Choice::Image(None))
            }
            Layout::Modular { ctsym: None, .. } => Err(usage(format!(
                "{home} has no lib/ct.sym, so it serves only release {}",
                self.version
            ))),
            Layout::Modular {
                ctsym: Some(ctsym), ..
            } => match ctsym.content(release)? {
                ReleaseContent::Classes => Ok(Choice::CtSym),
                ReleaseContent::SystemModules(modules) if release == self.version => {
                    Ok(Choice::Image(Some(modules)))
                }
                _ => Err(usage(format!(
                    "the lib/ct.sym of {home} doesn't have release {release}"
                ))),
            },
        }
    }

    /// The platform API for `release`: the classes javac sees with `--release <release>` on this
    /// JDK. Shared while anyone holds it.
    pub fn platform(&self, release: u32) -> Result<Arc<PlatformApi>> {
        if let Some(found) = self.lock().get(&release).and_then(Weak::upgrade) {
            return Ok(found);
        }
        let choice = self.platform_source(release)?;
        let platform = Arc::new(self.build(release, choice)?);
        let mut platforms = self.lock();
        if let Some(found) = platforms.get(&release).and_then(Weak::upgrade) {
            return Ok(found);
        }
        platforms.insert(release, Arc::downgrade(&platform));
        Ok(platform)
    }

    /// The platform API javac sees without `--release` (with `-source` and `-target`, or
    /// neither): the JDK's own image, every module of it, internal packages included, at the
    /// JDK's own release. It differs from `platform(<the JDK's version>)` when `ct.sym` has the
    /// JDK's own release, as from JDK 25: `ct.sym` has only what the modules export (observed
    /// with javac 27: `-source 17 --add-exports java.base/sun.security.jca=ALL-UNNAMED` finds
    /// `sun.security.jca.Providers`, which `--release 17` doesn't have, and `-source 8` finds it
    /// without the option). JDK 8's is its `platform(8)`. Shared while anyone holds it.
    pub fn image_platform(&self) -> Result<Arc<PlatformApi>> {
        if matches!(self.layout, Layout::Jdk8 { .. }) {
            return self.platform(8);
        }
        let mut slot = self
            .image_platform
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(found) = slot.upgrade() {
            return Ok(found);
        }
        let platform = Arc::new(self.build(self.version, Choice::Image(None))?);
        *slot = Arc::downgrade(&platform);
        Ok(platform)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Weak<PlatformApi>>> {
        self.platforms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn build(&self, release: u32, choice: Choice) -> Result<PlatformApi> {
        let (source, modules) = match (&self.layout, choice) {
            (Layout::Jdk8 { boot }, _) => (Source::Boot(boot.clone()), Vec::new()),
            (
                Layout::Modular {
                    ctsym: Some(ctsym), ..
                },
                Choice::CtSym,
            ) => {
                let index = ctsym.release(release)?;
                let modules = index.modules().to_vec();
                (
                    Source::CtSym {
                        ctsym: ctsym.clone(),
                        index,
                    },
                    modules,
                )
            }
            (Layout::Modular { image, .. }, Choice::Image(listed)) => {
                let allowed: HashSet<Box<str>> = match listed {
                    Some(listed) => listed.into_iter().map(Into::into).collect(),
                    None => image.modules().iter().cloned().collect(),
                };
                let mut modules = Vec::new();
                for module in image.modules().iter().filter(|m| allowed.contains(*m)) {
                    let Some(resource) = image.find_class(module, "module-info") else {
                        continue;
                    };
                    let descriptor = resource
                        .read()
                        .and_then(|bytes| ClassFile::parse(Arc::from(bytes))?.module_descriptor())
                        .map_err(|e| {
                            e.in_origin(Origin::Entry {
                                archive: image.path().unwrap_or(Path::new("")).into(),
                                entry: resource.name().into(),
                            })
                        })?;
                    modules.push(descriptor);
                }
                (
                    Source::Image {
                        image: image.clone(),
                        modules: allowed,
                    },
                    modules,
                )
            }
            (Layout::Modular { .. }, _) => {
                unreachable!("platform_source chose a source this JDK has")
            }
        };
        let visible = (!modules.is_empty()).then(|| visible_packages(&modules, release, &[]));
        Ok(PlatformApi {
            release,
            source,
            modules,
            visible,
        })
    }
}

enum Choice {
    CtSym,
    /// The image, filtered to the listed modules, or every module when `None`.
    Image(Option<Vec<String>>),
    Boot,
}

/// The packages code in the unnamed module can name: the unqualified exports of the modules
/// resolved by default, and of the modules they require (JEP 261). The default roots are the
/// modules that export a package without qualification and aren't marked "do not resolve by
/// default", with one difference for releases 9 and 10, which javac keeps with `--release 9` and
/// `--release 10` (observed with javac 27): there, of the `java.*` modules only `java.se` is a
/// root, so `java.xml.bind`, `java.corba`, and `java.smartcardio` aren't visible. `roots` are
/// more root modules, as javac's `--add-modules` gives them; `ALL-SYSTEM` makes every module one,
/// and `ALL-DEFAULT` adds the default roots, which are roots anyway.
fn visible_packages(
    modules: &[ModuleDescriptor],
    release: u32,
    roots: &[&str],
) -> HashSet<Box<str>> {
    let by_name: HashMap<&str, &ModuleDescriptor> =
        modules.iter().map(|m| (m.name.as_str(), m)).collect();
    let java_se_only = release <= 10 && by_name.contains_key("java.se");
    let all = roots.contains(&"ALL-SYSTEM");
    let is_root = |m: &ModuleDescriptor| {
        if all || roots.contains(&m.name.as_str()) {
            return true;
        }
        if java_se_only && m.name.starts_with("java.") {
            return m.name == "java.se";
        }
        m.is_resolved_by_default() && m.unqualified_exports().next().is_some()
    };
    let mut resolved: HashSet<&str> = HashSet::new();
    let mut work: Vec<&ModuleDescriptor> = modules.iter().filter(|m| is_root(m)).collect();
    while let Some(module) = work.pop() {
        if !resolved.insert(&module.name) {
            continue;
        }
        for requires in &module.requires {
            if let Some(required) = by_name.get(requires.module.as_str()) {
                work.push(required);
            }
        }
    }
    resolved
        .iter()
        .flat_map(|name| by_name[name].unqualified_exports())
        .map(Into::into)
        .collect()
}

#[derive(Debug)]
enum Source {
    CtSym {
        ctsym: Arc<CtSym>,
        index: CtSymRelease,
    },
    Image {
        image: Arc<Image>,
        modules: HashSet<Box<str>>,
    },
    Boot(Vec<BootEntry>),
}

/// Where a platform API's classes come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformSource {
    CtSym,
    Image,
    /// JDK 8's boot class path.
    BootClassPath,
}

/// The classes javac sees with `--release <release>` on a JDK.
#[derive(Debug)]
pub struct PlatformApi {
    release: u32,
    source: Source,
    modules: Vec<ModuleDescriptor>,
    /// `None` when there are no modules (release 8): every package is visible.
    visible: Option<HashSet<Box<str>>>,
}

impl PlatformApi {
    pub fn release(&self) -> u32 {
        self.release
    }

    pub fn source(&self) -> PlatformSource {
        match self.source {
            Source::CtSym { .. } => PlatformSource::CtSym,
            Source::Image { .. } => PlatformSource::Image,
            Source::Boot(_) => PlatformSource::BootClassPath,
        }
    }

    /// The module descriptors, for B2's readability; empty for release 8.
    pub fn modules(&self) -> &[ModuleDescriptor] {
        &self.modules
    }

    /// Whether code in the unnamed module can name classes in `package`: an unqualified export
    /// of a module resolved by default, or any package when there are no modules.
    pub fn is_visible(&self, package: &PackageName) -> bool {
        self.visible
            .as_ref()
            .is_none_or(|visible| visible.contains(package.as_str()))
    }

    /// The packages code in the unnamed module can name when `roots` are root modules too, as
    /// javac's `--add-modules` makes them (`ALL-SYSTEM` for every module), or `None` when there
    /// are no modules and every package is visible.
    pub fn visible_with_roots(&self, roots: &[&str]) -> Option<HashSet<Box<str>>> {
        (!self.modules.is_empty()).then(|| visible_packages(&self.modules, self.release, roots))
    }

    /// The visible packages, sorted, or `None` when every package is visible.
    pub fn visible_packages(&self) -> Option<Vec<&str>> {
        let mut packages: Vec<&str> = self.visible.as_ref()?.iter().map(|p| &**p).collect();
        packages.sort_unstable();
        Some(packages)
    }

    /// Any class the platform has, exported or not, for supertypes and signatures.
    pub fn find(&self, name: &ClassName) -> Result<Option<ClassBytes>> {
        match &self.source {
            Source::CtSym { ctsym, index } => {
                let Some((entry, _)) = index.find(name.as_str()) else {
                    return Ok(None);
                };
                let archive = ctsym.archive();
                let Some(entry) = archive.entry(entry) else {
                    return Ok(None);
                };
                let origin = archive.source().entry_origin(entry.name());
                Ok(Some(ClassBytes {
                    bytes: entry.read()?.into(),
                    origin,
                }))
            }
            Source::Image { image, modules } => {
                let found = image
                    .classes_named(name.as_str())
                    .find(|resource| modules.contains(resource.module()));
                let Some(resource) = found else {
                    return Ok(None);
                };
                let origin = Origin::Entry {
                    archive: image.path().unwrap_or(Path::new("")).into(),
                    entry: resource.name().into(),
                };
                Ok(Some(ClassBytes {
                    bytes: resource.read()?.into(),
                    origin,
                }))
            }
            Source::Boot(entries) => {
                for entry in entries {
                    if let Some(found) = entry.find(name)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
        }
    }

    /// The module of the class with internal name `name`, when the platform has the class and
    /// knows its module.
    pub fn module_of(&self, name: &ClassName) -> Option<String> {
        match &self.source {
            Source::CtSym { index, .. } => index.find(name.as_str())?.1.map(str::to_owned),
            Source::Image { image, modules } => image
                .classes_named(name.as_str())
                .find(|r| modules.contains(r.module()))
                .map(|r| r.module().to_owned()),
            Source::Boot(_) => None,
        }
    }

    /// The class names in `name`, or `None` when the platform has no such package.
    pub fn package(&self, name: &PackageName) -> Result<Option<PackageListing>> {
        match &self.source {
            Source::CtSym { index, .. } => Ok(index
                .package(name.as_str())
                .map(|names| PackageListing::new(names.map(Into::into).collect()))),
            Source::Image { image, modules } => {
                let names: Vec<Box<str>> = image
                    .package(name.as_str())
                    .filter(|r| r.is_class() && modules.contains(r.module()))
                    .map(|r| r.base().into())
                    .collect();
                Ok((!names.is_empty()).then(|| PackageListing::new(names)))
            }
            Source::Boot(entries) => {
                let mut listing: Option<PackageListing> = None;
                for entry in entries {
                    if let Some(found) = entry.package(name)? {
                        match &mut listing {
                            Some(listing) => listing.merge(found),
                            None => listing = Some(found),
                        }
                    }
                }
                Ok(listing)
            }
        }
    }

    /// Every class the platform serves, by internal name, sorted. For checks: B2 looks classes
    /// up by name.
    pub fn class_names(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = match &self.source {
            Source::CtSym { index, .. } => index
                .packages()
                .flat_map(|package| {
                    index
                        .package(package)
                        .into_iter()
                        .flatten()
                        .map(move |simple| join(package, simple))
                })
                .collect(),
            Source::Image { image, modules } => image
                .resources()
                .filter(|r| r.is_class() && modules.contains(r.module()))
                .map(|r| join(&r.package(), &r.base()))
                .collect(),
            Source::Boot(entries) => {
                let mut names = Vec::new();
                for entry in entries {
                    match entry {
                        BootEntry::RtCtSym(archive) => names.extend(
                            archive
                                .with_prefix(RT_CTSYM_PREFIX)
                                .filter_map(|e| {
                                    e.name()[RT_CTSYM_PREFIX.len()..].strip_suffix(".class")
                                })
                                .map(str::to_owned),
                        ),
                        BootEntry::Entry(entry) => names.extend(entry.class_names()?),
                    }
                }
                names
            }
        };
        names.sort_unstable();
        names.dedup();
        Ok(names)
    }
}

fn join(package: &str, simple: &str) -> String {
    if package.is_empty() {
        simple.to_owned()
    } else {
        format!("{package}/{simple}")
    }
}

impl ClassPathEntry {
    /// Every class name in a jar entry; directories aren't listed.
    pub(crate) fn class_names(&self) -> Result<Vec<String>> {
        Ok(self
            .as_jar()
            .map(|jar| {
                jar.entries()
                    .filter_map(|(name, _)| name.strip_suffix(".class"))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests;
