//! The safety properties for range formatting (docs/design/formatter-completeness.md, "Safety
//! properties", "Ranges"): the edits stay in the range's units and the gaps around them,
//! applying them gives a file that parses to the same tree, with the same comments, and
//! formatting that file again in the same range gives no edits.

use std::ops::Range;
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::comments::{CommentMap, is_comment};
use ferrion_syntax::{SyntaxKind, SyntaxTree};

use super::{
    Property, Violation, doc_comments, equivalence, first_difference, has_errors, imports,
    normalize_comment, violation,
};
use crate::range::{Expanded, Ranges};
use crate::{Edit, FormatOptions, apply_edits, format_range, regions};

/// Checks `edits`, which [`crate::format_range`] gave for `range` of `input` at
/// `options.release`: that each is inside the expanded range or the gaps around it, and, unless
/// it only changes whitespace, inside the expanded range and at token boundaries; that applying
/// them gives a file that parses to the same tree as the input (apart from the rewrites, as
/// [`super::verify`] compares trees), with no new errors and the same comments (those in the
/// range as `verify` compares them, the others as written); and that formatting that file again
/// in the same range, moved by the edits, gives no edits. An input with syntax errors is checked
/// the same way: its tree is compared, `Error` nodes and all.
pub fn verify_range(
    input: &SourceText,
    range: Range<u32>,
    edits: &[Edit],
    options: &FormatOptions,
) -> Result<(), Violation> {
    let parse_options = ParseOptions {
        release: options.release,
    };
    let before = parse(Arc::from(input.text()), parse_options);
    let text = input.text();
    let expanded = Ranges::new(&before.tree, input).expand(range.clone());
    let bom = if input.has_bom() { 3 } else { 0 };
    let mut at = 0;
    for edit in edits {
        let (start, end) = (edit.range.start as usize, edit.range.end as usize);
        if start < at || end < start || end > text.len() {
            return Err(violation(
                Property::Range,
                None,
                format!(
                    "the edit at {start}..{end} overlaps, is out of order, or is out of the text"
                ),
            ));
        }
        at = end;
        check_place(&before.tree, expanded.as_ref(), edit)?;
    }

    let result = apply_edits(text, edits);
    let mut bytes = if input.has_bom() {
        b"\xEF\xBB\xBF".to_vec()
    } else {
        Vec::new()
    };
    bytes.extend_from_slice(result.as_bytes());
    let output = SourceText::from_utf8(bytes).map_err(|reason| {
        violation(
            Property::NoNewErrors,
            None,
            format!("the result can't be read: {reason}"),
        )
    })?;
    let after = parse(Arc::from(output.text()), parse_options);
    if !has_errors(&before)
        && let Some(error) = after
            .diagnostics
            .iter()
            .find(|d| d.severity == ferrion_parser::Severity::Error)
    {
        return Err(violation(
            Property::NoNewErrors,
            Some(bom + error.span.start as usize),
            format!("{:?}: {}", error.kind, error.message()),
        ));
    }
    equivalence(&before.tree, &after.tree, bom)?;
    imports::compare(&before.tree, &after.tree, bom)?;
    compare_comments(&before.tree, &after.tree, expanded.as_ref(), bom)?;

    let moved = moved(edits, range.start, false)..moved(edits, range.end, true);
    match format_range(&after.tree, &output, moved) {
        Ok(again) if again.is_empty() => Ok(()),
        Ok(again) => Err(violation(
            Property::Idempotence,
            Some(bom + again[0].range.start as usize),
            "formatting the result again in the range changes it",
        )),
        Err(error) => Err(violation(
            Property::Idempotence,
            None,
            format!("formatting the result again in the range fails: {error}"),
        )),
    }
}

