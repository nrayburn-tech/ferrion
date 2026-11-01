//! Measuring one benchmark, in its own process, so that its peak RSS and CPU time are its own.
//!
//! Timing: one iteration to warm up, then iterations until the time asked for has passed (at
//! least [`MIN_ITERATIONS`]). Wall time and heap counts are taken per iteration, and the median
//! is reported; user and system time come from getrusage around all of them.
//!
//! Counting instructions: the process runs under callgrind, with collection off except inside
//! [`ferrion_bench_measured`], which runs one iteration after one to warm up. The count is exact
//! and nearly the same from run to run, so it's what regressions are judged by in CI.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::alloc;
use crate::benchmarks::Prepared;
use crate::results::BenchmarkResult;

pub const MIN_ITERATIONS: u64 = 5;
pub const MAX_ITERATIONS: u64 = 100_000;

/// The pattern callgrind's `--toggle-collect` matches [`ferrion_bench_measured`] by.
const TOGGLE: &str = "*ferrion_bench_measured*";

/// Runs one iteration. Collection is on only inside this function when counting instructions, so
/// its name must stay unique; it takes a trait object so there's a single copy of it, and does
/// something after the call so the call isn't made a tail call.
#[inline(never)]
pub fn ferrion_bench_measured(run: &mut dyn FnMut()) {
    run();
    black_box(());
}

/// Times `prepared` for about `seconds`.
pub fn time(name: &str, prepared: Prepared, seconds: f64) -> BenchmarkResult {
    let Prepared {
        input,
        bytes,
        mut run,
    } = prepared;
    let start = Instant::now();
    ferrion_bench_measured(&mut run);
    let first = start.elapsed().as_secs_f64().max(1e-9);
    let iterations = ((seconds / first) as u64).clamp(MIN_ITERATIONS, MAX_ITERATIONS);

    let mut wall = Vec::with_capacity(iterations as usize);
    let mut allocations = Vec::with_capacity(iterations as usize);
    let mut allocated = Vec::with_capacity(iterations as usize);
    let mut peak = Vec::with_capacity(iterations as usize);
    let usage_before = usage();
    for _ in 0..iterations {
        alloc::reset_peak();
        let before = alloc::snapshot();
        let start = Instant::now();
        ferrion_bench_measured(&mut run);
        wall.push(start.elapsed().as_nanos() as f64);
        let after = alloc::snapshot();
        allocations.push((after.count - before.count) as f64);
        allocated.push((after.allocated - before.allocated) as f64);
        peak.push(after.peak.saturating_sub(before.live) as f64);
    }
    let usage_after = usage();

    let mut metrics = BTreeMap::new();
    metrics.insert("wall_ns".to_owned(), median(&mut wall));
    metrics.insert("wall_min_ns".to_owned(), wall[0]);
    metrics.insert("allocations".to_owned(), median(&mut allocations));
    metrics.insert("heap_allocated_bytes".to_owned(), median(&mut allocated));
    metrics.insert("heap_peak_bytes".to_owned(), median(&mut peak));
    if let (Some(before), Some(after)) = (usage_before, usage_after) {
        let per_iteration =
            |a: Duration, b: Duration| (b - a).as_nanos() as f64 / iterations as f64;
        metrics.insert("user_ns".to_owned(), per_iteration(before.user, after.user));
        metrics.insert(
            "system_ns".to_owned(),
            per_iteration(before.system, after.system),
        );
        metrics.insert("peak_rss_bytes".to_owned(), after.max_rss as f64);
    }
    BenchmarkResult {
        name: name.to_owned(),
        input,
        bytes,
        iterations,
        metrics,
    }
}

/// Runs `prepared` once to warm up, then once inside [`ferrion_bench_measured`], for callgrind.
pub fn once(prepared: Prepared) {
    let mut run = prepared.run;
    run();
    ferrion_bench_measured(&mut run);
}

/// Sorts `values` and returns their median.
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

struct Usage {
    user: Duration,
    system: Duration,
    max_rss: u64,
}

#[cfg(unix)]
fn usage() -> Option<Usage> {
    use nix::sys::resource::{UsageWho, getrusage};
    use nix::sys::time::TimeValLike;

    let usage = getrusage(UsageWho::RUSAGE_SELF).ok()?;
    let duration = |time: nix::sys::time::TimeVal| {
        Duration::from_micros(u64::try_from(time.num_microseconds()).unwrap_or(0))
    };
    // macOS reports bytes, Linux and the BSDs kilobytes.
    let unit = if cfg!(target_vendor = "apple") {
        1
    } else {
        1024
    };
    Some(Usage {
        user: duration(usage.user_time()),
        system: duration(usage.system_time()),
        max_rss: u64::try_from(usage.max_rss()).unwrap_or(0) * unit,
    })
}

/// CPU time and peak RSS aren't measured on Windows yet.
#[cfg(not(unix))]
fn usage() -> Option<Usage> {
    None
}

/// Runs `command` (the benchmark's own process, in callgrind mode) under callgrind, writing its
/// profile to `out`, and returns the instructions counted.
pub fn callgrind(command: &Command, out: &Path) -> Result<f64> {
    let mut valgrind = Command::new("valgrind");
    valgrind
        .arg("--tool=callgrind")
        .arg("--collect-atstart=no")
        .arg(format!("--toggle-collect={TOGGLE}"))
        .arg(format!("--callgrind-out-file={}", out.display()))
        .arg(command.get_program())
        .args(command.get_args());
    let output = valgrind
        .output()
        .context("running valgrind; is it installed?")?;
    if !output.status.success() {
        bail!(
            "valgrind failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let profile =
        std::fs::read_to_string(out).with_context(|| format!("reading {}", out.display()))?;
    let total =
        total(&profile).with_context(|| format!("{} has no instruction total", out.display()))?;
    if total == 0 {
        bail!("callgrind counted nothing; did `{TOGGLE}` stop matching the measured function?");
    }
    Ok(total as f64)
}

/// The first event's total in a callgrind profile, which is the instruction count (`Ir`) when no
/// cache or branch simulation is asked for.
fn total(profile: &str) -> Option<u64> {
    profile
        .lines()
        .find_map(|line| {
            line.strip_prefix("summary:")
                .or_else(|| line.strip_prefix("totals:"))
        })?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_and_even_counts() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn total_reads_the_summary_or_totals_line() {
        let profile = "# callgrind format\nversion: 1\nevents: Ir\nsummary: 123456\n\nfn=(1) main\n\
                       0 12\n\ntotals: 123456\n";
        assert_eq!(total(profile), Some(123456));
        assert_eq!(total("events: Ir\ntotals: 42\n"), Some(42));
        assert_eq!(total("events: Ir\n"), None);
    }

    #[test]
    fn heap_counts_are_per_iteration() {
        let prepared = Prepared {
            input: "test".to_owned(),
            bytes: None,
            run: Box::new(|| {
                black_box(vec![1u8; 1 << 20]);
            }),
        };
        let result = time("test", prepared, 0.0);
        assert_eq!(result.iterations, MIN_ITERATIONS);
        // The counters are the process's, and other tests allocate and free on other threads at
        // the same time, so the counts are only roughly this iteration's.
        assert!(result.metrics["allocations"] >= 1.0);
        assert!(result.metrics["heap_allocated_bytes"] >= f64::from(1 << 20));
        assert!(result.metrics["heap_peak_bytes"] >= f64::from(1 << 19));
    }
}
