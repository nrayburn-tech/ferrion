//! Class file, jar, jimage, and `ct.sym` reading (docs/design/classfile.md).
//!
//! - [`class`]: a lazy view over a class file's bytes, with its constant pool, members,
//!   attributes, annotations, and module descriptor.
//! - [`descriptor`] and [`signature`]: field and method descriptors, and generic signatures,
//!   parsed when asked for.
//! - [`zip`], [`jar`], [`jimage`], and [`ctsym`]: the containers classes come from.
//! - [`ClassPath`] and [`ClassPathEntry`]: lookup by name in directories and jars.
//! - [`Jdk`] and [`PlatformApi`]: the classes javac sees with `--release <N>` on a JDK.
//! - [`ArchiveCache`]: archives and their indexes, shared across the process.
//!
//! Every reader returns `Result` and never panics on bad input; an error in one class doesn't stop
//! the rest. Everything is `Send + Sync`.

mod cache;
pub mod class;
mod classpath;
pub mod ctsym;
mod cursor;
pub mod descriptor;
pub mod error;
#[doc(hidden)]
pub mod invariants;
pub mod jar;
mod jdk;
pub mod jimage;
mod names;
pub mod signature;
mod source;
#[doc(hidden)]
pub mod testing;
pub mod text;
pub mod zip;

pub use cache::ArchiveCache;
pub use class::ClassFile;
pub use classpath::{ClassBytes, ClassPath, ClassPathEntry, PackageListing};
pub use error::{Error, ErrorKind, Origin, Result};
pub use jdk::{Jdk, PlatformApi, PlatformSource};
pub use names::{ClassName, ModuleName, PackageName};
pub use text::{JavaStr, Name};
