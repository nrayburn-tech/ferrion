//! The version check against javac on the inputs in `tests/version/`, one or more per row.
//!
//! Each `<case>.java` there has a `<case>.expected` with javac 27's diagnostics at every release
//! from 8 to 27, one per line: the releases, the phase (`parse` from `tools/javac-oracle`'s
//! `diagnostics` section, `later` from a full `javac --release N -XDrawDiagnostics` compile, only
//! for the cases whose feature javac reports after parsing), the severity, javac's code, the
//! feature's name or `expected`'s argument (`-` for none), and the offset javac reports. The files
//! were written by running both on the inputs; rerun them the same way after changing an input.
//!
//! [`cases`] parses each input at every release and compares the version check's diagnostics with
//! javac's, as the oracle comparison does: javac reports each feature once per file, at its first
//! use, and at most one error at an offset, so Ferrion's are reduced the same way first. A
//! javac diagnostic matches a version diagnostic with the same severity, code, argument, and
//! anchor, or any other Ferrion error at its offset (a syntax error the grammar reports). Later
//! diagnostics are compared by severity and anchor, and only at releases where javac's parse found
//! no error, since javac stops there. [`DIFFERENCES`] lists what is known to differ.

use std::collections::HashSet;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_java_version::{LATEST_RELEASE, SUPPORTED_RELEASES};

use super::read;
use crate::version::{FEATURES, Feature, javac_diagnostic};
use crate::{JavacPhase, ParseDiagnostic, ParseDiagnosticKind as D, ParseOptions, Severity, parse};

/// Cases whose input the parser can't parse yet, so they can't be compared. Each still fails to
/// parse at the latest release, which [`cases`] checks, so that a case leaves this list as soon
/// as its grammar lands.
const PENDING: &[&str] = &[];

/// Known differences from javac: in a case, at some releases, a diagnostic at an offset that only
/// one side reports. Each must still differ. Syntax errors the grammar reports and javac doesn't
/// aren't compared here: at 10, `(var a, b) -> a` gets the grammar's error for mixing `var` with an
/// inferred parameter first, where javac reports only that `var` lambda parameters need 11.
const DIFFERENCES: &[(&str, RangeInclusive<u32>, u32, &str)] = &[
    (
        "records_local",
        8..=15,
        34,
        "javac reads `record R(...)` in a block as a variable of type `record` below 16 and warns",
    ),
    (
        "records_local",
        8..=15,
        35,
        "javac's older reading of a local record: `';' expected` after the name",
    ),
    (
        "records_local",
        8..=15,
        41,
        "javac's older reading of a local record: `';' expected` after the header",
    ),
    (
        "records_member",
        8..=15,
        26,
        "javac reads `record` in a class body as a method's result type below 16 and warns",
    ),
    (
        "local_enums_annotated",
        8..=15,
        39,
        "javac 27 accepts a local enum with modifiers or annotations below 16",
    ),
    (
        "unnamed_variables_type_pattern",
        8..=8,
        65,
        "at 8, javac reports `_` in a type pattern as an unnamed variable as well as warning",
    ),
    (
        "unnamed_variables_in_switch",
        8..=8,
        72,
        "at 8, javac reports `_` in a case pattern with `use.of.underscore.not.allowed`",
    ),
    (
        "multiple_case_labels_patterns",
        8..=8,
        73,
        "at 8, javac reports `_` in a case pattern with `use.of.underscore.not.allowed`",
    ),
    (
        "multiple_case_labels_patterns",
        8..=8,
        83,
        "at 8, javac reports `_` in a case pattern with `use.of.underscore.not.allowed`",
    ),
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/version")
}

#[test]
fn every_feature_has_an_input() {
    for feature in Feature::all() {
        let path = dir().join(format!("{}.java", feature.id()));
        assert!(
            path.exists(),
            "{:?} has no input at {}",
            feature,
            path.display()
        );
    }
}

/// A diagnostic as javac's `.expected` lines give it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Javac {
    phase: JavacPhase,
    severity: Severity,
    code: String,
    argument: Option<String>,
    offset: u32,
}

/// The lines of an `.expected` file, each with its releases.
fn expected(text: &str) -> Vec<(HashSet<u32>, Javac)> {
    let mut out = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let fields: Vec<_> = line.split(' ').collect();
        let [releases, phase, severity, code, argument, offset] = fields[..] else {
            panic!("a malformed line: {line}");
        };
        let mut set = HashSet::new();
        for range in releases.split(',') {
            let (start, end) = range.split_once('-').unwrap_or((range, range));
            set.extend(start.parse::<u32>().unwrap()..=end.parse::<u32>().unwrap());
        }
        out.push((
            set,
            Javac {
                phase: match phase {
                    "parse" => JavacPhase::Parse,
                    _ => JavacPhase::Later,
                },
                severity: match severity {
                    "error" => Severity::Error,
                    _ => Severity::Warning,
                },
                code: code.to_owned(),
                argument: (argument != "-").then(|| argument.to_owned()),
                offset: offset.parse().unwrap(),
            },
        ));
    }
    out
}

