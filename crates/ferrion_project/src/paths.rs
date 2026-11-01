//! Path lists and lexical path handling.

use std::path::{Component, Path, PathBuf};

/// The separator of path lists on this platform, as javac takes them: `;` on Windows, else `:`.
pub const PATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// `path` made absolute against `base`, with `.` and `..` removed without touching the file
/// system, so that a path that doesn't exist yet is still compared correctly. A `..` that would
/// go above the root is dropped.
pub fn absolute(base: &Path, path: &Path) -> PathBuf {
    let joined = base.join(path);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The entries of a path list separated by `separator`, leaving out empty ones (`a::b`, or a
/// trailing separator).
pub fn split_list(list: &str, separator: char) -> impl Iterator<Item = &str> {
    list.split(separator).filter(|entry| !entry.is_empty())
}

/// Whether `path` names an archive by its extension (`.jar` or `.zip`, ignoring case). javac
/// reports a file with one of these that isn't a zip as an error, and skips other files that
/// aren't zips.
pub fn has_archive_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jar") || e.eq_ignore_ascii_case("zip"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths() {
        let base = Path::new("/work/project");
        assert_eq!(
            absolute(base, Path::new("a/./b")),
            Path::new("/work/project/a/b")
        );
        assert_eq!(
            absolute(base, Path::new("../lib/x.jar")),
            Path::new("/work/lib/x.jar")
        );
        assert_eq!(absolute(base, Path::new("/abs/../y")), Path::new("/y"));
        assert_eq!(absolute(base, Path::new("/../../z")), Path::new("/z"));
        assert_eq!(absolute(base, Path::new("")), Path::new("/work/project"));
    }

    #[test]
    fn lists() {
        assert_eq!(
            split_list("a.jar:b:", ':').collect::<Vec<_>>(),
            ["a.jar", "b"]
        );
        assert_eq!(
            split_list(r"C:\lib\a.jar;;C:\classes", ';').collect::<Vec<_>>(),
            [r"C:\lib\a.jar", r"C:\classes"]
        );
        assert_eq!(split_list("", ':').count(), 0);
        assert!(has_archive_extension(Path::new("x/A.JAR")));
        assert!(has_archive_extension(Path::new("x.zip")));
        assert!(!has_archive_extension(Path::new("x.pom")));
        assert!(!has_archive_extension(Path::new("classes")));
    }
}
