//! Class file, jar, jimage, and `ct.sym` reading (docs/design/classfile.md).
//!
//! - [`class`]: a lazy view over a class file's bytes, with its constant pool, members,
//!   attributes, annotations, and module descriptor.
//! - [`descriptor`] and [`signature`]: field and method descriptors, and generic signatures,
//!   parsed when asked for.
//!
//! Every reader returns `Result` and never panics on bad input; an error in one class doesn't stop
//! the rest. Everything is `Send + Sync`.

pub mod class;
mod cursor;
pub mod descriptor;
pub mod error;
#[doc(hidden)]
pub mod invariants;
pub mod signature;
#[doc(hidden)]
pub mod testing;
pub mod text;

pub use class::ClassFile;
pub use error::{Error, ErrorKind, Origin, Result};
pub use text::{JavaStr, Name};
