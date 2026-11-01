//! The package table: each source set's packages and the top-level types its own files declare
//! in them (docs/design/semantic-incremental.md#project-symbols). The first part of layer 3.
//!
//! Two kinds of derived entry make it up:
//!
//! - **The package index,** one per source set: each package some file of the set declares, with
//!   those files. It reads the set's file list and each file's package and top-level types
//!   (`TopLevel`), so it is rebuilt by a pass over the set's summaries when a file is added or
//!   removed or a file's top-level part changes, and an edit anywhere else in a summary doesn't
//!   reach it. A rebuilt index equal to the old one is cut off there.
//! - **Package entries,** one per package asked for: the package's files and its top-level types
//!   by simple name, each with the files declaring it (more than one makes it ambiguous). An entry
//!   reads only its own package's files in the index and their top-level parts, so a change to
//!   one package leaves the others' entries valid without recomputing them, and whatever reads an
//!   unchanged package's entry stays valid too.
//!
//! The class path's and the platform's packages are merged in by the source set's view (W4); this
//! table holds the set's own sources only.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::engine::derived::{Dep, Memo, Reads};
use crate::summary::{Summary, UnitKind};
use crate::{Cancelled, FileId, Fingerprint, Snapshot, SourceSetId};

/// What a file contributes to the package table: its package, and the simple names of its
/// top-level types in order. `package` is `None` for a `module-info.java`, which isn't in a
/// package; the unnamed package is the empty name.
#[derive(PartialEq, Eq, Hash, Debug)]
pub(crate) struct TopLevel {
    package: Option<Arc<str>>,
    types: Box<[Box<str>]>,
}

impl TopLevel {
    pub(crate) fn of(summary: &Summary) -> TopLevel {
        TopLevel {
            package: (summary.kind != UnitKind::ModuleInfo).then(|| summary.package_name().into()),
            types: summary.top_level_types().map(|t| t.name.clone()).collect(),
        }
    }
}

