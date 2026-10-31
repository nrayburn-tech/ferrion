//! Summarizing and printing results.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::allowed::{Allowed, Category, Counts};
use crate::check::{Located, Mismatch, Status};
use crate::inventory::SourceFile;
use crate::run::{FileResult, Timings};

/// Counts for a project at a release, or in total.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub files: u64,
    pub matched: u64,
    pub mismatched: u64,
    /// Files that aren't compared: they aren't UTF-8 or ASCII, or the check leaves out files
    /// javac rejects. They aren't failures.
    pub skipped: u64,
    pub round_trip_failures: u64,
    pub javac_failures: u64,
    pub panics: u64,
    /// Comparisons that stopped at, or left out a file for, an error javac reported. Not
    /// failures in themselves.
    pub javac_errors: u64,
    /// javac's side ran, but Ferrion has nothing to compare it with yet. Not failures.
    pub not_compared: u64,
    #[serde(skip)]
    pub allowed: Counts,
}

impl Tally {
    fn add(&mut self, result: &FileResult) {
        let outcome = &result.outcome;
        self.files += 1;
        match &outcome.status {
            Status::Matched => self.matched += 1,
            Status::Mismatched(_) => self.mismatched += 1,
            Status::Skipped(_) => self.skipped += 1,
            Status::JavacFailed(_) => self.javac_failures += 1,
            Status::Panicked(_) => self.panics += 1,
            Status::NotCompared => self.not_compared += 1,
        }
        self.round_trip_failures += u64::from(outcome.round_trip.is_some());
        self.javac_errors += u64::from(outcome.javac_error.is_some());
        self.allowed.merge(&outcome.allowed);
    }

    fn merge(&mut self, other: &Tally) {
        self.files += other.files;
        self.matched += other.matched;
        self.mismatched += other.mismatched;
        self.skipped += other.skipped;
        self.round_trip_failures += other.round_trip_failures;
        self.javac_failures += other.javac_failures;
        self.panics += other.panics;
        self.javac_errors += other.javac_errors;
        self.not_compared += other.not_compared;
        self.allowed.merge(&other.allowed);
    }

    pub fn failures(&self) -> u64 {
        self.mismatched + self.round_trip_failures + self.javac_failures + self.panics
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub check: &'static str,
    pub rows: Vec<Row>,
    pub total: Tally,
    pub allowed: Vec<AllowedCount>,
    pub failures: Vec<Failure>,
    /// Results that aren't failures but are worth knowing about: skipped files, and comparisons
    /// that stopped at a lexical error javac reported.
    pub notes: Vec<Failure>,
    /// What javac reported, for checks that summarize it.
    pub javac: JavacReport,
    pub timings: TimingsReport,
}

/// A summary of what javac reported across the run.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavacReport {
    /// Each diagnostic code javac reported, most frequent first.
    pub codes: Vec<CodeCount>,
    pub nodes: u64,
    pub nodes_without_start: u64,
    pub nodes_without_end: u64,
    /// The files with the deepest trees, deepest first, each at the release where it's deepest.
    pub deepest: Vec<Depth>,
}

#[derive(Serialize)]
pub struct CodeCount {
    pub code: String,
    /// How many times javac reported it.
    pub diagnostics: u64,
    /// In how many file-releases.
    pub files: u64,
}

#[derive(Serialize)]
pub struct Depth {
    pub file: String,
    pub release: u32,
    pub depth: u32,
}

/// How many of the deepest trees a report keeps.
const DEEPEST: usize = 50;

#[derive(Serialize)]
pub struct Row {
    pub project: String,
    pub release: u32,
    #[serde(flatten)]
    pub tally: Tally,
}

#[derive(Serialize)]
pub struct AllowedCount {
    pub id: &'static str,
    pub category: Category,
    pub count: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub file: String,
    pub release: u32,
    pub kind: &'static str,
    pub line: u32,
    pub column: u32,
    pub message: String,
    /// The items around a mismatch, for the first failures only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Mismatch>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingsReport {
    pub javac_files: usize,
    pub javac_runs: usize,
    pub javac_seconds: f64,
    pub compare_seconds: f64,
}

