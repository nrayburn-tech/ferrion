//! The engine: what it recomputes after each kind of edit, cancellation, the tree cache, and the
//! differential check against a fresh engine.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ferrion_semantic::invariants::check_edits;
use ferrion_semantic::{
    Cancelled, Engine, EngineOptions, FileId, FileInput, ProjectInput, SourceSetId, SourceSetInput,
    Stats, TableStats,
};

const MAIN: SourceSetId = SourceSetId::from_index(0);

fn project(files: &[(&str, &str)]) -> ProjectInput {
    ProjectInput {
        source_sets: vec![SourceSetInput {
            name: "main".to_owned(),
            release: 21,
            classpath: Vec::new(),
            platform: None,
            files: files
                .iter()
                .map(|(path, text)| FileInput::new(*path, *text))
                .collect(),
        }],
    }
}

/// Three packages, so an edit to one can be seen leaving the others alone.
fn three_packages() -> Engine {
    Engine::new(project(&[
        (
            "a/A.java",
            "package a;\nclass A {\n    void m() { int x = 1; }\n}\n",
        ),
        ("a/A2.java", "package a;\nclass A2 {}\n"),
        ("b/B.java", "package b;\nclass B {}\n"),
        ("c/C.java", "package c;\nclass C {}\n"),
    ]))
}

/// Asks for everything: every summary, the index, and every package entry.
fn ask_everything(engine: &Engine) -> Stats {
    let before = engine.stats();
    let snapshot = engine.snapshot();
    for &file in snapshot.files(MAIN).unwrap().iter() {
        snapshot.summary(file).unwrap();
    }
    let index = snapshot.package_index(MAIN).unwrap();
    for name in index.names() {
        snapshot.package(MAIN, name).unwrap().unwrap();
    }
    engine.stats() - before
}

fn file(engine: &Engine, n: usize) -> FileId {
    engine.snapshot().files(MAIN).unwrap()[n]
}

fn computed(n: u64, unchanged: u64, verified: u64) -> TableStats {
    TableStats {
        computed: n,
        unchanged,
        verified,
    }
}

#[test]
fn a_cold_run_computes_everything_once() {
    let engine = three_packages();
    let stats = ask_everything(&engine);
    assert_eq!(stats.parses, 4);
    assert_eq!(stats.summaries, computed(4, 0, 0));
    assert_eq!(stats.package_indexes, computed(1, 0, 0));
    assert_eq!(stats.packages, computed(3, 0, 0));
    // Asking again at the same revision does nothing.
    assert_eq!(ask_everything(&engine), Stats::default());
}

#[test]
fn a_body_edit_stops_at_the_summary() {
    let engine = three_packages();
    ask_everything(&engine);
    let a = file(&engine, 0);
    engine.set_text(
        a,
        Arc::from("package a;\nclass A {\n    void m() { int x = 2; x++; }\n}\n"),
    );
    let stats = ask_everything(&engine);
    assert_eq!(stats.parses, 1);
    // The edited file's summary is computed again, comes out equal, and is cut off; the others
    // are verified by their versions.
    assert_eq!(stats.summaries, computed(1, 1, 3));
    assert_eq!(stats.package_indexes, computed(0, 0, 1));
    assert_eq!(stats.packages, computed(0, 0, 3));
}

#[test]
fn a_signature_edit_doesnt_reach_the_package_table() {
    let engine = three_packages();
    ask_everything(&engine);
    let a = file(&engine, 0);
    engine.set_text(
        a,
        Arc::from("package a;\nclass A {\n    void renamed() {}\n}\n"),
    );
    let stats = ask_everything(&engine);
    // The summary changed, but not its package or top-level types, which is all the package
    // table reads.
    assert_eq!(stats.summaries, computed(1, 0, 3));
    assert_eq!(stats.package_indexes, computed(0, 0, 1));
    assert_eq!(stats.packages, computed(0, 0, 3));
}

#[test]
fn a_new_type_recomputes_its_package_only() {
    let engine = three_packages();
    ask_everything(&engine);
    let a = file(&engine, 0);
    engine.set_text(a, Arc::from("package a;\nclass A {}\nclass Another {}\n"));
    let stats = ask_everything(&engine);
    assert_eq!(stats.summaries, computed(1, 0, 3));
    // The index is computed again, since a file's top-level part changed, but its packages and
    // files didn't, so it comes out equal and package entries see the same files for theirs.
    assert_eq!(stats.package_indexes, computed(1, 1, 0));
    // Only `a` is computed again; `b` and `c` are verified without computing.
    assert_eq!(stats.packages, computed(1, 0, 2));
    let package = engine
        .snapshot()
        .package(MAIN, engine.names().intern("a"))
        .unwrap()
        .unwrap();
    let mut names: Vec<String> = package
        .types()
        .iter()
        .map(|t| engine.names().text(t.name).to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["A", "A2", "Another"]);
}

