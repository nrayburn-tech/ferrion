//! `format-check gjf`: compares Ferrion's output with Google Java Format's, file by file, on the
//! same input (see "Google Java Format comparison" in docs/design/formatter.md).
//!
//! Each file is formatted by Ferrion at the newest release it's meant for, and by GJF to a fixed
//! point (see [`crate::gjf`]). Files GJF rejects, files with `@formatter:off` or `@formatter:on`
//! (which GJF doesn't recognize), files whose GJF output doesn't settle, and files Ferrion skips
//! are left out, and counted. For the rest, the measure is the number of lines that differ: a line
//! diff's insertions plus deletions (see [`crate::diff`]).
//!
//! The totals per project are tracked in `gjf-diff.toml`. `--check` fails if a project's
//! differing lines are above the committed number, and `--update` writes the new numbers.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use corpus::sources::Source;
use ferrion_diagnostics::SourceText;
use serde::{Deserialize, Serialize};

use crate::diff::Diff;
use crate::gjf::{Gjf, Input, Outcome};
use crate::{Options, ferrion_output, percent};

/// Lines of context around a sample hunk.
const CONTEXT: usize = 3;

/// The texts that mark a formatter disable region, which GJF doesn't recognize.
const MARKERS: [&str; 2] = ["@formatter:off", "@formatter:on"];

#[derive(Default)]
struct Stats {
    compared: usize,
    identical: usize,
    /// Insertions plus deletions, over the files compared.
    lines: usize,
    rejected: usize,
    markers: usize,
    unsettled: usize,
    /// Files Ferrion's formatter skips, for syntax or version errors.
    skipped: usize,
    /// Files that aren't UTF-8 or ASCII, which Ferrion skips before parsing.
    not_utf8: usize,
    /// Files GJF changed again on its second run, so that they settled only on the third.
    slow_to_settle: usize,
    ferrion_time: Duration,
    gjf_time: Duration,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.compared += other.compared;
        self.identical += other.identical;
        self.lines += other.lines;
        self.rejected += other.rejected;
        self.markers += other.markers;
        self.unsettled += other.unsettled;
        self.skipped += other.skipped;
        self.not_utf8 += other.not_utf8;
        self.slow_to_settle += other.slow_to_settle;
        self.ferrion_time += other.ferrion_time;
        self.gjf_time += other.gjf_time;
    }

    fn print(&self, name: &str, tracked: Option<&Entry>) {
        let tracked = match tracked {
            Some(entry) => format!(" (committed: {})", entry.differing_lines),
            None => String::new(),
        };
        println!(
            "{name}: {} files compared, {} identical ({}), {} lines differ{tracked}; left out: {} \
             rejected by GJF, {} with formatter markers, {} that didn't settle, {} skipped by \
             Ferrion, {} not UTF-8; {} needed a second GJF pass; Ferrion in {:.2} s, GJF \
             in {:.1} s",
            self.compared,
            self.identical,
            percent(self.identical, self.compared),
            self.lines,
            self.rejected,
            self.markers,
            self.unsettled,
            self.skipped,
            self.not_utf8,
            self.slow_to_settle,
            self.ferrion_time.as_secs_f64(),
            self.gjf_time.as_secs_f64(),
        );
    }
}

/// A file whose two outputs differ.
struct Differing {
    display: String,
    lines: usize,
    /// The diff's first hunk, from Ferrion's output (`-`) to GJF's (`+`).
    first_hunk: String,
}

/// One project's numbers in `gjf-diff.toml`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Entry {
    pub compared: usize,
    pub identical: usize,
    pub differing_lines: usize,
}