impl Report {
    /// Summarizes results, keeping details for the first `detailed` failures.
    pub fn new(
        check: &'static str,
        files: &[SourceFile],
        results: &[FileResult],
        timings: &Timings,
        detailed: usize,
    ) -> Report {
        let mut rows: BTreeMap<(&str, u32), Tally> = BTreeMap::new();
        let mut failures = Vec::new();
        let mut notes = Vec::new();
        let mut javac = JavacReport::default();
        let mut codes: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        let mut depths = Vec::new();
        for result in results {
            let file = &files[result.file];
            rows.entry((&file.project, result.release))
                .or_default()
                .add(result);
            let failure = |kind, at: &Located, details| Failure {
                file: file.display.clone(),
                release: result.release,
                kind,
                line: at.line,
                column: at.column,
                message: at.message.clone(),
                details,
            };
            let outcome = &result.outcome;
            match &outcome.status {
                Status::Mismatched(mismatch) => {
                    let at = Located {
                        line: mismatch.line,
                        column: mismatch.column,
                        message: mismatch.summary.clone(),
                    };
                    let details = (failures.len() < detailed).then(|| (**mismatch).clone());
                    failures.push(failure("mismatch", &at, details));
                }
                Status::JavacFailed(message) => {
                    failures.push(failure("javac", &start(message), None))
                }
                Status::Panicked(message) => failures.push(failure("panic", &start(message), None)),
                Status::Skipped(reason) => notes.push(failure("skipped", &start(reason), None)),
                Status::Matched | Status::NotCompared => {}
            }
            let summary = &outcome.summary;
            for (i, code) in summary.codes.iter().enumerate() {
                let count = codes.entry(code).or_default();
                count.0 += 1;
                count.1 += u64::from(!summary.codes[..i].contains(code));
            }
            javac.nodes += summary.nodes;
            javac.nodes_without_start += summary.no_start;
            javac.nodes_without_end += summary.no_end;
            if summary.nodes > 0 {
                depths.push(Depth {
                    file: file.display.clone(),
                    release: result.release,
                    depth: summary.depth,
                });
            }
            if let Some(round_trip) = &outcome.round_trip {
                failures.push(failure("round-trip", round_trip, None));
            }
            if let Some(error) = &outcome.javac_error {
                notes.push(failure("javac-error", error, None));
            }
        }
        // Projects keep the order they were given in.
        let order: Vec<&str> = files.iter().map(|file| file.project.as_str()).collect();
        let mut rows: Vec<Row> = rows
            .into_iter()
            .map(|((project, release), tally)| Row {
                project: project.to_owned(),
                release,
                tally,
            })
            .collect();
        rows.sort_by_key(|row| {
            (
                order.iter().position(|&project| project == row.project),
                row.release,
            )
        });
        let mut total = Tally::default();
        for row in &rows {
            total.merge(&row.tally);
        }
        let allowed = Allowed::ALL
            .into_iter()
            .map(|allowed| AllowedCount {
                id: allowed.id(),
                category: allowed.category(),
                count: total.allowed.get(allowed),
            })
            .collect();
        javac.codes = codes
            .into_iter()
            .map(|(code, (diagnostics, files))| CodeCount {
                code: code.to_owned(),
                diagnostics,
                files,
            })
            .collect();
        javac
            .codes
            .sort_by_key(|count| std::cmp::Reverse((count.files, count.diagnostics)));
        depths.sort_by_key(|depth| std::cmp::Reverse(depth.depth));
        let mut seen = std::collections::HashSet::new();
        depths.retain(|depth| seen.insert(depth.file.clone()));
        depths.truncate(DEEPEST);
        javac.deepest = depths;
        Report {
            check,
            rows,
            total,
            allowed,
            failures,
            notes,
            javac,
            timings: TimingsReport {
                javac_files: timings.javac_files,
                javac_runs: timings.javac_runs,
                javac_seconds: timings.javac.as_secs_f64(),
                compare_seconds: timings.compare.as_secs_f64(),
            },
        }
    }

