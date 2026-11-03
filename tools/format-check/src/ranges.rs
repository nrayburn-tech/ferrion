//! `format-check ranges`: the range test (docs/design/formatter-completeness.md, "Testing").
//!
//! For each corpus file that parses without errors, at its first release, tokens are picked with
//! a seeded random generator, so a run is reproducible from the seed, the count per file, and the
//! corpus pins. Each makes a broken copy of the file, the kinds taking turns:
//!
//! - `delete`: the token deleted;
//! - `replace`: the token replaced by one from a small vocabulary (`goto`, which is never valid,
//!   a closing delimiter, `;`, `=`, or `.`), with spaces around it so that it never joins a
//!   neighbour.
//!
//! A copy that still parses without a syntax error is counted and left out. In each other copy,
//! every run of a unit's tokens (`format_range`'s units, as `Ranges::runs` lists them) that
//! doesn't touch the change is formatted through `format_range`, with a range covering its first
//! token, unless an error unit affects it. Its result is compared with the clean file's
//! `format_tree` output there: the units' text, the indentation before them, and, when the code
//! on either side is the clean file's too, the whole gap before them and the line breaks after.
//!
//! **Matching units.** Outside the change, the broken text is the clean text, shifted by the
//! change after it, so a broken run is matched with the clean run that has the same tokens at
//! the shifted offsets, the same kind of unit, and the same kinds of nodes around it, and whose
//! range expands to the same span, comments included. A run without such a match is one whose
//! structure recovery shifted (a lost `}` puts what follows in another block, for example): it
//! can't be compared, and the share of those is reported per category of the token broken.
//!
//! One unit a run is formatted in per broken copy, picked at random, is also checked with
//! `verify_range`. A difference in a compared unit, a `verify_range` violation, or a panic is a
//! failure; a difference in a copy whose imports aren't the clean file's, or whose import section
//! formatting keeps other imports of, is counted apart: the type-use annotations a file imports,
//! which an error in the imports or before them can lose, are the design's one known exception,
//! and the import section, one unit when it is rewritten, depends on the uses of its imports,
//! which the change can delete.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Instant;

use corpus::sources::Source;
use ferrion_diagnostics::SourceText;
use ferrion_formatter::invariants::{Expanded, Ranges, Run};
use ferrion_formatter::{Edit, FormatOptions, apply_edits, verify_range};
use ferrion_parser::syntax::SyntaxTree;
use ferrion_parser::{Parse, ParseOptions, Severity, SyntaxKind, parse};

use crate::{Options, on_small_stack, percent};

/// What `replace` puts in place of a token.
const VOCABULARY: [&str; 7] = ["goto", ")", "]", "}", ";", "=", "."];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Delete,
    Replace,
}

/// A change to a file's text: `start..end` of the clean text replaced by `inserted`.
#[derive(Clone, Debug)]
struct Mutation {
    kind: Kind,
    start: u32,
    end: u32,
    inserted: String,
}

impl Mutation {
    fn apply(&self, text: &str) -> String {
        let (start, end) = (self.start as usize, self.end as usize);
        [&text[..start], &self.inserted, &text[end..]].concat()
    }

    /// The change's span in the broken text.
    fn broken_span(&self) -> Range<u32> {
        self.start..self.start + self.inserted.len() as u32
    }

    /// Where the broken text's `offset`, outside the change, is in the clean text.
    fn to_clean(&self, offset: u32) -> Option<u32> {
        let changed = self.broken_span();
        if offset <= changed.start {
            Some(offset)
        } else if offset >= changed.end {
            Some(offset - changed.end + self.end)
        } else {
            None
        }
    }

    /// Whether `span` of the broken text touches the change, end points included.
    fn touches(&self, span: Range<u32>) -> bool {
        let changed = self.broken_span();
        span.start <= changed.end && changed.start <= span.end
    }
}

impl fmt::Display for Mutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Kind::Delete => write!(f, "delete at {}..{}", self.start, self.end),
            Kind::Replace => write!(
                f,
                "replace {}..{} with {:?}",
                self.start,
                self.end,
                self.inserted.trim()
            ),
        }
    }
}

