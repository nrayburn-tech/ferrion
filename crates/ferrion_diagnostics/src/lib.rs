//! Source files, spans, and diagnostic rendering (pretty, JSON, GitHub Actions).

mod line_index;
mod source;
mod span;

pub use line_index::{LineCol, LineIndex};
pub use source::{Encoding, LineEnding, SkipReason, SourceText};
pub use span::Span;
