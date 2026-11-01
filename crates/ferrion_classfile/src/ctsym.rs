//! `lib/ct.sym`, the file javac reads for `--release` (docs/design/classfile-ctsym.md). A zip of
//! class files named `.sig`, whose layout was learned by observation.
//!
//! Each top-level directory names a set of releases, one base-36 letter per release
//! (`89ABC`). Two layouts were observed, told apart by the entries themselves:
//!
//! - **flat** (JDK 11): `<letters>/<package>/<Class>.sig`, with module descriptors under
//!   `<letters>-modules/<module>/module-info.sig`;
//! - **modular** (JDK 17 and later): `<letters>/<module>/<package>/<Class>.sig`, with
//!   `<letters>/<module>/module-info.sig`.
//!
//! A release whose directory holds only a `system-modules` file (JDK 11 to 21, for the JDK's
//! own release) is served from the image.

use std::collections::HashMap;
use std::sync::Arc;

use crate::class::{ClassFile, ModuleDescriptor};
use crate::error::{Error, ErrorKind, Result};
use crate::zip::ZipArchive;

/// The newest release a letter can name: `Z`.
pub const MAX_RELEASE: u32 = 35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// JDK 11: no module directories; descriptors under `<letters>-modules/`.
    Flat,
    /// JDK 17 and later: a module directory under each release directory.
    Modular,
}

/// What `ct.sym` has for a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseContent {
    /// Class files: `ct.sym` serves the release.
    Classes,
    /// Only the list of modules javac makes visible when it reads the release from the image.
    SystemModules(Vec<String>),
    /// Nothing.
    Absent,
}

/// The letter for `release`, or `None` beyond `Z`.
pub fn letter(release: u32) -> Option<char> {
    char::from_digit(release, 36).map(|c| c.to_ascii_uppercase())
}

/// The releases a directory name stands for, or `None` when it isn't a release directory.
fn releases_of(name: &str) -> Option<Vec<u32>> {
    if name.is_empty() {
        return None;
    }
    name.chars()
        .map(|c| match c {
            '0'..='9' | 'A'..='Z' => c.to_digit(36),
            _ => None,
        })
        .collect()
}

struct Directory {
    name: Box<str>,
    releases: Vec<u32>,
    /// `<letters>-modules`, the flat layout's descriptor directory.
    modules: bool,
    classes: bool,
    system_modules: bool,
}

/// A `ct.sym` file, with its release directories found.
pub struct CtSym {
    archive: Arc<ZipArchive>,
    layout: Layout,
    directories: Vec<Directory>,
}

impl std::fmt::Debug for CtSym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtSym")
            .field("archive", &self.archive)
            .field("layout", &self.layout)
            .field("releases", &self.releases())
            .finish()
    }
}

/// One class of a release.
#[derive(Clone, Debug)]
struct Class {
    simple: Box<str>,
    entry: u32,
    module: Option<u16>,
}

/// The classes and modules of one release.
pub struct CtSymRelease {
    release: u32,
    packages: HashMap<Box<str>, Vec<Class>>,
    module_names: Vec<Box<str>>,
    modules: Vec<ModuleDescriptor>,
    problems: Vec<Error>,
}

impl CtSym {
    pub fn new(archive: Arc<ZipArchive>) -> CtSym {
        let mut directories: Vec<Directory> = Vec::new();
        let mut by_name: HashMap<&str, usize> = HashMap::new();
        let mut flat = false;
        for entry in archive.unique_entries() {
            let Some((top, rest)) = entry.name().split_once('/') else {
                continue;
            };
            let (letters, modules) = match top.strip_suffix("-modules") {
                Some(letters) => (letters, true),
                None => (top, false),
            };
            let Some(releases) = releases_of(letters) else {
                continue;
            };
            flat |= modules;
            let index = *by_name.entry(top).or_insert_with(|| {
                directories.push(Directory {
                    name: top.into(),
                    releases,
                    modules,
                    classes: false,
                    system_modules: false,
                });
                directories.len() - 1
            });
            let directory = &mut directories[index];
            if rest == "system-modules" {
                directory.system_modules = true;
            } else if rest.ends_with(".sig") && !modules && !rest.ends_with("/module-info.sig") {
                directory.classes = true;
            }
        }
        let layout = if flat { Layout::Flat } else { Layout::Modular };
        CtSym {
            archive,
            layout,
            directories,
        }
    }

