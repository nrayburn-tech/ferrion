//! Attribution's entries and edits: what a body edit, a constant's edit, and a signature edit
//! recompute of member tables, constants, and files' attribution, and that what is kept agrees
//! with a fresh engine's.

use std::sync::Arc;

use ferrion_semantic::{
    Engine, FileId, FileInput, ProjectInput, SourceSetId, SourceSetInput, Stats,
};

const MAIN: SourceSetId = SourceSetId::from_index(0);

fn project(files: &[(&str, &str)]) -> ProjectInput {
    ProjectInput {
        source_sets: vec![SourceSetInput {
            name: "main".to_owned(),
            release: 21,
            classpath: Vec::new(),
            platform: Some(ferrion_semantic::testing::stub_platform()),
            files: files
                .iter()
                .map(|(path, text)| FileInput::new(*path, *text))
                .collect(),
        }],
    }
}

const FILES: &[(&str, &str)] = &[
    (
        "p/A.java",
        "package p;\npublic class A {\n    public static final int X = B.Y + 1;\n    public int f;\n    void m() { int a = 1; }\n}\n",
    ),
    (
        "p/B.java",
        "package p;\nclass B extends A {\n    static final int Y = 41;\n    int g() { return f + X; }\n}\n",
    ),
    (
        "q/C.java",
        "package q;\nclass C {\n    int h(p.A a, p.A[] as) { return a.f + p.A.X + as[0].f; }\n}\n",
    ),
];

fn file(engine: &Engine, n: usize) -> FileId {
    engine.snapshot().files(MAIN).unwrap()[n]
}

/// Attributes every file, and gives each file's attribution as text.
fn attribute(engine: &Engine) -> (Stats, Vec<String>) {
    let before = engine.stats();
    let snapshot = engine.snapshot();
    let mut dumps = Vec::new();
    for &file in snapshot.files(MAIN).unwrap().iter() {
        dumps.push(snapshot.body_types_dump(file).unwrap());
    }
    (engine.stats() - before, dumps)
}

#[test]
fn a_cold_run_folds_constants_across_files() {
    let engine = Engine::new(project(FILES));
    let (stats, dumps) = attribute(&engine);
    assert_eq!(stats.body_types.computed, 3);
    assert!(dumps[2].contains("`p.A.X`: int = int 42"), "{}", dumps[2]);
    assert!(dumps[1].contains("`f + X`: int\n"), "{}", dumps[1]);
    assert!(dumps[1].contains("`X`: int = int 42"));
}

#[test]
fn a_body_edit_recomputes_its_own_file_only() {
    let engine = Engine::new(project(FILES));
    attribute(&engine);
    let a = file(&engine, 0);
    engine.set_text(
        a,
        Arc::from(
            "package p;\npublic class A {\n    public static final int X = B.Y + 1;\n    public int f;\n    void m() { int a = 2; a++; }\n}\n",
        ),
    );
    let (stats, dumps) = attribute(&engine);
    assert_eq!(stats.body_types.computed, 1);
    assert_eq!(stats.body_types.verified, 2);
    // `A.X`'s value reads `A`'s summary, which a body edit leaves alone.
    assert_eq!(stats.constants.computed, 0);
    assert_eq!(stats.members.computed, 0);
    let fresh = Engine::new(project(&[
        (
            "p/A.java",
            "package p;\npublic class A {\n    public static final int X = B.Y + 1;\n    public int f;\n    void m() { int a = 2; a++; }\n}\n",
        ),
        FILES[1],
        FILES[2],
    ]));
    assert_eq!(dumps, attribute(&fresh).1);
}

#[test]
fn a_constant_edit_reaches_the_files_that_use_it() {
    let engine = Engine::new(project(FILES));
    attribute(&engine);
    let b = file(&engine, 1);
    engine.set_text(
        b,
        Arc::from(
            "package p;\nclass B extends A {\n    static final int Y = 9;\n    int g() { return f + X; }\n}\n",
        ),
    );
    let (stats, dumps) = attribute(&engine);
    assert_eq!(stats.body_types.computed, 3);
    assert!(dumps[2].contains("`p.A.X`: int = int 10"), "{}", dumps[2]);
}

#[test]
fn a_signature_edit_that_changes_no_table_is_cut_off() {
    let engine = Engine::new(project(FILES));
    attribute(&engine);
    let a = file(&engine, 0);
    // A new method in `A` changes its entry and its table, which `C`'s attribution reads for
    // `as[0].f`.
    engine.set_text(
        a,
        Arc::from(
            "package p;\npublic class A {\n    public static final int X = B.Y + 1;\n    public int f;\n    void m() { int a = 1; }\n    void n() {}\n}\n",
        ),
    );
    let (stats, dumps) = attribute(&engine);
    assert_eq!(stats.members.computed, 1);
    assert!(dumps[2].contains("`as[0].f`: int"), "{}", dumps[2]);
    let fresh = Engine::new(project(&[
        (
            "p/A.java",
            "package p;\npublic class A {\n    public static final int X = B.Y + 1;\n    public int f;\n    void m() { int a = 1; }\n    void n() {}\n}\n",
        ),
        FILES[1],
        FILES[2],
    ]));
    assert_eq!(dumps, attribute(&fresh).1);
}