/// SplitMix64, as in `parser-check mutate`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n`, which isn't zero.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What kind of token a change broke, for the report.
fn category(kind: SyntaxKind) -> &'static str {
    use SyntaxKind as K;
    match kind {
        K::LBrace | K::RBrace => "braces",
        K::LParen | K::RParen => "parentheses",
        K::LBracket | K::RBracket => "brackets",
        K::Semi => "semicolons",
        K::Comma => "commas",
        K::Dot => "dots",
        K::Identifier => "identifiers",
        _ => {
            let name = format!("{kind:?}");
            if name.ends_with("Kw") {
                "keywords"
            } else if name.contains("Literal") || kind == K::TextBlock {
                "literals"
            } else {
                "operators and other"
            }
        }
    }
}

/// Totals for one category.
#[derive(Clone, Default)]
struct Tally {
    mutations: usize,
    /// Broken copies that parse without a syntax error.
    unbroken: usize,
    /// Broken copies whose error affects the whole file.
    whole_file: usize,
    /// Runs that don't touch the change.
    eligible: usize,
    /// Runs an error unit affects.
    affected: usize,
    /// Runs without a clean match: recovery shifted the structure.
    shifted: usize,
    equal: usize,
    /// Runs that differ in a copy whose imports, or kept imports, aren't the clean file's.
    import_differences: usize,
    failures: usize,
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        self.mutations += other.mutations;
        self.unbroken += other.unbroken;
        self.whole_file += other.whole_file;
        self.eligible += other.eligible;
        self.affected += other.affected;
        self.shifted += other.shifted;
        self.equal += other.equal;
        self.import_differences += other.import_differences;
        self.failures += other.failures;
    }
}

#[derive(Default)]
struct Stats {
    files: usize,
    /// Files skipped: not UTF-8, or with errors in the clean parse.
    skipped: usize,
    tallies: BTreeMap<&'static str, Tally>,
    failures: Vec<String>,
}

/// Runs the test on each target's files, prints a table per target and in total, and returns
/// whether nothing failed.
pub fn run(targets: Vec<(String, Vec<Source>)>, options: &Options) -> bool {
    let started = Instant::now();
    let (seed, per_file, limit) = (options.seed, options.per_file, options.limit);
    let mut total = Stats::default();
    for (name, sources) in targets {
        let stats = on_small_stack(move || check_all(&sources, seed, per_file, limit));
        print(&name, &stats);
        total.files += stats.files;
        total.skipped += stats.skipped;
        for (category, tally) in &stats.tallies {
            total.tallies.entry(category).or_default().add(tally);
        }
        total.failures.extend(stats.failures);
    }
    print("total", &total);
    println!(
        "{per_file} tokens per file (seed {seed}) in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    for failure in total.failures.iter().take(limit) {
        println!("FAILED {failure}");
    }
    let failed: usize = total.tallies.values().map(|t| t.failures).sum();
    if failed > limit {
        println!("... and {} more", failed - limit);
    }
    failed == 0
}

fn print(name: &str, stats: &Stats) {
    println!(
        "{name}: {} files ({} skipped: not UTF-8, or errors in the clean file)",
        stats.files, stats.skipped
    );
    println!(
        "  {:<20}{:>10}{:>10}{:>11}{:>10}{:>10}{:>18}{:>10}{:>9}{:>9}",
        "token broken",
        "mutations",
        "unbroken",
        "whole file",
        "units",
        "affected",
        "shifted",
        "equal",
        "imports",
        "failed",
    );
    let mut total = Tally::default();
    let rows = stats.tallies.iter().map(|(c, t)| (*c, t.clone()));
    for (category, tally) in rows.chain(std::iter::once(("all", Tally::default()))) {
        let tally = if category == "all" {
            total.clone()
        } else {
            total.add(&tally);
            tally
        };
        let shifted = format!(
            "{} ({})",
            tally.shifted,
            percent(tally.shifted, tally.eligible - tally.affected)
        );
        println!(
            "  {category:<20}{:>10}{:>10}{:>11}{:>10}{:>10}{shifted:>18}{:>10}{:>9}{:>9}",
            tally.mutations,
            tally.unbroken,
            tally.whole_file,
            tally.eligible,
            tally.affected,
            tally.equal,
            tally.import_differences,
            tally.failures,
        );
    }
}

/// A clean file: its text, parse, whole-file output, and runs by span.
struct Clean {
    text: Arc<str>,
    source: SourceText,
    release: u32,
    parsed: Parse,
}

