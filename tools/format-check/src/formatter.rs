//! The formatter this tool checks, behind one seam.
//!
//! TEMPORARY: `ferrion_formatter` isn't written yet, so this module stands in for it with the API
//! docs/design/formatter.md plans (`format`, `verify`, `FormatOptions`, `Formatted`, `Violation`),
//! and `format` returns every file unchanged. The GJF comparison then measures how far the corpus
//! itself is from GJF's style. Once the crate lands, add it to Cargo.toml and replace this module's
//! body with:
//!
//! ```ignore
//! pub use ferrion_formatter::{FormatOptions, Formatted, Violation, format, verify};
//!
//! pub const STAND_IN: bool = false;
//! ```

use ferrion_diagnostics::SourceText;
use ferrion_parser::ParseDiagnostic;

/// Whether [`format`] is the stand-in that returns its input.
pub const STAND_IN: bool = true;

pub struct FormatOptions {
    /// The Java release to parse for.
    #[allow(dead_code, reason = "the stand-in doesn't parse")]
    pub release: u32,
}

pub enum Formatted {
    /// The file is already formatted.
    Unchanged,
    /// The new file, byte-order mark and line endings included.
    #[allow(dead_code, reason = "the stand-in never changes a file")]
    Changed(Vec<u8>),
    /// Syntax or version errors; nothing was formatted.
    #[allow(dead_code, reason = "the stand-in never skips a file")]
    Skipped(Vec<ParseDiagnostic>),
}

/// A safety property that doesn't hold.
#[derive(Debug)]
pub struct Violation(String);

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// TEMPORARY: returns every file unchanged.
pub fn format(_source: &SourceText, _options: &FormatOptions) -> Formatted {
    Formatted::Unchanged
}

/// TEMPORARY: checks only that the output is the input, which is all the stand-in produces.
pub fn verify(
    input: &SourceText,
    output: &[u8],
    _options: &FormatOptions,
) -> Result<(), Violation> {
    if input.to_bytes() == output {
        Ok(())
    } else {
        Err(Violation(
            "the stand-in's output isn't its input".to_owned(),
        ))
    }
}