/// Checks that `edit` is where a range's edits may be: in the expanded range or the gaps around
/// it, and only whitespace outside the expanded range or between tokens.
fn check_place(
    tree: &SyntaxTree,
    expanded: Option<&Expanded>,
    edit: &Edit,
) -> Result<(), Violation> {
    let fail = |what: &str| {
        Err(violation(
            Property::Range,
            None,
            format!(
                "the edit at {}..{} {what}",
                edit.range.start, edit.range.end
            ),
        ))
    };
    let Some(expanded) = expanded else {
        return fail("is for a range that touches no token");
    };
    if edit.range.start < expanded.before || edit.range.end > expanded.after {
        return fail("is outside the range's units and the gaps around them");
    }
    let replaced = &tree.text()[edit.range.start as usize..edit.range.end as usize];
    let whitespace = |s: &str| {
        s.bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\x0c' | b'\n' | b'\r'))
    };
    if whitespace(replaced) && whitespace(&edit.text) {
        return Ok(());
    }
    if edit.range.start < expanded.units.start || edit.range.end > expanded.units.end {
        return fail("changes more than whitespace outside the range's units");
    }
    let starts_token = tree
        .token_at_offset(edit.range.start)
        .is_some_and(|t| t.span().start == edit.range.start);
    let ends_token = edit.range.end == 0
        || tree
            .token_at_offset(edit.range.end - 1)
            .is_some_and(|t| t.span().end == edit.range.end);
    if !starts_token || !ends_token {
        return fail("splits a token");
    }
    Ok(())
}

/// Compares the two trees' comments in order: by kind, and by text as written, except those in
/// the range's units outside disable regions, which compare as [`super::verify`] compares them.
fn compare_comments(
    before: &SyntaxTree,
    after: &SyntaxTree,
    expanded: Option<&Expanded>,
    bom: usize,
) -> Result<(), Violation> {
    let regions = regions::find(before, &CommentMap::new(before));
    let in_region = |id| {
        regions
            .iter()
            .any(|r| r.start <= id && r.end.is_none_or(|end| id <= end))
    };
    fn list(tree: &SyntaxTree) -> Vec<(SyntaxKind, &str, u32)> {
        tree.tokens()
            .filter(|t| is_comment(t.kind()))
            .map(|t| (t.kind(), t.text(), t.span().start))
            .collect()
    }
    let (a, b) = (list(before), list(after));
    let ids: Vec<_> = before
        .tokens()
        .filter(|t| is_comment(t.kind()))
        .map(|t| t.id())
        .collect();
    let normalized = |i: usize, (kind, text, start): (SyntaxKind, &str, u32), input: bool| {
        let in_units = expanded.is_some_and(|e| e.units.contains(&start));
        if !in_units || in_region(ids[i]) {
            return (kind, text.to_owned());
        }
        let text = if kind == SyntaxKind::DocComment {
            doc_comments::normalize(text)
        } else {
            normalize_comment(text, input && kind == SyntaxKind::LineComment)
        };
        (kind, text)
    };
    let a_norm: Vec<_> = a
        .iter()
        .enumerate()
        .map(|(i, &c)| normalized(i, c, true))
        .collect();
    // The output's comments are matched with the input's by position in the list, so the one in
    // the units is normalized when the input's is.
    let b_norm: Vec<_> = b
        .iter()
        .enumerate()
        .map(|(i, &(kind, text, _))| match a.get(i) {
            Some(&(_, _, start)) => {
                let (_, normalized) = normalized(i, (kind, text, start), false);
                (kind, normalized)
            }
            None => (kind, text.to_owned()),
        })
        .collect();
    if let Some(i) = first_difference(&a_norm, &b_norm) {
        let describe = |list: &[(SyntaxKind, String)]| {
            list.get(i)
                .map_or("nothing".to_owned(), |(_, text)| format!("{text:?}"))
        };
        return Err(violation(
            Property::Comments,
            b.get(i).map(|&(_, _, start)| bom + start as usize),
            format!(
                "comment {i} is {} in the input and {} in the result",
                describe(&a_norm),
                describe(&b_norm)
            ),
        ));
    }
    Ok(())
}

/// Where `offset` of the input is in the text the edits give. An offset inside an edit's span
/// moves to the start of its replacement, or with `to_end` (for a range's end) to its end; an
/// insertion at the offset counts as before it, or with `to_end` as after it, so that a range
/// keeps the tokens it had.
fn moved(edits: &[Edit], offset: u32, to_end: bool) -> u32 {
    let mut shift: i64 = 0;
    for edit in edits {
        let (start, end) = (edit.range.start, edit.range.end);
        let delta = edit.text.len() as i64 - i64::from(end - start);
        let before = if start == end {
            start < offset || start == offset && !to_end
        } else if end <= offset {
            true
        } else if start >= offset {
            false
        } else {
            let replacement = if to_end { edit.text.len() as i64 } else { 0 };
            return (i64::from(start) + shift + replacement) as u32;
        };
        if !before {
            break;
        }
        shift += delta;
    }
    (i64::from(offset) + shift) as u32
}

#[cfg(test)]
mod tests;
