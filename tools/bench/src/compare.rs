//! `benchmark compare`: the change in each metric between two results files, failing when a metric
//! with a threshold in `thresholds.toml` grows by more than it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::results::{self, METRICS, Results, Unit};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThresholdsFile {
    /// The largest growth allowed, as a percentage, by metric name.
    thresholds: BTreeMap<String, f64>,
}

pub fn read_thresholds(path: &Path) -> Result<BTreeMap<String, f64>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: ThresholdsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    for name in file.thresholds.keys() {
        if results::metric(name).is_none() {
            bail!("{}: unknown metric `{name}`", path.display());
        }
    }
    Ok(file.thresholds)
}

struct Row {
    benchmark: String,
    metric: String,
    base: String,
    head: String,
    change: String,
    threshold: String,
    verdict: &'static str,
}

struct Comparison {
    rows: Vec<Row>,
    notes: Vec<String>,
    regressions: usize,
}

/// Prints the comparison, as a table or in Markdown, and returns whether no metric grew past its
/// threshold.
pub fn run(
    base: &Results,
    head: &Results,
    thresholds: &BTreeMap<String, f64>,
    markdown: bool,
) -> bool {
    let Comparison {
        rows,
        notes,
        regressions,
    } = compare(base, head, thresholds);
    let title = format!(
        "{} → {}",
        base.commit.as_deref().unwrap_or("base"),
        head.commit.as_deref().unwrap_or("head")
    );
    let summary = match regressions {
        0 => "no regressions".to_owned(),
        1 => "1 regression".to_owned(),
        n => format!("{n} regressions"),
    };
    print!(
        "{}",
        if markdown {
            markdown_table(&title, &summary, &rows, &notes)
        } else {
            text_table(&title, &summary, &rows, &notes)
        }
    );
    regressions == 0
}

fn compare(base: &Results, head: &Results, thresholds: &BTreeMap<String, f64>) -> Comparison {
    let mut notes = Vec::new();
    if (&base.os, &base.arch) != (&head.os, &head.arch) {
        notes.push(format!(
            "the base ran on {} {} and the head on {} {}",
            base.os, base.arch, head.os, head.arch
        ));
    }
    let mut rows = Vec::new();
    let mut regressions = 0;
    for new in &head.benchmarks {
        let Some(old) = base.benchmarks.iter().find(|old| old.name == new.name) else {
            notes.push(format!("{} is new", new.name));
            continue;
        };
        if old.input != new.input {
            notes.push(format!(
                "{}: the inputs differ (base `{}`, head `{}`)",
                new.name, old.input, new.input
            ));
        }
        // Known metrics in their order, then any others by name.
        let mut names: Vec<&str> = METRICS.iter().map(|metric| metric.name).collect();
        names.extend(
            new.metrics
                .keys()
                .map(String::as_str)
                .filter(|name| results::metric(name).is_none()),
        );
        for name in names {
            let (Some(&before), Some(&after)) = (old.metrics.get(name), new.metrics.get(name))
            else {
                continue;
            };
            let unit = results::metric(name).map_or(Unit::Count, |metric| metric.unit);
            let change = if before == after {
                0.0
            } else if before == 0.0 {
                f64::INFINITY
            } else {
                100.0 * (after - before) / before
            };
            let threshold = thresholds.get(name).copied();
            let verdict = match threshold {
                Some(limit) if change > limit => {
                    regressions += 1;
                    "REGRESSION"
                }
                Some(limit) if change < -limit => "improved",
                _ => "",
            };
            rows.push(Row {
                benchmark: new.name.clone(),
                metric: results::metric(name)
                    .map_or(name, |metric| metric.label)
                    .to_owned(),
                base: results::format(before, unit),
                head: results::format(after, unit),
                change: format!("{change:+.2}%"),
                threshold: threshold.map_or(String::new(), |limit| format!("{limit}%")),
                verdict,
            });
        }
    }
    for old in &base.benchmarks {
        if !head.benchmarks.iter().any(|new| new.name == old.name) {
            notes.push(format!("{} wasn't run on the head", old.name));
        }
    }
    Comparison {
        rows,
        notes,
        regressions,
    }
}

fn cells(row: &Row) -> [&str; 7] {
    [
        &row.benchmark,
        &row.metric,
        &row.base,
        &row.head,
        &row.change,
        &row.threshold,
        row.verdict,
    ]
}

