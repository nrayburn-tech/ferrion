//! Deep-nesting stress tests (docs/design/parser-deep-nesting.md, "Testing"): generated inputs
//! for each construct that nests and each chain that mustn't count as nesting, at depths up to and
//! beyond [`NESTING_LIMIT`]. The inputs are `ferrion_parser::stress`'s patterns, which the tests of
//! the tree's other consumers share.
//!
//! Every pattern, at every depth, is parsed, checked with `invariants::check`, walked with the
//! tree's iterators (and dumped, when the tree is shallow enough), then dropped, on a thread with
//! a 256 KiB stack, so that recursion anywhere in that path overflows here instead of passing on
//! a large stack. Then:
//!
//! - **nesting** patterns parse without any diagnostic below the limit, and with exactly one
//!   `NestingTooDeep` past it, just past and far past;
//! - **chain** patterns parse without any diagnostic at 100,000 links, and some at up to about
//!   a million tokens;
//! - parsing time grows linearly: ten times the input takes well under a hundred times as long.
//!
//! Patterns whose grammar isn't written yet are in [`PENDING`]. They still get the first checks,
//! which hold for any input, but not the others, and their test fails as soon as a small input of
//! the pattern parses without errors, so that they move out of the list when the grammar lands.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrion_parser::stress::{LEVELS, Shape, below_limit, pattern, patterns};
use ferrion_parser::syntax::WalkEvent;
use ferrion_parser::{NESTING_LIMIT, Parse, ParseDiagnosticKind, ParseOptions, invariants, parse};

/// The patterns whose grammar isn't written yet. When one's test fails because it parses now,
/// move it out, and check `stress::LEVELS` for it. A pattern for new syntax starts here until its
/// grammar is written.
const PENDING: &[&str] = &[];

/// Nesting patterns that report `NestingTooDeep` more than once past the limit, against the
/// design's one diagnostic per mistake, with why. Their test fails once they report one, so that
/// they come off the list when it's fixed.
const REPORTED_MORE_THAN_ONCE: &[&str] = &[];

/// The stack size of the thread that parses.
const STACK_SIZE: usize = 256 * 1024;

/// A test for each pattern in `stress::patterns()`. [`every_pattern_has_a_test`] checks that the
/// list is complete.
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

/// What one parse showed.
struct Outcome {
    too_deep: usize,
    /// Diagnostics other than `NestingTooDeep`, as `kind @ anchor`.
    others: Vec<String>,
    /// Time spent in `parse` alone.
    time: Duration,
}

fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Parses `text`, checks the invariants, walks and dumps the tree, and drops it, all on a small
/// stack.
fn run(text: String) -> Outcome {
    on_small_stack(move || {
        let text: Arc<str> = Arc::from(text);
        let start = Instant::now();
        let parsed = parse(text.clone(), ParseOptions { release: 27 });
        let time = start.elapsed();
        invariants::check(&text, 27, &parsed).unwrap();
        walk(&parsed);
        let outcome = Outcome {
            too_deep: parsed
                .diagnostics
                .iter()
                .filter(|d| d.kind == ParseDiagnosticKind::NestingTooDeep)
                .count(),
            others: parsed
                .diagnostics
                .iter()
                .filter(|d| d.kind != ParseDiagnosticKind::NestingTooDeep)
                .map(|d| format!("{:?} @{}", d.kind, d.anchor))
                .collect(),
            time,
        };
        let copy = parsed.clone();
        drop(parsed);
        drop(copy);
        outcome
    })
}

/// Walks the tree with each of its iterators, from the root and from its deepest node.
fn walk(parsed: &Parse) {
    let tree = &parsed.tree;
    assert_eq!(tree.nodes().count(), tree.node_count());
    assert_eq!(tree.tokens().count(), tree.token_count());
    let (mut depth, mut deepest, mut deepest_node) = (0usize, 0usize, tree.root());
    for event in tree.root().preorder() {
        match event {
            WalkEvent::Enter(node) => {
                depth += 1;
                if depth > deepest {
                    deepest = depth;
                    deepest_node = node;
                }
            }
            WalkEvent::Leave(_) => depth -= 1,
        }
    }
    assert_eq!(depth, 0);
    // The deepest node's ancestors, itself included, are as many as its depth.
    assert_eq!(deepest_node.ancestors().count(), deepest);
    assert_eq!(deepest_node.descendants().count(), 1);
    let mut children = 0;
    for node in tree.nodes() {
        children += node.children().count();
    }
    assert_eq!(children + 1, tree.node_count());
    // The dump indents each line by its depth, so its size is quadratic in a deep tree's depth.
    if deepest <= 2_000 {
        assert!(!parsed.debug_dump().is_empty());
    }
}

