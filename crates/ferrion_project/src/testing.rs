//! Scratch directories, jars, and JDKs for tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ferrion_classfile::testing::{ClassBuilder, ImageBuilder, ZipBuilder};

use crate::{Environment, InputError};

pub(crate) struct Scratch(pub PathBuf);

impl Scratch {
    pub(crate) fn new(name: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ferrion-project-{}-{name}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Canonical, so that paths in messages and comparisons match what the file system says
        // (macOS's temporary directory is behind a symbolic link), without the `\\?\` prefix
        // Windows gives a canonical path, which a path a user passes doesn't have.
        let dir = dir.canonicalize().unwrap();
        let dir = match dir.to_str().and_then(|d| d.strip_prefix(r"\\?\")) {
            Some(plain) => PathBuf::from(plain),
            None => dir,
        };
        Scratch(dir)
    }

    /// `path`, absolute on Unix (`/opt/jdk`), as an input resolves it: itself, or on Windows,
    /// where a path without a drive isn't absolute, on the scratch directory's drive.
    pub(crate) fn rooted(&self, path: &str) -> PathBuf {
        self.0.join(path)
    }

    /// `path`, under the scratch directory, relative to it and with `/` between its parts.
    pub(crate) fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.0)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/")
    }

    /// The operating system's message for a file that doesn't exist, as an error shows it.
    pub(crate) fn not_found(&self) -> String {
        std::fs::metadata(self.path("not-there"))
            .unwrap_err()
            .to_string()
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    pub(crate) fn dir(&self, name: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A jar holding a class of each name, with a manifest if `manifest` isn't empty.
    pub(crate) fn jar(&self, name: &str, classes: &[&str], manifest: &str) -> PathBuf {
        let mut b = ZipBuilder::new();
        if !manifest.is_empty() {
            b.stored("META-INF/MANIFEST.MF", manifest.as_bytes());
        }
        for class in classes {
            b.stored(
                &format!("{class}.class"),
                &ClassBuilder::new(class).finish(),
            );
        }
        self.write(name, &b.finish())
    }

    /// A JDK 8 home, which serves only release 8.
    pub(crate) fn jdk8(&self, name: &str) -> PathBuf {
        let mut rt = ZipBuilder::new();
        rt.stored(
            "java/lang/Object.class",
            &ClassBuilder::new("java/lang/Object").finish(),
        );
        self.write(&format!("{name}/release"), b"JAVA_VERSION=\"1.8.0_502\"\n");
        self.write(&format!("{name}/jre/lib/rt.jar"), &rt.finish());
        self.path(name)
    }

    /// A runtime image without `ct.sym` at `version`, which serves only its own release.
    pub(crate) fn runtime(&self, name: &str, version: u32) -> PathBuf {
        let mut image = ImageBuilder::new();
        image.add(
            "java.base",
            "java/lang/Object.class",
            &ClassBuilder::new("java/lang/Object").finish(),
        );
        self.write(
            &format!("{name}/release"),
            format!("JAVA_VERSION=\"{version}.0.1\"\n").as_bytes(),
        );
        self.write(&format!("{name}/lib/modules"), &image.finish());
        self.path(name)
    }

    pub(crate) fn env(&self) -> Environment {
        Environment {
            working_dir: self.0.clone(),
            java_home: None,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The problems as the CLI prints them, one blank line between each, with the scratch
/// directory's path written as `$DIR` and `/` between a path's parts on every platform.
pub(crate) fn render(problems: &[InputError], scratch: &Path) -> String {
    problems
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
        .replace(&scratch.display().to_string(), "$DIR")
        .replace('\\', "/")
}

/// `path` as a JSON string, quoted and escaped, for a project model a test writes.
pub(crate) fn json(path: &Path) -> String {
    serde_json::to_string(path).unwrap()
}
