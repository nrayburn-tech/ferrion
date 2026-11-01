//! `parser-check mutate`: the corpus mutation test (docs/design/parser-error-recovery.md,
//! "Testing recovered trees").
//!
//! Each mutation picks a file and a token in it with a seeded random generator, so a run is
//! reproducible from the seed, the sample size, and the corpus pins. The kinds take turns:
//!
//! - `delete`: delete the token;
//! - `duplicate`: repeat the token after itself;
//! - `insert`: insert a token from a small vocabulary (`{`, `}`, `(`, `)`, `;`, `.`, `=`, or an
//!   identifier) before the token;
//! - `truncate`: cut the file before the token;
//! - `delete-delimiter`: delete a `(`, `)`, `[`, `]`, `{`, or `}`, which unbalances them.
//!
//! Inserted text is surrounded by spaces, so it never joins a neighbouring token. Files are parsed
//! at the first release they're meant for, and only files that parse without errors are mutated.
//!
//! For each mutation it checks, as `round-trip` does, that parsing the broken text doesn't panic,
//! reproduces the text, and keeps `ferrion_parser::invariants`, and that an error is reported when
//! the delimiters no longer balance. It then measures **preservation**: each member declaration of
//! the clean file that doesn't overlap the change is looked for in the broken file's tree, at the
//! same place (shifted by the change), with the same kind, name, and text. Statements in bodies are
//! measured the same way, by kind and text. "Overlap" is generous: a declaration that touches the
//! change, including one that starts right where a token is inserted, is left out. The share
//! found, per mutation kind, is the quality metric, and `thresholds.toml` sets the least share each
//! kind may have.
//!
//! Until the grammar is written, most files have errors at the parts not written yet (method
//! bodies, for example), so few files are mutated; the run reports how many were skipped.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use corpus::sources::Source;
use ferrion_diagnostics::SourceText;
use ferrion_parser::syntax::{SyntaxNode, SyntaxTree, token_sets};
use ferrion_parser::{Parse, ParseOptions, Severity, SyntaxKind, invariants, parse};
use serde::Deserialize;

use crate::{Options, on_small_stack, percent};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    Delete,
    Duplicate,
    Insert,
    Truncate,
    DeleteDelimiter,
}

const KINDS: [Kind; 5] = [
    Kind::Delete,
    Kind::Duplicate,
    Kind::Insert,
    Kind::Truncate,
    Kind::DeleteDelimiter,
];

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Delete => "delete",
            Kind::Duplicate => "duplicate",
            Kind::Insert => "insert",
            Kind::Truncate => "truncate",
            Kind::DeleteDelimiter => "delete-delimiter",
        }
    }
}

/// What `insert` inserts. The identifier is one no corpus file is likely to declare.
const VOCABULARY: [&str; 8] = ["{", "}", "(", ")", ";", ".", "=", "ferrionInserted"];

const DELIMITERS: [SyntaxKind; 6] = [
    SyntaxKind::LParen,
    SyntaxKind::RParen,
    SyntaxKind::LBracket,
    SyntaxKind::RBracket,
    SyntaxKind::LBrace,
    SyntaxKind::RBrace,
];

/// How many files to try for one mutation before giving up: a file must parse without errors,
/// and `delete-delimiter` needs one with a delimiter.
const ATTEMPTS: usize = 1000;

/// A change to a file's text: `start..end` of the clean text replaced by `inserted`.
#[derive(Clone, Debug, PartialEq, Eq)]
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

    /// How far text after the change moves.
    fn shift(&self) -> i64 {
        self.inserted.len() as i64 - i64::from(self.end - self.start)
    }

    /// Where the clean text's `start..end`, which doesn't touch the change, is in the broken text,
    /// or `None` if it touches the change.
    fn map(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        if end < self.start {
            Some((start, end))
        } else if start > self.end {
            let shift = |offset: u32| (i64::from(offset) + self.shift()) as u32;
            Some((shift(start), shift(end)))
        } else {
            None
        }
    }
}

impl fmt::Display for Mutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}..{}", self.kind.name(), self.start, self.end)?;
        if !self.inserted.is_empty() {
            write!(f, " inserting {:?}", self.inserted)?;
        }
        Ok(())
    }
}

