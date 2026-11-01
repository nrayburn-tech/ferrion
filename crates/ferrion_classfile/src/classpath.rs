//! Class path entries and lookup (docs/design/classfile.md, "Class path entries and lookup").

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, ErrorKind, Origin, Result};
use crate::jar::Jar;
use crate::names::{ClassName, PackageName};

/// The bytes of one class, with where they came from.
#[derive(Clone, Debug)]
pub struct ClassBytes {
    pub bytes: Arc<[u8]>,
    pub origin: Origin,
}

/// The class names in a package, without reading any class: simple names (`Map$Entry`), sorted
/// and without duplicates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageListing {
    names: Vec<Box<str>>,
}

impl PackageListing {
    pub fn new(mut names: Vec<Box<str>>) -> PackageListing {
        names.sort_unstable();
        names.dedup();
        PackageListing { names }
    }

    pub fn names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.names.iter().map(|n| &**n)
    }

    pub fn contains(&self, simple: &str) -> bool {
        self.names.binary_search_by(|n| (**n).cmp(simple)).is_ok()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Adds `other`'s names.
    pub fn merge(&mut self, other: PackageListing) {
        self.names.extend(other.names);
        self.names.sort_unstable();
        self.names.dedup();
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Directory(Arc<Path>),
    Jar(Arc<Jar>),
}

/// One class path entry: a directory, or a jar resolved for a release. Immutable once made, and
/// cheap to clone.
#[derive(Clone, Debug)]
pub struct ClassPathEntry(Kind);

impl ClassPathEntry {
    /// A directory of class files, read on demand and never cached: it is usually build
    /// output that changes.
    pub fn directory(path: &Path) -> ClassPathEntry {
        ClassPathEntry(Kind::Directory(path.into()))
    }

    pub fn jar(jar: Arc<Jar>) -> ClassPathEntry {
        ClassPathEntry(Kind::Jar(jar))
    }

    /// The jar, for a jar entry.
    pub fn as_jar(&self) -> Option<&Arc<Jar>> {
        match &self.0 {
            Kind::Jar(jar) => Some(jar),
            Kind::Directory(_) => None,
        }
    }

    pub fn find(&self, name: &ClassName) -> Result<Option<ClassBytes>> {
        match &self.0 {
            Kind::Directory(dir) => find_in_directory(dir, name),
            Kind::Jar(jar) => {
                let entry_name = format!("{}.class", name.as_str());
                let Some(entry) = jar.find(&entry_name) else {
                    return Ok(None);
                };
                let origin = jar.archive().source().entry_origin(entry.name());
                let bytes = entry.read()?;
                Ok(Some(ClassBytes {
                    bytes: bytes.into(),
                    origin,
                }))
            }
        }
    }

    pub fn package(&self, name: &PackageName) -> Result<Option<PackageListing>> {
        match &self.0 {
            Kind::Directory(dir) => list_directory(dir, name),
            Kind::Jar(jar) => Ok(jar
                .package(name.as_str())
                .map(|names| PackageListing::new(names.into_iter().map(Into::into).collect()))),
        }
    }
}

/// Whether `dir` has an entry spelled exactly `name`. Case-insensitive file systems (macOS and
/// Windows, usually) find `foo/BAR.class` for `foo/Bar.class`; javac on a case-sensitive one
/// wouldn't, so a case mismatch is treated as missing.
fn has_exact(dir: &Path, name: &str) -> Result<bool> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(Error::io(&e, dir)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(&e, dir))?;
        if entry.file_name() == name {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The directory for a package, or `None` when it doesn't exist with exactly that spelling.
fn package_dir(root: &Path, package: &str) -> Result<Option<PathBuf>> {
    let mut dir = root.to_path_buf();
    if package.is_empty() {
        return Ok(Some(dir));
    }
    for part in package.split('/') {
        if part.is_empty() || part == "." || part == ".." || !has_exact(&dir, part)? {
            return Ok(None);
        }
        dir.push(part);
    }
    Ok(Some(dir))
}

fn find_in_directory(root: &Path, name: &ClassName) -> Result<Option<ClassBytes>> {
    let file = format!("{}.class", name.simple());
    // A quick check before the exact-spelling walk.
    let mut path = root.join(name.package());
    path.push(&file);
    if !path.is_file() {
        return Ok(None);
    }
    let Some(dir) = package_dir(root, name.package())? else {
        return Ok(None);
    };
    if !has_exact(&dir, &file)? {
        return Ok(None);
    }
    let path = dir.join(&file);
    let bytes = std::fs::read(&path).map_err(|e| Error::io(&e, &path))?;
    if bytes.len() as u64 > crate::class::MAX_CLASS_SIZE {
        return Err(Error::new(
            ErrorKind::TooLarge,
            "the class file is above the size limit",
        )
        .in_origin(Origin::File(path.into())));
    }
    Ok(Some(ClassBytes {
        bytes: bytes.into(),
        origin: Origin::File(path.into()),
    }))
}

fn list_directory(root: &Path, package: &PackageName) -> Result<Option<PackageListing>> {
    let Some(dir) = package_dir(root, package.as_str())? else {
        return Ok(None);
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&e, &dir)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(&e, &dir))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if let Some(simple) = name.strip_suffix(".class")
            && entry.path().is_file()
        {
            names.push(simple.into());
        }
    }
    Ok((!names.is_empty()).then(|| PackageListing::new(names)))
}

