//! Deep inputs through resolution and attribution: the parser's deep-nesting stress patterns in
//! declarations and in bodies, and long chains of supertypes, resolved and attributed on a
//! 256 KiB stack, so that recursion
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

/// Every name in every file, every type's entry, and every file's attribution, at the current
/// revision.
fn resolve_everything(engine: &Engine) -> usize {
    let snapshot = engine.snapshot();
    snapshot.run_batch(1, |_| Ok(())).unwrap();
    let mut names = 0;
    for set in snapshot.source_sets().unwrap() {
        for &file in snapshot.files(set).unwrap().iter() {
            names += snapshot.file_names(file).unwrap().names.len();
            names += snapshot.body_types(file).unwrap().types.len();
        }
    }
    names
}

/// A stress pattern: its input with `n` repetitions.
type Generate = Box<dyn Fn(usize) -> String>;

/// The parser's stress patterns that nest in bodies, and its chains, as a field's initializer
/// or a method's body, and nesting that only bodies have: local classes, pattern bindings,
/// and scopes. Ten times the input must take well under a hundred times as long.
#[test]
fn bodies_of_stress_patterns() {
    let nest = |open: &str, middle: &str, close: &str, n: usize| {
        open.repeat(n) + middle + &close.repeat(n)
    };
    let field = |ty: &str, init: String| format!("class A {{ int a; {ty} x = {init}; }}\n");
    let method = |body: String| format!("class A {{ int a; void m(Object o) {{ {body} }} }}\n");
    let patterns: Vec<(&str, Generate)> = vec![
        (
            "parentheses",
            Box::new(move |n| field("int", nest("(", "a", ")", n))),
        ),
        (
            "blocks",
            Box::new(move |n| method(nest("{ int b; ", "b = a;", "}", n))),
        ),
        (
            "if statements",
            Box::new(move |n| method(nest("if (a > 0) ", "a++;", "", n))),
        ),
        (
            "anonymous classes",
            Box::new(move |n| field("Object", nest("new Object() { Object f = ", "a", "; }", n))),
        ),
        (
            "lambdas",
            Box::new(move |n| field("Object", nest("(Runnable) () -> ", "a", "", n))),
        ),
        (
            "lambda blocks",
            Box::new(move |n| field("Runnable", nest("() -> { Runnable r = ", "null", "; }", n))),
        ),
        (
            "conditionals",
            Box::new(move |n| field("int", nest("a > 0 ? ", "a", " : a", n))),
        ),
        (
            "switch expressions",
            Box::new(move |n| field("int", nest("switch (a) { default -> ", "a", "; }", n))),
        ),
        (
            "call arguments",
            Box::new(move |n| field("int", nest("f(", "a", ")", n))),
        ),
        (
            "local classes",
            Box::new(move |n| method(nest("class L { int b; void n() { ", "b = a;", "} }", n))),
        ),
        (
            "negated patterns",
            Box::new(move |n| field("boolean", nest("!(", "o instanceof String s", ")", n))),
        ),
        (
            "plus",
            Box::new(move |n| field("int", "a + ".repeat(n * 4) + "a")),
        ),
        (
            "method calls",
            Box::new(move |n| field("Object", "a".to_owned() + &".b()".repeat(n * 4))),
        ),
        (
            "field accesses",
            Box::new(move |n| field("Object", "a".to_owned() + &".b".repeat(n * 4))),
        ),
        (
            "qualified names",
            Box::new(move |n| field("Object", "java".to_owned() + &".b".repeat(n * 4))),
        ),
        (
            "array accesses",
            Box::new(move |n| field("Object", "a".to_owned() + &"[0]".repeat(n * 4))),
        ),
        (
            "else if",
            Box::new(move |n| {
                method("if (a > 0) {}".to_owned() + &" else if (a > 0) {}".repeat(n * 4))
            }),
        ),
        (
            "statements",
            Box::new(move |n| method("a = 1; ".repeat(n * 4))),
        ),
        (
            "generic locals",
            Box::new(move |n| method(nest("java.util.List<", "String", ">", n) + " l = null;")),
        ),
        (
            "string concatenations",
            Box::new(move |n| field("String", "\"ab\" + ".repeat(n * 4) + "a")),
        ),
        (
            "casts",
            Box::new(move |n| field("int", "(int) ".repeat(n) + "a")),
        ),
        (
            "pattern conjunctions",
            Box::new(move |n| {
                let tests: Vec<String> = (0..n / 10)
                    .map(|i| format!("o instanceof String s{i}"))
                    .collect();
                method(format!("if ({}) {{ }}", tests.join(" && ")))
            }),
        ),
    ];
    for (name, generate) in patterns {
        let small = generate(900);
        let large = generate(9_000);
        let name = name.to_owned();
        on_small_stack(&name.clone(), move || {
            let time = |text: String| {
                let engine = engine(vec![("p/A.java".to_owned(), text)]);
                let start = Instant::now();
                resolve_everything(&engine);
                start.elapsed()
            };
            let (small, large) = (time(small), time(large));
            assert!(
                large < small * 60 + std::time::Duration::from_millis(200),
                "{name}: {small:?} for the small input, {large:?} for ten times it"
            );
        });
    }
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

    // The member table of a class at the bottom of a long chain, through every table above it.
    const TABLES: usize = 20_000;
    let mut tables = String::from("package p;\n");
    for i in (1..TABLES).rev() {
        tables.push_str(&format!("class C{i} extends C{} {{}}\n", i - 1));
    }
    tables.push_str("class C0 { int f; void m() {} }\n");
    on_small_stack("the member table of a long chain", move || {
        let engine = engine(vec![("p/C.java".to_owned(), tables)]);
        let snapshot = engine.snapshot();
        let view = snapshot.source_sets().unwrap()[0];
        let mut cx = snapshot.types(view);
        let key = cx.lang(&format!("p/C{}", TABLES - 1)).unwrap().unwrap();
        let members = snapshot.members(view, key).unwrap();
        let f = snapshot.names().intern("f");
        assert!(members.field(f).is_some());
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

    // A long cycle, each of whose types records the others, so its cost is the square of its
    // length.
    const CYCLE: usize = 2_000;
    let mut cycle = String::from("package p;\n");
    for i in 0..CYCLE {
        cycle.push_str(&format!("class D{i} extends D{} {{}}\n", (i + 1) % CYCLE));
    }
    on_small_stack("a long cycle", move || {
        let engine = engine(vec![("p/D.java".to_owned(), cycle)]);
        resolve_everything(&engine);
        let snapshot = engine.snapshot();
        let file = snapshot.files(snapshot.source_sets().unwrap()[0]).unwrap()[0];
        let key = snapshot.type_key(file, 0).unwrap();
        assert_eq!(
            snapshot.supertypes(key).unwrap().unwrap().cycle.len(),
            CYCLE
        );
    });
}
