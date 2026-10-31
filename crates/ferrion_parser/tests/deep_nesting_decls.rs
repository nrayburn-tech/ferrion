//! Deeply nested declarations through `parse`, on a thread with a small stack, as in
//! `deep_nesting.rs`: classes in classes, enum constant bodies, and element value arrays.

use std::sync::Arc;

use ferrion_parser::{ParseOptions, invariants, parse};

fn check(text: String) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let text: Arc<str> = Arc::from(text);
            let parsed = parse(text.clone(), ParseOptions { release: 27 });
            invariants::check(&text, 27, &parsed).unwrap();
            drop(parsed);
        })
        .unwrap()
        .join()
        .unwrap();
}

const DEPTH: usize = 100_000;

#[test]
fn nested_classes() {
    check(format!(
        "{}{}",
        "class A {".repeat(DEPTH),
        "}".repeat(DEPTH)
    ));
    check(format!(
        "{}{}",
        "public static class A { int x; ".repeat(DEPTH),
        "}".repeat(DEPTH)
    ));
}

#[test]
fn nested_enum_constant_bodies() {
    check(format!(
        "{}{}",
        "enum E { A { ".repeat(DEPTH),
        "} }".repeat(DEPTH)
    ));
}

#[test]
fn nested_element_value_arrays() {
    check(format!(
        "@A({}1{}) class B {{}}",
        "{".repeat(DEPTH),
        "}".repeat(DEPTH)
    ));
    check(format!("@A(x = {}) class B {{}}", "{@A(y = ".repeat(DEPTH)));
}

#[test]
fn unclosed_declarations() {
    check("class A {".repeat(DEPTH));
    check("module m {".repeat(DEPTH));
    check("enum E { A(".repeat(DEPTH));
    check("@interface A { int v() default ".repeat(DEPTH));
}
