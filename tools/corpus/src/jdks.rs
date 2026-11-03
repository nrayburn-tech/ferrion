//! Finding a JDK of a feature version, so that each project's build and its javac oracle run on
//! the JDK its lock names (decision 1 of the corpus inputs review in docs/design/semantic.md).
//!
//! The JDK of version `N` is the first of:
//!
//! - `FERRION_JDK_<N>`, to choose one;
//! - `JAVA_HOME_<N>_X64` or `JAVA_HOME_<N>_ARM64`, which `actions/setup-java` sets for each JDK it
//!   sets up, as GitHub's runners do for theirs;
//! - `JAVA_HOME`, if it is JDK `N`;
//! - a JDK `N` that mise installed (`$MISE_DATA_DIR/installs/java`, by default
//!   `~/.local/share/mise/installs/java`), Temurin's first, then by name.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::lock::jdk_version;

/// The home of a JDK of feature version `version`, found as the module's docs say.
pub fn find(version: u32) -> Result<PathBuf> {
    let var = |name: String| std::env::var_os(name).map(PathBuf::from);
    if let Some(home) = var(format!("FERRION_JDK_{version}")) {
        return Ok(home);
    }
    for arch in ["X64", "ARM64"] {
        if let Some(home) = var(format!("JAVA_HOME_{version}_{arch}"))
            && is_version(&home, version)
        {
            return Ok(home);
        }
    }
    if let Some(home) = var("JAVA_HOME".to_owned())
        && is_version(&home, version)
    {
        return Ok(home);
    }
    let mise = match std::env::var_os("MISE_DATA_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/mise")),
    };
    if let Some(found) = mise.and_then(|dir| in_mise(&dir.join("installs/java"), version)) {
        // mise links shorter versions (`temurin-27`) to the install, which has one path.
        return Ok(found.canonicalize().unwrap_or(found));
    }
    bail!(
        "no JDK {version} found: set FERRION_JDK_{version} to one, or JAVA_HOME_{version}_X64 (as \
         actions/setup-java does), or install one with mise"
    )
}

fn is_version(home: &Path, version: u32) -> bool {
    jdk_version(home).is_ok_and(|v| v == version)
}

/// A JDK of `version` among mise's installs in `dir`: each directory is a JDK home, or has one
/// in `Contents/Home` (macOS's layout). Temurin's come first, then the others by name.
fn in_mise(dir: &Path, version: u32) -> Option<PathBuf> {
    let mut homes: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    homes.sort_by_key(|home| {
        let name = home.file_name().unwrap_or_default().to_string_lossy();
        (!name.starts_with("temurin-"), name.into_owned())
    });
    homes
        .into_iter()
        .flat_map(|home| [home.clone(), home.join("Contents/Home")])
        .find(|home| is_version(home, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mise_installs() {
        let dir = std::env::temp_dir().join(format!("ferrion-corpus-jdks-{}", std::process::id()));
        let jdk = |name: &str, version: &str| {
            let home = dir.join(name);
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(
                home.join("release"),
                format!("JAVA_VERSION=\"{version}\"\n"),
            )
            .unwrap();
        };
        jdk("corretto-21", "21.0.11");
        jdk("temurin-21.0.11+10", "21.0.11");
        jdk("corretto-25/Contents/Home", "25.0.1");
        std::fs::create_dir_all(dir.join("broken")).unwrap();
        assert_eq!(in_mise(&dir, 21), Some(dir.join("temurin-21.0.11+10")));
        assert_eq!(
            in_mise(&dir, 25),
            Some(dir.join("corretto-25/Contents/Home"))
        );
        assert_eq!(in_mise(&dir, 27), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