fn check_all(sources: &[Source], seed: u64, per_file: usize, limit: usize) -> Stats {
    let mut stats = Stats::default();
    let mut rng = Rng(seed);
    for source in sources {
        let Some(clean) = read_clean(source) else {
            stats.skipped += 1;
            continue;
        };
        stats.files += 1;
        let checked = panic::catch_unwind(AssertUnwindSafe(|| {
            check_file(
                &clean,
                &source.display,
                &mut rng,
                per_file,
                limit,
                &mut stats,
            )
        }));
        if checked.is_err() {
            stats
                .failures
                .push(format!("{}: the clean file panicked", source.display));
            stats.tallies.entry("panics").or_default().failures += 1;
        }
    }
    stats
}

fn read_clean(source: &Source) -> Option<Clean> {
    let bytes = std::fs::read(&source.path).ok()?;
    let decoded = SourceText::decode(bytes).ok()?;
    let text: Arc<str> = Arc::from(decoded.text());
    let release = source.releases[0];
    let parsed = parse(text.clone(), ParseOptions { release });
    if has_errors(&parsed) {
        return None;
    }
    Some(Clean {
        text,
        source: decoded,
        release,
        parsed,
    })
}

/// Whether `parsed` has a syntax or lexical error. Version errors don't count: the tree is
/// whole, and range formatting leaves them to its caller.
fn has_errors(parsed: &Parse) -> bool {
    parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && !d.kind.is_version())
}

/// The formatted text of `input[base..]`, up to some end, and where each input offset went,
/// for a list of edits inside it.
struct Output {
    text: String,
    map: OffsetMap,
    base: u32,
}

impl Output {
    fn new(input: &str, edits: &[Edit]) -> Output {
        Output::part(input, edits, 0..input.len() as u32)
    }

    /// The output for `part` of `input`, which holds every edit.
    fn part(input: &str, edits: &[Edit], part: Range<u32>) -> Output {
        let edits: Vec<Edit> = edits
            .iter()
            .map(|e| Edit {
                range: e.range.start - part.start..e.range.end - part.start,
                text: e.text.clone(),
            })
            .collect();
        let input = &input[part.start as usize..part.end as usize];
        Output {
            text: apply_edits(input, &edits),
            map: OffsetMap::new(&edits),
            base: part.start,
        }
    }

    /// The output for the input from `from`, the start of a piece of text, to `to`, the end of
    /// one: an insertion at either end is a gap's, outside.
    fn pieces(&self, from: u32, to: u32) -> &str {
        let (from, to) = (from - self.base, to - self.base);
        &self.text[self.map.map(from, true)..self.map.map(to, false)]
    }

    /// The output for the gap from `from`, the end of a piece of text, to `to`, the start of
    /// one: an insertion at either end is inside.
    fn gap(&self, from: u32, to: u32) -> &str {
        let (from, to) = (from - self.base, to - self.base);
        &self.text[self.map.map(from, false)..self.map.map(to, true)]
    }
}

/// Where the input's offsets are in the text edits give, for offsets between pieces of text.
struct OffsetMap {
    edits: Vec<(u32, u32)>,
    /// The shift after each edit, from all of them up to it.
    shifts: Vec<i64>,
}

impl OffsetMap {
    fn new(edits: &[Edit]) -> OffsetMap {
        let mut shift = 0;
        let shifts = edits
            .iter()
            .map(|e| {
                shift += e.text.len() as i64 - i64::from(e.range.end - e.range.start);
                shift
            })
            .collect();
        OffsetMap {
            edits: edits.iter().map(|e| (e.range.start, e.range.end)).collect(),
            shifts,
        }
    }

    /// Where `offset` is in the output: after an insertion there with `after_insertion`, before
    /// it otherwise; at the start of a replacement it is inside of.
    fn map(&self, offset: u32, after_insertion: bool) -> usize {
        let mut i = self.edits.partition_point(|&(_, end)| end < offset);
        while let Some(&(start, end)) = self.edits.get(i)
            && end == offset
            && (start < end || after_insertion)
        {
            i += 1;
        }
        let shift = if i == 0 { 0 } else { self.shifts[i - 1] };
        let offset = match self.edits.get(i) {
            Some(&(start, end)) if start < offset && offset < end => start,
            _ => offset,
        };
        (i64::from(offset) + shift) as usize
    }
}

