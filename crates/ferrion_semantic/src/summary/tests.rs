use std::sync::Arc;

use ferrion_parser::{ParseOptions, parse};

use super::*;

fn summary_of(text: &str) -> Summary {
    let tree = parse(Arc::from(text), ParseOptions { release: 27 }).tree;
    summarize(&tree, false)
}

const BASE: &str = r#"package p;

import java.util.List;

/** A class. */
public class A<T> extends B implements C {
    static final int LIMIT = 10;
    int count = 0;
    Runnable task = () -> { System.out.println(); };
    Object anonymous = new Object() { int hidden; };

    static { System.out.println("static"); }

    { count = 1; }

    A() { super(); }

    @Deprecated(since = "1")
    public <R> List<R> map(T value, int... rest) throws Exception {
        int local = 1;
        class Local { void m() {} }
        return null;
    }

    enum E { X, Y { void z() {} } }

    @interface Q { int v() default 1; }
}
"#;

/// Replaces the one occurrence of `old` in [`BASE`] with `new`.
fn edit(old: &str, new: &str) -> String {
    assert_eq!(BASE.matches(old).count(), 1, "{old:?} isn't in BASE once");
    BASE.replacen(old, new, 1)
}

#[test]
fn body_edits_leave_the_summary_and_its_fingerprint_equal() {
    let base = summary_of(BASE);
    let edits = [
        // Statements in method, constructor, and initializer bodies.
        edit("int local = 1;", "int local = 2; local++;"),
        edit("super();", "super(); count = 3;"),
        edit("{ count = 1; }", "{ count = 1; count++; }"),
        edit("System.out.println(\"static\");", ""),
        // Local and anonymous classes.
        edit(
            "class Local { void m() {} }",
            "class Local { void m() {} int n; }",
        ),
        edit("int hidden;", "int hidden; void shown() {}"),
        // Lambda bodies and other initializers of fields that aren't final.
        edit("int count = 0;", "int count = 42;"),
        edit("System.out.println(); };", "}; "),
        // An enum constant's body, which is an anonymous class.
        edit("void z() {}", "void z() { int w; }"),
        // Comments, documentation, and whitespace, which move every offset after them.
        edit(
            "/** A class. */",
            "/** A class,\n * documented at length. */\n// and a comment\n",
        ),
        edit("package p;", "\n\n\npackage   p ;"),
        edit("A() {", "/* the constructor */ A()\n{"),
    ];
    for text in &edits {
        let edited = summary_of(text);
        assert_eq!(edited, base, "the summary changed after:\n{text}");
        assert_eq!(Fingerprint::of(&edited), Fingerprint::of(&base));
    }
}

#[test]
fn declaration_edits_change_the_summary() {
    let base = summary_of(BASE);
    let edits = [
        edit("map(T value", "flatMap(T value"),
        edit("int... rest", "long... rest"),
        edit("throws Exception", "throws java.io.IOException"),
        edit(
            "static final int LIMIT = 10;",
            "static final int LIMIT = 11;",
        ),
        edit("@Deprecated(since = \"1\")", "@Deprecated(since = \"2\")"),
        edit("int v() default 1;", "int v() default 2;"),
        edit("X, Y", "X, W, Y"),
        edit("X, Y {", "X { void x() {} }, Y {"),
        edit(
            "import java.util.List;",
            "import java.util.List;\nimport java.util.Map;",
        ),
        edit("public class A<T>", "public final class A<T>"),
        edit("extends B", "extends D"),
        edit("int count = 0;", "int count = 0, other;"),
        edit(
            "A() { super(); }",
            "A() { super(); }\n    A(int x) { this(); }",
        ),
    ];
    for text in &edits {
        let edited = summary_of(text);
        assert_ne!(edited, base, "the summary didn't change after:\n{text}");
        assert_ne!(Fingerprint::of(&edited), Fingerprint::of(&base));
    }
}

#[test]
fn paths_name_enclosing_types() {
    let summary = summary_of("class A { class B { interface C {} } enum D {} } class E {}");
    let paths: Vec<String> = (0..summary.types.len()).map(|i| summary.path(i)).collect();
    assert_eq!(paths, ["A", "A.B", "A.B.C", "A.D", "E"]);
    let top: Vec<&str> = summary.top_level_types().map(|t| &*t.name).collect();
    assert_eq!(top, ["A", "E"]);
}

#[test]
fn type_refs_print_as_written() {
    let summary = summary_of(
        "class A { java.util.Map.@X Entry<? extends K, int @Y [][]>[] f; Outer<A<B<C>>> g; }",
    );
    let types: Vec<String> = summary.types[0]
        .fields
        .iter()
        .map(|f| f.ty.to_string())
        .collect();
    assert_eq!(
        types,
        [
            "java.util.Map.@X Entry<? extends K, int @Y [][]>[]",
            "Outer<A<B<C>>>"
        ]
    );
}

/// The parser's deep-nesting stress patterns that reach declarations
/// (crates/ferrion_parser/tests/nesting_stress.rs), summarized, hashed, compared, printed where
/// that is linear, and dropped on a 256 KiB stack, so recursion anywhere overflows here.
#[test]
fn deep_nesting_doesnt_recurse() {
    let nest = |open: &str, middle: &str, close: &str, n: usize| {
        open.repeat(n) + middle + &close.repeat(n)
    };
    let cases: Vec<(&str, String, bool)> = vec![
        ("classes", nest("class C { ", "", "}", 9_000), false),
        (
            "classes past the limit",
            nest("class C { ", "", "}", 100_000),
            false,
        ),
        (
            "generic types",
            format!("class A {{ {} x; }}\n", nest("List<", "String", ">", 9_000)),
            true,
        ),
        (
            "generic types past the limit",
            format!(
                "class A {{ {} x; }}\n",
                nest("List<", "String", ">", 100_000)
            ),
            true,
        ),
        (
            "annotations",
            nest("@A(", "", ")", 9_000) + " package p;\n",
            true,
        ),
        (
            "qualified import",
            format!("import {}A;\n", "a.".repeat(500_000)),
            true,
        ),
        (
            "parentheses in a constant",
            format!(
                "class A {{ final int x = {}; }}\n",
                nest("(", "1", ")", 9_000)
            ),
            true,
        ),
        (
            "members",
            format!("class A {{ {}}}\n", "int x; ".repeat(100_000)),
            true,
        ),
    ];
    for (name, text, dump) in cases {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let tree = parse(Arc::from(text), ParseOptions { release: 27 }).tree;
                let summary = summarize(&tree, false);
                let copy = summary.clone();
                assert_eq!(copy, summary, "{name}");
                assert_eq!(Fingerprint::of(&copy), Fingerprint::of(&summary), "{name}");
                if dump {
                    assert!(!summary.debug_dump().is_empty(), "{name}");
                }
                drop(copy);
                drop(summary);
                drop(tree);
            })
            .unwrap()
            .join()
            .unwrap_or_else(|_| panic!("{name} failed"));
    }
}
