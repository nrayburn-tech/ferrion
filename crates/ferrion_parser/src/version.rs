//! Language versions: the feature table, the restricted identifiers, and the version check
//! (docs/design/parser-language-versions.md).
//!
//! The parser accepts every release's syntax and builds the same tree whatever the release. The
//! check then walks the tree once and reports, for the tree's release:
//!
//! - each use of a [`Feature`] the release doesn't have, or of a preview feature;
//! - restricted identifiers ([`RestrictedName`], and `_`) used where the release doesn't allow
//!   them, and, as warnings, where a later release won't;
//! - an unqualified call to a method named `yield`, a warning before 14 and an error from it;
//! - a `;` before an import, a warning before 21 and an error from it.
//!
//! Warnings are javac's parsing-phase warnings. Ferrion produces them only to compare them with
//! javac's, and never shows them to users.

mod check;
mod table;

#[cfg(test)]
mod tests;

pub use table::{FEATURES, Feature, FeatureRow, JavacDiagnostic, RestrictedName, Status};

use crate::{ParseDiagnostic, ParseDiagnosticKind as D, Severity, SyntaxTree};

/// The diagnostics for constructs the tree's release doesn't allow, in source order. Walks the
/// tree's arrays in loops, so it never overflows the stack.
pub fn check(tree: &SyntaxTree) -> Vec<ParseDiagnostic> {
    check::check(tree)
}

/// What javac reports for a version diagnostic: its code (without `compiler.err.` or
/// `compiler.warn.`) and the argument that tells features apart. `None` for the kinds the version
/// check doesn't report. This is the one mapping of version diagnostics to javac's, for the
/// oracle comparison and the version check's tests.
pub fn javac_diagnostic(kind: D, severity: Severity) -> Option<JavacDiagnostic> {
    let warning = severity == Severity::Warning;
    let code = match kind {
        D::FeatureNotInRelease { feature, .. } | D::PreviewFeature(feature) => {
            return Some(feature.row().javac);
        }
        D::RestrictedTypeName { name, .. } | D::RestrictedType { name, .. } if warning => {
            name.javac_warning()
        }
        D::RestrictedTypeName { .. } => "restricted.type.not.allowed",
        D::RestrictedType { .. } => "restricted.type.not.allowed.here",
        D::VarArray => "restricted.type.not.allowed.array",
        D::VarCompound => "restricted.type.not.allowed.compound",
        D::VarWithDeclaredTypes => "invalid.lambda.parameter.declaration",
        D::UnderscoreIdentifier { release } if release >= 22 && !warning => {
            "use.of.underscore.not.allowed.non.variable"
        }
        D::UnderscoreIdentifier { .. } => "underscore.as.identifier",
        D::UnderscoreNotAllowed => "use.of.underscore.not.allowed",
        D::UnderscoreWithBrackets => "use.of.underscore.not.allowed.with.brackets",
        D::YieldCall { .. } => "invalid.yield",
        D::ExtraneousSemicolon { .. } => "extraneous.semicolon",
        _ => return None,
    };
    Some(JavacDiagnostic {
        code,
        argument: None,
    })
}
