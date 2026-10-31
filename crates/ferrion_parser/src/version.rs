//! Language versions: the feature table and the restricted identifiers
//! (docs/design/parser-language-versions.md).
//!
//! The parser accepts every release's syntax and builds the same tree whatever the release. A
//! check after parsing reports what the tree's release doesn't allow, from this table.

mod table;

#[cfg(test)]
mod tests;

pub use table::{FEATURES, Feature, FeatureRow, JavacDiagnostic, RestrictedName, Status};