/// SplitMix64: small, fast, and the same everywhere, which is all a reproducible sample needs.
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

/// A token `kind` may change, chosen with `rng`: any token but trivia, characters the lexer
/// couldn't make a token of, and the end of the file, or for `delete-delimiter` a delimiter.
fn pick_token(tree: &SyntaxTree, kind: Kind, rng: &mut Rng) -> Option<(u32, u32, SyntaxKind)> {
    let candidates: Vec<_> = tree
        .tokens()
        .filter(|t| {
            !t.is_trivia()
                && t.kind() != SyntaxKind::Eof
                && t.kind() != SyntaxKind::ErrorToken
                && (kind != Kind::DeleteDelimiter || DELIMITERS.contains(&t.kind()))
        })
        .map(|t| (t.span().start, t.span().end, t.kind()))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    Some(candidates[rng.below(candidates.len())])
}

fn mutation(kind: Kind, text: &str, token: (u32, u32), rng: &mut Rng) -> Mutation {
    let (start, end) = token;
    let token_text = &text[start as usize..end as usize];
    let (start, end, inserted) = match kind {
        Kind::Delete | Kind::DeleteDelimiter => (start, end, String::new()),
        Kind::Duplicate => (end, end, format!(" {token_text}")),
        Kind::Insert => {
            let word = VOCABULARY[rng.below(VOCABULARY.len())];
            (start, start, format!(" {word} "))
        }
        Kind::Truncate => (start, text.len() as u32, String::new()),
    };
    Mutation {
        kind,
        start,
        end,
        inserted,
    }
}

/// What preservation is measured for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    /// Member declarations, at any depth, including local and anonymous classes' members.
    Members,
    /// Statements in bodies.
    Statements,
    /// Package and import declarations, which the grammar has today.
    Imports,
}

const GROUPS: [Group; 3] = [Group::Members, Group::Statements, Group::Imports];

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Members => "members",
            Group::Statements => "statements",
            Group::Imports => "imports",
        }
    }
}

/// A member declaration, statement, or import, by where it is and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit {
    kind: SyntaxKind,
    group: Group,
    start: u32,
    end: u32,
    /// The declared or imported names; empty for statements and initializers.
    name: String,
}

/// The `Member` kinds in java.grammar.
const MEMBER_KINDS: [SyntaxKind; 10] = [
    SyntaxKind::ClassDecl,
    SyntaxKind::InterfaceDecl,
    SyntaxKind::EnumDecl,
    SyntaxKind::RecordDecl,
    SyntaxKind::AnnotationTypeDecl,
    SyntaxKind::FieldDecl,
    SyntaxKind::MethodDecl,
    SyntaxKind::ConstructorDecl,
    SyntaxKind::CompactConstructorDecl,
    SyntaxKind::Initializer,
];

/// The `Stmt` kinds in java.grammar, apart from local type declarations, which are members.
const STATEMENT_KINDS: [SyntaxKind; 19] = [
    SyntaxKind::Block,
    SyntaxKind::LocalVarDecl,
    SyntaxKind::EmptyStmt,
    SyntaxKind::ExprStmt,
    SyntaxKind::IfStmt,
    SyntaxKind::AssertStmt,
    SyntaxKind::SwitchStmt,
    SyntaxKind::WhileStmt,
    SyntaxKind::DoStmt,
    SyntaxKind::ForStmt,
    SyntaxKind::ForEachStmt,
    SyntaxKind::LabeledStmt,
    SyntaxKind::BreakStmt,
    SyntaxKind::ContinueStmt,
    SyntaxKind::ReturnStmt,
    SyntaxKind::ThrowStmt,
    SyntaxKind::SynchronizedStmt,
    SyntaxKind::TryStmt,
    SyntaxKind::YieldStmt,
];

/// The nodes whose `Stmt` children are statements. A `Block` elsewhere is a body (of a method,
/// a lambda, `try`), and a `LocalVarDecl` elsewhere is in a `for` header or a resource.
const STATEMENT_PARENTS: [SyntaxKind; 8] = [
    SyntaxKind::Block,
    SyntaxKind::SwitchGroup,
    SyntaxKind::LabeledStmt,
    SyntaxKind::IfStmt,
    SyntaxKind::WhileStmt,
    SyntaxKind::DoStmt,
    SyntaxKind::ForStmt,
    SyntaxKind::ForEachStmt,
];

