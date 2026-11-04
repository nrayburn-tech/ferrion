//! Counting a types run, per project and in total, and printing it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

/// Counts for a project, or for every project.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub source_sets: u64,
    /// Source sets javac threw on as a whole, so their files were compiled one at a time.
    pub source_set_crashes: u64,
    /// Source sets the oracle's JVM gave no output for: failures.
    pub jvm_failures: u64,
    pub cached: u64,
    /// Files of the source sets, and files annotation processors generated.
    pub files: u64,
    pub generated_files: u64,
    pub excluded: Excluded,
    /// Files compared: every file not left out.
    pub compared_files: u64,
    pub lombok_types: u64,
    /// In compared files, outside Lombok types: entries of each aspect.
    pub expressions: u64,
    pub constants: u64,
    pub calls: u64,
    /// Calls javac made up: implicit `super()` calls and enum constants' `new`.
    pub implicit_calls: u64,
    pub variables: u64,
    /// Expressions by javac's tree kind.
    pub expression_kinds: BTreeMap<String, u64>,
    /// Variables by finality.
    pub finality: BTreeMap<String, u64>,
    /// Relation queries, by query.
    pub relations: BTreeMap<&'static str, u64>,
    /// Source sets whose relations javac threw on.
    pub relation_errors: u64,
    pub comparison: Comparison,
    /// For files left out for javac's errors, how many have each error code.
    pub error_codes: BTreeMap<String, u64>,
    /// Switch errors (exhaustiveness, dominance, duplicate labels) by code, in every file.
    pub switch_errors: BTreeMap<String, u64>,
    /// Files whose only language errors are switch errors.
    pub files_with_only_switch_errors: u64,
    /// The oracle's own time for the source sets it ran (its `millis`), and the size of its
    /// output.
    pub javac_seconds: f64,
    pub output_bytes: u64,
    /// javac's analysis time summed over the compared files, in microseconds.
    pub analyze_micros: u64,
    /// Ferrion's time.
    pub ferrion_seconds: f64,
    /// The most fuel one relation query used, with the source set and query, and the largest
    /// relations arena, in types.
    pub relation_fuel: u64,
    pub relation_fuel_query: String,
    pub relation_arena: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Excluded {
    /// javac reported a language error in the file.
    pub javac_errors: u64,
    /// javac threw on the file, even on its own.
    pub crashes: u64,
    /// Module declarations, which have no expressions.
    pub module_declarations: u64,
    /// javac never analyzed the file.
    pub unanalyzed: u64,
    /// Ferrion's results for the source set failed.
    pub ferrion_failures: u64,
    /// A file a processor generated, whose text isn't what the cached run of javac saw.
    pub stale_generated: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub matched: u64,
    /// By aspect.
    pub matched_by: BTreeMap<&'static str, u64>,
    /// By aspect, then category.
    pub mismatched: BTreeMap<&'static str, BTreeMap<&'static str, u64>>,
    /// Mismatches listed in `differences/types-known.txt`, by category.
    pub known: BTreeMap<String, u64>,
    /// Entries (relation queries included) in source sets Ferrion has no results for.
    pub not_compared: u64,
    pub extra: u64,
    /// Ferrion's entries a later milestone types.
    pub left: u64,
    /// Ferrion's entries that wait for overload resolution.
    pub waiting: u64,
    /// Entries counted apart, by aspect and what they wait for.
    pub apart: BTreeMap<String, u64>,
    /// javac's parameters of implicit constructors, which have no counterpart.
    pub implicit_parameters: u64,
    /// Ferrion's entries in annotations on record components that javac dropped from its trees.
    pub dropped_annotations: u64,
    /// Relation queries about a type javac couldn't resolve.
    pub unresolved: u64,
}

impl Tally {
    pub fn add_mismatch(&mut self, aspect: &'static str, category: &'static str) {
        *self
            .comparison
            .mismatched
            .entry(aspect)
            .or_default()
            .entry(category)
            .or_default() += 1;
    }

