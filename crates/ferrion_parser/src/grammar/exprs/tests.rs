//! Expressions on their own, before the statement grammar can hold them: snapshots of fragments
//! in `tests/snapshots/exprs/fragments/` (see `grammar/types/tests.rs` for the harness), and long
//! chains and deep nesting on a small stack.

use ferrion_java_version::LATEST_RELEASE;
use ferrion_syntax::SyntaxKind::{self as K, *};

use crate::Parse;
use crate::diagnostic::ParseDiagnosticKind as D;
use crate::grammar::types::tests::{Fragment, check_snapshots, on_small_stack, run};

const EXPR: Fragment = Fragment {
    grammar: super::expr,
    missing: D::ExpressionExpected,
};

fn expr(text: &str) -> Parse {
    run(text, LATEST_RELEASE, EXPR)
}

fn kinds(parsed: &Parse) -> Vec<K> {
    parsed.tree.nodes().map(|n| n.kind()).collect()
}

fn diagnostics(parsed: &Parse) -> Vec<(D, u32)> {
    parsed
        .diagnostics
        .iter()
        .map(|d| (d.kind, d.anchor))
        .collect()
}

#[test]
fn fragment_snapshots() {
    check_snapshots("exprs/fragments", "expr", EXPR);
}

// ----- Precedence and chains -----

#[test]
fn binary_operators_nest_by_precedence() {
    let parsed = expr("a || b && c | d ^ e & f == g < h << i + j * k");
    // Each operator's right operand is the tighter one: one BinaryExpr per operator, each the
    // right child of the one before.
    let binaries = kinds(&parsed).iter().filter(|&&k| k == BinaryExpr).count();
    assert_eq!(binaries, 10);
    let root = parsed.tree.root().first_child().unwrap();
    assert_eq!(root.kind(), BinaryExpr);
    assert_eq!(root.first_child().unwrap().text(), "a");
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn shifts_are_runs_of_adjacent_greater_thans() {
    let parsed = expr("a >> b >>> c > d >= e");
    assert!(parsed.diagnostics.is_empty());
    // `>` binds less tightly than the shifts: ((a >> b) >>> c) > d, then >= e.
    let root = parsed.tree.root().first_child().unwrap();
    assert_eq!(root.text(), "a >> b >>> c > d >= e");
    assert_eq!(root.first_child().unwrap().text(), "a >> b >>> c > d");
    let parsed = expr("a > > b");
    assert_eq!(diagnostics(&parsed), [(D::ExpressionExpected(Gt), 4)]);
}

#[test]
fn compound_shift_assignments() {
    for text in ["a >>= 1", "a >>>= 1"] {
        let parsed = expr(text);
        assert!(parsed.diagnostics.is_empty(), "{text}");
        assert_eq!(kinds(&parsed)[1], AssignExpr, "{text}");
    }
}

#[test]
fn assignments_and_conditionals_nest_to_the_right() {
    let parsed = expr("a = b += c ? d : e ? f : g");
    assert!(parsed.diagnostics.is_empty());
    let root = parsed.tree.root().first_child().unwrap();
    assert_eq!(root.kind(), AssignExpr);
    let value = root.children().nth(1).unwrap();
    assert_eq!(value.kind(), AssignExpr);
    let conditional = value.children().nth(1).unwrap();
    assert_eq!(conditional.text(), "c ? d : e ? f : g");
    assert_eq!(conditional.children().nth(2).unwrap().text(), "e ? f : g");
}

fn deep(text: String) {
    on_small_stack(move || {
        let parsed = expr(&text);
        assert!(
            parsed.diagnostics.is_empty(),
            "{:?}",
            &parsed.diagnostics[..1]
        );
        drop(parsed);
    });
}

#[test]
fn long_chains_parse_in_loops() {
    let n = 100_000;
    deep(format!("x{}", " + x".repeat(n)));
    deep(format!("x{}", ".m(1)".repeat(n)));
    deep(format!("x{}", ".f[0]".repeat(n)));
    deep(format!("{}x", "- ".repeat(n)));
    deep(format!("{}x", "(T) ".repeat(n)));
    deep(format!("x{}", " = x".repeat(n)));
    deep(format!("x{}", " ? x : x".repeat(n)));
    deep(format!("x{}", " instanceof T == true".repeat(n)));
}

#[test]
fn deep_nesting_on_a_small_stack() {
    let n = 100_000;
    for text in [
        format!("{}x{}", "(".repeat(n), ")".repeat(n)),
        format!("{}x{}", "f(".repeat(n), ")".repeat(n)),
        format!("{}x", "y -> ".repeat(n)),
        format!("{}x{}", "a ? ".repeat(n), " : b".repeat(n)),
        format!("new int[] {}1{}", "{".repeat(n), "}".repeat(n)),
        format!("(List{}x{}) y", "<List".repeat(n), ">".repeat(n)),
        format!("{}x{}", "new A(".repeat(n), ")".repeat(n)),
    ] {
        on_small_stack(move || {
            let parsed = expr(&text);
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .all(|d| d.kind == D::NestingTooDeep),
                "{:?}",
                &parsed.diagnostics[..1]
            );
            assert_eq!(parsed.diagnostics.len(), 1);
        });
    }
}

#[test]
fn nesting_below_the_limit_has_no_error() {
    deep(format!("{}x{}", "(".repeat(5_000), ")".repeat(5_000)));
    deep(format!(
        "(List{}x{}) y",
        "<List".repeat(3_000),
        ">".repeat(3_000)
    ));
}
