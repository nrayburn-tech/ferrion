//! Deep inputs through resolution: the parser's deep-nesting stress patterns that reach
//! declarations, and long chains of supertypes, resolved on a 256 KiB stack, so that recursion
//! on the input's depth anywhere but under `ferrion_stack::ensure_stack` overflows here, and
//! work that grows faster than the input shows as a slow test.

use std::sync::Arc;
use std::time::Instant;

use ferrion_semantic::testing::stub_platform;
use ferrion_semantic::{Engine, FileInput, ProjectInput, SourceSetInput};

/// An engine over one source set with the stand-in platform and `files`.
fn engine(files: Vec<(String, String)>) -> Engine {
    let mut set = SourceSetInput::new("main", 27);
    set.platform = Some(stub_platform());
    set.files = files
        .into_iter()
        .map(|(path, text)| FileInput::new(path, text))
        .collect();
    Engine::new(ProjectInput {
        source_sets: vec![set],
    })
}

/// Runs `work` on a thread with a 256 KiB stack.
fn on_small_stack(name: &str, work: impl FnOnce() + Send + 'static) {
    let start = Instant::now();
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap_or_else(|_| panic!("{name} failed"));
    eprintln!("{name}: {:?}", start.elapsed());
}

/// Every name in every file, and every type's entry, at the current revision.
fn resolve_everything(engine: &Engine) -> usize {
    let snapshot = engine.snapshot();
    snapshot.run_batch(1, |_| Ok(())).unwrap();
    let mut names = 0;
    for set in snapshot.source_sets().unwrap() {
        for &file in snapshot.files(set).unwrap().iter() {
            names += snapshot.decl_names(file).unwrap().names.len();
        }
    }
    names
}

#[test]
fn nesting_stress_patterns() {
    let nest = |open: &str, middle: &str, close: &str, n: usize| {
        open.repeat(n) + middle + &close.repeat(n)
    };
    let cases: Vec<(&str, String)> = vec![
        ("classes", nest("class C { ", "", "}", 9_000)),
        (
            "classes past the limit",
            nest("class C { ", "", "}", 100_000),
        ),
        (
            "generic types",
            format!("class A {{ {} x; }}\n", nest("List<", "String", ">", 9_000)),
        ),
        (
            "generic types past the limit",
            format!(
                "class A {{ {} x; }}\n",
                nest("List<", "String", ">", 100_000)
            ),
        ),
        ("annotations", nest("@A(", "", ")", 9_000) + " class A {}\n"),
        (
            "qualified import",
            format!("import {}A;\nclass B {{}}\n", "a.".repeat(100_000)),
        ),
        (
            "qualified type",
            format!("class B {{ {}A x; }}\n", "a.".repeat(100_000)),
        ),
        (
            "members",
            format!("class A {{ {}}}\n", "List<String> x; ".repeat(50_000)),
        ),
        (
            "member types",
            format!("class A {{ {}}}\n", "class M extends A {} ".repeat(20_000)),
        ),
    ];
    for (name, text) in cases {
        on_small_stack(name, move || {
            let engine = engine(vec![("p/A.java".to_owned(), text)]);
            resolve_everything(&engine);
        });
    }
}

#[test]
fn long_chains_of_supertypes() {
    const N: usize = 100_000;
    // Most derived first, so the first entry asked for reads the whole chain.
    let mut down = String::from("package p;\n");
    for i in (1..N).rev() {
        down.push_str(&format!("class C{i} extends C{} {{}}\n", i - 1));
    }
    down.push_str("class C0 {}\n");
    on_small_stack("a chain declared most derived first", move || {
        let engine = engine(vec![("p/C.java".to_owned(), down)]);
        assert!(resolve_everything(&engine) >= N);
        // Changing the root reaches every entry below it through the chain, verified on the
        // same small stack.
        let file = engine
            .snapshot()
            .files(engine.snapshot().source_sets().unwrap()[0])
            .unwrap()[0];
        let text = engine.snapshot().text(file).unwrap();
        let edited = text.replace("class C0 {}", "class C0 implements Runnable {}");
        engine.set_text(file, Arc::from(edited));
        resolve_everything(&engine);
    });

    // One file per class, root first: each entry reads the one above it, already made.
    let files: Vec<(String, String)> = (0..N / 10)
        .map(|i| {
            let extends = if i == 0 {
                String::new()
            } else {
                format!(" extends C{}", i - 1)
            };
            (
                format!("p/C{i}.java"),
                format!("package p; class C{i}{extends} {{}}\n"),
            )
        })
        .collect();
    on_small_stack("a chain across files", move || {
        let engine = engine(files);
        resolve_everything(&engine);
    });

    // A cycle as long as the chain.
    let mut cycle = String::from("package p;\n");
    for i in 0..N / 10 {
        cycle.push_str(&format!(
            "class D{i} extends D{} {{}}\n",
            (i + 1) % (N / 10)
        ));
    }
    on_small_stack("a long cycle", move || {
        let engine = engine(vec![("p/D.java".to_owned(), cycle)]);
        resolve_everything(&engine);
        let snapshot = engine.snapshot();
        let file = snapshot.files(snapshot.source_sets().unwrap()[0]).unwrap()[0];
        let key = snapshot.type_key(file, 0).unwrap();
        assert_eq!(
            snapshot.supertypes(key).unwrap().unwrap().cycle.len(),
            N / 10
        );
    });
}