    pub fn merge(&mut self, other: &Tally) {
        self.source_sets += other.source_sets;
        self.source_set_crashes += other.source_set_crashes;
        self.jvm_failures += other.jvm_failures;
        self.cached += other.cached;
        self.files += other.files;
        self.generated_files += other.generated_files;
        let (e, o) = (&mut self.excluded, &other.excluded);
        e.javac_errors += o.javac_errors;
        e.crashes += o.crashes;
        e.module_declarations += o.module_declarations;
        e.unanalyzed += o.unanalyzed;
        e.ferrion_failures += o.ferrion_failures;
        e.stale_generated += o.stale_generated;
        self.compared_files += other.compared_files;
        self.lombok_types += other.lombok_types;
        self.expressions += other.expressions;
        self.constants += other.constants;
        self.calls += other.calls;
        self.implicit_calls += other.implicit_calls;
        self.variables += other.variables;
        merge_counts(&mut self.expression_kinds, &other.expression_kinds);
        merge_counts(&mut self.finality, &other.finality);
        for (query, count) in &other.relations {
            *self.relations.entry(query).or_default() += count;
        }
        self.relation_errors += other.relation_errors;
        let (c, o) = (&mut self.comparison, &other.comparison);
        c.matched += o.matched;
        merge_counts(&mut c.matched_by, &o.matched_by);
        for (aspect, categories) in &o.mismatched {
            let ours = c.mismatched.entry(aspect).or_default();
            for (category, count) in categories {
                *ours.entry(category).or_default() += count;
            }
        }
        merge_counts(&mut c.known, &o.known);
        c.not_compared += o.not_compared;
        c.extra += o.extra;
        c.left += o.left;
        c.waiting += o.waiting;
        merge_counts(&mut c.apart, &o.apart);
        c.implicit_parameters += o.implicit_parameters;
        c.dropped_annotations += o.dropped_annotations;
        c.unresolved += o.unresolved;
        merge_counts(&mut self.error_codes, &other.error_codes);
        merge_counts(&mut self.switch_errors, &other.switch_errors);
        self.files_with_only_switch_errors += other.files_with_only_switch_errors;
        self.javac_seconds += other.javac_seconds;
        self.output_bytes += other.output_bytes;
        self.analyze_micros += other.analyze_micros;
        self.ferrion_seconds += other.ferrion_seconds;
        if other.relation_fuel > self.relation_fuel {
            self.relation_fuel = other.relation_fuel;
            self.relation_fuel_query = other.relation_fuel_query.clone();
        }
        self.relation_arena = self.relation_arena.max(other.relation_arena);
    }

    pub fn mismatches(&self) -> u64 {
        self.comparison
            .mismatched
            .values()
            .flat_map(BTreeMap::values)
            .sum()
    }

    pub fn failures(&self) -> u64 {
        self.mismatches()
            + self.comparison.extra
            + self.jvm_failures
            + self.excluded.ferrion_failures
    }

    pub fn relation_queries(&self) -> u64 {
        self.relations.values().sum()
    }
}

fn merge_counts<K: Clone + Ord>(ours: &mut BTreeMap<K, u64>, theirs: &BTreeMap<K, u64>) {
    for (key, count) in theirs {
        *ours.entry(key.clone()).or_default() += count;
    }
}

/// One source set's run.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRow {
    pub source_set: String,
    pub files: u64,
    pub error_files: u64,
    pub cached: bool,
    pub seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// A mismatch, for the report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MismatchRow {
    pub file: String,
    pub aspect: &'static str,
    pub start: i64,
    pub end: i64,
    pub kind: String,
    pub category: &'static str,
    pub javac: String,
    pub ferrion: Option<String>,
}

/// A Ferrion entry where javac has none, for the report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraRow {
    pub file: String,
    pub aspect: &'static str,
    pub start: i64,
    pub end: i64,
    pub ferrion: String,
}

/// javac's analysis time of one compared file.
#[derive(Clone, Debug, Serialize)]
pub struct FileTime {
    pub file: String,
    pub micros: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub projects: BTreeMap<String, Tally>,
    pub total: Tally,
    pub source_sets: Vec<SetRow>,
    pub mismatches: Vec<MismatchRow>,
    pub extras: Vec<ExtraRow>,
    /// Files processors generated whose text isn't what the cached run of javac saw.
    pub stale_generated: Vec<String>,
    pub crashes: Vec<(String, String)>,
    pub errors: Vec<(String, String)>,
    /// Every compared file's analysis time, slowest first.
    pub times: Vec<FileTime>,
    pub wall_seconds: f64,
}

/// The value at quantile `q` of `sorted`, which is in ascending order.
pub fn quantile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