fn check_file(
    clean: &Clean,
    display: &str,
    rng: &mut Rng,
    per_file: usize,
    limit: usize,
    stats: &mut Stats,
) {
    let tree = &clean.parsed.tree;
    let tokens: Vec<_> = tree
        .tokens()
        .filter(|t| !t.is_trivia() && t.kind() != SyntaxKind::Eof)
        .collect();
    if tokens.is_empty() {
        return;
    }
    let clean_ranges = Ranges::new(tree, &clean.source);
    let clean_output = Output::new(&clean.text, clean_ranges.edits());
    let clean_runs: HashMap<(u32, u32), Run> = clean_ranges
        .runs()
        .into_iter()
        .map(|run| (run_span(tree, &run), run))
        .collect();
    for i in 0..per_file {
        let token = tokens[rng.below(tokens.len())];
        let kind = if i % 2 == 0 {
            Kind::Delete
        } else {
            Kind::Replace
        };
        let span = token.span();
        let inserted = match kind {
            Kind::Delete => String::new(),
            Kind::Replace => {
                let mut word = VOCABULARY[rng.below(VOCABULARY.len())];
                if word == token.text() {
                    word = "goto";
                }
                format!(" {word} ")
            }
        };
        let mutation = Mutation {
            kind,
            start: span.start,
            end: span.end,
            inserted,
        };
        let category = category(token.kind());
        let tally = stats.tallies.entry(category).or_default();
        tally.mutations += 1;
        let mut failures = Vec::new();
        let checked = panic::catch_unwind(AssertUnwindSafe(|| {
            check_mutation(
                clean,
                &clean_ranges,
                &clean_output,
                &clean_runs,
                &mutation,
                rng,
                tally,
                &mut failures,
            )
        }));
        if let Err(payload) = checked {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            failures.push(format!("panicked: {message}"));
        }
        for failure in failures {
            tally.failures += 1;
            if stats.failures.len() < limit {
                stats.failures.push(format!(
                    "{display} at {}, {mutation}: {failure}",
                    clean.release
                ));
            }
        }
    }
}

/// The text of `tree`'s imports, in order.
fn imports(tree: &SyntaxTree) -> Vec<&str> {
    tree.nodes()
        .filter(|n| n.kind() == SyntaxKind::ImportDecl)
        .map(|n| n.text())
        .collect()
}

/// The span of `run`'s tokens, from the first's start to the last's end.
fn run_span(tree: &SyntaxTree, run: &Run) -> (u32, u32) {
    (
        tree.token(run.first).span().start,
        tree.token(run.last).span().end,
    )
}

#[allow(clippy::too_many_arguments)]
fn check_mutation(
    clean: &Clean,
    clean_ranges: &Ranges<'_>,
    clean_output: &Output,
    clean_runs: &HashMap<(u32, u32), Run>,
    mutation: &Mutation,
    rng: &mut Rng,
    tally: &mut Tally,
    failures: &mut Vec<String>,
) {
    let broken_text = mutation.apply(&clean.text);
    let Ok(broken_source) = SourceText::from_utf8(broken_text.clone().into_bytes()) else {
        return;
    };
    let release = clean.release;
    let parsed = parse(Arc::from(broken_text.as_str()), ParseOptions { release });
    if !has_errors(&parsed) {
        tally.unbroken += 1;
        return;
    }
    let tree = &parsed.tree;
    let mut imports_changed = imports(tree) != imports(&clean.parsed.tree);
    if tree.error_units().first() == Some(&tree.root()) {
        tally.whole_file += 1;
        return;
    }
    let ranges = Ranges::new(tree, &broken_source);
    // Which imports formatting removes depends on the code that uses them, which the change can
    // be.
    imports_changed |= ranges.kept_imports() != clean_ranges.kept_imports();
    let clean_tree = &clean.parsed.tree;
    // Each run's expanded range, and its clean match when it has one: `None` for a run that
    // touches the change, or that an error affects, or without a match.
    let runs = ranges.runs();
    let mut matches: Vec<Option<(Expanded, Expanded)>> = Vec::with_capacity(runs.len());
    for run in &runs {
        let first = tree.token(run.first).span();
        let Some(expanded) = ranges.expand(first.start..first.end) else {
            matches.push(None);
            continue;
        };
        if mutation.touches(expanded.units.clone()) {
            matches.push(None);
            continue;
        }
        tally.eligible += 1;
        if ranges.affected(&expanded).is_some() {
            tally.affected += 1;
            matches.push(None);
            continue;
        }
        let (start, end) = run_span(tree, run);
        let matched = (|| {
            let span = (mutation.to_clean(start)?, mutation.to_clean(end)?);
            let clean_run = clean_runs.get(&span)?;
            if clean_run.kind != run.kind
                || !clean_tree
                    .node(clean_run.owner)
                    .ancestors()
                    .map(|n| n.kind())
                    .eq(tree.node(run.owner).ancestors().map(|n| n.kind()))
            {
                return None;
            }
            let first = clean_tree.token(clean_run.first).span();
            let clean_expanded = clean_ranges.expand(first.start..first.end)?;
            let units = (
                mutation.to_clean(expanded.units.start)?,
                mutation.to_clean(expanded.units.end)?,
            );
            (units == (clean_expanded.units.start, clean_expanded.units.end))
                .then_some(clean_expanded)
        })();
        if matched.is_none() {
            tally.shifted += 1;
        }
        matches.push(matched.map(|clean_expanded| (expanded, clean_expanded)));
    }
    let mut formatted = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let Some((expanded, clean_expanded)) = &matches[i] else {
            continue;
        };
        let (start, end) = run_span(tree, run);
        let first = tree.token(run.first).span();
        let range = first.start..first.end;
        let edits = match ranges.format(range.clone()) {
            Ok(edits) => edits,
            Err(error) => {
                failures.push(format!("the unit at {start}..{end}: {error}"));
                continue;
            }
        };
        // The gaps around the units depend on the units on either side too, so they are
        // compared only next to a unit that matches.
        let neighbours = (
            i > 0 && matches[i - 1].is_some(),
            matches.get(i + 1).is_some_and(Option::is_some),
        );
        match compare(
            &broken_text,
            &edits,
            expanded,
            clean_output,
            clean_expanded,
            mutation,
            neighbours,
        ) {
            Ok(()) => tally.equal += 1,
            Err(_) if imports_changed => tally.import_differences += 1,
            Err(difference) => {
                failures.push(format!("the unit at {start}..{end}: {difference}"));
            }
        }
        formatted.push((range, edits));
    }
    // One unit formatted is checked with verify_range.
    if !formatted.is_empty() {
        let (range, edits) = &formatted[rng.below(formatted.len())];
        let options = FormatOptions { release };
        if let Err(violation) = verify_range(&broken_source, range.clone(), edits, &options) {
            failures.push(format!("verify_range for {range:?}: {violation}"));
        }
    }
}

