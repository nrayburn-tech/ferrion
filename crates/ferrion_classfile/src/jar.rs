//! Jars: the manifest, and multi-release jars resolved for a release
//! (docs/design/classfile-containers.md, "Jars"). The rules are the JAR File Specification's and
//! JEP 238's.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::zip::{ZipArchive, ZipEntry};

const MANIFEST: &str = "META-INF/MANIFEST.MF";
const VERSIONS: &str = "META-INF/versions/";

/// The main section of a jar's manifest, as far as Ferrion reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// `Multi-Release: true`, ignoring case.
    pub multi_release: bool,
    /// `Class-Path`: relative URLs, which `ferrion_project` resolves against the jar's directory.
    pub class_path: Vec<String>,
    /// `Automatic-Module-Name`, which names the module a jar without a `module-info` is on the
    /// module path.
    pub automatic_module_name: Option<String>,
}

impl Manifest {
    /// Parses a manifest's main section: lines end in CR LF, LF, or CR; a line starting with a
    /// space continues the previous one; the section ends at the first blank line; names are
    /// compared ignoring ASCII case.
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Error::malformed(e.valid_up_to() as u64, "the manifest isn't UTF-8"))?;
        let mut headers: Vec<(String, String)> = Vec::new();
        let mut offset = 0;
        for line in lines(text) {
            let line_offset = offset;
            offset += line.len();
            if line.is_empty() {
                break;
            }
            if let Some(rest) = line.strip_prefix(' ') {
                let Some((_, value)) = headers.last_mut() else {
                    return Err(Error::malformed(
                        line_offset as u64,
                        "the manifest starts with a continuation line",
                    ));
                };
                value.push_str(rest);
                continue;
            }
            let Some((name, value)) = line.split_once(':') else {
                return Err(Error::malformed(
                    line_offset as u64,
                    "a manifest line without `:`",
                ));
            };
            let valid_name = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if !valid_name {
                return Err(Error::malformed(
                    line_offset as u64,
                    format!("a bad manifest header name `{name}`"),
                ));
            }
            let Some(value) = value.strip_prefix(' ') else {
                return Err(Error::malformed(
                    line_offset as u64,
                    "a manifest header without a space after `:`",
                ));
            };
            headers.push((name.to_owned(), value.to_owned()));
        }
        let mut manifest = Manifest::default();
        // The first of a repeated header wins.
        let get = |wanted: &str| {
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                .map(|(_, v)| v)
        };
        if let Some(value) = get("Multi-Release") {
            manifest.multi_release = value.eq_ignore_ascii_case("true");
        }
        if let Some(value) = get("Class-Path") {
            manifest.class_path = value
                .split(' ')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if let Some(value) = get("Automatic-Module-Name") {
            manifest.automatic_module_name = Some(value.trim().to_owned());
        }
        Ok(manifest)
    }
}

/// Splits text into lines ending in CR LF, LF, or CR, without their ends. A line's length plus
/// its end is what the offset advances by, so offsets in errors stay close; they count the
/// lines without their ends.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let line = &rest[..end];
        let after = &rest[end..];
        rest = if let Some(next) = after.strip_prefix("\r\n") {
            next
        } else if after.is_empty() {
            after
        } else {
            &after[1..]
        };
        Some(line)
    })
}

/// A jar resolved for a release: for a multi-release jar, each name maps to its entry for that
/// release. Shared through `Arc`; immutable once built.
pub struct Jar {
    archive: Arc<ZipArchive>,
    release: u32,
    manifest: Manifest,
    manifest_problem: Option<Error>,
    /// (entry index, bytes to skip in its name), sorted by the name after the skip.
    resolved: Vec<(u32, u16)>,
    /// For each package (internal form), the positions in `resolved` of its `.class` files.
    packages: HashMap<Box<str>, Vec<u32>>,
}