/// An ordered list of entries. The first entry that has a class wins, as with javac.
#[derive(Clone, Debug, Default)]
pub struct ClassPath {
    entries: Vec<ClassPathEntry>,
}

impl ClassPath {
    pub fn new(entries: Vec<ClassPathEntry>) -> ClassPath {
        ClassPath { entries }
    }

    pub fn entries(&self) -> &[ClassPathEntry] {
        &self.entries
    }

    /// The class from the first entry that has it. An entry that fails is an error for this
    /// lookup.
    pub fn find(&self, name: &ClassName) -> Result<Option<ClassBytes>> {
        for entry in &self.entries {
            if let Some(found) = entry.find(name)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// The class names in `name` across every entry, or `None` when no entry has the package.
    pub fn package(&self, name: &PackageName) -> Result<Option<PackageListing>> {
        let mut listing: Option<PackageListing> = None;
        for entry in &self.entries {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::tests::Scratch;
    use crate::testing::ZipBuilder;
    use crate::zip::ZipArchive;

    #[test]
    fn first_entry_wins() {
        let scratch = Scratch::new("classpath");
        scratch.write("classes/p/A.class", b"dir A");
        scratch.write("classes/p/Only.class", b"dir only");
        scratch.write("classes/p/notes.txt", b"");
        let mut b = ZipBuilder::new();
        b.stored("p/A.class", b"jar A")
            .stored("p/B.class", b"jar B");
        let jar = Arc::new(Jar::new(
            Arc::new(ZipArchive::from_bytes(Arc::from(b.finish())).unwrap()),
            21,
        ));
        let path = ClassPath::new(vec![
            ClassPathEntry::directory(&scratch.0.join("classes")),
            ClassPathEntry::jar(jar.clone()),
        ]);
        let read = |name: &str| {
            path.find(&ClassName::new(name))
                .unwrap()
                .map(|c| c.bytes.to_vec())
        };
        assert_eq!(read("p/A").unwrap(), b"dir A");
        assert_eq!(read("p/B").unwrap(), b"jar B");
        assert_eq!(read("p/Only").unwrap(), b"dir only");
        assert_eq!(read("p/Missing"), None);
        assert_eq!(read("q/A"), None);
        let listing = path.package(&PackageName::new("p")).unwrap().unwrap();
        assert_eq!(listing.names().collect::<Vec<_>>(), ["A", "B", "Only"]);
        assert!(path.package(&PackageName::new("q")).unwrap().is_none());
        // The origin names the file.
        let found = path.find(&ClassName::new("p/Only")).unwrap().unwrap();
        assert!(matches!(found.origin, Origin::File(p) if p.ends_with("p/Only.class")));
    }

    #[test]
    fn spelling_must_match_exactly() {
        let scratch = Scratch::new("case");
        scratch.write("classes/foo/Bar.class", b"bar");
        let entry = ClassPathEntry::directory(&scratch.0.join("classes"));
        assert!(entry.find(&ClassName::new("foo/Bar")).unwrap().is_some());
        // Found on a case-insensitive file system, but not with this spelling.
        assert!(entry.find(&ClassName::new("foo/BAR")).unwrap().is_none());
        assert!(entry.find(&ClassName::new("FOO/Bar")).unwrap().is_none());
        assert!(entry.package(&PackageName::new("FOO")).unwrap().is_none());
        assert!(
            entry
                .find(&ClassName::new("../classes/foo/Bar"))
                .unwrap()
                .is_none()
        );
    }
}
