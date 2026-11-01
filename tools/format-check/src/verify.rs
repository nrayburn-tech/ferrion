//! `format-check verify`: formats every file at each release it's meant for, and checks the four
//! safety properties of SCOPE.md on the output with the formatter's `verify`: idempotence,
//! equivalence, comments kept, and no new syntax errors (see "Safety properties" in
//! docs/design/formatter.md). Files the formatter skips, for syntax or version errors, are counted;
//! skipping isn't a failure. A panic, or a property that doesn't hold, is.
//!
//! Throughput counts only the time spent in `format`, which parses and formats: megabytes (10^6
//! bytes) and files per second, on one thread, summed over every release a file is formatted at.
//! `verify`'s time is reported separately, and the process's peak resident set at the end.

use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use corpus::sources::Source;
use ferrion_diagnostics::SourceText;

use crate::formatter::{self, FormatOptions, Formatted};
use crate::{Options, on_small_stack};

#[derive(Default)]
struct Stats {
    files: usize,
    formats: usize,
    /// Bytes formatted, counting a file once per release.
    bytes: u64,
    format_time: Duration,
    verify_time: Duration,
    changed: usize,
    /// Formats the formatter skipped, for syntax or version errors.
    skipped: usize,
    /// Files that aren't UTF-8 or ASCII, which Ferrion skips before parsing.
    not_utf8: usize,
    failures: Vec<String>,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.files += other.files;
        self.formats += other.formats;
        self.bytes += other.bytes;
        self.format_time += other.format_time;
        self.verify_time += other.verify_time;
        self.changed += other.changed;
        self.skipped += other.skipped;
        self.not_utf8 += other.not_utf8;
    }

    fn print(&self, name: &str) {
        let seconds = self.format_time.as_secs_f64();
        let rate = |n: f64| {
            if seconds > 0.0 { n / seconds } else { 0.0 }
        };
        println!(
            "{name}: {} files, {} formats, {:.1} MB in {seconds:.2} s of formatting: {:.1} MB/s, \
             {:.0} files/s; verified in {:.2} s; {} changed, {} skipped (syntax or version \
             errors), {} skipped (not UTF-8), {} failed",
            self.files,
            self.formats,
            self.bytes as f64 / 1e6,
            rate(self.bytes as f64 / 1e6),
            rate(self.formats as f64),
            self.verify_time.as_secs_f64(),
            self.changed,
            self.skipped,
            self.not_utf8,
            self.failures.len(),
        );
    }
}

/// Checks each target's files, prints a line per target, a total, and the peak memory, and returns
/// whether every file passed.
pub fn run(targets: Vec<(String, Vec<Source>)>, options: &Options) -> bool {
    if formatter::STAND_IN {
        println!("the formatter isn't wired in yet: every file is returned unchanged");
    }
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
    if let Some(bytes) = peak_rss() {
        println!("peak RSS: {:.1} MB", bytes as f64 / 1e6);
    }
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
        let Ok(text) = SourceText::decode(bytes) else {
            stats.not_utf8 += 1;
            continue;
        };
        stats.files += 1;
        for &release in &source.releases {
            if verbose {
                eprintln!("{} at {release}", source.display);
            }
            let options = FormatOptions { release };
            let start = Instant::now();
            let formatted =
                panic::catch_unwind(AssertUnwindSafe(|| formatter::format(&text, &options)));
            stats.format_time += start.elapsed();
            stats.formats += 1;
            stats.bytes += text.text().len() as u64;
            let failure = match formatted {
                Err(_) => Some("format panicked".to_owned()),
                Ok(Formatted::Skipped(_)) => {
                    stats.skipped += 1;
                    None
                }
                Ok(formatted) => {
                    let output = match formatted {
                        Formatted::Changed(output) => {
                            stats.changed += 1;
                            output
                        }
                        _ => text.to_bytes(),
                    };
                    let start = Instant::now();
                    let verified = panic::catch_unwind(AssertUnwindSafe(|| {
                        formatter::verify(&text, &output, &options)
                    }));
                    stats.verify_time += start.elapsed();
                    match verified {
                        Ok(Ok(())) => None,
                        Ok(Err(violation)) => Some(violation.to_string()),
                        Err(_) => Some("verify panicked".to_owned()),
                    }
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

/// The process's peak resident set, in bytes.
#[cfg(unix)]
fn peak_rss() -> Option<u64> {
    use nix::sys::resource::{UsageWho, getrusage};

    let usage = getrusage(UsageWho::RUSAGE_SELF).ok()?;
    // macOS reports bytes, Linux and the BSDs kilobytes.
    let unit = if cfg!(target_vendor = "apple") {
        1
    } else {
        1024
    };
    Some(u64::try_from(usage.max_rss()).ok()? * unit)
}

/// Peak memory isn't measured on Windows yet.
#[cfg(not(unix))]
fn peak_rss() -> Option<u64> {
    None
}