    /// The report as text: the table, then the first `limit` failures.
    pub fn text(&self, limit: usize) -> String {
        let mut out = String::new();
        let header = [
            "project",
            "release",
            "files",
            "matched",
            "mismatched",
            "skipped",
            "round-trip",
            "javac-crash",
            "panics",
            "javac-errors",
        ];
        let not_compared = self.total.not_compared > 0;
        let mut header = header.map(str::to_owned).to_vec();
        if not_compared {
            header.push("not-compared".to_owned());
        }
        let mut table = vec![header];
        let row = |project: &str, release: String, t: &Tally| {
            let mut row = vec![
                project.to_owned(),
                release,
                t.files.to_string(),
                t.matched.to_string(),
                t.mismatched.to_string(),
                t.skipped.to_string(),
                t.round_trip_failures.to_string(),
                t.javac_failures.to_string(),
                t.panics.to_string(),
                t.javac_errors.to_string(),
            ];
            if not_compared {
                row.push(t.not_compared.to_string());
            }
            row
        };
        for r in &self.rows {
            table.push(row(&r.project, r.release.to_string(), &r.tally));
        }
        if self.rows.len() > 1 {
            table.push(row("total", String::new(), &self.total));
        }
        let widths: Vec<usize> = (0..table[0].len())
            .map(|i| table.iter().map(|row| row[i].len()).max().unwrap())
            .collect();
        for row in &table {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate() {
                if i == 0 {
                    write!(line, "{cell:<w$}", w = widths[i]).unwrap();
                } else {
                    write!(line, "  {cell:>w$}", w = widths[i]).unwrap();
                }
            }
            writeln!(out, "{}", line.trim_end()).unwrap();
        }

        if not_compared {
            writeln!(
                out,
                "\n{} file-releases weren't compared: Ferrion's side of `{}` isn't plugged in \
                 yet, so only javac's side ran",
                self.total.not_compared, self.check
            )
            .unwrap();
        }
        out += &self.javac.text(limit);

        if !self.total.allowed.is_empty() {
            let applied: Vec<String> = self
                .allowed
                .iter()
                .filter(|allowed| allowed.count > 0)
                .map(|allowed| format!("{} ({})", allowed.id, allowed.count))
                .collect();
            writeln!(out, "\nallowed differences: {}", applied.join(", ")).unwrap();
        }
        let shown = self.notes.len().min(limit);
        if shown > 0 {
            writeln!(
                out,
                "\n{} notes (skipped files, and files javac reported an error for); the first \
                 {shown}:",
                self.notes.len()
            )
            .unwrap();
        }
        for note in &self.notes[..shown] {
            writeln!(
                out,
                "  {}: {}:{}:{} (release {}): {}",
                note.kind, note.file, note.line, note.column, note.release, note.message
            )
            .unwrap();
        }

        let shown = self.failures.len().min(limit);
        if shown > 0 {
            writeln!(
                out,
                "\n{} failures; the first {shown}:",
                self.failures.len()
            )
            .unwrap();
        }
        for failure in &self.failures[..shown] {
            writeln!(
                out,
                "\n{}: {}:{}:{} (release {}): {}",
                failure.kind,
                failure.file,
                failure.line,
                failure.column,
                failure.release,
                failure.message
            )
            .unwrap();
            if let Some(details) = &failure.details {
                write_sides(&mut out, details);
            }
        }
        out
    }
}

impl JavacReport {
    /// The summary as text, showing the `limit` most frequent codes and deepest trees.
    fn text(&self, limit: usize) -> String {
        let mut out = String::new();
        if !self.codes.is_empty() {
            let shown = self.codes.len().min(limit);
            writeln!(
                out,
                "\njavac reported {} diagnostic codes; the {shown} in the most file-releases \
                 (diagnostics, file-releases):",
                self.codes.len()
            )
            .unwrap();
            for count in &self.codes[..shown] {
                writeln!(
                    out,
                    "  {:<60} {:>7} {:>7}",
                    count.code, count.diagnostics, count.files
                )
                .unwrap();
            }
        }
        if self.nodes > 0 {
            writeln!(
                out,
                "\njavac's trees: {} nodes, {} without a start position, {} without an end position",
                self.nodes, self.nodes_without_start, self.nodes_without_end
            )
            .unwrap();
            let shown = self.deepest.len().min(limit);
            if shown > 0 {
                writeln!(out, "the {shown} deepest:").unwrap();
            }
            for depth in &self.deepest[..shown] {
                writeln!(
                    out,
                    "  {:>6}  {} (release {})",
                    depth.depth, depth.file, depth.release
                )
                .unwrap();
            }
        }
        out
    }
}

/// A failure message that has no position of its own.
fn start(message: &str) -> Located {
    Located {
        line: 1,
        column: 1,
        message: message.to_owned(),
    }
}