/// Compares every target's files, prints a line per target, a total, the largest differences, and
/// a sample of hunks, and returns whether the run passed: always, unless `--check` finds a target
/// above its committed number.
pub fn run(targets: Vec<(String, Vec<Source>)>, options: &Options) -> Result<bool> {
    let dirs = corpus::Dirs::new()?;
    let gjf = Gjf::new(&dirs, options.jobs, options.batch)?;
    let mut tracking = Tracking::read(&options.tracking)?;
    let mut total = Stats::default();
    let mut found = Found::default();
    let mut results = Vec::new();
    println!(
        "Ferrion's {} compared with GJF {}",
        if crate::formatter::STAND_IN {
            "input, unchanged (the formatter isn't wired in yet),"
        } else {
            "output"
        },
        gjf.version
    );
    for (name, sources) in targets {
        let mut stats = Stats::default();
        compare(&gjf, &sources, &mut stats, &mut found)?;
        stats.print(&name, tracking.get(&name));
        total.add(&stats);
        results.push((
            name,
            Entry {
                compared: stats.compared,
                identical: stats.identical,
                differing_lines: stats.lines,
            },
        ));
    }
    total.print("total", None);

    let Found {
        mut differing,
        rejected,
        unsettled,
        slow,
    } = found;
    differing.sort_by(|a, b| b.lines.cmp(&a.lines).then(a.display.cmp(&b.display)));
    if !differing.is_empty() && options.largest > 0 {
        println!("\nlargest differences:");
        for file in differing.iter().take(options.largest) {
            println!("  {:>6} lines  {}", file.lines, file.display);
        }
    }
    // An even spread over the differing files, in path order, so the sample isn't all from one
    // project or one kind of file.
    differing.sort_by(|a, b| a.display.cmp(&b.display));
    let sample = options.hunks.min(differing.len());
    if sample > 0 {
        println!("\nsample hunks, from Ferrion's output (-) to GJF's (+):");
        for k in 0..sample {
            let file = &differing[k * differing.len() / sample];
            println!("\n{} ({} lines differ)", file.display, file.lines);
            print!("{}", file.first_hunk);
        }
    }
    for (title, list) in [
        ("rejected by GJF", &rejected),
        ("didn't settle", &unsettled),
        ("needed a second GJF pass", &slow),
    ] {
        if !list.is_empty() {
            println!("\n{title}:");
            for line in list.iter().take(options.limit) {
                println!("  {line}");
            }
            if list.len() > options.limit {
                println!("  ... and {} more", list.len() - options.limit);
            }
        }
    }

    let mut passed = true;
    if options.check {
        println!();
        for (name, entry) in &results {
            match tracking.get(name) {
                None => println!("{name}: not in {}", options.tracking.display()),
                Some(committed) if entry.differing_lines > committed.differing_lines => {
                    println!(
                        "{name}: {} lines differ, more than the {} committed",
                        entry.differing_lines, committed.differing_lines
                    );
                    passed = false;
                }
                Some(committed) if entry.differing_lines < committed.differing_lines => println!(
                    "{name}: {} lines differ, fewer than the {} committed; record them with \
                     `--update`",
                    entry.differing_lines, committed.differing_lines
                ),
                Some(_) => {}
            }
        }
    }
    if options.update {
        for (name, entry) in results {
            tracking.set(name, entry);
        }
        tracking.write(&options.tracking)?;
        println!("\nupdated {}", options.tracking.display());
    }
    Ok(passed)
}

/// The files to list after the totals.
#[derive(Default)]
struct Found {
    differing: Vec<Differing>,
    /// Each with GJF's first error, or why it didn't settle.
    rejected: Vec<String>,
    unsettled: Vec<String>,
    slow: Vec<String>,
}

/// Compares one target's files, adding to `stats` and `found`.
fn compare(gjf: &Gjf, sources: &[Source], stats: &mut Stats, found: &mut Found) -> Result<()> {
    // The files both tools format: each one's source, input, and Ferrion's output.
    let mut files: Vec<(&Source, Vec<u8>, Vec<u8>)> = Vec::new();
    for source in sources {
        let bytes = std::fs::read(&source.path)
            .with_context(|| format!("reading {}", source.path.display()))?;
        let Ok(text) = SourceText::decode(bytes.clone()) else {
            stats.not_utf8 += 1;
            continue;
        };
        if MARKERS.iter().any(|marker| text.text().contains(marker)) {
            stats.markers += 1;
            continue;
        }
        let release = *source.releases.last().unwrap();
        let start = Instant::now();
        let output = ferrion_output(&text, release);
        stats.ferrion_time += start.elapsed();
        match output {
            Some(output) => files.push((source, bytes, output)),
            None => stats.skipped += 1,
        }
    }

    let inputs: Vec<Input> = files
        .iter()
        .map(|(source, bytes, _)| Input {
            name: file_name(&source.path),
            bytes,
        })
        .collect();
    let start = Instant::now();
    let outcomes = gjf.fixed_points(&inputs)?;
    stats.gjf_time += start.elapsed();

    for ((source, _, ferrion), outcome) in files.iter().zip(outcomes) {
        match outcome {
            Outcome::Rejected(error) => {
                stats.rejected += 1;
                found.rejected.push(format!("{}: {error}", source.display));
            }
            Outcome::Unsettled(reason) => {
                stats.unsettled += 1;
                found
                    .unsettled
                    .push(format!("{}: {reason}", source.display));
            }
            Outcome::Settled { output, runs } => {
                stats.compared += 1;
                // The last run is the one that changed nothing, so a file GJF formats in one pass
                // settles in two runs, or in one if it was already formatted.
                if runs > 2 {
                    stats.slow_to_settle += 1;
                    found.slow.push(source.display.clone());
                }
                if *ferrion == output {
                    stats.identical += 1;
                    continue;
                }
                let diff = Diff::new(ferrion, &output);
                let lines = diff.changed();
                stats.lines += lines;
                let first_hunk = diff
                    .hunks(CONTEXT)
                    .first()
                    .map(|hunk| diff.show(hunk))
                    .unwrap_or_default();
                found.differing.push(Differing {
                    display: source.display.clone(),
                    lines,
                    first_hunk,
                });
            }
        }
    }
    Ok(())
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Input.java")
}

