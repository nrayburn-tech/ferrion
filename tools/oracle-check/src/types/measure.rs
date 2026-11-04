//! `oracle measure`: what attribution costs a project (docs/design/semantic-attribution.md#measurements):
//! a cold run on one thread and on four against the names of bodies alone, and an edit inside a
//! body of its largest file and of a large hand-written one, end to end. No javac.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use corpus::Dirs;
use corpus::project::Project;
use ferrion_semantic::{Cancelled, Engine, FileId, Snapshot};
use ferrion_syntax::{SyntaxKind, SyntaxNode, WalkEvent};

use crate::names::ferrion::Ferrion;

pub const USAGE: &str = "\
usage: oracle measure <project> [<file>...]

Measures a project's cold attribution, on one thread and four, against the names of its bodies
alone, and edits inside a body of its largest file and of each file whose path contains a <file>:
a statement added, and a `var`'s initializer changed so that its type does, each followed by what a
language server would ask for: everything project-wide, the file's names, and its attribution. Then
every file's attribution again, to see which the edit reached.";

/// Each cold run's repetitions, of which the fastest counts; each edit's, of which the range shows.
const REPETITIONS: usize = 3;

fn cancelled(_: Cancelled) -> anyhow::Error {
    anyhow!("cancelled with no edit running")
}

