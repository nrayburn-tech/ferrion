//! Counting a names run, per project and in total, and printing it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;

use super::compare::{Category, Part};

/// Counts for a project, or for every project.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub source_sets: u64,
    /// Source sets javac threw on as a whole, so their files were compiled one at a time.
    pub source_set_crashes: u64,
    /// Source sets the oracle's JVM gave no output for: failures.
    pub jvm_failures: u64,
    /// Source sets whose output came from the cache.
    pub cached: u64,
    /// Files of the source sets, and files annotation processors generated.
    pub files: u64,
    pub generated_files: u64,
    /// Files left out of the comparison, by cause.
    pub excluded: Excluded,
    /// Files compared: every file not left out.
    pub compared_files: u64,
    /// Types left out for Lombok.
    pub lombok_types: u64,
    /// The names in compared files, by part.
    pub names: BTreeMap<&'static str, u64>,
    /// The names in B2's part, by context and then the kind of symbol.
    pub b2: BTreeMap<String, BTreeMap<String, u64>>,
    pub comparison: Comparison,
    /// For files left out for javac's errors, how many have each error code.
    pub error_codes: BTreeMap<String, u64>,
    /// Annotation processors registered on the classpath.
    pub processors: BTreeSet<String>,
    /// The oracle's own time for the source sets it ran, and the time to read cached ones.
    pub javac_seconds: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Excluded {
    /// javac reported an error in the file.
    pub javac_errors: u64,
    /// Of those, generated files.
    pub generated_with_errors: u64,
    /// javac threw on the file, even on its own, or never parsed it.
    pub crashes: u64,
    /// `module-info.java`: the oracle compiles in the unnamed module.
    pub module_declarations: u64,
    /// javac stopped before analyzing the file, after an error elsewhere (a processor's).
    pub unanalyzed: u64,
    /// Ferrion's results for the source set failed.
    pub ferrion_failures: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub matched: u64,
    pub mismatched: BTreeMap<&'static str, u64>,
    /// Mismatches listed in `differences/names-known.txt`, by category.
    pub known: BTreeMap<String, u64>,
    /// Names in B2's part in source sets Ferrion has no results for yet.
    pub not_compared: u64,
    /// Ferrion's names where javac has none in B2's part.
    pub extra: u64,
}

impl Tally {
    pub fn add_part(&mut self, part: Part, context: &str, kind: &str) {
        *self.names.entry(part.name()).or_default() += 1;
        if part == Part::B2 {
            *self
                .b2
                .entry(context.to_owned())
                .or_default()
                .entry(kind.to_owned())
                .or_default() += 1;
        }
    }

    pub fn add_mismatch(&mut self, category: Category) {
        *self
            .comparison
            .mismatched
            .entry(category.name())
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
        e.generated_with_errors += o.generated_with_errors;
        e.crashes += o.crashes;
        e.module_declarations += o.module_declarations;
        e.unanalyzed += o.unanalyzed;
        e.ferrion_failures += o.ferrion_failures;
        self.compared_files += other.compared_files;
        self.lombok_types += other.lombok_types;
        for (part, count) in &other.names {
            *self.names.entry(part).or_default() += count;
        }
        for (context, kinds) in &other.b2 {
            let ours = self.b2.entry(context.clone()).or_default();
            for (kind, count) in kinds {
                *ours.entry(kind.clone()).or_default() += count;
            }
        }
        let (c, o) = (&mut self.comparison, &other.comparison);
        c.matched += o.matched;
        for (category, count) in &o.mismatched {
            *c.mismatched.entry(category).or_default() += count;
        }
        for (category, count) in &o.known {
            *c.known.entry(category.clone()).or_default() += count;
        }
        c.not_compared += o.not_compared;
        c.extra += o.extra;
        for (code, count) in &other.error_codes {
            *self.error_codes.entry(code.clone()).or_default() += count;
        }
        self.processors.extend(other.processors.iter().cloned());
        self.javac_seconds += other.javac_seconds;
    }

    pub fn mismatches(&self) -> u64 {
        self.comparison.mismatched.values().sum()
    }

    pub fn failures(&self) -> u64 {
        self.mismatches() + self.jvm_failures
    }

    fn b2_names(&self) -> u64 {
        self.names.get(Part::B2.name()).copied().unwrap_or(0)
    }
}

/// One source set's run.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRow {
    pub source_set: String,
    pub release: u32,
    pub files: u64,
    pub generated_files: u64,
    pub error_files: u64,
    pub cached: bool,
    /// The oracle's wall time, or the time to read its cached output.
    pub seconds: f64,
    pub processors: Vec<String>,
    /// The JVM gave no output, or javac threw on the whole source set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// Classpath entries the lock has that the oracle can't use.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<String>,
}

/// A mismatch, for the report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MismatchRow {
    pub file: String,
    pub start: i64,
    pub end: i64,
    pub context: String,
    pub category: &'static str,
    pub javac: String,
    pub ferrion: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub projects: BTreeMap<String, Tally>,
    pub total: Tally,
    pub source_sets: Vec<SetRow>,
    pub mismatches: Vec<MismatchRow>,
    /// Files javac threw on, with the exception.
    pub crashes: Vec<(String, String)>,
    /// Files left out for javac's errors, with the first.
    pub errors: Vec<(String, String)>,
    pub wall_seconds: f64,
}

