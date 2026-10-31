//! `parser-check round-trip`: parses every file at each release it's meant for, and checks that
//! parsing doesn't panic, that the tree's tokens reproduce the file's bytes (after any byte-order
//! mark, which the tree doesn't hold), and that `ferrion_parser::invariants::check` holds.
//!
//! Throughput counts only the time spent in `parse`, so the checks don't skew it: megabytes
//! (10^6 bytes) and files parsed per second, on one thread, summed over every release a file is
//! parsed at. It also counts the parses with syntax errors and with `NestingTooDeep`, so later
//! rounds can see the grammar fill in. Neither fails the check: the oracle comparison decides
//! which errors are right.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use corpus::sources::Source;
use ferrion_diagnostics::SourceText;
use ferrion_parser::{Parse, ParseDiagnosticKind, ParseOptions, Severity, invariants, parse};

use crate::{Options, on_small_stack};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Default)]
struct Stats {
    files: usize,
    parses: usize,
    /// Bytes parsed, counting a file once per release.
    bytes: u64,
    parse_time: Duration,
    /// Files that aren't UTF-8 or ASCII, which Ferrion skips.
    skipped: usize,
    with_errors: usize,
    too_deep: usize,
    failures: Vec<String>,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.files += other.files;
        self.parses += other.parses;
        self.bytes += other.bytes;
        self.parse_time += other.parse_time;
        self.skipped += other.skipped;
        self.with_errors += other.with_errors;
        self.too_deep += other.too_deep;
    }

    fn print(&self, name: &str) {
        let seconds = self.parse_time.as_secs_f64();
        let rate = |n: f64| {
            if seconds > 0.0 { n / seconds } else { 0.0 }
        };
        println!(
            "{name}: {} files, {} parses, {:.1} MB in {seconds:.2} s of parsing: {:.1} MB/s, {:.0} \
             files/s; {} with syntax errors, {} too deeply nested, {} skipped (not UTF-8), {} \
             failed",
            self.files,
            self.parses,
            self.bytes as f64 / 1e6,
            rate(self.bytes as f64 / 1e6),
            rate(self.parses as f64),
            self.with_errors,
            self.too_deep,
            self.skipped,
            self.failures.len(),
        );
    }
}

/// Checks each target's files, prints a line per target and a total, and returns whether every
/// file passed.
pub fn run(targets: Vec<(String, Vec<Source>)>, options: &Options) -> bool {
    let verbose = options.verbose;
    let mut total = Stats::default();
    let mut failures = Vec::new();
    for (name, sources) in targets {
        let stats = on_small_stack(move || check_all(&sources, verbose));
        stats.print(&name);
        total.add(&stats);
        failures.extend(stats.failures);
    }
    total.print("total");
    for failure in failures.iter().take(options.limit) {
        println!("FAILED {failure}");
    }
    if failures.len() > options.limit {
        println!("... and {} more", failures.len() - options.limit);
    }
    failures.is_empty()
}

fn check_all(sources: &[Source], verbose: bool) -> Stats {
    let mut stats = Stats::default();
    for source in sources {
        let bytes = match std::fs::read(&source.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                stats.failures.push(format!("{}: {error}", source.display));
                continue;
            }
        };
        let decoded = match SourceText::decode(bytes.clone()) {
            Ok(decoded) => decoded,
            Err(_) => {
                stats.skipped += 1;
                continue;
            }
        };
        stats.files += 1;
        let text: Arc<str> = Arc::from(decoded.text());
        for &release in &source.releases {
            if verbose {
                eprintln!("{} at {release}", source.display);
            }
            let start = Instant::now();
            let parsed = panic::catch_unwind(|| parse(text.clone(), ParseOptions { release }));
            stats.parse_time += start.elapsed();
            stats.parses += 1;
            stats.bytes += text.len() as u64;
            let failure = match parsed {
                Err(_) => Some("parse panicked".to_owned()),
                Ok(parsed) => {
                    count(&mut stats, &parsed);
                    check(&bytes, &text, release, &parsed)
                }
            };
            if let Some(failure) = failure {
                stats
                    .failures
                    .push(format!("{} at {release}: {failure}", source.display));
            }
        }
    }
    stats
}

fn count(stats: &mut Stats, parsed: &Parse) {
    if parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        stats.with_errors += 1;
    }
    if parsed
        .diagnostics
        .iter()
        .any(|d| d.kind == ParseDiagnosticKind::NestingTooDeep)
    {
        stats.too_deep += 1;
    }
}

/// What's wrong with `parsed`, a parse of `bytes` decoded as `text`, if anything.
fn check(bytes: &[u8], text: &Arc<str>, release: u32, parsed: &Parse) -> Option<String> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
        let mut reproduced = Vec::with_capacity(body.len());
        for token in parsed.tree.tokens() {
            reproduced.extend_from_slice(token.text().as_bytes());
        }
        if reproduced != body {
            let at = reproduced
                .iter()
                .zip(body)
                .position(|(a, b)| a != b)
                .unwrap_or(reproduced.len().min(body.len()));
            return Err(format!(
                "the tree's tokens don't reproduce the file: they differ at byte {at}"
            ));
        }
        invariants::check(text, release, parsed)
    }));
    match result {
        Ok(Ok(())) => None,
        Ok(Err(problem)) => Some(problem),
        Err(_) => Some("the checks panicked".to_owned()),
    }
}
