//! Deeply nested input through the formatter (docs/design/formatter.md, "Deep nesting"), on the
//! parser's generated stress patterns (`ferrion_parser::stress`):
//!
//! - **stack safety:** every pattern, at the most repetitions that parse without errors, is
//!   formatted on a thread with a 256 KiB stack, so that recursion without `ensure_stack`
//!   overflows here instead of passing on a large stack. Indentation grows with depth, so the
//!   output of a deeply nested pattern is large; only that it's made is checked;
//! - **linear time:** formatting ten times the repetitions takes about as much longer as the input
//!   plus the output grows, at depths where the output stays small. The output of nested blocks
//!   grows with the square of the depth, so time is compared with its size, not the depth's.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrion_diagnostics::SourceText;
use ferrion_formatter::{Formatted, format_tree};
use ferrion_parser::stress::{Shape, below_limit, full_depth, pattern, patterns};
use ferrion_parser::{ParseOptions, Severity, parse};

/// The stack size of the thread that formats.
const STACK_SIZE: usize = 256 * 1024;

/// A test for each pattern. [`every_pattern_has_a_test`] checks that the list is complete.
macro_rules! tests {
    ($($name:ident,)*) => {
        const TESTED: &[&str] = &[$(stringify!($name),)*];

        $(
            #[test]
            fn $name() {
                check_pattern(stringify!($name));
            }
        )*
    };
}

tests! {
    parentheses,
    blocks,
    if_statements,
    classes,
    anonymous_classes,
    lambdas,
    lambda_blocks,
    if_blocks,
    array_initializers,
    generic_types,
    conditionals,
    switch_expressions,
    call_arguments,
    annotations,
    plus,
    string_concatenation,
    method_calls,
    field_accesses,
    array_accesses,
    else_if,
    conditional_else,
    prefix_operators,
    casts,
    statements,
    members,
    qualified_import,
    qualified_package,
    imports,
}

#[test]
fn every_pattern_has_a_test() {
    let names: Vec<&str> = patterns().iter().map(|p| p.name).collect();
    assert_eq!(
        names, TESTED,
        "add a test for each pattern to `tests!`, in the same order"
    );
}

/// What formatting one input took.
struct Outcome {
    /// The input's and the output's sizes, in bytes.
    size: usize,
    /// Time spent in `format_tree` alone.
    time: Duration,
}

/// Parses `text`, which must parse without errors, and formats it, on a small stack.
fn run(text: String) -> Outcome {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let source = SourceText::decode(text).unwrap();
            let parsed = parse(Arc::from(source.text()), ParseOptions { release: 27 });
            let errors: Vec<_> = parsed
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .collect();
            assert!(errors.is_empty(), "{errors:?}");
            let start = Instant::now();
            let formatted = format_tree(&parsed.tree, &source);
            let time = start.elapsed();
            let output = match &formatted {
                Formatted::Unchanged => 0,
                Formatted::Changed(output) => output.len(),
                Formatted::Skipped(_) => panic!("skipped"),
            };
            drop(formatted);
            drop(parsed);
            Outcome {
                size: source.text().len() + output,
                time,
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

fn check_pattern(name: &str) {
    let pattern = pattern(name);
    run((pattern.generate)(full_depth(&pattern)));
    let (small, large) = match pattern.shape {
        Shape::Nesting => (below_limit(&pattern) / 100, below_limit(&pattern) / 10),
        Shape::Chain => (10_000, 100_000),
    };
    check_linear(name, pattern.generate, small, large);
}

/// Checks that formatting a larger input takes well under the square of how much larger the
/// input and output are, which quadratic work would take. The fastest of three runs is used,
/// against noise.
fn check_linear(name: &str, generate: fn(usize) -> String, small: usize, large: usize) {
    let fastest = |n: usize| {
        let runs: Vec<Outcome> = (0..3).map(|_| run(generate(n))).collect();
        let size = runs[0].size;
        (size, runs.into_iter().map(|o| o.time).min().unwrap())
    };
    let ((s_small, t_small), (s_large, t_large)) = (fastest(small), fastest(large));
    let ratio = (s_large / s_small.max(1)) as u32;
    // Four times linear, plus a little for timer resolution and allocation.
    let bound = t_small * ratio * 4 + Duration::from_millis(20);
    assert!(
        t_large <= bound,
        "{name}: {large} ({s_large} bytes in and out) took {t_large:?}, {small} ({s_small} bytes) \
         took {t_small:?}; more than linear"
    );
}
