//! The results file `run --out` writes and `compare` reads, and how metrics are shown.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Results {
    pub format_version: u32,
    /// `git describe --always --dirty` of the checkout that ran, when it's a git checkout.
    pub commit: Option<String>,
    pub os: String,
    pub arch: String,
    pub benchmarks: Vec<BenchmarkResult>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BenchmarkResult {
    pub name: String,
    /// What one iteration processes, such as `jedis: 812 files, 8.7 MB`. Comparing results whose
    /// inputs differ is warned about.
    pub input: String,
    /// Bytes one iteration processes, for throughput, when that means something.
    pub bytes: Option<u64>,
    /// Iterations timed, after one to warm up.
    pub iterations: u64,
    /// By the names in [`METRICS`]. Values are per iteration, except the peak RSS.
    pub metrics: BTreeMap<String, f64>,
}

impl Results {
    pub fn new(benchmarks: Vec<BenchmarkResult>) -> Results {
        let commit = std::process::Command::new("git")
            .args(["describe", "--always", "--dirty"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
        Results {
            format_version: FORMAT_VERSION,
            commit,
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            benchmarks,
        }
    }

    pub fn read(path: &Path) -> Result<Results> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let results: Results =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if results.format_version != FORMAT_VERSION {
            bail!(
                "{}: unsupported format version {}",
                path.display(),
                results.format_version
            );
        }
        Ok(results)
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text + "\n").with_context(|| format!("writing {}", path.display()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Nanoseconds,
    Bytes,
    Count,
}

pub struct Metric {
    pub name: &'static str,
    pub label: &'static str,
    pub unit: Unit,
}

/// Every metric, in the order they're shown.
pub const METRICS: &[Metric] = &[
    // Counted by callgrind for one iteration, with `run --callgrind`.
    Metric {
        name: "instructions",
        label: "instructions",
        unit: Unit::Count,
    },
    // The median iteration.
    Metric {
        name: "wall_ns",
        label: "wall time",
        unit: Unit::Nanoseconds,
    },
    Metric {
        name: "wall_min_ns",
        label: "wall time (fastest)",
        unit: Unit::Nanoseconds,
    },
    // CPU time from getrusage over every timed iteration, divided by their number.
    Metric {
        name: "user_ns",
        label: "user time",
        unit: Unit::Nanoseconds,
    },
    Metric {
        name: "system_ns",
        label: "system time",
        unit: Unit::Nanoseconds,
    },
    // The process's peak resident set, including its inputs, from getrusage.
    Metric {
        name: "peak_rss_bytes",
        label: "peak RSS",
        unit: Unit::Bytes,
    },
    // The most heap one iteration held at once, beyond what was live before it.
    Metric {
        name: "heap_peak_bytes",
        label: "heap peak",
        unit: Unit::Bytes,
    },
    Metric {
        name: "heap_allocated_bytes",
        label: "heap allocated",
        unit: Unit::Bytes,
    },
    Metric {
        name: "allocations",
        label: "allocations",
        unit: Unit::Count,
    },
];

pub fn metric(name: &str) -> Option<&'static Metric> {
    METRICS.iter().find(|metric| metric.name == name)
}

/// `value` in `unit`, scaled to three significant digits or so: `81.2 ms`, `8.71 MB`, `1.23 G`.
/// Megabytes are 10^6 bytes, as elsewhere in the tools.
pub fn format(value: f64, unit: Unit) -> String {
    let scales: &[(f64, &str)] = match unit {
        Unit::Nanoseconds => &[(1e9, " s"), (1e6, " ms"), (1e3, " µs"), (1.0, " ns")],
        Unit::Bytes => &[(1e9, " GB"), (1e6, " MB"), (1e3, " kB"), (1.0, " B")],
        Unit::Count => &[(1e9, " G"), (1e6, " M"), (1e3, " k"), (1.0, "")],
    };
    let (scale, name) = scales
        .iter()
        .find(|(scale, _)| value.abs() >= *scale)
        .copied()
        .unwrap_or(scales[scales.len() - 1]);
    let scaled = value / scale;
    let digits = if (scale == 1.0 && unit != Unit::Nanoseconds) || scaled.abs() >= 100.0 {
        0
    } else if scaled.abs() >= 10.0 {
        1
    } else {
        2
    };
    format!("{scaled:.digits$}{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_a_scale() {
        assert_eq!(format(81_234_567.0, Unit::Nanoseconds), "81.2 ms");
        assert_eq!(format(1_500.0, Unit::Nanoseconds), "1.50 µs");
        assert_eq!(format(12.0, Unit::Nanoseconds), "12.0 ns");
        assert_eq!(format(8_712_345.0, Unit::Bytes), "8.71 MB");
        assert_eq!(format(512.0, Unit::Bytes), "512 B");
        assert_eq!(format(1_234_567_890.0, Unit::Count), "1.23 G");
        assert_eq!(format(7.0, Unit::Count), "7");
        assert_eq!(format(0.0, Unit::Count), "0");
    }
}