fn units(tree: &SyntaxTree) -> Vec<Unit> {
    let mut units = Vec::new();
    for node in tree.nodes() {
        let kind = node.kind();
        let group = if MEMBER_KINDS.contains(&kind) {
            Group::Members
        } else if STATEMENT_KINDS.contains(&kind)
            && node
                .parent()
                .is_some_and(|parent| STATEMENT_PARENTS.contains(&parent.kind()))
        {
            Group::Statements
        } else if kind == SyntaxKind::PackageDecl || kind == SyntaxKind::ImportDecl {
            Group::Imports
        } else {
            continue;
        };
        let span = node.span();
        units.push(Unit {
            kind,
            group,
            start: span.start,
            end: span.end,
            name: name(node, group),
        });
    }
    units
}

/// A member's declared name, or for a field the names of its declarators, comma-separated; the
/// name a package or import declaration names; nothing for a statement.
fn name(node: SyntaxNode<'_>, group: Group) -> String {
    fn own_name(node: SyntaxNode<'_>) -> Option<&str> {
        node.direct_tokens()
            .find(|t| token_sets::IDENT.contains(&t.kind()))
            .map(|t| t.text())
    }
    if group == Group::Statements {
        return String::new();
    }
    if group == Group::Imports {
        let name = node.children().find(|c| c.kind() == SyntaxKind::Name);
        return name.map_or("", |n| n.text()).to_owned();
    }
    if node.kind() == SyntaxKind::FieldDecl {
        let names: Vec<&str> = node
            .children()
            .filter(|c| c.kind() == SyntaxKind::VariableDeclarator)
            .filter_map(own_name)
            .collect();
        names.join(",")
    } else {
        own_name(node).unwrap_or_default().to_owned()
    }
}

/// Whether the delimiters in `tree`'s tokens fail to balance, which no valid file does.
fn unbalanced(tree: &SyntaxTree) -> bool {
    let mut depth = [0i64; 3];
    for token in tree.tokens() {
        let (i, step) = match token.kind() {
            SyntaxKind::LParen => (0, 1),
            SyntaxKind::RParen => (0, -1),
            SyntaxKind::LBracket => (1, 1),
            SyntaxKind::RBracket => (1, -1),
            SyntaxKind::LBrace => (2, 1),
            SyntaxKind::RBrace => (2, -1),
            _ => continue,
        };
        depth[i] += step;
        if depth[i] < 0 {
            return true;
        }
    }
    depth != [0; 3]
}

/// Whether `parsed` has a lexical or syntax error. Version errors don't count: the tree is
/// complete, and a mutation that only changes what the release allows broke nothing.
fn has_errors(parsed: &Parse) -> bool {
    parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && !d.kind.is_version())
}

/// Totals for one mutation kind.
#[derive(Clone, Default)]
struct Tally {
    mutations: usize,
    failures: usize,
    /// Broken files without an error, whether or not the change broke the syntax.
    no_error: usize,
    /// Preservation per group, in the order of [`GROUPS`].
    shares: [Share; 3],
}

#[derive(Clone, Copy, Default)]
struct Share {
    eligible: usize,
    preserved: usize,
}

impl Share {
    fn ratio(self) -> Option<f64> {
        (self.eligible > 0).then(|| self.preserved as f64 / self.eligible as f64)
    }
}

impl fmt::Display for Share {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}/{})",
            percent(self.preserved, self.eligible),
            self.preserved,
            self.eligible
        )
    }
}

/// The least share of member declarations, statements, and imports each mutation kind may
/// preserve.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Thresholds {
    members: BTreeMap<String, f64>,
    statements: BTreeMap<String, f64>,
    imports: BTreeMap<String, f64>,
}

impl Thresholds {
    fn table(&self, group: Group) -> &BTreeMap<String, f64> {
        match group {
            Group::Members => &self.members,
            Group::Statements => &self.statements,
            Group::Imports => &self.imports,
        }
    }
}

