//! Deeply nested input through `parse`, on a thread with a small stack, so that recursion while
//! parsing or dropping shows up as a stack overflow (docs/design/parser-deep-nesting.md). The
//! core's nesting limit is tested on its own in `src/parser/tests.rs`; these cover the grammar
//! written so far.

use std::sync::Arc;

use ferrion_parser::{ParseOptions, invariants, parse};

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
fn unclosed_delimiters() {
    check(format!("@A({} package p;\n", "(".repeat(100_000)));
    check("{".repeat(100_000));
}
