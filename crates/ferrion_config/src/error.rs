//! Config errors: what is wrong, and where in the config file.

use std::fmt;

use ferrion_diagnostics::{Excerpt, Label, LineIndex, Severity, Span};

/// The most errors a config reports; past it, only how many more there are.
pub const MAX_ERRORS: usize = 50;

/// What kind of error a [`ConfigError`] is. Each has a stable name, [`ErrorCode::as_str`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorCode {
    /// The file isn't JSONC. Nothing after it is checked.
    Syntax,
    /// A key is given twice in one object.
    DuplicateKey,
    /// Values nest more than [`MAX_DEPTH`](crate::MAX_DEPTH) deep.
    TooDeep,
    /// A value has the wrong type: a string where an array belongs, say.
    WrongType,
    /// A key that isn't one of the keys of its object.
    UnknownKey,
    /// A string where a level belongs that isn't one.
    UnknownLevel,
    /// A required key is missing: a rule setting's `level`.
    MissingKey,
    /// `files` is an empty array.
    EmptyFiles,
    /// A glob that isn't valid, or can't match a path relative to the working directory.
    InvalidGlob,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 9] = [
        ErrorCode::Syntax,
        ErrorCode::DuplicateKey,
        ErrorCode::TooDeep,
        ErrorCode::WrongType,
        ErrorCode::UnknownKey,
        ErrorCode::UnknownLevel,
        ErrorCode::MissingKey,
        ErrorCode::EmptyFiles,
        ErrorCode::InvalidGlob,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Syntax => "syntax-error",
            ErrorCode::DuplicateKey => "duplicate-key",
            ErrorCode::TooDeep => "too-deep",
            ErrorCode::WrongType => "wrong-type",
            ErrorCode::UnknownKey => "unknown-key",
            ErrorCode::UnknownLevel => "unknown-level",
            ErrorCode::MissingKey => "missing-key",
            ErrorCode::EmptyFiles => "empty-files",
            ErrorCode::InvalidGlob => "invalid-glob",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A span related to an error, marked with `-` in an excerpt: where a duplicate key was first
/// given, say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Related {
    pub span: Span,
    pub label: String,
}

/// One error in a config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub code: ErrorCode,
    pub message: String,
    /// The value, key, or token the error is about, in bytes of the config's text (after any
    /// byte-order mark).
    pub span: Span,
    /// A short text shown under the span.
    pub label: Option<String>,
    /// A line shown after the excerpt.
    pub help: Option<String>,
    pub related: Option<Related>,
    /// A replacement for the span's text that would fix the error, when one is known: the key or
    /// level a "did you mean" suggests, without quotes.
    pub suggestion: Option<String>,
}

impl ConfigError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>, span: Span) -> ConfigError {
        ConfigError {
            code,
            message: message.into(),
            span,
            label: None,
            help: None,
            related: None,
            suggestion: None,
        }
    }

    pub(crate) fn label(mut self, label: impl Into<String>) -> ConfigError {
        self.label = Some(label.into());
        self
    }

    pub(crate) fn help(mut self, help: impl Into<String>) -> ConfigError {
        self.help = Some(help.into());
        self
    }

    pub(crate) fn related(mut self, span: Span, label: impl Into<String>) -> ConfigError {
        self.related = Some(Related {
            span,
            label: label.into(),
        });
        self
    }

    /// Labels the error "did you mean `x`?" when there is a suggestion.
    pub(crate) fn suggest(mut self, suggestion: Option<&str>) -> ConfigError {
        if let Some(name) = suggestion {
            self.label = Some(format!("did you mean `{name}`?"));
            self.suggestion = Some(name.to_owned());
        }
        self
    }

    /// The error as an excerpt, to render in `pretty` output.
    pub fn excerpt(&self) -> Excerpt {
        let mut labels = vec![Label::primary(self.span, self.label.clone())];
        if let Some(related) = &self.related {
            labels.push(Label::secondary(related.span, Some(related.label.clone())));
        }
        Excerpt {
            severity: Severity::Error,
            message: self.message.clone(),
            labels,
            help: self.help.clone(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Every error in a config file, in file order, up to [`MAX_ERRORS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigErrors {
    errors: Vec<ConfigError>,
    omitted: usize,
}

impl ConfigErrors {
    /// Sorts the errors into file order and keeps the first [`MAX_ERRORS`].
    pub(crate) fn new(mut errors: Vec<ConfigError>) -> ConfigErrors {
        debug_assert!(!errors.is_empty());
        errors.sort_by_key(|e| e.span.start);
        let omitted = errors.len().saturating_sub(MAX_ERRORS);
        errors.truncate(MAX_ERRORS);
        ConfigErrors { errors, omitted }
    }

    /// The errors reported, at most [`MAX_ERRORS`].
    pub fn errors(&self) -> &[ConfigError] {
        &self.errors
    }

    /// How many errors there were past the first [`MAX_ERRORS`].
    pub fn omitted(&self) -> usize {
        self.omitted
    }

    /// The errors in `pretty` form, for a config file at `path` with the text `text`, ending with
    /// "and N more errors" if some were left out.
    pub fn render(&self, path: &str, text: &str) -> String {
        let index = LineIndex::new(text);
        let mut out = String::new();
        for (i, error) in self.errors.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&error.excerpt().render(path, text, &index));
        }
        match self.omitted {
            0 => {}
            1 => out.push_str("\nand 1 more error\n"),
            n => out.push_str(&format!("\nand {n} more errors\n")),
        }
        out
    }
}

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.errors.len() + self.omitted;
        match total {
            1 => write!(f, "{}", self.errors[0]),
            _ => write!(f, "{} ({} more)", self.errors[0], total - 1),
        }
    }
}

impl std::error::Error for ConfigErrors {}