impl std::fmt::Debug for Jar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jar")
            .field("archive", &self.archive)
            .field("release", &self.release)
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl Jar {
    /// Resolves `archive` for `release`. A manifest that can't be read or parsed is reported by
    /// [`Jar::manifest_problem`] and treated as empty.
    pub fn new(archive: Arc<ZipArchive>, release: u32) -> Jar {
        let (manifest, manifest_problem) = match archive.find(MANIFEST).map(|entry| entry.read()) {
            None => (Manifest::default(), None),
            Some(Ok(bytes)) => match Manifest::parse(&bytes) {
                Ok(manifest) => (manifest, None),
                Err(error) => (
                    Manifest::default(),
                    Some(error.in_origin(archive.source().entry_origin(MANIFEST))),
                ),
            },
            Some(Err(error)) => (Manifest::default(), Some(error)),
        };
        let versioned = manifest.multi_release && release >= 9;
        // (name for the release, version, entry); version 0 is the base.
        let mut candidates: Vec<(&str, u32, u32)> = Vec::with_capacity(archive.len());
        for entry in archive.unique_entries() {
            let name = entry.name();
            match name.strip_prefix(VERSIONS) {
                // In a multi-release jar, versioned entries aren't names of their own at any
                // release, release 8 included (as `JarFile`'s versioned view shows).
                Some(_) if manifest.multi_release && !versioned => continue,
                Some(rest) if versioned => {
                    let Some((number, path)) = rest.split_once('/') else {
                        continue;
                    };
                    let Some(version) = parse_version(number) else {
                        continue;
                    };
                    // `JarFile` serves names under `META-INF/` from the base only: a versioned
                    // `META-INF/` entry neither replaces a base one nor adds a name.
                    if (9..=release).contains(&version)
                        && !path.is_empty()
                        && !path.starts_with("META-INF/")
                    {
                        candidates.push((path, version, entry.index()));
                    }
                }
                _ => candidates.push((name, 0, entry.index())),
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(b.0).then(b.1.cmp(&a.1)));
        candidates.dedup_by(|b, a| a.0 == b.0);
        let mut packages: HashMap<Box<str>, Vec<u32>> = HashMap::new();
        let mut resolved = Vec::with_capacity(candidates.len());
        for (position, &(name, _, index)) in candidates.iter().enumerate() {
            let skip = (archive_name(&archive, index).len() - name.len()) as u16;
            resolved.push((index, skip));
            // A name with an empty segment (`/A.class`, `p//A.class`) names no class.
            if name.ends_with(".class") && !name.starts_with('/') && !name.contains("//") {
                let package = name.rsplit_once('/').map_or("", |(package, _)| package);
                packages
                    .entry(package.into())
                    .or_default()
                    .push(position as u32);
            }
        }
        Jar {
            archive,
            release,
            manifest,
            manifest_problem,
            resolved,
            packages,
        }
    }

    pub fn archive(&self) -> &Arc<ZipArchive> {
        &self.archive
    }

    pub fn release(&self) -> u32 {
        self.release
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Why the manifest was treated as empty, if it was.
    pub fn manifest_problem(&self) -> Option<&Error> {
        self.manifest_problem.as_ref()
    }

    fn resolved_name(&self, position: usize) -> &str {
        let (index, skip) = self.resolved[position];
        &archive_name(&self.archive, index)[usize::from(skip)..]
    }

    fn entry(&self, position: usize) -> ZipEntry<'_> {
        let (index, _) = self.resolved[position];
        self.archive
            .entry(index)
            .expect("the index came from the archive")
    }

    /// The entry for `name` at this jar's release.
    pub fn find(&self, name: &str) -> Option<ZipEntry<'_>> {
        let position = self.resolved.binary_search_by(|&(index, skip)| {
            archive_name(&self.archive, index)[usize::from(skip)..].cmp(name)
        });
        position.ok().map(|position| self.entry(position))
    }

    /// Every name at this jar's release, sorted, with its entry.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = (&str, ZipEntry<'_>)> + '_ {
        (0..self.resolved.len())
            .map(|position| (self.resolved_name(position), self.entry(position)))
    }

    /// Reads the class with internal name `name` (`java/util/Map$Entry`), if the jar has it.
    pub fn read_class(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.find(&format!("{name}.class")) else {
            return Ok(None);
        };
        entry.read().map(Some)
    }

    /// Whether the jar has a class in `package` (internal form).
    pub fn has_package(&self, package: &str) -> bool {
        self.packages.contains_key(package)
    }

    /// The simple names (`Map$Entry`, without `.class`) of the classes in `package`, sorted.
    pub fn package(&self, package: &str) -> Option<Vec<&str>> {
        let positions = self.packages.get(package)?;
        Some(
            positions
                .iter()
                .map(|&position| {
                    let name = self.resolved_name(position as usize);
                    let simple = name.rsplit_once('/').map_or(name, |(_, simple)| simple);
                    simple.strip_suffix(".class").unwrap_or(simple)
                })
                .collect(),
        )
    }

    /// Every package with a class, in no particular order.
    pub fn packages(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(|k| &**k)
    }
}

fn archive_name(archive: &ZipArchive, index: u32) -> &str {
    archive.entry(index).map_or("", |entry| entry.name())
}