impl Thresholds {
    fn load(path: &Path) -> Result<Thresholds> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let thresholds: Thresholds =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        for table in GROUPS.map(|group| thresholds.table(group)) {
            for kind in KINDS {
                if !table.contains_key(kind.name()) {
                    bail!("{}: no threshold for `{}`", path.display(), kind.name());
                }
            }
            if let Some(name) = table
                .keys()
                .find(|name| !KINDS.iter().any(|k| k.name() == name.as_str()))
            {
                bail!("{}: unknown mutation kind `{name}`", path.display());
            }
        }
        Ok(thresholds)
    }
}

/// Runs the test on `sources` and prints the results. `Ok(false)` if a mutation failed its checks
/// or a share is below its threshold.
pub fn run(sources: Vec<Source>, options: &Options) -> Result<bool> {
    let thresholds = Thresholds::load(&options.thresholds)?;
    if sources.is_empty() {
        bail!("no files to mutate");
    }
    let (seed, sample, limit, verbose) =
        (options.seed, options.sample, options.limit, options.verbose);
    let outcome = on_small_stack(move || mutate_all(&sources, seed, sample, limit, verbose))?;

    println!(
        "mutate: {sample} mutations (seed {seed}) of {} files; {} files skipped for parse errors",
        outcome.files_mutated, outcome.files_with_errors
    );
    println!(
        "{:<17}{:>10}{:>10}{:>10}  {:<28}{:<28}imports preserved",
        "kind", "mutations", "failures", "no error", "members preserved", "statements preserved",
    );
    let mut passed = true;
    let mut gate = Vec::new();
    for kind in KINDS {
        let tally = outcome.tallies.get(&kind).cloned().unwrap_or_default();
        let [members, statements, imports] = tally.shares.map(|share| share.to_string());
        println!(
            "{:<17}{:>10}{:>10}{:>10}  {members:<28}{statements:<28}{imports}",
            kind.name(),
            tally.mutations,
            tally.failures,
            tally.no_error,
        );
        for (group, share) in GROUPS.into_iter().zip(tally.shares) {
            let what = group.name();
            let least = thresholds.table(group)[kind.name()];
            match share.ratio() {
                Some(ratio) if ratio < least => gate.push(format!(
                    "{} {what}: {share} preserved, below the threshold of {:.2}%",
                    kind.name(),
                    least * 100.0
                )),
                None if least > 0.0 => gate.push(format!(
                    "{} {what}: nothing measured, but the threshold is {:.2}%",
                    kind.name(),
                    least * 100.0
                )),
                _ => {}
            }
        }
    }
    for failure in outcome.failures.iter().take(limit) {
        println!("FAILED {failure}");
    }
    let failed = outcome.tallies.values().map(|t| t.failures).sum::<usize>();
    if failed > limit {
        println!("... and {} more", failed - limit);
    }
    if verbose {
        for lost in &outcome.lost {
            println!("lost {lost}");
        }
    }
    if failed > 0 {
        passed = false;
    }
    for problem in &gate {
        println!("BELOW THRESHOLD {problem}");
        passed = false;
    }
    Ok(passed)
}

struct Outcome {
    tallies: BTreeMap<Kind, Tally>,
    /// The first failures, up to the limit.
    failures: Vec<String>,
    /// The first declarations and statements lost, up to the limit, with `--verbose`.
    lost: Vec<String>,
    files_mutated: usize,
    files_with_errors: usize,
}

/// A file read for mutation, and its clean parse.
struct Clean {
    text: Arc<str>,
    release: u32,
    parsed: Parse,
}