    pub fn archive(&self) -> &Arc<ZipArchive> {
        &self.archive
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The releases with class files, sorted.
    pub fn releases(&self) -> Vec<u32> {
        let mut releases: Vec<u32> = self
            .directories
            .iter()
            .filter(|d| d.classes)
            .flat_map(|d| d.releases.iter().copied())
            .collect();
        releases.sort_unstable();
        releases.dedup();
        releases
    }

    fn check(release: u32) -> Result<()> {
        if release > MAX_RELEASE {
            return Err(Error::unsupported(format!(
                "release {release} can't be named in ct.sym, whose letters end at Z (release {MAX_RELEASE})"
            )));
        }
        Ok(())
    }

    /// What `ct.sym` has for `release`.
    pub fn content(&self, release: u32) -> Result<ReleaseContent> {
        CtSym::check(release)?;
        let directories = || {
            self.directories
                .iter()
                .filter(|d| d.releases.contains(&release))
        };
        if directories().any(|d| d.classes) {
            return Ok(ReleaseContent::Classes);
        }
        for directory in directories().filter(|d| d.system_modules) {
            let name = format!("{}/system-modules", directory.name);
            let Some(entry) = self.archive.find(&name) else {
                continue;
            };
            let bytes = entry.read()?;
            let text = String::from_utf8(bytes).map_err(|_| {
                Error::new(ErrorKind::Malformed, "system-modules isn't UTF-8")
                    .in_origin(self.archive.source().entry_origin(&name))
            })?;
            let modules = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
            return Ok(ReleaseContent::SystemModules(modules));
        }
        Ok(ReleaseContent::Absent)
    }

    /// Indexes `release`'s classes and reads its module descriptors. A release without class
    /// files gives an index with none.
    pub fn release(&self, release: u32) -> Result<CtSymRelease> {
        CtSym::check(release)?;
        let mut result = CtSymRelease {
            release,
            packages: HashMap::new(),
            module_names: Vec::new(),
            modules: Vec::new(),
            problems: Vec::new(),
        };
        let mut module_ids: HashMap<String, u16> = HashMap::new();
        let mut descriptors: Vec<u32> = Vec::new();
        // Each name is taken from the first directory that has it (C8 found none in two).
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for directory in self
            .directories
            .iter()
            .filter(|d| d.releases.contains(&release))
        {
            let prefix = format!("{}/", directory.name);
            for entry in self.archive.with_prefix(&prefix) {
                let path = &entry.name()[prefix.len()..];
                let Some(path) = path.strip_suffix(".sig") else {
                    continue;
                };
                if !seen.insert(path) {
                    continue;
                }
                let (module, path) = match (self.layout, directory.modules) {
                    (Layout::Flat, true) => match path.split_once('/') {
                        Some((_, "module-info")) => {
                            descriptors.push(entry.index());
                            continue;
                        }
                        _ => continue,
                    },
                    (Layout::Flat, false) => (None, path),
                    (Layout::Modular, _) => {
                        let Some((module, path)) = path.split_once('/') else {
                            continue;
                        };
                        if path == "module-info" {
                            descriptors.push(entry.index());
                            continue;
                        }
                        let next = module_ids.len() as u16;
                        let id = *module_ids.entry(module.to_owned()).or_insert(next);
                        (Some(id), path)
                    }
                };
                let (package, simple) = path.rsplit_once('/').unwrap_or(("", path));
                result
                    .packages
                    .entry(package.into())
                    .or_default()
                    .push(Class {
                        simple: simple.into(),
                        entry: entry.index(),
                        module,
                    });
            }
        }
        for index in descriptors {
            let Some(entry) = self.archive.entry(index) else {
                continue;
            };
            let descriptor = entry.read().and_then(|bytes| {
                let class = ClassFile::parse(Arc::from(bytes))?;
                class.module_descriptor()
            });
            match descriptor {
                Ok(descriptor) => result.modules.push(descriptor),
                Err(error) => result
                    .problems
                    .push(error.in_origin(self.archive.source().entry_origin(entry.name()))),
            }
        }
        result.modules.sort_by(|a, b| a.name.cmp(&b.name));
        let mut names: Vec<(String, u16)> = module_ids.into_iter().collect();
        names.sort_by_key(|(_, id)| *id);
        result.module_names = names
            .into_iter()
            .map(|(name, _)| name.into_boxed_str())
            .collect();
        if self.layout == Layout::Flat {
            // A class's module is the one that exports its package.
            for descriptor in &result.modules {
                let id = result.module_names.len() as u16;
                result.module_names.push(descriptor.name.as_str().into());
                for export in &descriptor.exports {
                    if let Some(classes) = result.packages.get_mut(export.package.as_str()) {
                        for class in classes.iter_mut().filter(|c| c.module.is_none()) {
                            class.module = Some(id);
                        }
                    }
                }
            }
        }
        for classes in result.packages.values_mut() {
            classes.sort_by(|a, b| a.simple.cmp(&b.simple));
        }
        Ok(result)
    }
}

impl std::fmt::Debug for CtSymRelease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtSymRelease")
            .field("release", &self.release)
            .field("packages", &self.packages.len())
            .field("modules", &self.modules.len())
            .finish_non_exhaustive()
    }
}