/// A `META-INF/versions/` directory name as a release: decimal digits only.
fn parse_version(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::testing::ZipBuilder;

    #[test]
    fn manifests() {
        let manifest = Manifest::parse(
            b"Manifest-Version: 1.0\r\nmulti-release: TRUE\r\nClass-Path: a.jar  lib/b.jar\r\n  c.jar\r\n\r\nName: x\r\nMulti-Release: false\r\n",
        )
        .unwrap();
        assert!(manifest.multi_release);
        assert_eq!(manifest.class_path, ["a.jar", "lib/b.jar", "c.jar"]);
        let manifest = Manifest::parse(b"Multi-Release: true\rClass-Path: x.jar\n").unwrap();
        assert!(manifest.multi_release);
        assert_eq!(manifest.class_path, ["x.jar"]);
        assert_eq!(manifest.automatic_module_name, None);
        let manifest = Manifest::parse(b"automatic-module-name: com.example.lib\n").unwrap();
        assert_eq!(
            manifest.automatic_module_name.as_deref(),
            Some("com.example.lib")
        );
        assert!(
            !Manifest::parse(b"Multi-Release: yes\n")
                .unwrap()
                .multi_release
        );
        assert_eq!(Manifest::parse(b"").unwrap(), Manifest::default());
        for bad in [
            &b" continued\n"[..],
            b"NoColon\n",
            b"Bad Name: x\n",
            b"Name:x\n",
            b"\xff",
        ] {
            assert!(Manifest::parse(bad).is_err(), "{bad:?}");
        }
    }

    fn jar(multi_release: bool) -> Arc<ZipArchive> {
        let mut b = ZipBuilder::new();
        if multi_release {
            b.stored(MANIFEST, b"Manifest-Version: 1.0\nMulti-Release: true\n\n");
        }
        b.stored("p/A.class", b"base A")
            .stored("p/B.class", b"base B")
            .stored("p/r.txt", b"base r")
            .stored("META-INF/versions/9/p/A.class", b"9 A")
            .stored("META-INF/versions/11/p/A.class", b"11 A")
            .stored("META-INF/versions/11/p/C.class", b"11 C")
            .stored("META-INF/versions/17/p/B.class", b"17 B")
            .stored("META-INF/versions/x/p/B.class", b"x B")
            .stored("META-INF/versions/9/module-info.class", b"9 module")
            .stored("META-INF/LICENSE.txt", b"base license")
            .stored("META-INF/versions/9/META-INF/LICENSE.txt", b"9 license")
            .stored("META-INF/versions/9/META-INF/jpms.args", b"9 args");
        Arc::new(ZipArchive::from_bytes(Arc::from(b.finish())).unwrap())
    }

    fn read(jar: &Jar, name: &str) -> Option<String> {
        jar.read_class(name)
            .unwrap()
            .map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn multi_release_jars() {
        let archive = jar(true);
        let at = |release| Jar::new(archive.clone(), release);
        let jar8 = at(8);
        assert!(!jar8.manifest().multi_release || jar8.release() == 8);
        assert_eq!(read(&jar8, "p/A").as_deref(), Some("base A"));
        assert_eq!(read(&jar8, "p/C"), None);
        assert_eq!(read(&jar8, "module-info"), None);
        assert!(jar8.find("META-INF/versions/9/p/A.class").is_none());
        let jar10 = at(10);
        assert_eq!(read(&jar10, "p/A").as_deref(), Some("9 A"));
        assert_eq!(read(&jar10, "module-info").as_deref(), Some("9 module"));
        let jar11 = at(11);
        assert_eq!(read(&jar11, "p/A").as_deref(), Some("11 A"));
        assert_eq!(read(&jar11, "p/C").as_deref(), Some("11 C"));
        assert_eq!(read(&jar11, "p/B").as_deref(), Some("base B"));
        let jar21 = at(21);
        assert_eq!(read(&jar21, "p/B").as_deref(), Some("17 B"));
        assert_eq!(jar21.package("p").unwrap(), ["A", "B", "C"]);
        // Versioned entries aren't names of their own in a multi-release jar at 9 or later.
        assert!(jar21.find("META-INF/versions/11/p/C.class").is_none());
        assert_eq!(jar21.find("p/r.txt").unwrap().read().unwrap(), b"base r");
        assert!(jar21.has_package("") && !jar21.has_package("q"));
        // Names under `META-INF/` come from the base only, as in `coherence-26.07.jar`.
        assert_eq!(
            jar21.find("META-INF/LICENSE.txt").unwrap().read().unwrap(),
            b"base license"
        );
        assert!(jar21.find("META-INF/jpms.args").is_none());
    }

    #[test]
    fn plain_jars_ignore_versions() {
        let jar = Jar::new(jar(false), 21);
        assert_eq!(read(&jar, "p/A").as_deref(), Some("base A"));
        assert_eq!(read(&jar, "p/C"), None);
        assert!(jar.find("META-INF/versions/11/p/C.class").is_some());
        assert!(jar.manifest_problem().is_none());
    }

    #[test]
    fn bad_manifests_are_reported_and_ignored() {
        let mut b = ZipBuilder::new();
        b.stored(MANIFEST, b"Multi-Release true\n")
            .stored("META-INF/versions/9/p/A.class", b"9 A");
        let jar = Jar::new(
            Arc::new(ZipArchive::from_bytes(Arc::from(b.finish())).unwrap()),
            11,
        );
        assert_eq!(jar.manifest_problem().unwrap().kind(), ErrorKind::Malformed);
        assert_eq!(read(&jar, "p/A"), None);
    }
}
