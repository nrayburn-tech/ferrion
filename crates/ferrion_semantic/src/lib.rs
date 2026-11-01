//! Symbols and name resolution, kept up to date as files change (docs/design/semantic.md).
//!
//! The [`Engine`] holds a project's inputs (source sets and their files) and everything derived
//! from them, in layers: each file's [`summary`] of its declarations, then each source set's
//! [`packages`] table, then (from W4 on) its views, type entries, and resolved names. Results are
//! kept between edits and checked by what they read, so a change inside a method body costs that
//! file's work only. Queries go through a [`Snapshot`], and return [`Cancelled`] once an input
//! changes under them. docs/design/semantic-incremental.md describes the engine, and
//! docs/design/semantic-names.md what it computes.

mod engine;
mod fingerprint;
mod input;
#[doc(hidden)]
pub mod invariants;
mod names;
pub mod packages;
pub mod summary;

pub use engine::{
    Cancelled, Engine, EngineOptions, FileInfo, Revision, Sizes, Snapshot, SourceSetInfo, Stats,
    TableStats,
};
pub use fingerprint::Fingerprint;
pub use input::{FileId, FileInput, ProjectInput, SourceSetId, SourceSetInput};
pub use names::{Interner, Name};