#[test]
fn an_unchanged_package_entry_is_cut_off() {
    let engine = three_packages();
    ask_everything(&engine);
    // Moving a type between two files of a package changes the package's entry; moving it back
    // in two steps, the second step's entry is computed again and equals the first's.
    let (a, a2) = (file(&engine, 0), file(&engine, 1));
    engine.set_text(a, Arc::from("package a;\nclass A {}\nclass A2 {}\n"));
    engine.set_text(a2, Arc::from("package a;\n"));
    let first = ask_everything(&engine);
    assert_eq!(first.packages, computed(1, 0, 2));
    // A comment changes the text, not the summary: nothing past it.
    engine.set_text(a2, Arc::from("package a;\n// empty\n"));
    let second = ask_everything(&engine);
    assert_eq!(second.summaries, computed(1, 1, 3));
    assert_eq!(second.packages, computed(0, 0, 3));
}

#[test]
fn a_new_file_recomputes_the_index_and_its_package() {
    let engine = three_packages();
    ask_everything(&engine);
    engine.add_file(
        MAIN,
        FileInput::new("b/B2.java", "package b;\nclass B2 {}\n"),
    );
    let stats = ask_everything(&engine);
    assert_eq!(stats.parses, 1);
    assert_eq!(stats.summaries, computed(1, 0, 4));
    assert_eq!(stats.package_indexes, computed(1, 0, 0));
    // The index changed, but only `b`'s files in it.
    assert_eq!(stats.packages, computed(1, 0, 2));

    // A new package, and a removed one.
    engine.add_file(MAIN, FileInput::new("d/D.java", "package d;\nclass D {}\n"));
    engine.remove_file(file(&engine, 3));
    let snapshot = engine.snapshot();
    let index = snapshot.package_index(MAIN).unwrap();
    let mut names: Vec<String> = index
        .names()
        .map(|n| engine.names().text(n).to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["a", "b", "d"]);
    let name = |n: &str| engine.names().intern(n);
    assert!(snapshot.package(MAIN, name("c")).unwrap().is_none());
    assert!(snapshot.package(MAIN, name("d")).unwrap().is_some());
}

#[test]
fn a_release_change_reparses_its_source_set() {
    let engine = three_packages();
    ask_everything(&engine);
    engine.set_release(MAIN, 8);
    let stats = ask_everything(&engine);
    assert_eq!(stats.parses, 4);
    assert_eq!(stats.summaries, computed(4, 4, 0));
    let tree = engine.snapshot().tree(file(&engine, 0)).unwrap();
    assert_eq!(tree.release(), 8);
}

#[test]
fn duplicate_types_are_ambiguous() {
    let engine = Engine::new(project(&[
        ("r1/p/A.java", "package p;\npublic class A {}\n"),
        (
            "r2/p/A.java",
            "package p;\npublic class A {}\nclass Only {}\n",
        ),
    ]));
    let snapshot = engine.snapshot();
    let name = |n: &str| engine.names().intern(n);
    let package = snapshot.package(MAIN, name("p")).unwrap().unwrap();
    assert!(package.get(name("A")).unwrap().is_ambiguous());
    assert!(!package.get(name("Only")).unwrap().is_ambiguous());
    assert!(package.get(name("Missing")).is_none());
}

#[test]
fn queries_are_cancelled_once_the_revision_moves() {
    let engine = three_packages();
    let snapshot = engine.snapshot();
    let a = file(&engine, 0);
    snapshot.summary(a).unwrap();
    engine.set_text(a, Arc::from("package a;\n"));
    assert_eq!(snapshot.summary(a), Err(Cancelled));
    assert_eq!(snapshot.package_index(MAIN).err(), Some(Cancelled));
    assert_eq!(snapshot.check(), Err(Cancelled));
    // A new snapshot sees the change.
    let summary = engine.snapshot().summary(a).unwrap();
    assert!(summary.types.is_empty());
}

/// Many files, so that a batch runs long enough to be interrupted.
fn large_project(files: usize) -> ProjectInput {
    let mut inputs = Vec::new();
    for i in 0..files {
        let mut text = format!("package p{};\nclass C{i} {{\n", i % 20);
        for m in 0..40 {
            text.push_str(&format!(
                "    int m{m}(int a) {{ return a + {m} * (a - 1); }}\n"
            ));
        }
        text.push_str("}\n");
        inputs.push(FileInput::new(format!("p/C{i}.java"), text));
    }
    ProjectInput {
        source_sets: vec![SourceSetInput {
            name: "main".to_owned(),
            release: 21,
            classpath: Vec::new(),
            platform: None,
            files: inputs,
        }],
    }
}

