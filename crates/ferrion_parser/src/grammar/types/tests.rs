//! Types on their own, before the declaration grammar can hold them, and the fragment harness
//! the expression tests share.
//!
//! Each `.type` file in `tests/snapshots/types/fragments/` holds types separated by `;`, and each
//! `.expr` file in `tests/snapshots/exprs/fragments/` expressions. The fragments are parsed one
//! after the other inside a compilation unit, and the tree and diagnostics are compared with the
//! `.tree` file next to each. A first line `// release: N` parses at release N. To write the
//! snapshots after an intended change, run with `FERRION_BLESS=1`, then review the diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_parser --lib fragment_snapshots
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_java_version::LATEST_RELEASE;
use ferrion_syntax::SyntaxKind::{self as K, *};

use crate::builder::Builder;
use crate::diagnostic::{Expected, ParseDiagnostic, ParseDiagnosticKind as D};
use crate::parser::{CompletedMarker, Parser, TokenSet};
use crate::{Parse, invariants};

/// A grammar function to parse fragments with, and the diagnostic for a fragment it can't start.
#[derive(Clone, Copy)]
pub(crate) struct Fragment {
    pub grammar: fn(&mut Parser<'_>) -> Option<CompletedMarker>,
    pub missing: fn(K) -> D,
}

pub(crate) const TYPE: Fragment = Fragment {
    grammar: super::type_,
    missing: D::TypeExpected,
};

/// Parses `text` as fragments separated by `;` inside a compilation unit, and checks the result.
pub(crate) fn run(text: &str, release: u32, fragment: Fragment) -> Parse {
    let text: Arc<str> = Arc::from(text);
    let lexed = ferrion_lexer::lex(&text);
    let mut p = Parser::new(&text, &lexed.tokens, release);
    let m = p.start();
    let mut progress = p.progress();
    while !p.at_eof() {
        if (fragment.grammar)(&mut p).is_none() {
            let span = p.nth_span(0);
            p.error((fragment.missing)(p.current()), span, span.start);
        }
        // Whatever the fragment left, up to the `;` that ends it.
        if !p.at_eof() && !p.eat(Semi) {
            p.error_expected(Expected::Token(Semi));
            if p.skip_until(TokenSet::new(&[Semi])).is_none() && !p.at(Semi) {
                p.error_and_bump();
            }
            p.eat(Semi);
        }
        if !progress.advanced(&mut p) {
            break;
        }
    }
    p.bump_eof();
    m.complete(&mut p, CompilationUnit);
    let (events, mut diagnostics) = p.finish();
    diagnostics.extend(lexed.errors.iter().map(ParseDiagnostic::lexical));
    diagnostics.sort_by_key(|d| d.span.start);
    let errors: Vec<_> = lexed.errors.iter().map(|e| e.span).collect();
    let tree = Builder::new(text.clone(), lexed.tokens, &errors, release).build(events);
    let parsed = Parse { tree, diagnostics };
    invariants::check(&text, release, &parsed).unwrap();
    parsed
}

/// Compares every `.{extension}` file in `tests/snapshots/{dir}` with its `.tree` file.
pub(crate) fn check_snapshots(dir: &str, extension: &str, fragment: Fragment) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let bless = std::env::var_os("FERRION_BLESS").is_some();
    let mut inputs: Vec<PathBuf> = fs::read_dir(root.join(dir))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no inputs in {dir}");
    let mut failures = Vec::new();
    for input in &inputs {
        let text = fs::read_to_string(input).unwrap();
        let release = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// release: "))
            .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
        let actual = run(&text, release, fragment).debug_dump();
        let expected_path = input.with_extension("tree");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{}: the tree differs from {}:\n{actual}",
                    input.strip_prefix(&root).unwrap().display(),
                    expected_path.display()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} snapshots failed (run with FERRION_BLESS=1 to update them):\n{}",
        failures.len(),
        inputs.len(),
        failures.join("\n")
    );
}

/// Runs `f` on a thread with a small stack, so that recursion where a loop belongs, or a missing
/// `ensure_stack`, overflows.
pub(crate) fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn fragment_snapshots() {
    check_snapshots("types/fragments", "type", TYPE);
}

#[test]
fn scan_type_finds_where_types_end() {
    let cases: &[(&str, Option<usize>)] = &[
        ("a", Some(1)),
        ("a.b.c x", Some(5)),
        ("int[][] x", Some(5)),
        ("java.util.Map<K, List<V>>.Entry x", Some(15)),
        ("@A(x = (1)) a.@B b<@C ?> @D [] x", Some(23)),
        ("a < b", Some(1)),
        ("a<b, c >> d", Some(6)),
        ("a[0]", Some(1)),
        ("a.b.", Some(3)),
        ("a.this", Some(1)),
        ("void", None),
        ("1", None),
    ];
    for &(text, end) in cases {
        let text: Arc<str> = Arc::from(text);
        let lexed = ferrion_lexer::lex(&text);
        let p = Parser::new(&text, &lexed.tokens, LATEST_RELEASE);
        assert_eq!(super::scan_type(&p, 0), end, "{text}");
    }
}

#[test]
fn nested_type_arguments_on_a_small_stack() {
    on_small_stack(|| {
        let depth = 5_000;
        let text = format!("List{}x{}", "<List".repeat(depth), ">".repeat(depth));
        let parsed = run(&text, LATEST_RELEASE, TYPE);
        assert!(parsed.diagnostics.is_empty());
    });
    on_small_stack(|| {
        let depth = 100_000;
        let text = format!("List{}x{}", "<List".repeat(depth), ">".repeat(depth));
        let parsed = run(&text, LATEST_RELEASE, TYPE);
        let kinds: Vec<_> = parsed.diagnostics.iter().map(|d| d.kind).collect();
        assert_eq!(kinds, [D::NestingTooDeep]);
    });
}
