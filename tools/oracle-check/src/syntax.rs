//! The parser checks: `oracle syntax` compares syntax errors with javac's parsing-phase errors,
//! and `oracle tree` compares tree positions with javac's. What javac reports, and the
//! normalization, are in docs/design/parser-oracle-normalization.md.
//!
//! Ferrion's side is behind [`Parser`], which the parser implements when it exists. Until then,
//! [`NoParser`] stands in: both checks run javac, cache its output, and summarize it (diagnostic
//! codes, tree sizes and depths), and every result is [`Status::NotCompared`].
//!
//! The comparisons themselves are provisional, and the parser work settles them along with the
//! normalization: `syntax` compares whether there are errors and, if there are, the first one in
//! source order by code and position; `tree` compares every node of files javac reports no errors
//! for.

use std::panic::{AssertUnwindSafe, catch_unwind};

use ferrion_diagnostics::{LineIndex, SourceText};

use crate::check::{Check, Located, Mismatch, Outcome, Status, Summary};
use crate::oracle::{Diagnostic, Node, Output, Section};

/// Items shown before and after the first difference.
const CONTEXT: usize = 3;

/// Ferrion's side of the parser checks, in javac's terms. Offsets are byte offsets into the text
/// as [`SourceText::text`] gives it, after any byte-order mark; the checks convert them to javac's
/// UTF-16 offsets.
pub trait Parser: Sync {
    /// A parsed file, before the version check.
    type Parsed: Sync;

    /// Parses a file, or returns `None` when there's no parser to compare. Called once per file,
    /// however many releases it's compared at.
    fn parse(&self, source: &SourceText) -> Option<Self::Parsed>;

    /// The syntax errors at `release`, version errors included, with javac's codes.
    fn errors(&self, parsed: &Self::Parsed, release: u32) -> Vec<SyntaxError>;

    /// The tree with javac's node kinds, in the pre-order of javac's `TreeScanner`.
    fn tree(&self, parsed: &Self::Parsed) -> Vec<TreeNode>;
}

/// A syntax error, as javac would report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// javac's diagnostic code, such as `compiler.err.expected`.
    pub code: String,
    /// The byte offset javac reports the error at (its diagnostic's `position`).
    pub position: u32,
}

/// A tree node, as javac would have it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    /// javac's `Tree.Kind` name, such as `METHOD_INVOCATION`.
    pub kind: &'static str,
    /// Byte offsets, or `None` where javac has no position.
    pub start: Option<u32>,
    pub end: Option<u32>,
    /// The depth below the compilation unit, which is 0.
    pub depth: u32,
}

/// The stand-in until there is a parser: it parses nothing, so nothing is compared.
pub struct NoParser;

impl Parser for NoParser {
    type Parsed = std::convert::Infallible;

    fn parse(&self, _source: &SourceText) -> Option<Self::Parsed> {
        None
    }

    fn errors(&self, parsed: &Self::Parsed, _release: u32) -> Vec<SyntaxError> {
        match *parsed {}
    }

    fn tree(&self, parsed: &Self::Parsed) -> Vec<TreeNode> {
        match *parsed {}
    }
}

/// A file prepared for either check: its text, for positions, and the parser's result.
pub struct Prepared<T> {
    text: String,
    index: LineIndex,
    bom: bool,
    /// The parse, `None` without a parser, or the parser's panic message.
    parsed: Result<Option<T>, String>,
}

impl<T> Prepared<T> {
    fn new<P: Parser<Parsed = T>>(parser: &P, source: &SourceText) -> Prepared<T> {
        let parsed = catch_unwind(AssertUnwindSafe(|| parser.parse(source))).map_err(|payload| {
            payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "the parser panicked".to_owned())
        });
        Prepared {
            text: source.text().to_owned(),
            index: LineIndex::new(source.text()),
            bom: source.has_bom(),
            parsed,
        }
    }

    /// A byte offset as javac counts it: UTF-16, with the byte-order mark as one character.
    fn utf16(&self, byte: u32) -> i64 {
        i64::from(self.index.utf16_offset(byte) + u32::from(self.bom))
    }

    /// The 1-based line and column of a javac offset, or of the start for -1.
    fn line_col(&self, utf16: i64) -> (u32, u32) {
        let utf16 = u32::try_from(utf16).unwrap_or(0);
        let byte = self
            .index
            .byte_offset(utf16.saturating_sub(u32::from(self.bom)))
            .unwrap_or(self.text.len() as u32);
        let at = self.index.line_col(byte);
        (at.line + 1, at.col + 1)
    }

    fn located(&self, diagnostic: &Diagnostic) -> Located {
        let (line, column) = self.line_col(diagnostic.position);
        Located {
            line,
            column,
            message: format!(
                "{} ({})",
                diagnostic.message.lines().next().unwrap_or_default(),
                diagnostic.code
            ),
        }
    }
}