/// Compares the broken file's units, `expanded`, formatted with `edits` as `format_range` gave
/// them, with the clean file's output for its units, `clean_expanded`: their text, the gap before
/// them (all of it when the unit before matches too, otherwise its indentation), and the line
/// breaks after them when the unit after matches.
fn compare(
    broken: &str,
    edits: &[Edit],
    expanded: &Expanded,
    clean: &Output,
    clean_expanded: &Expanded,
    mutation: &Mutation,
    (before_matches, after_matches): (bool, bool),
) -> Result<(), String> {
    let result = Output::part(broken, edits, expanded.before..expanded.after);
    let (units, clean_units) = (&expanded.units, &clean_expanded.units);
    let ours = result.pieces(units.start, units.end);
    let theirs = clean.pieces(clean_units.start, clean_units.end);
    if ours != theirs {
        return Err(format!("formats as {ours:?}, not {theirs:?}"));
    }
    // The gap before: all of it when the code before is the clean file's, otherwise the
    // indentation.
    let before = expanded.before..units.start;
    if !mutation.touches(before.clone()) {
        let ours = result.gap(before.start, before.end);
        let theirs = clean.gap(clean_expanded.before, clean_units.start);
        let same_code =
            before_matches && mutation.to_clean(before.start) == Some(clean_expanded.before);
        let indentation = |gap: &str| gap.rsplit(['\n', '\r']).next().unwrap_or("").to_owned();
        let differs = if same_code {
            ours != theirs
        } else {
            indentation(ours) != indentation(theirs)
        };
        if differs {
            return Err(format!("is after {ours:?}, not {theirs:?}"));
        }
    }
    // The line breaks after, when the code after is the clean file's.
    let after = units.end..expanded.after;
    if after_matches
        && !mutation.touches(after.clone())
        && mutation.to_clean(after.end) == Some(clean_expanded.after)
    {
        let breaks = |gap: &str| {
            gap.matches('\n').count() + gap.matches('\r').count() - gap.matches("\r\n").count()
        };
        let ours = result.gap(after.start, after.end);
        let theirs = clean.gap(clean_units.end, clean_expanded.after);
        if breaks(ours) != breaks(theirs) {
            return Err(format!("is before {ours:?}, not {theirs:?}"));
        }
    }
    Ok(())
}