impl Report {
    pub fn text(&self, limit: usize) -> String {
        let mut out = String::new();
        let header = format!(
            "{:<16} {:>5} {:>7} {:>6} {:>8} {:>7} {:>6} {:>6} {:>7} {:>9} {:>8} {:>9} {:>8}",
            "project",
            "sets",
            "files",
            "gen",
            "compared",
            "errors",
            "crash",
            "module",
            "lombok",
            "B2 names",
            "B3",
            "other",
            "javac s"
        );
        writeln!(out, "{header}").unwrap();
        let row = |out: &mut String, name: &str, t: &Tally| {
            let other: u64 = t
                .names
                .iter()
                .filter(|(part, _)| **part != Part::B2.name() && **part != Part::B3.name())
                .map(|(_, n)| n)
                .sum();
            writeln!(
                out,
                "{:<16} {:>5} {:>7} {:>6} {:>8} {:>7} {:>6} {:>6} {:>7} {:>9} {:>8} {:>9} {:>8.1}",
                name,
                t.source_sets,
                t.files,
                t.generated_files,
                t.compared_files,
                t.excluded.javac_errors,
                t.excluded.crashes,
                t.excluded.module_declarations,
                t.lombok_types,
                t.b2_names(),
                t.names.get(Part::B3.name()).copied().unwrap_or(0),
                other,
                t.javac_seconds,
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
             ({} of them generated), {} javac crashed on, {} module declarations, {} javac didn't \
             analyze; {} Lombok types left out",
            t.files - t.generated_files,
            t.generated_files,
            t.excluded.javac_errors,
            t.excluded.generated_with_errors,
            t.excluded.crashes,
            t.excluded.module_declarations,
            t.excluded.unanalyzed,
            t.lombok_types
        )
        .unwrap();
        let parts: Vec<String> = Part::ALL
            .iter()
            .map(|part| format!("{} {}", part.name(), t.names.get(part.name()).unwrap_or(&0)))
            .collect();
        writeln!(out, "names in compared files: {}", parts.join(", ")).unwrap();

        writeln!(out, "\nB2 names by context and kind of symbol:").unwrap();
        let kinds: BTreeSet<&String> = t.b2.values().flat_map(|k| k.keys()).collect();
        write!(out, "  {:<20}", "").unwrap();
        for kind in &kinds {
            write!(out, " {kind:>10}").unwrap();
        }
        writeln!(out).unwrap();
        for (context, counts) in &t.b2 {
            write!(out, "  {context:<20}").unwrap();
            for kind in &kinds {
                write!(out, " {:>10}", counts.get(*kind).unwrap_or(&0)).unwrap();
            }
            writeln!(out).unwrap();
        }

        let c = &t.comparison;
        writeln!(
            out,
            "\ncomparison: {} matched, {} mismatched, {} known, {} not compared (no Ferrion \
             results yet), {} extra on Ferrion's side",
            c.matched,
            t.mismatches(),
            c.known.values().sum::<u64>(),
            c.not_compared,
            c.extra
        )
        .unwrap();
        for (category, count) in &c.mismatched {
            writeln!(out, "  {category}: {count}").unwrap();
        }
        for (category, count) in &c.known {
            writeln!(out, "  known, {category}: {count}").unwrap();
        }

        if !t.error_codes.is_empty() {
            writeln!(out, "\nfiles left out for javac errors, by error code:").unwrap();
            let mut codes: Vec<(&String, &u64)> = t.error_codes.iter().collect();
            codes.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (code, count) in codes.iter().take(limit.max(10)) {
                writeln!(out, "  {count:>7} {code}").unwrap();
            }
        }
        if !t.processors.is_empty() {
            writeln!(out, "\nannotation processors found on classpaths:").unwrap();
            for processor in &t.processors {
                writeln!(out, "  {processor}").unwrap();
            }
        }
        let failed: Vec<&SetRow> = self
            .source_sets
            .iter()
            .filter(|s| s.failure.is_some())
            .collect();
        if !failed.is_empty() {
            writeln!(out, "\nsource sets javac failed on:").unwrap();
            for set in failed.iter().take(limit) {
                writeln!(
                    out,
                    "  {}: {}",
                    set.source_set,
                    set.failure.as_deref().unwrap_or("")
                )
                .unwrap();
            }
        }
        for (file, crash) in self.crashes.iter().take(limit) {
            writeln!(out, "\ncrash {file}: {crash}").unwrap();
        }
        if !self.errors.is_empty() {
            writeln!(out, "\nfirst errors of files left out:").unwrap();
            for (file, error) in self.errors.iter().take(limit) {
                writeln!(out, "  {file}: {error}").unwrap();
            }
        }
        for mismatch in self.mismatches.iter().take(limit) {
            writeln!(
                out,
                "\nFAIL {} {}-{} ({}, {}): javac {}, Ferrion {}",
                mismatch.file,
                mismatch.start,
                mismatch.end,
                mismatch.context,
                mismatch.category,
                mismatch.javac,
                mismatch.ferrion.as_deref().unwrap_or("nothing")
            )
            .unwrap();
        }
        let slowest = self
            .source_sets
            .iter()
            .filter(|s| !s.cached)
            .max_by(|a, b| a.seconds.total_cmp(&b.seconds));
        write!(
            out,
            "\n{} source sets ({} cached) in {:.1}s",
            self.source_sets.len(),
            t.cached,
            self.wall_seconds
        )
        .unwrap();
        if let Some(slowest) = slowest {
            write!(
                out,
                "; the slowest javac run: {} ({} files), {:.1}s",
                slowest.source_set, slowest.files, slowest.seconds
            )
            .unwrap();
        }
        writeln!(out).unwrap();
        out
    }
}
