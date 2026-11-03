//! The safety properties on arbitrary input, for the `format` fuzz target in `fuzz/` and the
//! tests of its reduced failures. Not part of the public API.

use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, Severity, parse};

use crate::{FormatOptions, Formatted, apply_edits, format_edits, format_tree, verify_parsed};

/// The releases [`check_bytes`] formats at: the parser's fuzzing releases, the LTS releases and
/// the latest, which include both readings of `yield`.
pub const RELEASES: &[u32] = ferrion_parser::invariants::RELEASES;

/// Reads `bytes` as a source file, and at each of [`RELEASES`] where it parses without errors,
/// formats it and checks the four safety properties. Bytes that aren't a supported source file
/// are skipped, as the formatter skips them.
pub fn check_bytes(bytes: &[u8]) -> Result<(), String> {
    let Ok(source) = SourceText::decode(bytes) else {
        return Ok(());
    };
    for &release in RELEASES {
        check_source(&source, release).map_err(|e| format!("at release {release}: {e}"))?;
    }
    Ok(())
}

/// Formats `source` at `release`, unless it has errors there, and checks the safety properties
/// of the output, and that the edits give it.
pub fn check_source(source: &SourceText, release: u32) -> Result<(), String> {
    let parsed = parse(Arc::from(source.text()), ParseOptions { release });
    if parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return Ok(());
    }
    let output = match format_tree(&parsed.tree, source) {
        Formatted::Unchanged => source.to_bytes(),
        Formatted::Changed(output) => output,
        Formatted::Skipped(_) => return Err("a file without errors was skipped".to_owned()),
    };
    verify_parsed(source, &parsed, &output, &FormatOptions { release })
        .map_err(|v| v.to_string())?;
    let edits = format_edits(&parsed.tree, source).ok_or("a file without errors has no edits")?;
    let bom = if source.has_bom() { 3 } else { 0 };
    if apply_edits(source.text(), &edits).as_bytes() != &output[bom..] {
        return Err("the edits don't give the output".to_owned());
    }
    Ok(())
}
