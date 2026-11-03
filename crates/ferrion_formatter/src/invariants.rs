//! The safety properties on arbitrary input, for the `format` fuzz target in `fuzz/` and the
//! tests of its reduced failures, and the doc comment reflow's, for the `javadoc` target. Not part
//! of the public API.

use std::ops::Range;
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, Severity, parse};

pub use crate::range::{Expanded, Ranges, Run};
use crate::{
    FormatOptions, Formatted, RangeError, apply_edits, doc_comments, format_edits, format_range,
    format_tree, verify, verify_parsed, verify_range,
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
/// formats it and checks the four safety properties. At every release, errors or not, it also
/// formats a range picked from the bytes and checks it (see [`check_range`]). Bytes that aren't
/// a supported source file are skipped, as the formatter skips them.
pub fn check_bytes(bytes: &[u8]) -> Result<(), String> {
    let Ok(source) = SourceText::decode(bytes) else {
        return Ok(());
    };
    let range = pick_range(source.text());
    for &release in RELEASES {
        check_source(&source, release).map_err(|e| format!("at release {release}: {e}"))?;
        check_range(&source, release, range.clone())
            .map_err(|e| format!("at release {release}, in {range:?}: {e}"))?;
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

/// Formats `range` of `source` at `release`, syntax errors or not, and checks the result with
/// `verify_range` (docs/design/formatter-completeness.md, "Testing"). In a file without errors,
/// the range is never affected, and formatting the whole result gives what formatting the whole
/// input gives: a range never takes a file where whole-file formatting wouldn't.
pub fn check_range(source: &SourceText, release: u32, range: Range<u32>) -> Result<(), String> {
    let parsed = parse(Arc::from(source.text()), ParseOptions { release });
    let errors = parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);
    let options = FormatOptions { release };
    let edits = match format_range(&parsed.tree, source, range.clone()) {
        Ok(edits) => edits,
        Err(RangeError::Affected { .. }) if parsed.tree.root().contains_error() => return Ok(()),
        Err(error) => return Err(format!("a tree without syntax errors: {error}")),
    };
    verify_range(source, range, &edits, &options).map_err(|v| v.to_string())?;
    if errors {
        return Ok(());
    }
    let mut result = if source.has_bom() {
        b"\xEF\xBB\xBF".to_vec()
    } else {
        Vec::new()
    };
    result.extend_from_slice(apply_edits(source.text(), &edits).as_bytes());
    let result = SourceText::from_utf8(result).map_err(|e| format!("the result: {e}"))?;
    let whole = |source: &SourceText| {
        let parsed = parse(Arc::from(source.text()), ParseOptions { release });
        match format_tree(&parsed.tree, source) {
            Formatted::Unchanged => Some(source.to_bytes()),
            Formatted::Changed(output) => Some(output),
            Formatted::Skipped(_) => None,
        }
    };
    if whole(&result) != whole(source) {
        return Err("formatting the whole result differs from formatting the whole input".into());
    }
    Ok(())
}

/// A range of `text` picked from its bytes, so that a fuzz input decides its range: mostly a few
/// bytes, sometimes to the end, at character boundaries.
pub fn pick_range(text: &str) -> Range<u32> {
    // FNV-1a.
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let len = text.len() as u64;
    let start = hash % (len + 1);
    let rest = len - start;
    let length = if (hash >> 32) % 4 == 0 {
        rest
    } else {
        (hash >> 40) % (rest.min(40) + 1)
    };
    let boundary = |mut i: usize| {
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i as u32
    };
    boundary(start as usize)..boundary((start + length) as usize)
}