/// `gjf-diff.toml`: the comments at its top, then each project's numbers, in the file's order.
struct Tracking {
    header: String,
    entries: Vec<(String, Entry)>,
}

impl Tracking {
    fn read(path: &Path) -> Result<Tracking> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        // The header is everything before the first table.
        let header_end = text
            .lines()
            .take_while(|line| !line.starts_with('['))
            .map(|line| line.len() + 1)
            .sum::<usize>()
            .min(text.len());
        let header = text[..header_end].trim_end().to_owned();
        let table: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let mut entries = Vec::new();
        // The table's own order is alphabetical, so the file's is taken from its headers.
        for line in text.lines() {
            let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            let name = name.trim().trim_matches('"');
            let value = table
                .get(name)
                .with_context(|| format!("{}: no table `{name}`", path.display()))?;
            let entry: Entry = value
                .clone()
                .try_into()
                .with_context(|| format!("{}: in `{name}`", path.display()))?;
            entries.push((name.to_owned(), entry));
        }
        if entries.len() != table.len() {
            bail!(
                "{}: every project must be a table of its own",
                path.display()
            );
        }
        Ok(Tracking { header, entries })
    }

    fn get(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, entry)| entry)
    }

    fn set(&mut self, name: String, entry: Entry) {
        match self.entries.iter_mut().find(|(n, _)| *n == name) {
            Some((_, existing)) => *existing = entry,
            None => self.entries.push((name, entry)),
        }
    }

    fn text(&self) -> String {
        let mut text = self.header.clone();
        for (name, entry) in &self.entries {
            text.push_str(&format!(
                "\n\n[{name}]\ncompared = {}\nidentical = {}\ndiffering-lines = {}",
                entry.compared, entry.identical, entry.differing_lines
            ));
        }
        text.push('\n');
        text
    }

    fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.text()).with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACKING: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/gjf-diff.toml");

    #[test]
    fn the_tracking_file_round_trips() {
        let tracking = Tracking::read(Path::new(TRACKING)).unwrap();
        assert!(tracking.header.starts_with('#'));
        assert!(!tracking.entries.is_empty());
        assert_eq!(
            tracking.text(),
            std::fs::read_to_string(TRACKING).unwrap(),
            "gjf-diff.toml isn't in the form `--update` writes"
        );
    }

    #[test]
    fn tracking_updates_keep_the_order_and_the_header() {
        let dir =
            std::env::temp_dir().join(format!("format-check-tracking-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gjf-diff.toml");
        std::fs::write(
            &path,
            "# A comment.\n\n[b]\ncompared = 2\nidentical = 1\ndiffering-lines = 9\n\n\
             [a]\ncompared = 1\nidentical = 1\ndiffering-lines = 0\n",
        )
        .unwrap();
        let mut tracking = Tracking::read(&path).unwrap();
        let entry = |differing_lines| Entry {
            compared: 2,
            identical: 1,
            differing_lines,
        };
        assert_eq!(tracking.get("b"), Some(&entry(9)));
        tracking.set("b".to_owned(), entry(5));
        tracking.set("c".to_owned(), entry(7));
        tracking.write(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# A comment.\n\n[b]\ncompared = 2\nidentical = 1\ndiffering-lines = 5\n\n\
             [a]\ncompared = 1\nidentical = 1\ndiffering-lines = 0\n\n\
             [c]\ncompared = 2\nidentical = 1\ndiffering-lines = 7\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