fn mutate_all(
    sources: &[Source],
    seed: u64,
    sample: usize,
    limit: usize,
    verbose: bool,
) -> Result<Outcome> {
    let mut rng = Rng(seed);
    let mut outcome = Outcome {
        tallies: BTreeMap::new(),
        failures: Vec::new(),
        lost: Vec::new(),
        files_mutated: 0,
        files_with_errors: 0,
    };
    // Whether each file parses without errors, once known.
    let mut usable: HashMap<usize, bool> = HashMap::new();
    for i in 0..sample {
        let kind = KINDS[i % KINDS.len()];
        let mut chosen = None;
        for _ in 0..ATTEMPTS {
            let index = rng.below(sources.len());
            if usable.get(&index) == Some(&false) {
                continue;
            }
            let Some(clean) = read_clean(&sources[index]) else {
                usable.insert(index, false);
                continue;
            };
            let first = usable.insert(index, true).is_none();
            if first {
                outcome.files_mutated += 1;
            }
            if let Some((start, end, _)) = pick_token(&clean.parsed.tree, kind, &mut rng) {
                chosen = Some((index, clean, (start, end)));
                break;
            }
        }
        outcome.files_with_errors = usable.values().filter(|ok| !**ok).count();
        let Some((index, clean, token)) = chosen else {
            bail!(
                "found no file to make mutation {i} ({}) in after {ATTEMPTS} tries",
                kind.name()
            );
        };
        let mutation = mutation(kind, &clean.text, token, &mut rng);
        let display = &sources[index].display;
        let tally = outcome.tallies.entry(kind).or_default();
        tally.mutations += 1;
        match check_mutation(&clean, &mutation, tally) {
            Ok(lost) => {
                if verbose {
                    for unit in lost {
                        if outcome.lost.len() < limit {
                            outcome.lost.push(format!(
                                "{display} at {}, {mutation}: {:?} {:?} at {}..{}",
                                clean.release, unit.kind, unit.name, unit.start, unit.end
                            ));
                        }
                    }
                }
            }
            Err(problem) => {
                tally.failures += 1;
                if outcome.failures.len() < limit {
                    outcome.failures.push(format!(
                        "{display} at {}, {mutation}: {problem}",
                        clean.release
                    ));
                }
            }
        }
    }
    Ok(outcome)
}

/// Reads and parses `source` at its first release, or `None` if it can't be decoded or has
/// lexical or syntax errors.
fn read_clean(source: &Source) -> Option<Clean> {
    let bytes = std::fs::read(&source.path).ok()?;
    let text: Arc<str> = Arc::from(SourceText::decode(bytes).ok()?.text());
    let release = source.releases[0];
    let parsed = panic::catch_unwind(|| parse(text.clone(), ParseOptions { release })).ok()?;
    if has_errors(&parsed) {
        return None;
    }
    Some(Clean {
        text,
        release,
        parsed,
    })
}

