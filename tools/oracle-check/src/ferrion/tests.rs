//! The mapping checked end to end, without a JDK: Ferrion's errors and tree for each snapshot
//! case in `parser-cases/`, against javac's in the case's snapshot, with the checks' own
//! comparisons.

use std::collections::BTreeMap;

use ferrion_diagnostics::SourceText;

use super::*;
use crate::check::{Check, Status};
use crate::oracle::{Diagnostic, Node, Output};
use crate::parser_cases;
use crate::syntax::{Prepared, Syntax, Tree};

/// The comparisons that don't match yet, by case and check, each with the reason. A comparison
/// listed here that matches fails the test, so the list stays current.
const PENDING: &[(&str, &str, &str)] = &[];

/// javac's output for a case at each release, read back from its snapshot: the diagnostics at
/// every release, and the tree at the last.
fn snapshot_outputs(snapshot: &str) -> BTreeMap<u32, Output> {
    let mut outputs: BTreeMap<u32, Output> = BTreeMap::new();
    let mut release = 0;
    let mut in_tree = false;
    for line in snapshot.lines().skip(1) {
        if let Some(rest) = line.strip_prefix("diagnostics at ") {
            release = rest.trim_end_matches(':').parse().unwrap();
            in_tree = false;
            outputs.entry(release).or_default().diagnostics = Some(Vec::new());
            continue;
        }
        if let Some(rest) = line.strip_prefix("tree at ") {
            release = rest.trim_end_matches(':').parse().unwrap();
            in_tree = true;
            outputs.entry(release).or_default().tree = Some(Vec::new());
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields == ["none"] {
            continue;
        }
        let span = |field: &str| -> (i64, i64) {
            let (start, end) = field.split_once("..").unwrap();
            (start.parse().unwrap(), end.parse().unwrap())
        };
        let at = |field: &str| -> i64 { field.strip_prefix('@').unwrap().parse().unwrap() };
        let output = outputs.get_mut(&release).unwrap();
        if in_tree {
            let indent = line.len() - line.trim_start().len();
            let (start, end) = span(fields[1]);
            let node: Node = (
                fields[0].to_owned(),
                start,
                end,
                (indent as u32 - 2) / 2,
                at(fields[2]),
                u8::from(fields.get(3) == Some(&"shared")),
            );
            output.tree.as_mut().unwrap().push(node);
        } else {
            let (start, end) = span(fields[2]);
            output.diagnostics.as_mut().unwrap().push(Diagnostic {
                kind: fields[0].to_owned(),
                code: fields[1].to_owned(),
                start,
                position: at(fields[3]),
                end,
                args: Vec::new(),
                message: String::new(),
            });
        }
    }
    outputs
}

/// Why a status isn't a match, or `None` if it is (or the check left the file out).
fn failure(status: &Status) -> Option<String> {
    match status {
        Status::Matched | Status::Skipped(_) => None,
        Status::Mismatched(mismatch) => {
            let side = |rows: &[Option<String>]| -> String {
                rows.iter()
                    .map(|row| format!("      {}", row.as_deref().unwrap_or("-")))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Some(format!(
                "{}:{}: {}\n    ferrion:\n{}\n    javac:\n{}",
                mismatch.line,
                mismatch.column,
                mismatch.summary,
                side(&mismatch.ferrion),
                side(&mismatch.javac)
            ))
        }
        other => Some(format!("{other:?}")),
    }
}

#[test]
fn the_mapping_matches_every_snapshot_case() {
    let mut unexpected = Vec::new();
    let mut now_matching = Vec::new();
    let mut seen = Vec::new();
    for case in parser_cases::cases().unwrap() {
        let name = case.file_stem().unwrap().to_str().unwrap().to_owned();
        let bytes = std::fs::read(&case).unwrap();
        let snapshot = std::fs::read_to_string(case.with_extension("expected")).unwrap();
        let source = SourceText::decode(bytes).unwrap();
        let prepared = Prepared::new(&source);
        let outputs = snapshot_outputs(&snapshot);
        let mut results: Vec<(&str, Option<String>)> = Vec::new();
        for (&release, output) in &outputs {
            if output.diagnostics.is_none() {
                continue;
            }
            let status = Syntax(Ferrion).compare(&prepared, release, output).status;
            let failure = failure(&status).map(|f| format!("at {release}: {f}"));
            results.push(("syntax", failure));
        }
        if let Some((&release, output)) = outputs.iter().rev().find(|(_, o)| o.tree.is_some()) {
            let status = Tree(Ferrion).compare(&prepared, release, output).status;
            let failure = failure(&status).map(|f| format!("at {release}: {f}"));
            results.push(("tree", failure));
        }
        for check in ["syntax", "tree"] {
            let failures: Vec<&String> = results
                .iter()
                .filter(|(c, _)| *c == check)
                .filter_map(|(_, f)| f.as_ref())
                .collect();
            let pending = PENDING.iter().any(|(c, k, _)| *c == name && *k == check);
            seen.push((name.clone(), check));
            match (failures.is_empty(), pending) {
                (true, true) => now_matching.push(format!("{name} {check}")),
                (false, false) => unexpected.push(format!(
                    "{name} {check}:\n  {}",
                    failures
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join("\n  ")
                )),
                _ => {}
            }
        }
    }
    for (case, check, _) in PENDING {
        assert!(
            seen.iter().any(|(c, k)| c == case && k == check),
            "PENDING names {case} {check}, which isn't a case's check"
        );
    }
    assert!(
        now_matching.is_empty(),
        "these now match; remove them from PENDING: {now_matching:?}"
    );
    assert!(
        unexpected.is_empty(),
        "Ferrion's side doesn't match javac's snapshot:\n{}",
        unexpected.join("\n")
    );
}

#[test]
fn snapshots_read_back() {
    let snapshot = concat!(
        "# header\n",
        "diagnostics at 8:\n",
        "  ERROR compiler.err.x 1..2 @1 [a] m\n",
        "diagnostics at 16:\n",
        "  none\n",
        "tree at 16:\n",
        "  COMPILATION_UNIT 0..13 @0 \"x\"\n",
        "    MODIFIERS -1..-1 @-1 shared\n",
    );
    let outputs = snapshot_outputs(snapshot);
    let diagnostic = &outputs[&8].diagnostics.as_ref().unwrap()[0];
    assert_eq!(
        (diagnostic.code.as_str(), diagnostic.position),
        ("compiler.err.x", 1)
    );
    assert_eq!(outputs[&16].diagnostics.as_ref().unwrap().len(), 0);
    assert_eq!(
        outputs[&16].tree.as_ref().unwrap()[1],
        ("MODIFIERS".to_owned(), -1, -1, 1, -1, 1)
    );
}

fn first_error(text: &str, release: u32) -> Option<(String, u32)> {
    let source = SourceText::decode(text.as_bytes().to_vec()).unwrap();
    let parsed = Ferrion.parse(&source, release).unwrap();
    Ferrion
        .errors(&parsed)
        .into_iter()
        .min_by_key(|e| e.position)
        .map(|e| (e.code, e.position))
}

/// Codes and positions observed with javac 27 on the same inputs.
#[test]
fn lexical_errors_have_javac_codes_and_positions() {
    let cases: &[(&str, &str, u32)] = &[
        ("class A { int a = 0x; }", "invalid.hex.number", 18),
        ("class A { double a = 1e+; }", "malformed.fp.lit", 21),
        (
            "class A { int a = 0b1012; }",
            "illegal.digit.in.binary.literal",
            21,
        ),
        (
            "class A { int a = 0b2; }",
            "illegal.digit.in.binary.literal",
            18,
        ),
        ("class A { int a = 1_; }", "illegal.underscore", 19),
        ("class A { int a = 1__; }", "illegal.underscore", 20),
        ("class A { double a = 0x.p1; }", "invalid.hex.number", 21),
        (
            "class A { int a = 0129; }",
            "illegal.digit.in.octal.literal",
            21,
        ),
        ("class A { double a = 0x1.0; }", "malformed.fp.lit", 21),
        ("class A { int a = 0b; }", "invalid.binary.number", 18),
        ("class A { long a = 0x_1L; }", "illegal.underscore", 21),
        ("class A { String s = \"a\\qb\"; }", "illegal.esc.char", 24),
        ("class A { char c = 'ab'; }", "unclosed.char.lit", 19),
        (
            "class A { char c = '\n'; }",
            "illegal.line.end.in.char.lit",
            19,
        ),
        ("class A { int # x; }", "illegal.char", 14),
        (
            "class A { double b = 1.2\\u0663; }",
            "illegal.nonascii.digit",
            24,
        ),
    ];
    for &(text, code, position) in cases {
        assert_eq!(
            first_error(text, 27),
            Some((format!("compiler.err.{code}"), position)),
            "{text:?}"
        );
    }
}

/// javac reports an error at the end of the file as `premature.eof`, at the end of the last token,
/// and its code for a token that can't start a declaration depends on the release.
#[test]
fn errors_at_the_end_of_the_file_and_by_release() {
    assert_eq!(
        first_error("class A {\n  int x = 1\n\n", 27),
        Some(("compiler.err.premature.eof".into(), 21))
    );
    assert_eq!(
        first_error("class A {}\n}\n", 8),
        Some(("compiler.err.expected3".into(), 11))
    );
    assert_eq!(
        first_error("class A {}\n}\n", 17),
        Some(("compiler.err.expected4".into(), 11))
    );
    assert_eq!(
        first_error("class A {}\n}\n", 25),
        Some(("compiler.err.class.method.or.field.expected".into(), 11))
    );
}