#[test]
fn an_edit_doesnt_wait_for_a_running_batch() {
    let engine = Arc::new(Engine::new(large_project(2_000)));
    let started = Arc::new(AtomicUsize::new(0));
    let runner = {
        let (engine, started) = (engine.clone(), started.clone());
        std::thread::spawn(move || {
            let snapshot = engine.snapshot();
            snapshot.run_batch(2, |_| {
                started.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
        })
    };
    // Wait until the batch is under way, in its first phase if the machine is fast.
    let deadline = Instant::now() + Duration::from_secs(10);
    while engine.stats().summaries.computed < 50 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let a = engine.snapshot().files(SourceSetId::from_index(0)).unwrap()[0];
    let set = Instant::now();
    engine.set_text(a, Arc::from("package p0;\nclass C0 {}\n"));
    let setter = set.elapsed();
    let result = runner.join().unwrap();
    let stopped = set.elapsed();
    // The setter returns at once, and the batch stops soon after, without running every file.
    assert!(
        setter < Duration::from_millis(50),
        "the setter took {setter:?}"
    );
    assert_eq!(result, Err(Cancelled));
    assert!(
        stopped < Duration::from_secs(2),
        "the batch took {stopped:?} to stop"
    );
    assert!(started.load(Ordering::Relaxed) < 2_000);
    // What it stored is still right: a batch at the new revision completes and agrees.
    let snapshot = engine.snapshot();
    snapshot.run_batch(2, |_| Ok(())).unwrap();
    assert!(snapshot.summary(a).unwrap().types[0].methods.is_empty());
}

#[test]
fn a_batch_runs_each_file_once_on_any_number_of_threads() {
    for threads in [1, 3, 8] {
        let engine = Engine::new(large_project(100));
        let seen = AtomicUsize::new(0);
        let snapshot = engine.snapshot();
        snapshot
            .run_batch(threads, |_| {
                seen.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .unwrap();
        assert_eq!(seen.load(Ordering::Relaxed), 100);
        assert_eq!(snapshot.package_index(MAIN).unwrap().len(), 20);
        // A callback that stops the batch stops it.
        let calls = AtomicUsize::new(0);
        let result = snapshot.run_batch(threads, |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(Cancelled)
        });
        assert_eq!(result, Err(Cancelled));
        assert!(calls.load(Ordering::Relaxed) <= threads);
    }
}

#[test]
fn the_tree_cache_keeps_to_its_budget() {
    let input = large_project(50);
    let engine = Engine::with_options(
        input,
        EngineOptions {
            tree_cache_bytes: 200 << 10,
        },
    );
    let snapshot = engine.snapshot();
    snapshot.run_batch(1, |_| Ok(())).unwrap();
    let sizes = engine.sizes();
    assert!(sizes.tree_bytes <= 200 << 10, "{sizes:?}");
    assert!(sizes.trees > 0 && sizes.trees < 50, "{sizes:?}");
    assert_eq!(sizes.summaries, 50);
    // The last phase reads the trees last file first, for their names: the first file's tree
    // is kept and shared, and the last one's was evicted and is parsed again.
    let files = snapshot.files(MAIN).unwrap();
    let first = snapshot.tree(files[0]).unwrap();
    assert!(Arc::ptr_eq(&first, &snapshot.tree(files[0]).unwrap()));
    let before = engine.stats().parses;
    snapshot.tree(files[49]).unwrap();
    assert_eq!(engine.stats().parses, before + 1);
}

/// A splitmix64 stream, for choices that are random but repeat.
fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (z ^ (z >> 31)) as u8
        })
        .collect()
}

#[test]
fn random_edits_agree_with_a_fresh_engine() {
    for seed in 0..60 {
        let bytes = random_bytes(seed, 600);
        if let Err(problem) = check_edits(None, &bytes, false) {
            panic!("seed {seed}: {problem}");
        }
    }
}

/// The same, with a batch running at the old revision while each edit lands, so that entries
/// computed at a revision already gone are stored and later verified.
#[test]
fn random_edits_during_batches_agree_with_a_fresh_engine() {
    for seed in 100..120 {
        let bytes = random_bytes(seed, 600);
        if let Err(problem) = check_edits(None, &bytes, true) {
            panic!("seed {seed}: {problem}");
        }
    }
}

/// A file from the parser's deep-nesting stress patterns among the project's, edited like the
/// others.
#[test]
fn random_edits_with_a_deeply_nested_file() {
    let classes = "class C { ".repeat(5_000) + &"}".repeat(5_000);
    let generics = format!(
        "class G {{ {}String{} x; }}",
        "List<".repeat(5_000),
        ">".repeat(5_000)
    );
    for (seed, text) in [(200, classes), (201, generics)] {
        let bytes = random_bytes(seed, 300);
        if let Err(problem) = check_edits(Some(format!("package p;\n{text}")), &bytes, false) {
            panic!("seed {seed}: {problem}");
        }
    }
}