const HEADER: [&str; 7] = [
    "benchmark",
    "metric",
    "base",
    "head",
    "change",
    "threshold",
    "",
];

fn text_table(title: &str, summary: &str, rows: &[Row], notes: &[String]) -> String {
    let mut widths = HEADER.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(cells(row)) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = format!("{title}: {summary}\n\n");
    let mut line = |cells: [&str; 7]| {
        let mut text = String::new();
        for (i, (cell, width)) in cells.iter().zip(widths).enumerate() {
            let pad = width - cell.chars().count();
            // Numbers are right-aligned.
            if (2..=5).contains(&i) {
                write!(text, "{}{cell}  ", " ".repeat(pad)).unwrap();
            } else {
                write!(text, "{cell}{}  ", " ".repeat(pad)).unwrap();
            }
        }
        out.push_str(text.trim_end());
        out.push('\n');
    };
    line(HEADER);
    let mut last = "";
    for row in rows {
        let mut cells = cells(row);
        // The benchmark's name only on its first row.
        if cells[0] == last {
            cells[0] = "";
        } else {
            last = &row.benchmark;
        }
        line(cells);
    }
    for note in notes {
        writeln!(out, "note: {note}").unwrap();
    }
    out
}

fn markdown_table(title: &str, summary: &str, rows: &[Row], notes: &[String]) -> String {
    let mut out = format!("### Benchmarks: {summary}\n\n{title}\n\n");
    out.push_str("| benchmark | metric | base | head | change | threshold | |\n");
    out.push_str("| --- | --- | ---: | ---: | ---: | ---: | --- |\n");
    for row in rows {
        let verdict = match row.verdict {
            "REGRESSION" => "**regression**",
            other => other,
        };
        let [benchmark, metric, base, head, change, threshold, _] = cells(row);
        writeln!(
            out,
            "| {benchmark} | {metric} | {base} | {head} | {change} | {threshold} | {verdict} |"
        )
        .unwrap();
    }
    if !notes.is_empty() {
        out.push('\n');
        for note in notes {
            writeln!(out, "- {note}").unwrap();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::BenchmarkResult;

    fn results(benchmarks: &[(&str, &[(&str, f64)])]) -> Results {
        let mut results = Results::new(Vec::new());
        results.benchmarks = benchmarks
            .iter()
            .map(|(name, metrics)| BenchmarkResult {
                name: (*name).to_owned(),
                input: "input".to_owned(),
                bytes: None,
                iterations: 5,
                metrics: metrics
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), *value))
                    .collect(),
            })
            .collect();
        results
    }

    #[test]
    fn growth_past_a_threshold_is_a_regression() {
        let thresholds = BTreeMap::from([("instructions".to_owned(), 2.0)]);
        let base = results(&[
            ("a", &[("instructions", 1000.0), ("wall_ns", 100.0)]),
            ("b", &[("instructions", 1000.0)]),
            ("c", &[("instructions", 1000.0)]),
            ("gone", &[("instructions", 1000.0)]),
        ]);
        let head = results(&[
            // Within the threshold, and an ungated metric that doubled.
            ("a", &[("instructions", 1015.0), ("wall_ns", 200.0)]),
            ("b", &[("instructions", 1030.0)]),
            ("c", &[("instructions", 900.0)]),
            ("new", &[("instructions", 1000.0)]),
        ]);
        let comparison = compare(&base, &head, &thresholds);
        assert_eq!(comparison.regressions, 1);
        let verdicts: Vec<(&str, &str, &str)> = comparison
            .rows
            .iter()
            .map(|row| (row.benchmark.as_str(), row.metric.as_str(), row.verdict))
            .collect();
        assert_eq!(
            verdicts,
            [
                ("a", "instructions", ""),
                ("a", "wall time", ""),
                ("b", "instructions", "REGRESSION"),
                ("c", "instructions", "improved"),
            ]
        );
        assert_eq!(
            comparison.notes,
            ["new is new", "gone wasn't run on the head"]
        );
    }

    #[test]
    fn the_thresholds_file_names_known_metrics() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("thresholds.toml");
        let thresholds = read_thresholds(&path).unwrap();
        assert!(thresholds.contains_key("instructions"));
    }
}
