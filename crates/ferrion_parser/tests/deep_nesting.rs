//! Deeply nested input through `parse`, on a thread with a small stack, so that recursion while
//! parsing or dropping shows up as a stack overflow (docs/design/parser-deep-nesting.md). The
//! core's nesting limit is tested on its own in `src/parser/tests.rs`; these cover the grammar
//! written so far.

use std::sync::Arc;

use ferrion_parser::{ParseDiagnosticKind, ParseOptions, invariants, parse};

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn check(text: String) {
    on_small_stack(move || {
        let text: Arc<str> = Arc::from(text);
        let parsed = parse(text.clone(), ParseOptions { release: 27 });
        invariants::check(&text, 27, &parsed).unwrap();
        drop(parsed);
    });
}

#[test]
fn nested_parentheses() {
    let depth = 100_000;
    check(format!(
        "import a.b;\nclass A {{ int x = {}1{}; }}\n",
        "(".repeat(depth),
        ")".repeat(depth)
    ));
}

#[test]
fn nested_annotation_arguments() {
    let depth = 100_000;
    check(format!(
        "@A({}1{}) package p;\n",
        "(".repeat(depth),
        ")".repeat(depth)
    ));
    check(format!(
        "@A({}) package p;\n",
        "@A(".repeat(depth) + &")".repeat(depth)
    ));
}

#[test]
fn nested_blocks() {
    for depth in [1_000, 100_000] {
        check(format!(
            "class A {{ void m() {{ {}{} }} }}\n",
            "{".repeat(depth),
            "}".repeat(depth)
        ));
    }
}

#[test]
fn nested_statements() {
    let depth = 100_000;
    check(format!(
        "class A {{ void m() {{ {} ; }} }}\n",
        "if (a) while (b) ".repeat(depth)
    ));
}

#[test]
fn long_else_if_chain() {
    // A chain, not nesting: the nesting limit doesn't apply.
    let length = 10_000;
    let text = format!(
        "class A {{ void m(int a) {{ if (a == 0) ; {} else ; }} }}\n",
        "else if (a == 1) ; ".repeat(length)
    );
    on_small_stack(move || {
        let text: Arc<str> = Arc::from(text);
        let parsed = parse(text.clone(), ParseOptions { release: 27 });
        invariants::check(&text, 27, &parsed).unwrap();
        assert!(
            !parsed
                .diagnostics
                .iter()
                .any(|d| d.kind == ParseDiagnosticKind::NestingTooDeep)
        );
    });
}

#[test]
fn nested_switches() {
    let depth = 100_000;
    check(format!(
        "class A {{ void m() {{ {} ; {} }} }}\n",
        "switch (a) { case 1 -> { ".repeat(depth),
        "} }".repeat(depth)
    ));
}

#[test]
fn nested_record_patterns() {
    let depth = 100_000;
    check(format!(
        "class A {{ void m() {{ switch (o) {{ case {}var x{} -> {{ }} }} }} }}\n",
        "R(".repeat(depth),
        ")".repeat(depth)
    ));
}

#[test]
fn unclosed_delimiters() {
    check(format!("@A({} package p;\n", "(".repeat(100_000)));
    check("{".repeat(100_000));
}

#[test]
fn expression_chains() {
    let n = 100_000;
    let chains = [
        " + x".repeat(n),
        ".m(1)".repeat(n),
        "[0]".repeat(n),
        " = x".repeat(n),
        " ? x : x".repeat(n),
    ];
    for chain in chains {
        check(format!("class A {{ Object f = x{chain}; }}\n"));
    }
    check(format!("class A {{ int f = {}x; }}\n", "- ".repeat(n)));
    check(format!("class A {{ Object f = {}x; }}\n", "(T) ".repeat(n)));
}

#[test]
fn nested_expressions() {
    let n = 100_000;
    for (open, close) in [
        ("f(", ")"),
        ("new A(", ")"),
        ("x -> ", ""),
        ("a ? ", " : b"),
        ("{", "}"),
    ] {
        check(format!(
            "class A {{ Object f = {}x{}; }}\n",
            open.repeat(n),
            close.repeat(n)
        ));
    }
}

#[test]
fn nested_type_arguments() {
    let n = 100_000;
    check(format!(
        "class A {{ List{}x{} f; }}\n",
        "<List".repeat(n),
        ">".repeat(n)
    ));
    check(format!(
        "class A {{ Object f = (List{}x{}) y; }}\n",
        "<List".repeat(n),
        ">".repeat(n)
    ));
}