fn check_pattern(name: &str) {
    let pattern = pattern(name);
    let generate = pattern.generate;
    let depths: Vec<usize> = match pattern.shape {
        Shape::Nesting => vec![
            1_000,
            below_limit(&pattern),
            NESTING_LIMIT as usize + 1,
            100_000,
        ],
        Shape::Chain => {
            let mut depths = vec![1_000, 100_000];
            if pattern.longest > 100_000 {
                depths.push(pattern.longest);
            }
            depths
        }
    };
    // Every input is safe to parse, walk, and drop, whether or not its grammar is written.
    let outcomes: Vec<(usize, Outcome)> = depths.iter().map(|&n| (n, run(generate(n)))).collect();

    if PENDING.contains(&name) {
        let small = run(generate(3));
        assert!(
            !small.others.is_empty() || small.too_deep > 0,
            "{name} parses without errors now: move it out of PENDING in {}",
            file!()
        );
        return;
    }

    let more_than_once = REPORTED_MORE_THAN_ONCE.contains(&name);
    let mut reported_once = true;
    for (n, outcome) in &outcomes {
        let past_limit = pattern.shape == Shape::Nesting && *n > NESTING_LIMIT as usize;
        if past_limit && more_than_once {
            assert!(outcome.too_deep > 0, "{name} at {n}: no NestingTooDeep");
            reported_once &= outcome.too_deep == 1;
        } else if past_limit {
            assert_eq!(
                outcome.too_deep, 1,
                "{name} at {n}: expected one NestingTooDeep"
            );
        } else {
            assert_eq!(outcome.too_deep, 0, "{name} at {n}: too deeply nested");
            assert!(
                outcome.others.is_empty(),
                "{name} at {n}: {:?}",
                outcome.others
            );
        }
    }

    assert!(
        !(more_than_once && reported_once),
        "{name} reports one NestingTooDeep now: take it off REPORTED_MORE_THAN_ONCE in {}",
        file!()
    );

    let (small, large) = match pattern.shape {
        Shape::Nesting => (below_limit(&pattern) / 10, below_limit(&pattern)),
        Shape::Chain => (10_000, 100_000),
    };
    check_linear(name, generate, small, large);
}

/// Checks that parsing ten times the input takes well under a hundred times as long, which
/// quadratic work would take. The fastest of three runs is used, against noise.
fn check_linear(name: &str, generate: fn(usize) -> String, small: usize, large: usize) {
    let fastest = |n: usize| (0..3).map(|_| run(generate(n)).time).min().unwrap();
    let (t_small, t_large) = (fastest(small), fastest(large));
    let ratio = large as u32 / small as u32;
    // Four times linear, plus a little for timer resolution and allocation.
    let bound = t_small * ratio * 4 + Duration::from_millis(20);
    assert!(
        t_large <= bound,
        "{name}: {large} took {t_large:?}, {small} took {t_small:?}; more than linear"
    );
}

#[test]
fn every_pattern_has_a_test() {
    let names: Vec<&str> = patterns().iter().map(|p| p.name).collect();
    assert_eq!(
        names, TESTED,
        "add a test for each pattern to `tests!`, in the same order"
    );
}

#[test]
fn every_pending_pattern_exists() {
    let names: Vec<&str> = patterns().iter().map(|p| p.name).collect();
    for name in PENDING {
        assert!(
            names.contains(name),
            "PENDING names {name}, which isn't a pattern"
        );
    }
    for (name, _) in LEVELS {
        assert!(
            names.contains(name),
            "LEVELS names {name}, which isn't a pattern"
        );
    }
    for name in REPORTED_MORE_THAN_ONCE {
        assert!(
            names.contains(name),
            "REPORTED_MORE_THAN_ONCE names {name}, which isn't a pattern"
        );
    }
}