/// What javac reported, for the report's summary.
fn summarize(output: &Output) -> Summary {
    let mut summary = Summary {
        codes: output
            .diagnostics
            .iter()
            .flatten()
            .map(|diagnostic| diagnostic.code.clone())
            .collect(),
        ..Summary::default()
    };
    for (_, start, end, depth, _, _) in output.tree.iter().flatten() {
        summary.nodes += 1;
        summary.no_start += u64::from(*start < 0);
        summary.no_end += u64::from(*end < 0);
        summary.depth = summary.depth.max(*depth);
    }
    summary
}

/// javac's errors, in the order it reported them. Warnings don't count.
fn javac_errors(output: &Output) -> Vec<&Diagnostic> {
    output
        .diagnostics
        .iter()
        .flatten()
        .filter(|diagnostic| diagnostic.is_error())
        .collect()
}

/// The common start of both checks' outcomes: javac's summary, its first error, and the status
/// when there's nothing to compare.
fn start_outcome<'a, T>(prepared: &'a Prepared<T>, output: &Output) -> (Outcome, Option<&'a T>) {
    let mut outcome = Outcome {
        summary: summarize(output),
        javac_error: javac_errors(output)
            .first()
            .map(|diagnostic| prepared.located(diagnostic)),
        ..Outcome::default()
    };
    match &prepared.parsed {
        Err(panic) => {
            outcome.status = Status::Panicked(panic.clone());
            (outcome, None)
        }
        Ok(None) => {
            outcome.status = Status::NotCompared;
            (outcome, None)
        }
        Ok(Some(parsed)) => (outcome, Some(parsed)),
    }
}

/// `oracle syntax`: syntax errors against javac's parsing-phase errors.
pub struct Syntax<P>(pub P);

impl<P: Parser> Check for Syntax<P> {
    type Prepared = Prepared<P::Parsed>;

    fn name(&self) -> &'static str {
        "syntax"
    }

    fn sections(&self) -> &'static [Section] {
        &[Section::Diagnostics]
    }

    fn prepare(&self, source: &SourceText, _bytes: &[u8]) -> Self::Prepared {
        Prepared::new(&self.0, source)
    }

    fn depends_on_release(&self) -> bool {
        true
    }

    fn compare(&self, prepared: &Self::Prepared, release: u32, output: &Output) -> Outcome {
        let (mut outcome, parsed) = start_outcome(prepared, output);
        let Some(parsed) = parsed else {
            return outcome;
        };
        let ours: Vec<(String, i64)> = self
            .0
            .errors(parsed, release)
            .into_iter()
            .map(|error| (error.code, prepared.utf16(error.position)))
            .collect();
        let javac: Vec<(String, i64)> = javac_errors(output)
            .into_iter()
            .map(|diagnostic| (diagnostic.code.clone(), diagnostic.position))
            .collect();
        outcome.status = compare_errors(prepared, &ours, &javac);
        outcome
    }
}

/// Compares errors as `(code, position)`: both sides must have errors or neither, and the first
/// in source order must be the same.
fn compare_errors<T>(
    prepared: &Prepared<T>,
    ours: &[(String, i64)],
    javac: &[(String, i64)],
) -> Status {
    let first = |errors: &[(String, i64)]| -> Option<(String, i64)> {
        errors.iter().min_by_key(|(_, position)| *position).cloned()
    };
    let (a, b) = (first(ours), first(javac));
    if a == b {
        return Status::Matched;
    }
    let summary = match (&a, &b) {
        (Some((code, _)), None) => format!("ferrion reports {code}, javac reports no error"),
        (None, Some((code, _))) => format!("javac reports {code}, ferrion reports no error"),
        (Some((ours, _)), Some((theirs, _))) if ours != theirs => {
            format!("ferrion's first error is {ours}, javac's is {theirs}")
        }
        _ => "the first errors are at different positions".to_owned(),
    };
    let position = b
        .as_ref()
        .or(a.as_ref())
        .map_or(0, |(_, position)| *position);
    let (line, column) = prepared.line_col(position);
    let show = |errors: &[(String, i64)]| -> Vec<Option<String>> {
        let mut sorted = errors.to_vec();
        sorted.sort_by_key(|(_, position)| *position);
        (0..=CONTEXT)
            .map(|i| {
                sorted
                    .get(i)
                    .map(|(code, position)| format!("{code} at {position}"))
            })
            .collect()
    };
    Status::Mismatched(Box::new(Mismatch {
        line,
        column,
        summary,
        ferrion: show(ours),
        javac: show(javac),
        at: 0,
    }))
}

