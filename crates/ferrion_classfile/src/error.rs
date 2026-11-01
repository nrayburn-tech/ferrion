//! Errors from every reader in the crate.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

/// What went wrong, in the categories of the design (docs/design/classfile.md, "Errors").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The file can't be read.
    Io,
    /// A structure is truncated, an index points outside its table or at the wrong kind of
    /// constant, a length overruns its parent, or text isn't valid modified UTF-8.
    Malformed,
    /// Well-formed, but not handled: a compression method other than stored and deflate, an
    /// encrypted entry, a class file version newer than Ferrion knows, a jimage major version
    /// other than 1, or a name containing a lone surrogate.
    Unsupported,
    /// A declared size above a limit.
    TooLarge,
    /// The file's size, modification time, or identity changed since it was opened.
    Changed,
    /// The request can't be served, such as a release a JDK doesn't have.
    Usage,
}

/// Where the bytes in error came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A file on disk: a class file in a directory, or an archive itself.
    File(Arc<Path>),
    /// An entry of an archive: a zip entry, a `ct.sym` entry, or a jimage resource.
    Entry { archive: Arc<Path>, entry: Arc<str> },
    /// Bytes in memory, such as a test input.
    Memory,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::File(path) => write!(f, "{}", path.display()),
            Origin::Entry { archive, entry } => write!(f, "{}({entry})", archive.display()),
            Origin::Memory => f.write_str("<memory>"),
        }
    }
}

/// An error from reading a class file or a container. Small enough to pass around in `Result`.
#[derive(Clone, PartialEq, Eq)]
pub struct Error(Box<Inner>);

#[derive(Clone, PartialEq, Eq)]
struct Inner {
    kind: ErrorKind,
    message: String,
    offset: Option<u64>,
    origin: Option<Origin>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error(Box::new(Inner {
            kind,
            message: message.into(),
            offset: None,
            origin: None,
        }))
    }

    pub(crate) fn malformed(offset: u64, message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Malformed, message).at(offset)
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::Unsupported, message)
    }

    pub(crate) fn too_large(message: impl Into<String>) -> Error {
        Error::new(ErrorKind::TooLarge, message)
    }

    /// Sets the byte offset, if none is set yet.
    pub(crate) fn at(mut self, offset: u64) -> Error {
        self.0.offset.get_or_insert(offset);
        self
    }

    /// Sets the origin, if none is set yet: the innermost reader knows best.
    pub fn in_origin(mut self, origin: Origin) -> Error {
        if self.0.origin.is_none() {
            self.0.origin = Some(origin);
        }
        self
    }

    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// The byte offset of the problem within its origin, when known.
    pub fn offset(&self) -> Option<u64> {
        self.0.offset
    }

    pub fn origin(&self) -> Option<&Origin> {
        self.0.origin.as_ref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(origin) = &self.0.origin {
            write!(f, "{origin}: ")?;
        }
        f.write_str(&self.0.message)?;
        if let Some(offset) = self.0.offset {
            write!(f, " (at byte {offset})")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {self}", self.0.kind)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        let error = Error::malformed(12, "truncated").in_origin(Origin::Entry {
            archive: Path::new("a.jar").into(),
            entry: "p/A.class".into(),
        });
        assert_eq!(
            error.to_string(),
            "a.jar(p/A.class): truncated (at byte 12)"
        );
        assert_eq!(error.kind(), ErrorKind::Malformed);
        // The first origin and offset win.
        let error = error.in_origin(Origin::Memory).at(3);
        assert_eq!(error.offset(), Some(12));
        assert!(matches!(error.origin(), Some(Origin::Entry { .. })));
        assert!(std::mem::size_of::<Result<()>>() <= std::mem::size_of::<usize>());
    }
}