/// Parses the broken text, checks it, and adds its preservation to `tally`. Returns the units
/// lost, or what's wrong.
fn check_mutation(
    clean: &Clean,
    mutation: &Mutation,
    tally: &mut Tally,
) -> Result<Vec<Unit>, String> {
    let broken: Arc<str> = Arc::from(mutation.apply(&clean.text));
    let release = clean.release;
    let parsed = panic::catch_unwind(|| parse(broken.clone(), ParseOptions { release }))
        .map_err(|_| "parse panicked".to_owned())?;
    let checked = panic::catch_unwind(AssertUnwindSafe(|| {
        invariants::check(&broken, release, &parsed)
    }));
    match checked {
        Ok(Ok(())) => {}
        Ok(Err(problem)) => return Err(problem),
        Err(_) => return Err("the checks panicked".to_owned()),
    }
    let errors = has_errors(&parsed);
    if !errors {
        tally.no_error += 1;
    }
    if !errors && unbalanced(&parsed.tree) {
        return Err("no error reported, but the delimiters don't balance".to_owned());
    }

    let found: HashMap<(SyntaxKind, u32, u32), Unit> = units(&parsed.tree)
        .into_iter()
        .map(|unit| ((unit.kind, unit.start, unit.end), unit))
        .collect();
    let mut lost = Vec::new();
    for unit in units(&clean.parsed.tree) {
        let Some((start, end)) = mutation.map(unit.start, unit.end) else {
            continue;
        };
        let preserved = found.get(&(unit.kind, start, end)).is_some_and(|other| {
            other.name == unit.name
                && broken[start as usize..end as usize]
                    == clean.text[unit.start as usize..unit.end as usize]
        });
        let share = &mut tally.shares[GROUPS.iter().position(|g| *g == unit.group).unwrap()];
        share.eligible += 1;
        if preserved {
            share.preserved += 1;
        } else {
            lost.push(unit);
        }
    }
    Ok(lost)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(text: &str) -> Clean {
        let text: Arc<str> = Arc::from(text);
        Clean {
            parsed: parse(text.clone(), ParseOptions { release: 27 }),
            text,
            release: 27,
        }
    }

    #[test]
    fn the_generator_is_reproducible() {
        let mut a = Rng(1);
        let mut b = Rng(1);
        let first: Vec<u64> = (0..5).map(|_| a.next()).collect();
        assert_eq!(first, (0..5).map(|_| b.next()).collect::<Vec<_>>());
        assert_ne!(Rng(2).next(), first[0]);
        // SplitMix64's usual reference output, so the sample can't change with the platform.
        assert_eq!(Rng(1_234_567).next(), 6_457_827_717_110_365_317);
    }

    #[test]
    fn mutations() {
        let text = "import a.b;\nimport c.d;\n";
        let mut rng = Rng(1);
        let token = (7, 8); // `a`
        let delete = mutation(Kind::Delete, text, token, &mut rng);
        assert_eq!(delete.apply(text), "import .b;\nimport c.d;\n");
        let duplicate = mutation(Kind::Duplicate, text, token, &mut rng);
        assert_eq!(duplicate.apply(text), "import a a.b;\nimport c.d;\n");
        let truncate = mutation(Kind::Truncate, text, token, &mut rng);
        assert_eq!(truncate.apply(text), "import ");
        let insert = mutation(Kind::Insert, text, token, &mut rng);
        let broken = insert.apply(text);
        assert!(broken.starts_with("import  ") && broken.ends_with(" a.b;\nimport c.d;\n"));

        // Text before the change stays put, text after it moves, and text touching it has no place.
        assert_eq!(delete.map(0, 6), Some((0, 6)));
        assert_eq!(delete.map(12, 23), Some((11, 22)));
        assert_eq!(delete.map(0, 11), None);
        assert_eq!(delete.map(8, 11), None);
        assert_eq!(duplicate.map(12, 23), Some((14, 25)));
        assert_eq!(truncate.map(12, 23), None);
    }

    #[test]
    fn tokens_to_change() {
        let tree = clean("import a.b;\n// c\n").parsed.tree;
        let mut rng = Rng(7);
        for _ in 0..50 {
            let (_, _, kind) = pick_token(&tree, Kind::Delete, &mut rng).unwrap();
            assert!(!kind.is_trivia() && kind != SyntaxKind::Eof);
        }
        assert_eq!(pick_token(&tree, Kind::DeleteDelimiter, &mut rng), None);
        let tree = clean("@A(1) package p;\n").parsed.tree;
        let (_, _, kind) = pick_token(&tree, Kind::DeleteDelimiter, &mut rng).unwrap();
        assert!(DELIMITERS.contains(&kind));
    }

    #[test]
    fn balance() {
        assert!(!unbalanced(&clean("@A(1) package p;").parsed.tree));
        assert!(unbalanced(&clean("@A(1 package p;").parsed.tree));
        assert!(unbalanced(&clean("@A)1( package p;").parsed.tree));
        assert!(unbalanced(&clean("class A { void m() { }").parsed.tree));
    }

    fn change(kind: Kind, start: u32, end: u32, inserted: &str) -> Mutation {
        Mutation {
            kind,
            start,
            end,
            inserted: inserted.to_owned(),
        }
    }

    #[test]
    fn preservation() {
        let clean = clean("import a.b;\nimport c.d;\nimport e.f;\n");
        let imports = |tally: &Tally| {
            let share = tally.shares[2];
            (share.preserved, share.eligible)
        };

        // The import whose `;` is deleted touches the change; the others are found, shifted.
        let mut tally = Tally::default();
        let lost = check_mutation(&clean, &change(Kind::Delete, 10, 11, ""), &mut tally).unwrap();
        assert!(lost.is_empty());
        assert_eq!(imports(&tally), (2, 2));
        assert_eq!(tally.no_error, 0);

        let mut tally = Tally::default();
        check_mutation(&clean, &change(Kind::Truncate, 24, 36, ""), &mut tally).unwrap();
        assert_eq!(imports(&tally), (2, 2));

        // An unclosed comment makes the imports after it comments.
        let mut tally = Tally::default();
        let lost =
            check_mutation(&clean, &change(Kind::Insert, 11, 11, " /* "), &mut tally).unwrap();
        assert_eq!(imports(&tally), (0, 2));
        let names: Vec<&str> = lost.iter().map(|unit| unit.name.as_str()).collect();
        assert_eq!(names, ["c.d", "e.f"]);
    }
}