/// `oracle tree`: tree positions against javac's, for files javac reports no errors for.
pub struct Tree<P>(pub P);

impl<P: Parser> Check for Tree<P> {
    type Prepared = Prepared<P::Parsed>;

    fn name(&self) -> &'static str {
        "tree"
    }

    fn sections(&self) -> &'static [Section] {
        &[Section::Diagnostics, Section::Tree]
    }

    fn prepare(&self, source: &SourceText, _bytes: &[u8]) -> Self::Prepared {
        Prepared::new(&self.0, source)
    }

    fn compare(&self, prepared: &Self::Prepared, _release: u32, output: &Output) -> Outcome {
        let (mut outcome, parsed) = start_outcome(prepared, output);
        let Some(parsed) = parsed else {
            return outcome;
        };
        // What javac's tree looks like after an error depends on its recovery, which Ferrion
        // doesn't follow, so such files are left out.
        if let Some(error) = outcome.javac_error.take() {
            outcome.status = Status::Skipped(format!(
                "javac reports an error at {}:{}, so the tree isn't compared: {}",
                error.line, error.column, error.message
            ));
            return outcome;
        }
        let position = |offset: Option<u32>| offset.map_or(-1, |byte| prepared.utf16(byte));
        let ours: Vec<(&str, i64, i64, u32)> = self
            .0
            .tree(parsed)
            .iter()
            .map(|node| {
                (
                    node.kind,
                    position(node.start),
                    position(node.end),
                    node.depth,
                )
            })
            .collect();
        let javac: Vec<(&str, i64, i64, u32)> = output
            .tree
            .iter()
            .flatten()
            .map(|(kind, start, end, depth, _, _): &Node| (kind.as_str(), *start, *end, *depth))
            .collect();
        outcome.status = match ours.iter().zip(&javac).position(|(a, b)| a != b) {
            None if ours.len() == javac.len() => Status::Matched,
            None => Status::Mismatched(Box::new(describe_tree(
                prepared,
                &ours,
                &javac,
                ours.len().min(javac.len()),
            ))),
            Some(at) => Status::Mismatched(Box::new(describe_tree(prepared, &ours, &javac, at))),
        };
        outcome
    }
}

type Row<'a> = (&'a str, i64, i64, u32);