fn write_sides(out: &mut String, mismatch: &Mismatch) {
    let width = mismatch
        .ferrion
        .iter()
        .flatten()
        .map(|item| item.chars().count())
        .max()
        .unwrap_or(0)
        .max("ferrion".len());
    writeln!(out, "     {:<width$}   javac", "ferrion").unwrap();
    for (i, (ours, javac)) in mismatch.ferrion.iter().zip(&mismatch.javac).enumerate() {
        if ours.is_none() && javac.is_none() {
            continue;
        }
        let marker = if i == mismatch.at { ">" } else { " " };
        let ours = ours.as_deref().unwrap_or("-");
        let javac = javac.as_deref().unwrap_or("-");
        let pad = width - ours.chars().count();
        writeln!(out, "   {marker} {ours}{:pad$}   {javac}", "").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{Outcome, Summary};

    fn file(project: &str, path: &str) -> SourceFile {
        SourceFile {
            project: project.into(),
            path: path.into(),
            display: format!("{project}/{path}"),
            releases: vec![8],
            bytes: Vec::new(),
            source: Err(String::new()),
            key: [0; 32],
        }
    }

    #[test]
    fn tallies_and_failures() {
        let files = [file("a", "A.java"), file("b", "B.java")];
        let mismatch = Mismatch {
            line: 3,
            column: 7,
            summary: "ferrion has GT, javac has GTGT".into(),
            ferrion: vec![Some("GT 5..6 \">\"".into()), None],
            javac: vec![
                Some("GTGT 5..7 \">>\"".into()),
                Some("EOF 7..7 \"\"".into()),
            ],
            at: 0,
        };
        let round_trip = Located {
            line: 1,
            column: 2,
            message: "gap".into(),
        };
        let results = [
            FileResult {
                file: 0,
                release: 8,
                outcome: Outcome::default(),
            },
            FileResult {
                file: 0,
                release: 17,
                outcome: Outcome {
                    status: Status::Mismatched(Box::new(mismatch)),
                    round_trip: Some(round_trip),
                    ..Outcome::default()
                },
            },
            FileResult {
                file: 1,
                release: 8,
                outcome: Outcome::with_status(Status::Skipped("UTF-16".into())),
            },
        ];
        let report = Report::new("tokens", &files, &results, &Timings::default(), 10);
        assert_eq!(report.rows.len(), 3);
        assert_eq!(report.total.files, 3);
        assert_eq!(report.total.matched, 1);
        assert_eq!(report.total.mismatched, 1);
        assert_eq!(report.total.skipped, 1);
        assert_eq!(report.total.failures(), 2);
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.failures[0].kind, "mismatch");
        assert_eq!(report.failures[1].kind, "round-trip");

        let text = report.text(10);
        assert!(
            text.contains("mismatch: a/A.java:3:7 (release 17): ferrion has GT, javac has GTGT")
        );
        assert!(
            text.contains(r#"   > GT 5..6 ">"   GTGT 5..7 ">>""#),
            "{text}"
        );
        assert!(text.contains(r#"     -             EOF 7..7 """#), "{text}");
        assert!(text.contains("round-trip: a/A.java:1:2 (release 17): gap"));
        assert!(
            text.contains("  skipped: b/B.java:1:1 (release 8): UTF-16"),
            "{text}"
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["total"]["roundTripFailures"], 1);
        assert_eq!(json["rows"][0]["project"], "a");
        assert_eq!(json["failures"][0]["details"]["at"], 0);
    }

    #[test]
    fn javac_summaries() {
        let files = [file("a", "A.java"), file("a", "B.java")];
        let outcome = |codes: &[&str], depth| Outcome {
            status: Status::NotCompared,
            summary: Summary {
                codes: codes.iter().map(|&code| code.to_owned()).collect(),
                nodes: 10,
                no_start: 2,
                no_end: 1,
                depth,
            },
            ..Outcome::default()
        };
        let results = [
            FileResult {
                file: 0,
                release: 8,
                outcome: outcome(&["compiler.err.expected", "compiler.err.expected"], 4),
            },
            FileResult {
                file: 0,
                release: 17,
                outcome: outcome(&[], 5),
            },
            FileResult {
                file: 1,
                release: 8,
                outcome: outcome(&["compiler.err.premature.eof", "compiler.err.expected"], 3),
            },
        ];
        let report = Report::new("tree", &files, &results, &Timings::default(), 10);
        assert_eq!(report.total.not_compared, 3);
        assert_eq!(report.total.failures(), 0);
        let codes: Vec<(&str, u64, u64)> = report
            .javac
            .codes
            .iter()
            .map(|count| (count.code.as_str(), count.diagnostics, count.files))
            .collect();
        assert_eq!(
            codes,
            [
                ("compiler.err.expected", 3, 2),
                ("compiler.err.premature.eof", 1, 1)
            ]
        );
        assert_eq!(report.javac.nodes, 30);
        assert_eq!(report.javac.nodes_without_start, 6);
        // Each file once, at the release where it's deepest.
        let deepest: Vec<(&str, u32, u32)> = report
            .javac
            .deepest
            .iter()
            .map(|depth| (depth.file.as_str(), depth.release, depth.depth))
            .collect();
        assert_eq!(deepest, [("a/A.java", 17, 5), ("a/B.java", 8, 3)]);
        let text = report.text(10);
        assert!(text.contains("not-compared"), "{text}");
        assert!(text.contains("3 file-releases weren't compared"), "{text}");
        assert!(text.contains("javac reported 2 diagnostic codes"), "{text}");
    }
}