impl Report {
    pub fn text(&self, limit: usize, slowest: usize) -> String {
        let mut out = String::new();
        writeln!(
            out,
            "{:<16} {:>5} {:>7} {:>8} {:>7} {:>10} {:>9} {:>9} {:>9} {:>9} {:>8} {:>9}",
            "project",
            "sets",
            "files",
            "compared",
            "errors",
            "exprs",
            "calls",
            "consts",
            "vars",
            "relations",
            "javac s",
            "output MB"
        )
        .unwrap();
        let row = |out: &mut String, name: &str, t: &Tally| {
            writeln!(
                out,
                "{:<16} {:>5} {:>7} {:>8} {:>7} {:>10} {:>9} {:>9} {:>9} {:>9} {:>8.1} {:>9.1}",
                name,
                t.source_sets,
                t.files,
                t.compared_files,
                t.excluded.javac_errors,
                t.expressions,
                t.calls,
                t.constants,
                t.variables,
                t.relation_queries(),
                t.javac_seconds,
                t.output_bytes as f64 / 1e6,
            )
            .unwrap();
        };
        for (name, tally) in &self.projects {
            row(&mut out, name, tally);
        }
        if self.projects.len() > 1 {
            row(&mut out, "total", &self.total);
        }
        let t = &self.total;
        writeln!(
            out,
            "\nfiles: {} of the source sets and {} generated; left out: {} with javac errors \
             ({} with switch errors only), {} javac never analyzed, {} javac crashed on, {} \
             module declarations, {} in source sets Ferrion failed on, {} generated with other \
             text than javac's cached run saw; {} Lombok types left out",
            t.files - t.generated_files,
            t.generated_files,
            t.excluded.javac_errors,
            t.files_with_only_switch_errors,
            t.excluded.unanalyzed,
            t.excluded.crashes,
            t.excluded.module_declarations,
            t.excluded.ferrion_failures,
            t.excluded.stale_generated,
            t.lombok_types
        )
        .unwrap();
        writeln!(
            out,
            "calls: {} ({} implicit); variables by finality: {}",
            t.calls,
            t.implicit_calls,
            t.finality
                .iter()
                .map(|(f, n)| format!("{f} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        let mut kinds: Vec<(&String, &u64)> = t.expression_kinds.iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        writeln!(
            out,
            "expressions by kind: {}",
            kinds
                .iter()
                .take(12)
                .map(|(k, n)| format!("{k} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        writeln!(
            out,
            "relation queries ({} source sets lost theirs to javac throwing): {}",
            t.relation_errors,
            t.relations
                .iter()
                .map(|(q, n)| format!("{q} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        if !t.switch_errors.is_empty() {
            writeln!(out, "switch errors, by code:").unwrap();
            for (code, count) in &t.switch_errors {
                writeln!(out, "  {count:>7} {code}").unwrap();
            }
        }

        let mut times: Vec<u64> = self.times.iter().map(|t| t.micros).collect();
        times.sort_unstable();
        writeln!(
            out,
            "\njavac's analysis time per compared file: median {:.1} ms, p90 {:.1} ms, p99 {:.1} \
             ms, p99.9 {:.1} ms, max {:.1} ms; {:.1} s in all",
            quantile(&times, 0.5) as f64 / 1e3,
            quantile(&times, 0.9) as f64 / 1e3,
            quantile(&times, 0.99) as f64 / 1e3,
            quantile(&times, 0.999) as f64 / 1e3,
            times.last().copied().unwrap_or(0) as f64 / 1e3,
            t.analyze_micros as f64 / 1e6,
        )
        .unwrap();
        for time in self.times.iter().take(slowest) {
            writeln!(out, "  {:>9.1} ms {}", time.micros as f64 / 1e3, time.file).unwrap();
        }

        let c = &t.comparison;
        writeln!(
            out,
            "\ncomparison: {} matched, {} mismatched, {} known, {} not compared (no Ferrion \
             results), {} relation queries about types javac couldn't resolve, {} parameters of \
             implicit constructors; on Ferrion's side, {} extra, {} in annotations on record \
             components javac dropped, {} left to a later milestone, and {} waiting for overload \
             resolution",
            c.matched,
            t.mismatches(),
            c.known.values().sum::<u64>(),
            c.not_compared,
            c.unresolved,
            c.implicit_parameters,
            c.extra,
            c.dropped_annotations,
            c.left,
            c.waiting
        )
        .unwrap();
        for (aspect, count) in &c.matched_by {
            writeln!(out, "  matched, {aspect}: {count}").unwrap();
        }
        for (label, count) in &c.apart {
            writeln!(out, "  apart, {label}: {count}").unwrap();
        }
        for (aspect, categories) in &c.mismatched {
            for (category, count) in categories {
                writeln!(out, "  {aspect} {category}: {count}").unwrap();
            }
        }
        for (category, count) in &c.known {
            writeln!(out, "  known, {category}: {count}").unwrap();
        }
        if t.ferrion_seconds > 0.0 {
            writeln!(
                out,
                "Ferrion: {:.1} s; the most fuel a relation query used: {} ({}); the largest \
                 relations arena: {} types",
                t.ferrion_seconds, t.relation_fuel, t.relation_fuel_query, t.relation_arena
            )
            .unwrap();
        }

        if !t.error_codes.is_empty() {
            writeln!(out, "\nfiles left out for javac errors, by error code:").unwrap();
            let mut codes: Vec<(&String, &u64)> = t.error_codes.iter().collect();
            codes.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (code, count) in codes.iter().take(limit.max(10)) {
                writeln!(out, "  {count:>7} {code}").unwrap();
            }
        }
        for set in self
            .source_sets
            .iter()
            .filter(|s| s.failure.is_some())
            .take(limit)
        {
            writeln!(
                out,
                "\nsource set failed: {}: {}",
                set.source_set,
                set.failure.as_deref().unwrap_or("")
            )
            .unwrap();
        }
        for (file, crash) in self.crashes.iter().take(limit) {
            writeln!(out, "\ncrash {file}: {crash}").unwrap();
        }
        for mismatch in self.mismatches.iter().take(limit) {
            writeln!(
                out,
                "\nFAIL {} {} {}-{} ({}, {}): javac {}, Ferrion {}",
                mismatch.file,
                mismatch.aspect,
                mismatch.start,
                mismatch.end,
                mismatch.kind,
                mismatch.category,
                mismatch.javac,
                mismatch.ferrion.as_deref().unwrap_or("nothing")
            )
            .unwrap();
        }
        for file in self.stale_generated.iter().take(limit) {
            writeln!(
                out,
                "\nSTALE {file}: not the text javac's cached run saw (docs/design/\
                 types-oracle-normalization.md#generated-text-that-changes)"
            )
            .unwrap();
        }
        for extra in self.extras.iter().take(limit) {
            writeln!(
                out,
                "\nEXTRA {} {} {}-{}: Ferrion {}",
                extra.file, extra.aspect, extra.start, extra.end, extra.ferrion
            )
            .unwrap();
        }
        if self.projects.len() > 1 && (c.matched_by.len() > 1 || !c.apart.is_empty()) {
            out.push_str(&self.comparison_rows());
        }
        writeln!(
            out,
            "\n{} source sets ({} cached) in {:.1}s",
            self.source_sets.len(),
            t.cached,
            self.wall_seconds
        )
        .unwrap();
        out
    }
}

impl Report {
    /// A row per project of what was compared, for the design's results.
    fn comparison_rows(&self) -> String {
        let mut out = String::from(
            "\n| Project | Matched | Expressions | Constants | Calls | Variables | Waiting | Left \
             | Implicit parameters | Known | Mismatched | Extra |\n| --- | --- | --- | --- | --- \
             | --- | --- | --- | --- | --- | --- | --- |\n",
        );
        let mut row = |name: &str, t: &Tally| {
            let c = &t.comparison;
            let aspect = |a: &str| c.matched_by.get(a).copied().unwrap_or(0);
            writeln!(
                out,
                "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                c.matched,
                aspect("expression"),
                aspect("constant"),
                aspect("call"),
                aspect("variable"),
                c.waiting,
                c.left,
                c.implicit_parameters,
                c.known.values().sum::<u64>(),
                t.mismatches(),
                c.extra,
            )
            .unwrap();
        };
        for (name, tally) in &self.projects {
            row(name, tally);
        }
        row("total", &self.total);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles() {
        let sorted = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(quantile(&sorted, 0.5), 6);
        assert_eq!(quantile(&sorted, 0.0), 1);
        assert_eq!(quantile(&sorted, 1.0), 10);
        assert_eq!(quantile(&[], 0.5), 0);
    }

    #[test]
    fn merging_and_failures() {
        let mut a = Tally::default();
        a.add_mismatch("expression", "type");
        let mut b = Tally {
            jvm_failures: 1,
            ..Tally::default()
        };
        b.add_mismatch("expression", "type");
        b.add_mismatch("call", "method");
        a.merge(&b);
        assert_eq!(a.mismatches(), 3);
        assert_eq!(a.failures(), 4);
        assert_eq!(a.comparison.mismatched["expression"]["type"], 2);
        a.comparison.extra = 2;
        assert_eq!(a.failures(), 6);
    }
}