fn describe_tree<T>(prepared: &Prepared<T>, ours: &[Row], javac: &[Row], at: usize) -> Mismatch {
    let summary = match (ours.get(at), javac.get(at)) {
        (Some(a), Some(b)) if a.0 != b.0 => format!("ferrion has {}, javac has {}", a.0, b.0),
        (Some(a), Some(b)) if a.3 != b.3 => format!("{} is at a different depth", a.0),
        (Some(a), Some(_)) => format!("the positions of {} differ", a.0),
        (Some(a), None) => format!("ferrion has {} after javac's last node", a.0),
        (None, Some(b)) => format!("ferrion has nothing where javac has {}", b.0),
        (None, None) => unreachable!("trees differ at {at}"),
    };
    let position = javac.get(at).or(ours.get(at)).map_or(0, |row| row.1);
    let (line, column) = prepared.line_col(position);
    let start = at.saturating_sub(CONTEXT);
    let window = |rows: &[Row]| -> Vec<Option<String>> {
        (start..at + CONTEXT + 1)
            .map(|i| {
                rows.get(i).map(|(kind, start, end, depth)| {
                    format!("{}{kind} {start}..{end}", "  ".repeat(*depth as usize))
                })
            })
            .collect()
    };
    Mismatch {
        line,
        column,
        summary,
        ferrion: window(ours),
        javac: window(javac),
        at: at - start,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in parser that gives fixed results, to test the comparisons before the real one
    /// exists.
    struct Fixed {
        errors: Vec<SyntaxError>,
        tree: Vec<TreeNode>,
    }

    impl Parser for Fixed {
        type Parsed = ();

        fn parse(&self, _source: &SourceText) -> Option<()> {
            Some(())
        }

        fn errors(&self, _parsed: &(), release: u32) -> Vec<SyntaxError> {
            // Like a version check: errors only before 16.
            if release < 16 {
                self.errors.clone()
            } else {
                Vec::new()
            }
        }

        fn tree(&self, _parsed: &()) -> Vec<TreeNode> {
            self.tree.clone()
        }
    }

    fn diagnostic(kind: &str, code: &str, position: i64) -> Diagnostic {
        Diagnostic {
            kind: kind.into(),
            code: code.into(),
            start: position,
            position,
            end: position,
            args: Vec::new(),
            message: "message\n  more".into(),
        }
    }

    fn source(text: &str) -> SourceText {
        SourceText::decode(text.as_bytes().to_vec()).unwrap()
    }

    const RECORD: &str = "compiler.err.feature.not.supported.in.source.plural";

    #[test]
    fn without_a_parser_nothing_is_compared() {
        let text = source("record R() {}\n");
        let output = Output {
            diagnostics: Some(vec![
                diagnostic("WARNING", "compiler.warn.w", 0),
                diagnostic("ERROR", RECORD, 0),
            ]),
            tree: Some(vec![
                ("COMPILATION_UNIT".into(), 0, 13, 0, 0, 0),
                ("RECORD".into(), 0, 13, 1, 7, 0),
                ("MODIFIERS".into(), -1, -1, 2, -1, 0),
            ]),
            ..Output::default()
        };
        let check = Tree(NoParser);
        let prepared = check.prepare(&text, text.text().as_bytes());
        let outcome = check.compare(&prepared, 8, &output);
        assert_eq!(outcome.status, Status::NotCompared);
        assert_eq!(outcome.summary.codes, ["compiler.warn.w", RECORD]);
        assert_eq!(
            (
                outcome.summary.nodes,
                outcome.summary.no_start,
                outcome.summary.depth
            ),
            (3, 1, 2)
        );
        let error = outcome.javac_error.unwrap();
        assert_eq!((error.line, error.column), (1, 1));
        assert_eq!(error.message, format!("message ({RECORD})"));
    }

    #[test]
    fn syntax_errors_are_compared_at_each_release() {
        let text = source("record R() {}\n");
        let check = Syntax(Fixed {
            errors: vec![SyntaxError {
                code: RECORD.into(),
                position: 0,
            }],
            tree: Vec::new(),
        });
        assert!(check.depends_on_release());
        let prepared = check.prepare(&text, text.text().as_bytes());
        let errors = Output {
            diagnostics: Some(vec![diagnostic("ERROR", RECORD, 0)]),
            ..Output::default()
        };
        let clean = Output {
            diagnostics: Some(vec![diagnostic("WARNING", "compiler.warn.w", 3)]),
            ..Output::default()
        };
        assert_eq!(check.compare(&prepared, 8, &errors).status, Status::Matched);
        assert_eq!(check.compare(&prepared, 16, &clean).status, Status::Matched);
        let Status::Mismatched(mismatch) = check.compare(&prepared, 16, &errors).status else {
            panic!("expected a mismatch");
        };
        assert_eq!(
            mismatch.summary,
            format!("javac reports {RECORD}, ferrion reports no error")
        );
        let Status::Mismatched(mismatch) = check.compare(&prepared, 8, &clean).status else {
            panic!("expected a mismatch");
        };
        assert_eq!(mismatch.ferrion[0], Some(format!("{RECORD} at 0")));
    }

    #[test]
    fn trees_are_compared_in_javac_terms() {
        // `é` is two bytes and one UTF-16 unit, so byte offsets after it differ from javac's.
        let text = source("/*é*/ class A {}");
        let node = |kind, start, end, depth| TreeNode {
            kind,
            start,
            end,
            depth,
        };
        let check = Tree(Fixed {
            errors: Vec::new(),
            tree: vec![
                node("COMPILATION_UNIT", Some(7), Some(17), 0),
                node("CLASS", Some(7), Some(17), 1),
                node("MODIFIERS", None, None, 2),
            ],
        });
        let prepared = check.prepare(&text, text.text().as_bytes());
        let mut output = Output {
            diagnostics: Some(Vec::new()),
            tree: Some(vec![
                ("COMPILATION_UNIT".into(), 6, 16, 0, 6, 0),
                ("CLASS".into(), 6, 16, 1, 6, 0),
                ("MODIFIERS".into(), -1, -1, 2, -1, 0),
            ]),
            ..Output::default()
        };
        assert_eq!(
            check.compare(&prepared, 27, &output).status,
            Status::Matched
        );

        output.tree.as_mut().unwrap()[1].2 = 15;
        let Status::Mismatched(mismatch) = check.compare(&prepared, 27, &output).status else {
            panic!("expected a mismatch");
        };
        assert_eq!(mismatch.summary, "the positions of CLASS differ");
        assert_eq!(mismatch.at, 1);
        assert_eq!(mismatch.javac[1].as_deref(), Some("  CLASS 6..15"));

        // A file javac reports an error for isn't compared.
        output.diagnostics = Some(vec![diagnostic("ERROR", "compiler.err.expected", 6)]);
        let outcome = check.compare(&prepared, 27, &output);
        assert!(matches!(outcome.status, Status::Skipped(_)));
    }
}
