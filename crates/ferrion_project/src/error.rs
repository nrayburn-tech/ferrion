//! Where values came from, and the errors and warnings that name them.

use std::fmt;
use std::path::PathBuf;

/// Where a value came from, for messages.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A command-line flag, by its name (`--classpath`).
    Flag(&'static str),
    /// A line of the `--classpath-file`, counted from 1. `path` is as the flag gave it.
    ClassPathFile {
        path: PathBuf,
        line: u32,
    },
    /// The project model, as `--project-model` gave its path. `source_set` is empty for the model
    /// as a whole.
    ProjectModel {
        path: PathBuf,
        source_set: String,
    },
    /// The `Class-Path` attribute of a jar's manifest.
    Manifest {
        jar: PathBuf,
    },
    JavaHome,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Flag(flag) => f.write_str(flag),
            Origin::ClassPathFile { path, line } => {
                write!(f, "--classpath-file {}, line {line}", path.display())
            }
            Origin::ProjectModel { path, source_set } if source_set.is_empty() => {
                write!(f, "project model `{}`", path.display())
            }
            Origin::ProjectModel { path, source_set } => write!(
                f,
                "project model `{}`, source set `{source_set}`",
                path.display()
            ),
            Origin::Manifest { jar } => {
                write!(f, "Class-Path in the manifest of `{}`", jar.display())
            }
            Origin::JavaHome => f.write_str("JAVA_HOME"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    /// A usage error: the run exits with status 2 before any file is changed.
    Error,
    Warning,
}

/// A problem with the inputs: the value, where it came from, and what to do.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputError {
    pub severity: Severity,
    pub message: String,
    pub origin: Origin,
    pub help: Option<String>,
}

impl InputError {
    pub fn error(message: impl Into<String>, origin: Origin) -> InputError {
        InputError {
            severity: Severity::Error,
            message: message.into(),
            origin,
            help: None,
        }
    }

    pub fn warning(message: impl Into<String>, origin: Origin) -> InputError {
        InputError {
            severity: Severity::Warning,
            ..InputError::error(message, origin)
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> InputError {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// The message, then `from:` and `help:` lines:
///
/// ```text
/// error: classpath entry `/home/me/lib/gone.jar` doesn't exist
///   from: --classpath-file build/cp.txt, line 3
/// ```
impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{severity}: {}\n  from: {}", self.message, self.origin)?;
        if let Some(help) = &self.help {
            write!(f, "\n  help: {help}")?;
        }
        Ok(())
    }
}

impl std::error::Error for InputError {}

/// Drops repeats of a message, keeping the first: many source sets share one JDK or jar, and one
/// problem with it is reported once.
pub(crate) fn dedup(problems: &mut Vec<InputError>) {
    let mut seen = std::collections::HashSet::new();
    problems.retain(|p| seen.insert((p.severity, p.message.clone())));
}