pub fn run(mut args: impl Iterator<Item = String>) -> Result<bool> {
    let name = args.next().context(USAGE)?;
    let wanted: Vec<String> = args.collect();
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    let project = corpus::select(&projects, std::slice::from_ref(&name))?[0];
    let jdk = corpus::model::lock_jdk(&dirs, project)?;
    let home = corpus::jdks::find(jdk)?;
    let cache_root = match std::env::var_os("FERRION_ORACLE_CACHE") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle"),
    };
    let jdks = std::collections::HashMap::from([(name.clone(), home)]);
    let ferrion = Ferrion::new(
        &[(name.clone(), 1)],
        &jdks,
        cache_root.join("scratch"),
        4,
        false,
    )?;
    let input = ferrion.input(&name).map_err(|e| anyhow!(e))?;
    let count: usize = input.source_sets.iter().map(|s| s.files.len()).sum();
    let bytes: usize = input
        .source_sets
        .iter()
        .flat_map(|s| &s.files)
        .map(|f| f.text.len())
        .sum();
    println!(
        "{name}: {} source sets, {count} files, {:.1} MB",
        input.source_sets.len(),
        bytes as f64 / 1e6
    );

    println!(
        "\n| Threads | Project-wide | Names of bodies | Names and attribution | Attribution |"
    );
    println!("| --- | --- | --- | --- | --- |");
    for threads in [1, 4] {
        let (mut symbols, mut names, mut both) = (f64::MAX, f64::MAX, f64::MAX);
        for _ in 0..REPETITIONS {
            let engine = Engine::new(input.clone());
            let snapshot = engine.snapshot();
            let start = Instant::now();
            let files = snapshot.run_symbols(threads).map_err(cancelled)?;
            symbols = symbols.min(start.elapsed().as_secs_f64());
            let start = Instant::now();
            snapshot
                .run_names(threads, &files, |_| Ok(()))
                .map_err(cancelled)?;
            names = names.min(start.elapsed().as_secs_f64());
            let start = Instant::now();
            snapshot
                .run_names(threads, &files, |file| {
                    snapshot.body_types(file)?;
                    Ok(())
                })
                .map_err(cancelled)?;
            both = both.min(start.elapsed().as_secs_f64());
        }
        println!(
            "| {threads} | {symbols:.2} s | {names:.2} s | {both:.2} s | {:.2} s |",
            both - names
        );
    }

    // A cold run on four threads, with every file's attribution kept.
    let engine = Engine::new(input.clone());
    let targets = {
        let snapshot = engine.snapshot();
        let files = snapshot.run_symbols(4).map_err(cancelled)?;
        let mut largest = (0, files[0]);
        let mut targets = Vec::new();
        let (mut kept, mut arena) = (0, 0);
        for &file in &files {
            let types = snapshot.body_types(file).map_err(cancelled)?;
            kept += types.bytes();
            arena = arena.max(types.bytes());
            let text = snapshot.text(file).map_err(cancelled)?;
            if text.len() > largest.0 {
                largest = (text.len(), file);
            }
            let path = snapshot.file(file).map_err(cancelled)?.path;
            let path = path.to_string_lossy().into_owned();
            if wanted.iter().any(|w| path.contains(w.as_str())) && !targets.contains(&file) {
                targets.push(file);
            }
        }
        println!(
            "\nEvery file's attribution kept: {:.0} MB; the largest result {:.1} MB",
            kept as f64 / 1e6,
            arena as f64 / 1e6
        );
        targets.insert(0, largest.1);
        targets
    };

    println!(
        "\n| File | Bytes | Edit | Ask (everything project-wide, names, attribution) | Attribution alone | Every file's attribution then | Other files computed |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for target in targets {
        let snapshot = engine.snapshot();
        let path = snapshot.file(target).map_err(cancelled)?.path;
        let short = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let original = snapshot.text(target).map_err(cancelled)?;
        let tree = snapshot.tree(target).map_err(cancelled)?;
        let Some(at) = first_body(tree.root()) else {
            println!("| {short} | no method with a body |");
            continue;
        };
        drop(snapshot);
        let with = |statement: &str| -> Arc<str> {
            let mut text = original.to_string();
            text.insert_str(at, statement);
            Arc::from(text.as_str())
        };
        let edits = [
            (
                "a statement added",
                with(" int ferrionMeasure = 0; "),
                with(""),
            ),
            (
                "a `var`'s type changed",
                with(" var ferrionMeasure = 1; "),
                with(" var ferrionMeasure = 1L; "),
            ),
        ];
        for (label, first, second) in &edits {
            let (mut asks, mut alone, mut all, mut others) =
                (Vec::new(), Vec::new(), Vec::new(), 0);
            // The state before: the first text, attributed.
            set_and_ask(&engine, target, first.clone())?;
            for text in [second, first].into_iter().cycle().take(2 * REPETITIONS) {
                let (ask, attribution) = set_and_ask(&engine, target, text.clone())?;
                asks.push(ask);
                alone.push(attribution);
                let before = engine.stats().body_types.computed;
                let start = Instant::now();
                let snapshot = engine.snapshot();
                for set in snapshot.source_sets().map_err(cancelled)? {
                    for &file in snapshot.files(set).map_err(cancelled)?.iter() {
                        snapshot.body_types(file).map_err(cancelled)?;
                    }
                }
                all.push(start.elapsed().as_secs_f64() * 1e3);
                others = others.max(engine.stats().body_types.computed - before);
            }
            let range = |values: &[f64]| {
                let least = values.iter().copied().fold(f64::MAX, f64::min);
                let most = values.iter().copied().fold(0.0, f64::max);
                format!("{least:.0} to {most:.0} ms")
            };
            println!(
                "| {short} | {} | {label} | {} | {} | {} | {others} |",
                original.len(),
                range(&asks),
                range(&alone),
                range(&all),
            );
        }
        set_and_ask(&engine, target, original.clone())?;
    }
    Ok(true)
}

/// Gives `target` its text, and asks for what a language server would: everything project-wide,
/// the file's names, and its attribution. The time in all, and of attribution alone, in ms.
fn set_and_ask(engine: &Engine, target: FileId, text: Arc<str>) -> Result<(f64, f64)> {
    let start = Instant::now();
    engine.set_text(target, text);
    let snapshot: Snapshot = engine.snapshot();
    snapshot.run_symbols(4).map_err(cancelled)?;
    snapshot.file_names(target).map_err(cancelled)?;
    let names = start.elapsed().as_secs_f64();
    snapshot.body_types(target).map_err(cancelled)?;
    let all = start.elapsed().as_secs_f64();
    Ok((all * 1e3, (all - names) * 1e3))
}

/// Where a statement can go at the start of the first method's body.
fn first_body(root: SyntaxNode<'_>) -> Option<usize> {
    for event in root.preorder() {
        if let WalkEvent::Enter(node) = event
            && node.kind() == SyntaxKind::MethodDecl
            && let Some(block) = node.children().find(|c| c.kind() == SyntaxKind::Block)
        {
            return Some(block.first_token().span().end as usize);
        }
    }
    None
}
