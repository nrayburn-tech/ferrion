//! `ferrion.jsonc`: loading, validation, and flat-config resolution.
//!
//! A config is loaded from text ([`Config::from_text`], for an editor buffer) or from a file
//! ([`Config::from_path`]); nothing else here touches the file system. Loading checks the whole
//! config and collects every error, with a span in the text, so the CLI can show an excerpt and the
//! language server a diagnostic from the same error. A valid config answers, for a path relative to
//! the working directory, whether a global ignore leaves it out, which entries match it, and the
//! settings they give it ([`Config::resolve`]).
//!
//! See docs/design/cli.md for the file's format and its rules.

mod error;
mod glob;
pub mod jsonc;
pub mod keys;
mod model;
mod suggest;
mod validate;

use std::fmt;
use std::path::{Path, PathBuf};

use ferrion_diagnostics::{SkipReason, SourceText};

pub use error::{ConfigError, ConfigErrors, ErrorCode, MAX_ERRORS, Related};
pub use glob::{Glob, GlobError};
pub use jsonc::MAX_DEPTH;
pub use model::{
    Config, Entry, FormatSettings, IgnoredBy, Level, LinterOptions, Resolution, ResolvedRule, Rule,
    Sourced,
};
pub use suggest::{did_you_mean, edit_distance};

/// The config file's name in the working directory, where it is read from by default.
pub const DEFAULT_FILE_NAME: &str = "ferrion.jsonc";

impl Config {
    /// Loads a config from its text. A leading byte-order mark is skipped; spans are byte offsets
    /// into `text` as given.
    pub fn from_text(text: &str) -> Result<Config, ConfigErrors> {
        let parsed = jsonc::parse(text).map_err(|e| ConfigErrors::new(vec![*e]))?;
        let mut errors = parsed.duplicates;
        validate::check(&parsed.value, &mut errors);
        if errors.is_empty() {
            Ok(Config::build(&parsed.value))
        } else {
            Err(ConfigErrors::new(errors))
        }
    }

    /// Reads and loads the config file at `path`. It must be UTF-8, with or without a byte-order
    /// mark; spans are offsets into its text after the mark, which [`LoadError::Invalid`] carries.
    pub fn from_path(path: &Path) -> Result<Config, LoadError> {
        let error = |kind| LoadError {
            path: path.to_owned(),
            kind,
        };
        let bytes = std::fs::read(path).map_err(|e| error(LoadErrorKind::Read(e)))?;
        let source = SourceText::from_utf8(bytes).map_err(|e| error(LoadErrorKind::Encoding(e)))?;
        Config::from_text(source.text()).map_err(|errors| {
            error(LoadErrorKind::Invalid {
                text: source.text().to_owned(),
                errors,
            })
        })
    }
}

/// Why a config file couldn't be loaded.
#[derive(Debug)]
pub struct LoadError {
    pub path: PathBuf,
    pub kind: LoadErrorKind,
}

#[derive(Debug)]
pub enum LoadErrorKind {
    /// The file couldn't be read.
    Read(std::io::Error),
    /// The file isn't UTF-8.
    Encoding(SkipReason),
    /// The file was read, and has errors. `text` is its text after any byte-order mark, which the
    /// errors' spans point into.
    Invalid { text: String, errors: ConfigErrors },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display();
        match &self.kind {
            LoadErrorKind::Read(e) => write!(f, "can't read the config file {path}: {e}"),
            LoadErrorKind::Encoding(e) => write!(f, "can't read the config file {path}: {e}"),
            LoadErrorKind::Invalid { errors, .. } => write!(f, "{path}: {errors}"),
        }
    }
}

impl std::error::Error for LoadError {}
