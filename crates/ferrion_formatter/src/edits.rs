//! Edits (docs/design/formatter-completeness.md, "Edits"): the difference between the input and
//! the formatter's output, as replacements of spans of the input. Output tokens are input tokens
//! in order, so the difference is in the gaps between the pieces the printer wrote and in the
//! pieces whose text it changed (a re-indented comment, a rewrite), which the printer's record
//! gives without comparing text at large.

use std::ops::Range;

use crate::printer::Piece;

/// A replacement of the input's bytes `range` (of `SourceText::text`, after any byte-order mark)
/// with `text`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    pub range: Range<u32>,
    pub text: String,
}

/// The edits that turn `input` into `output` (byte-order mark included, `bom` bytes of it),
/// from the printer's `pieces`. A gap that changed is one edit, without the bytes it shares with
/// the output at either end, so an edit never covers more than it changes; a piece whose text
/// changed is one edit over the whole piece, so a rewrite is never split.
pub fn from_pieces(input: &str, output: &[u8], bom: usize, pieces: &[Piece]) -> Vec<Edit> {
    let mut edits = Vec::new();
    let mut input_at = 0;
    let mut output_at = bom;
    for piece in pieces {
        debug_assert!(
            piece.input.start >= input_at,
            "a piece before the previous one in the input"
        );
        gap(
            &mut edits,
            input,
            output,
            input_at..piece.input.start,
            output_at..piece.output.start,
        );
        let written = &output[piece.output.clone()];
        if input.as_bytes()[piece.input.start as usize..piece.input.end as usize] != *written {
            edits.push(Edit {
                range: piece.input.clone(),
                text: text(written),
            });
        }
        input_at = piece.input.end;
        output_at = piece.output.end;
    }
    gap(
        &mut edits,
        input,
        output,
        input_at..input.len() as u32,
        output_at..output.len(),
    );
    edits
}

/// Adds the edit for one gap, if it changed (see [`gap_edit`]).
fn gap(edits: &mut Vec<Edit>, input: &str, output: &[u8], from: Range<u32>, to: Range<usize>) {
    let before = &input[from.start as usize..from.end as usize];
    edits.extend(gap_edit(before, &output[to], from.start));
}

/// The edit that turns `before`, a gap of the input at `start`, into `after`, or `None` if they
/// are the same: the bytes after the prefix and before the suffix they share. Gaps hold only
/// whitespace and line terminators, which are ASCII, so trimming never splits a character.
pub(crate) fn gap_edit(before: &str, after: &[u8], start: u32) -> Option<Edit> {
    let before = before.as_bytes();
    if before == after {
        return None;
    }
    let prefix = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    Some(Edit {
        range: start + prefix as u32..start + (before.len() - suffix) as u32,
        text: text(&after[prefix..after.len() - suffix]),
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("the printer writes UTF-8 and edits end at characters")
}

/// Applies `edits`, in order and not overlapping, as [`crate::format_edits`] returns them, to
/// `text`.
pub fn apply_edits(text: &str, edits: &[Edit]) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut at = 0;
    for edit in edits {
        assert!(
            edit.range.start as usize >= at,
            "the edits overlap or are out of order"
        );
        out.push_str(&text[at..edit.range.start as usize]);
        out.push_str(&edit.text);
        at = edit.range.end as usize;
    }
    out.push_str(&text[at..]);
    out
}