/// Ferrion's diagnostics as javac would report them: each feature javac names in its diagnostic
/// once, at its first use, and in each phase one error at an offset.
fn as_javac_reports(diagnostics: &[ParseDiagnostic]) -> Vec<&ParseDiagnostic> {
    let mut features = HashSet::new();
    let mut errors = HashSet::new();
    let mut out = Vec::new();
    for d in diagnostics {
        if let D::FeatureNotInRelease { feature, .. } | D::PreviewFeature(feature) = d.kind
            && feature.row().javac.argument.is_some()
            && !features.insert(feature)
        {
            continue;
        }
        if d.severity == Severity::Error && !errors.insert((d.javac_phase, d.anchor)) {
            continue;
        }
        out.push(d);
    }
    out
}

/// Whether a mismatch is a known difference, recording which one in `used`.
fn is_difference(case: &str, release: u32, offset: u32, used: &mut HashSet<usize>) -> bool {
    let found = DIFFERENCES
        .iter()
        .position(|(c, releases, o, _)| *c == case && releases.contains(&release) && *o == offset);
    used.extend(found);
    found.is_some()
}

/// Compares one case at one release, adding a line to `failures` for each mismatch.
fn compare(
    case: &str,
    text: &Arc<str>,
    release: u32,
    javac: &[Javac],
    failures: &mut Vec<String>,
    used: &mut HashSet<usize>,
) {
    let parsed = parse(text.clone(), ParseOptions { release });
    let reported = as_javac_reports(&parsed.diagnostics);
    let javac_parse_error = javac
        .iter()
        .any(|j| j.phase == JavacPhase::Parse && j.severity == Severity::Error);
    let compared = |phase: JavacPhase| phase == JavacPhase::Parse || !javac_parse_error;
    let matches = |d: &ParseDiagnostic, j: &Javac| {
        d.javac_phase == j.phase
            && d.severity == j.severity
            && d.anchor == j.offset
            && (j.phase == JavacPhase::Later || {
                let javac = javac_diagnostic(d.kind, d.severity).expect("a version diagnostic");
                javac.code == j.code && javac.argument == j.argument.as_deref()
            })
    };
    for d in reported.iter().filter(|d| d.kind.is_version()) {
        if d.javac_phase == JavacPhase::Never || !compared(d.javac_phase) {
            continue;
        }
        if !javac.iter().any(|j| matches(d, j)) && !is_difference(case, release, d.anchor, used) {
            failures.push(format!(
                "{case} at {release}: Ferrion reports {:?} {:?} at {}, javac doesn't",
                d.severity, d.kind, d.anchor
            ));
        }
    }
    for j in javac.iter().filter(|j| compared(j.phase)) {
        let by_version = reported
            .iter()
            .any(|d| d.kind.is_version() && matches(d, j));
        let by_grammar = reported.iter().any(|d| {
            !d.kind.is_version()
                && j.phase == JavacPhase::Parse
                && d.severity == j.severity
                && d.anchor == j.offset
        });
        if !by_version && !by_grammar && !is_difference(case, release, j.offset, used) {
            failures.push(format!(
                "{case} at {release}: javac reports {j:?}, Ferrion doesn't"
            ));
        }
    }
}

#[test]
fn cases() {
    let mut names: Vec<String> = fs::read_dir(dir())
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            (path.extension()? == "java").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort();
    assert!(names.len() >= FEATURES.len());
    for pending in PENDING {
        assert!(names.iter().any(|n| n == pending), "{pending} has no input");
    }
    let mut failures = Vec::new();
    let mut used = HashSet::new();
    for name in &names {
        let text: Arc<str> = Arc::from(read(&dir().join(format!("{name}.java"))));
        let expected = expected(&read(&dir().join(format!("{name}.expected"))));
        if PENDING.contains(&name.as_str()) {
            let parsed = parse(
                text.clone(),
                ParseOptions {
                    release: LATEST_RELEASE,
                },
            );
            if parsed.diagnostics.iter().all(|d| d.kind.is_version()) {
                failures.push(format!("{name} parses now: remove it from PENDING"));
            }
            continue;
        }
        for release in SUPPORTED_RELEASES {
            let javac: Vec<Javac> = expected
                .iter()
                .filter(|(releases, _)| releases.contains(&release))
                .map(|(_, j)| j.clone())
                .collect();
            compare(name, &text, release, &javac, &mut failures, &mut used);
        }
    }
    for (i, difference) in DIFFERENCES.iter().enumerate() {
        if !used.contains(&i) {
            failures.push(format!("{difference:?} isn't a difference any more"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
