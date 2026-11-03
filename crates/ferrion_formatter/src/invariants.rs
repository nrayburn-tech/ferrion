//! The safety properties on arbitrary input, for the `format` fuzz target in `fuzz/` and the
//! tests of its reduced failures, and the doc comment reflow's, for the `javadoc` target. Not part
//! of the public API.

use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, Severity, parse};

use crate::{
    FormatOptions, Formatted, apply_edits, doc_comments, format_edits, format_tree, verify,
    verify_parsed,
};

/// The columns [`check_javadoc_reflow`] reflows a comment at, picked by its first byte: up to a
/// little past the limit, where every line overflows.
const REFLOW_COLUMNS: u32 = 104;

/// Reads `bytes` as a traditional doc comment's content, the text between `/**` and `*/`, as the
/// `javadoc` fuzz target does, and, if the rules reflow it, checks the reflow at a column the
/// first byte picks (docs/design/formatter-completeness.md, "Testing"): reflowing the result at
/// the same column gives it again, and both have the input's content, as the comments check
/// compares it.
pub fn check_javadoc_reflow(bytes: &[u8]) -> Result<(), String> {
    let column = bytes.first().map_or(0, |&b| u32::from(b) % REFLOW_COLUMNS);
    let content = match SourceText::decode(bytes) {
        Ok(source) => source.text().to_owned(),
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    };
    if content.contains("*/") {
        return Ok(());
    }
    let comment = format!("/**{content}*/");
    let reflow = |text: &str| -> Option<String> {
        let layout = doc_comments::javadoc::layout(text, 0, text.len() as u32)?;
        Some(doc_comments::javadoc::render(&layout, text, column).join("\n"))
    };
    let Some(once) = reflow(&comment) else {
        return Ok(());
    };
    let twice = reflow(&once).ok_or_else(|| format!("{once:?}, reflowed, isn't reflowed again"))?;
    if twice != once {
        return Err(format!(
            "reflowing again at column {column} changes {once:?} to {twice:?}"
        ));
    }
    let (before, after) = (
        verify::doc_comments::content(&comment),
        verify::doc_comments::content(&once),
    );
    if before != after {
        return Err(format!(
            "reflowing {comment:?} at column {column} changes its content: {before:?} to {after:?}"
        ));
    }
    Ok(())
}

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
