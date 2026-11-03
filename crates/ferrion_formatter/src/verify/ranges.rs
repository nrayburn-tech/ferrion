//! The safety properties for range formatting (docs/design/formatter-completeness.md, "Safety
//! properties", "Ranges"): the edits stay in the range's units and the gaps around them,
//! applying them gives a file that parses to the same tree, with the same comments, and
//! formatting that file again in the same range gives no edits.

use std::ops::Range;
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::comments::{CommentMap, is_comment};
use ferrion_syntax::{SyntaxKind, SyntaxToken, SyntaxTree};

use super::imports::SectionComment;
use super::{
    Property, Violation, doc_comments, equivalence, first_difference, has_errors, imports,
    normalize_comment, violation,
};
use crate::printer::trim_end;
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
    let whitespace = |s: &str| {
        s.bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\x0c' | b'\n' | b'\r'))
    };
    if whitespace(&edit.text) && only_gap(tree, edit.range.clone()) {
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

/// Whether `range` of `tree`'s text holds only what goes between pieces of text: whitespace and
/// line terminators, a line terminator written as a Unicode escape included, and a line
/// comment's trailing whitespace, which the printer doesn't write.
fn only_gap(tree: &SyntaxTree, range: Range<u32>) -> bool {
    let text = tree.text();
    let mut token = tree.token_at_offset(range.start);
    while let Some(t) = token
        && t.span().start < range.end
    {
        let span = t.span();
        let part = &text[span.start.max(range.start) as usize..span.end.min(range.end) as usize];
        let gap = matches!(
            t.kind(),
            SyntaxKind::Whitespace | SyntaxKind::LineTerminator
        ) || part.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\x0c'));
        if !gap {
            return false;
        }
        token = t.next_token();
    }
    true
}

/// Compares the two trees' comments in order: by kind, and by text as written, apart from a line
/// comment's trailing whitespace, except those in the range's units outside disable regions,
/// which compare as [`super::verify`] compares them. When the range holds the whole import
/// section, the comments around the imports, which a rewrite moves, are compared as `verify`
/// compares them.
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
    // When the range rewrote the import section, its comments may have moved, and are compared
    // as the whole file's are.
    let rewritten = expanded.is_some_and(|e| section_in(before, e));
    let section = |tree: &SyntaxTree| {
        if rewritten {
            imports::comment_region(tree)
        } else {
            0..0
        }
    };
    let (section_a, section_b) = (section(before), section(after));
    let in_section = |t: SyntaxToken<'_>, input: bool| -> SectionComment {
        let text = if t.kind() == SyntaxKind::DocComment {
            doc_comments::normalize(t.text())
        } else {
            // Only a comment the range formats gets the space after its `//`.
            let formatted = input
                && expanded.is_some_and(|e| e.units.contains(&t.span().start))
                && !in_region(t.id());
            normalize_comment(t.text(), formatted && t.kind() == SyntaxKind::LineComment)
        };
        SectionComment {
            token: t.id(),
            kind: t.kind(),
            text,
            start: t.span().start as usize,
        }
    };
    let mut a_section = Vec::new();
    let mut b_section = Vec::new();
    let mut a = Vec::new();
    let mut ids = Vec::new();
    for t in before.tokens().filter(|t| is_comment(t.kind())) {
        if section_a.contains(&t.id().index()) {
            a_section.push(in_section(t, true));
        } else {
            a.push((t.kind(), t.text(), t.span().start));
            ids.push(t.id());
        }
    }
    let mut b = Vec::new();
    for t in after.tokens().filter(|t| is_comment(t.kind())) {
        if section_b.contains(&t.id().index()) {
            b_section.push(in_section(t, false));
        } else {
            b.push((t.kind(), t.text(), t.span().start));
        }
    }
    let normalized = |i: usize, (kind, text, start): (SyntaxKind, &str, u32), input: bool| {
        let in_units = expanded.is_some_and(|e| e.units.contains(&start));
        if in_region(ids[i]) {
            return (kind, text.to_owned());
        }
        if !in_units {
            // The gap before the units starts after the line before's code, and a line
            // comment's trailing whitespace is in it.
            let text = match kind {
                SyntaxKind::LineComment | SyntaxKind::DocLineComment => trim_end(text),
                _ => text,
            };
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
    imports::compare_comments(before, after, &a_section, &b_section, bom)
}

/// Whether `expanded` holds all of `tree`'s imports, so that the range rewrote the section if
/// whole-file formatting does.
fn section_in(tree: &SyntaxTree, expanded: &Expanded) -> bool {
    let mut imports = tree
        .root()
        .children()
        .filter(|n| n.kind() == SyntaxKind::ImportDecl);
    let Some(first) = imports.next() else {
        return false;
    };
    let last = imports.last().unwrap_or(first);
    expanded.units.start <= first.span().start && last.span().end <= expanded.units.end
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