impl CtSymRelease {
    pub fn release(&self) -> u32 {
        self.release
    }

    /// The module descriptors of the release, sorted by name; empty for releases before 9.
    pub fn modules(&self) -> &[ModuleDescriptor] {
        &self.modules
    }

    /// Descriptors that couldn't be read.
    pub fn problems(&self) -> &[Error] {
        &self.problems
    }

    /// Every package with a class.
    pub fn packages(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(|k| &**k)
    }

    /// The simple names of the classes in `package`, sorted.
    pub fn package(&self, package: &str) -> Option<impl Iterator<Item = &str>> {
        self.packages
            .get(package)
            .map(|classes| classes.iter().map(|c| &*c.simple))
    }

    /// The entry index and module of the class with internal name `name`.
    pub fn find(&self, name: &str) -> Option<(u32, Option<&str>)> {
        let (package, simple) = name.rsplit_once('/').unwrap_or(("", name));
        let classes = self.packages.get(package)?;
        let position = classes.binary_search_by(|c| (*c.simple).cmp(simple)).ok()?;
        let class = &classes[position];
        Some((
            class.entry,
            class.module.map(|id| &*self.module_names[usize::from(id)]),
        ))
    }

    /// The module of the classes in `package`, when known.
    pub fn module_of(&self, package: &str) -> Option<&str> {
        let class = self.packages.get(package)?.first()?;
        class.module.map(|id| &*self.module_names[usize::from(id)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ClassBuilder, ZipBuilder};

    fn module_info(name: &str, exports: &[&str]) -> Vec<u8> {
        let mut b = ClassBuilder::new("module-info");
        b.set_super(None);
        b.access = 0x8000;
        let module = b.module(name);
        let mut contents = Vec::new();
        for value in [module, 0, 0, 0, exports.len() as u16] {
            contents.extend_from_slice(&value.to_be_bytes());
        }
        for export in exports {
            let package = b.package(export);
            for value in [package, 0, 0] {
                contents.extend_from_slice(&value.to_be_bytes());
            }
        }
        contents.extend_from_slice(&[0; 6]);
        let attribute = b.attribute("Module", &contents);
        b.class_attribute(attribute);
        b.finish()
    }

    fn class(name: &str) -> Vec<u8> {
        ClassBuilder::new(name).finish()
    }

    /// JDK 11's layout, in miniature.
    fn flat() -> CtSym {
        let mut b = ZipBuilder::new();
        b.deflated("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n")
            .deflated("879A/java/lang/Object.sig", &class("java/lang/Object"))
            .deflated("9A/java/lang/Module.sig", &class("java/lang/Module"))
            .deflated("9A/jdk/internal/Hidden.sig", &class("jdk/internal/Hidden"))
            .deflated(
                "9A-modules/java.base/module-info.sig",
                &module_info("java.base", &["java/lang"]),
            )
            .deflated("B/system-modules", b"java.base\njava.desktop\n");
        CtSym::new(Arc::new(
            ZipArchive::from_bytes(Arc::from(b.finish())).unwrap(),
        ))
    }

    /// JDK 17's layout, in miniature.
    fn modular() -> CtSym {
        let mut b = ZipBuilder::new();
        b.deflated(
            "8/java.base/java/lang/Object.sig",
            &class("java/lang/Object"),
        )
        .deflated(
            "9ABC/java.base/java/lang/Object.sig",
            &class("java/lang/Object"),
        )
        .deflated(
            "9ABC/java.base/module-info.sig",
            &module_info("java.base", &["java/lang"]),
        )
        .deflated(
            "BC/jdk.httpserver/com/sun/net/httpserver/HttpServer.sig",
            &class("com/sun/net/httpserver/HttpServer"),
        )
        .deflated("H/system-modules", b"java.base\n");
        CtSym::new(Arc::new(
            ZipArchive::from_bytes(Arc::from(b.finish())).unwrap(),
        ))
    }

    #[test]
    fn letters() {
        assert_eq!(letter(8), Some('8'));
        assert_eq!(letter(10), Some('A'));
        assert_eq!(letter(27), Some('R'));
        assert_eq!(letter(35), Some('Z'));
        assert_eq!(letter(36), None);
        assert_eq!(releases_of("879A"), Some(vec![8, 7, 9, 10]));
        assert_eq!(releases_of("META-INF"), None);
    }

    #[test]
    fn flat_layout() {
        let ctsym = flat();
        assert_eq!(ctsym.layout(), Layout::Flat);
        assert_eq!(ctsym.releases(), [7, 8, 9, 10]);
        assert_eq!(ctsym.content(8).unwrap(), ReleaseContent::Classes);
        assert_eq!(
            ctsym.content(11).unwrap(),
            ReleaseContent::SystemModules(vec!["java.base".into(), "java.desktop".into()])
        );
        assert_eq!(ctsym.content(12).unwrap(), ReleaseContent::Absent);
        assert_eq!(
            ctsym.content(36).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
        let r8 = ctsym.release(8).unwrap();
        assert!(r8.modules().is_empty());
        assert_eq!(r8.find("java/lang/Object").map(|(_, m)| m), Some(None));
        assert!(r8.find("java/lang/Module").is_none());
        let r10 = ctsym.release(10).unwrap();
        assert_eq!(r10.modules().len(), 1);
        assert_eq!(r10.find("java/lang/Module").unwrap().1, Some("java.base"));
        // A class in a package no module exports belongs to no known module.
        assert_eq!(r10.find("jdk/internal/Hidden").unwrap().1, None);
        assert_eq!(
            r10.package("java/lang").unwrap().collect::<Vec<_>>(),
            ["Module", "Object"]
        );
    }

    #[test]
    fn modular_layout() {
        let ctsym = modular();
        assert_eq!(ctsym.layout(), Layout::Modular);
        assert_eq!(ctsym.releases(), [8, 9, 10, 11, 12]);
        assert!(
            matches!(ctsym.content(17).unwrap(), ReleaseContent::SystemModules(m) if m == ["java.base"])
        );
        let r8 = ctsym.release(8).unwrap();
        assert!(r8.modules().is_empty());
        assert_eq!(r8.find("java/lang/Object").unwrap().1, Some("java.base"));
        let r11 = ctsym.release(11).unwrap();
        assert_eq!(r11.modules()[0].name, "java.base");
        assert_eq!(
            r11.find("com/sun/net/httpserver/HttpServer").unwrap().1,
            Some("jdk.httpserver")
        );
        assert_eq!(r11.module_of("java/lang"), Some("java.base"));
        let r9 = ctsym.release(9).unwrap();
        assert!(r9.find("com/sun/net/httpserver/HttpServer").is_none());
    }
}