/// A source set's packages, each with the files that declare it.
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct PackageIndex {
    packages: BTreeMap<Arc<str>, PackageFiles>,
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub(crate) struct PackageFiles {
    files: Arc<[FileId]>,
    /// The fingerprint of `files`, which a package entry reads.
    pub(crate) fingerprint: Fingerprint,
}

impl PackageIndex {
    /// Every package's name, sorted; the unnamed package is the empty name.
    pub fn names(&self) -> impl Iterator<Item = &Arc<str>> {
        self.packages.keys()
    }

    /// The files declaring `package`, in the source set's order.
    pub fn files(&self, package: &str) -> Option<&[FileId]> {
        self.packages.get(package).map(|p| &*p.files)
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub(crate) fn package_files(&self, package: &str) -> Option<&PackageFiles> {
        self.packages.get(package)
    }
}

/// A package of a source set's own sources.
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct Package {
    name: Arc<str>,
    files: Arc<[FileId]>,
    /// Sorted by name.
    types: Box<[PackageType]>,
}

/// A top-level type of a package, by simple name.
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct PackageType {
    pub name: Box<str>,
    /// The files declaring a top-level type of this name, in the source set's order. More than
    /// one makes every use ambiguous (docs/design/semantic-names.md#duplicates).
    pub declarations: Box<[FileId]>,
}

impl Package {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The files declaring the package, `package-info.java` and files without types included.
    pub fn files(&self) -> &[FileId] {
        &self.files
    }

    /// The top-level types, sorted by name.
    pub fn types(&self) -> &[PackageType] {
        &self.types
    }

    pub fn get(&self, name: &str) -> Option<&PackageType> {
        let index = self.types.binary_search_by(|t| (*t.name).cmp(name)).ok()?;
        Some(&self.types[index])
    }
}

impl PackageType {
    pub fn is_ambiguous(&self) -> bool {
        self.declarations.len() > 1
    }
}

impl Snapshot {
    /// `set`'s package index.
    pub fn package_index(&self, set: SourceSetId) -> Result<Arc<PackageIndex>, Cancelled> {
        self.check()?;
        Ok(self.package_index_memo(set)?.value.clone())
    }

    /// The package `name` of `set`'s own sources, or `None` if none of its files declares it.
    pub fn package(&self, set: SourceSetId, name: &str) -> Result<Option<Arc<Package>>, Cancelled> {
        self.check()?;
        let package = self.package_memo(set, &Arc::from(name))?.value.clone();
        Ok((!package.files.is_empty()).then_some(package))
    }

    pub(crate) fn package_index_memo(
        &self,
        set: SourceSetId,
    ) -> Result<Arc<Memo<PackageIndex>>, Cancelled> {
        self.shared()
            .package_indexes
            .get(self, &set, |reads| package_index(reads, set))
    }

    pub(crate) fn package_memo(
        &self,
        set: SourceSetId,
        name: &Arc<str>,
    ) -> Result<Arc<Memo<Package>>, Cancelled> {
        self.shared()
            .packages
            .get(self, &(set, name.clone()), |reads| {
                package(reads, set, name)
            })
    }
}

/// The accessors computations read the package table through, recording each read.
impl Reads<'_> {
    /// `set`'s package index. Records the whole index: for a computation that lists packages.
    #[allow(dead_code, reason = "for views, which W4 adds")]
    pub(crate) fn package_index(
        &mut self,
        set: SourceSetId,
    ) -> Result<Arc<PackageIndex>, Cancelled> {
        let memo = self.snapshot.package_index_memo(set)?;
        self.record(Dep::PackageIndex(set), memo.fingerprint);
        Ok(memo.value.clone())
    }

    /// The files of package `name` in `set`'s index. Records only that package's part of it.
    pub(crate) fn package_files(
        &mut self,
        set: SourceSetId,
        name: &Arc<str>,
    ) -> Result<Option<Arc<[FileId]>>, Cancelled> {
        let index = self.snapshot.package_index_memo(set)?;
        let files = index.value.package_files(name);
        self.record(
            Dep::PackageFiles(set, name.clone()),
            files.map_or(Fingerprint::ABSENT, |p| p.fingerprint),
        );
        Ok(files.map(|p| p.files.clone()))
    }

    /// The package entry `name` of `set`. Records it.
    #[allow(dead_code, reason = "for type entries and views, which W4 adds")]
    pub(crate) fn package(
        &mut self,
        set: SourceSetId,
        name: &Arc<str>,
    ) -> Result<Arc<Package>, Cancelled> {
        let memo = self.snapshot.package_memo(set, name)?;
        self.record(Dep::Package(set, name.clone()), memo.fingerprint);
        Ok(memo.value.clone())
    }
}

fn package_index(reads: &mut Reads<'_>, set: SourceSetId) -> Result<PackageIndex, Cancelled> {
    let mut packages: BTreeMap<Arc<str>, Vec<FileId>> = BTreeMap::new();
    for &file in reads.set_files(set)?.iter() {
        reads.check()?;
        if let Some(package) = &reads.top_level(file)?.package {
            packages.entry(package.clone()).or_default().push(file);
        }
    }
    Ok(PackageIndex {
        packages: packages
            .into_iter()
            .map(|(name, files)| {
                let fingerprint = Fingerprint::of(&files);
                let files = PackageFiles {
                    files: Arc::from(files),
                    fingerprint,
                };
                (name, files)
            })
            .collect(),
    })
}

fn package(reads: &mut Reads<'_>, set: SourceSetId, name: &Arc<str>) -> Result<Package, Cancelled> {
    let files = reads
        .package_files(set, name)?
        .unwrap_or_else(|| Arc::from([]));
    let mut types: BTreeMap<Box<str>, Vec<FileId>> = BTreeMap::new();
    for &file in files.iter() {
        reads.check()?;
        for ty in &reads.top_level(file)?.types {
            types.entry(ty.clone()).or_default().push(file);
        }
    }
    Ok(Package {
        name: name.clone(),
        files,
        types: types
            .into_iter()
            .map(|(name, files)| PackageType {
                name,
                declarations: files.into_boxed_slice(),
            })
            .collect(),
    })
}
