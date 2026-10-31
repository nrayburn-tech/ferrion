//! Statements on their own: each input is parsed as one block, without the declaration grammar
//! around it, so that statements can be tested before the declarations are written.
//!
//! `tests/block-snapshots/` holds inputs and their expected dumps, compared like the snapshots
//! of whole files in `tests/snapshots.rs`: a first line `// release: N` parses at release N, and
//! `FERRION_BLESS=1` writes the dumps:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_parser --lib block_snapshots
//! ```
//!

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_java_version::LATEST_RELEASE;
use ferrion_syntax::SyntaxKind::{self, *};

use crate::builder::Builder;
use crate::parser::Parser;
use crate::{NESTING_LIMIT, Parse, ParseDiagnostic, ParseDiagnosticKind, invariants};

/// Parses `text` as a block, inside a compilation unit, and checks the result.
fn parse_block(text: &str, release: u32) -> Parse {
    let text: Arc<str> = Arc::from(text);
    let lexed = ferrion_lexer::lex(&text);
    let mut p = Parser::new(&text, &lexed.tokens, release);
    let m = p.start();
    super::block(&mut p);
    let mut progress = p.progress();
    while !p.at_eof() {
        p.error_and_bump();
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

fn count(parsed: &Parse, kind: SyntaxKind) -> usize {
    parsed.tree.nodes().filter(|n| n.kind() == kind).count()
}

fn token_kinds(parsed: &Parse) -> Vec<SyntaxKind> {
    parsed
        .tree
        .tokens()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind())
        .collect()
}

fn diagnostic_kinds(parsed: &Parse) -> Vec<ParseDiagnosticKind> {
    parsed.diagnostics.iter().map(|d| d.kind).collect()
}

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn block_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/block-snapshots");
    let bless = std::env::var_os("FERRION_BLESS").is_some();
    let mut inputs = Vec::new();
    collect(&dir, &mut inputs);
    inputs.sort();
    assert!(!inputs.is_empty(), "no inputs in {}", dir.display());
    let mut failures = Vec::new();
    for input in &inputs {
        let text = fs::read_to_string(input).unwrap();
        let release = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// release: "))
            .map_or(LATEST_RELEASE, |n| n.trim().parse().unwrap());
        let actual = parse_block(&text, release).debug_dump();
        let expected_path = input.with_extension("tree");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            if bless {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{}: the tree differs from {}:\n{actual}",
                    input.display(),
                    expected_path.display()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} block snapshots failed (run with FERRION_BLESS=1 to update them):\n{}",
        failures.len(),
        inputs.len(),
        failures.join("\n")
    );
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "java") {
            out.push(path);
        }
    }
}

#[test]
fn yield_is_a_statement_from_release_14() {
    for text in [
        "{ yield (1); }",
        "{ yield x; }",
        "{ yield -1; }",
        "{ yield ++x; }",
    ] {
        let at_13 = parse_block(text, 13);
        assert_eq!(count(&at_13, YieldStmt), 0, "{text} at 13");
        assert!(!token_kinds(&at_13).contains(&YieldKw));
        let at_14 = parse_block(text, 14);
        assert_eq!(count(&at_14, YieldStmt), 1, "{text} at 14");
        assert_eq!(token_kinds(&at_14)[1], YieldKw);
    }
}

#[test]
fn yield_stays_a_name_where_javac_reads_one() {
    for text in [
        "{ yield = 1; }",
        "{ yield.f(); }",
        "{ yield[0] = 1; }",
        "{ yield++; }",
        "{ yield--; }",
        "{ yield: ; }",
        "{ yield<T> x; }",
        "{ yield -> 1; }",
        "{ yield ? 1 : 2; }",
        "{ yield instanceof A; }",
        "{ yield::m; }",
        "{ yield += 1; }",
    ] {
        let parsed = parse_block(text, LATEST_RELEASE);
        assert_eq!(count(&parsed, YieldStmt), 0, "{text}");
        assert!(!token_kinds(&parsed).contains(&YieldKw), "{text}");
    }
    let labeled = parse_block("{ yield: ; }", LATEST_RELEASE);
    assert_eq!(count(&labeled, LabeledStmt), 1);
    assert!(labeled.diagnostics.is_empty());
}

#[test]
fn else_if_chains_are_flat() {
    let parsed = parse_block("{ if (a) ; else if (b) ; else if (c) ; else ; }", 27);
    assert_eq!(count(&parsed, IfStmt), 3);
    // Each `if` after the first is the else branch of the one before.
    let ifs: Vec<_> = parsed.tree.nodes().filter(|n| n.kind() == IfStmt).collect();
    assert_eq!(ifs[1].parent(), Some(ifs[0]));
    assert_eq!(ifs[2].parent(), Some(ifs[1]));
}

#[test]
fn a_member_modifier_closes_the_block() {
    let parsed = parse_block("{ ; private int x; }", 27);
    assert_eq!(
        diagnostic_kinds(&parsed)[0],
        ParseDiagnosticKind::ExpressionExpected(PrivateKw)
    );
    let block = parsed.tree.root().first_child().unwrap();
    assert_eq!(block.kind(), Block);
    assert_eq!(block.text(), "{ ;");
}

#[test]
fn orphaned_keywords() {
    for (text, kind) in [
        ("{ else ; }", ParseDiagnosticKind::ElseWithoutIf),
        ("{ catch (E e) {} }", ParseDiagnosticKind::CatchWithoutTry),
        ("{ finally {} }", ParseDiagnosticKind::FinallyWithoutTry),
        ("{ case X -> ; }", ParseDiagnosticKind::CaseOutsideSwitch),
        ("{ default: ; }", ParseDiagnosticKind::CaseOutsideSwitch),
        ("{ if (a) } ", ParseDiagnosticKind::StatementExpected),
        (
            "{ while (a) default: ; }",
            ParseDiagnosticKind::StatementExpected,
        ),
        ("{ try {} }", ParseDiagnosticKind::TryWithoutCatch),
        ("{ try f(); }", ParseDiagnosticKind::TryWithoutCatch),
        ("{ switch (x) { ; } }", ParseDiagnosticKind::CaseExpected),
        (
            "{ switch (x) { case default -> {} } }",
            ParseDiagnosticKind::DefaultNotAllowed,
        ),
        (
            "{ switch (x) { default when x -> {} } }",
            ParseDiagnosticKind::GuardNotAllowed,
        ),
        (
            "{ switch (x) { case X -> return; } }",
            ParseDiagnosticKind::StatementNotAllowedInRule,
        ),
        (
            "{ if (a) class C {} }",
            ParseDiagnosticKind::LocalClassNotAllowed,
        ),
        (
            "{ sealed class C {} }",
            ParseDiagnosticKind::SealedLocalClass,
        ),
        (
            "{ if (a) sealed class C {} }",
            ParseDiagnosticKind::SealedLocalClass,
        ),
        (
            "{ switch (x) { case X -> } }",
            ParseDiagnosticKind::StatementExpected,
        ),
        (
            "{ switch (x) { case X -> default -> {} } }",
            ParseDiagnosticKind::StatementExpected,
        ),
    ] {
        let parsed = parse_block(text, 27);
        assert!(diagnostic_kinds(&parsed).contains(&kind), "{text}");
    }
}

#[test]
fn anchors_match_javac() {
    // Offsets observed with the javac oracle, relative to the block.
    for (text, kind, anchor) in [
        // `else.without.if` at the `else`.
        (
            "{ if (a) ; else else ; }",
            ParseDiagnosticKind::ElseWithoutIf,
            16,
        ),
        // `illegal.start.of.stmt` at the `}`.
        ("{ while (a) }", ParseDiagnosticKind::StatementExpected, 12),
        // `try.without.catch.finally.or.resource.decls` at the `try`.
        (
            "{ try f(); catch (E e) {} }",
            ParseDiagnosticKind::TryWithoutCatch,
            2,
        ),
        // `class.not.allowed` at `class`.
        (
            "{ if (a) final class C {} }",
            ParseDiagnosticKind::LocalClassNotAllowed,
            15,
        ),
        // `expected3` at the stray token.
        (
            "{ switch (x) { case X -> {} ; } }",
            ParseDiagnosticKind::CaseExpected,
            28,
        ),
        // `guard.not.allowed` at `when`.
        (
            "{ switch (x) { case null, default when a -> {} } }",
            ParseDiagnosticKind::GuardNotAllowed,
            34,
        ),
        // `illegal.start.of.stmt` at the `}` of a rule without a body.
        (
            "{ switch (x) { case X -> } }",
            ParseDiagnosticKind::StatementExpected,
            25,
        ),
        // `default.label.not.allowed` at `default`.
        (
            "{ switch (x) { case null, X, default -> {} } }",
            ParseDiagnosticKind::DefaultNotAllowed,
            29,
        ),
    ] {
        let parsed = parse_block(text, 27);
        let found = parsed.diagnostics.iter().find(|d| d.kind == kind);
        assert_eq!(found.map(|d| d.anchor), Some(anchor), "{text}");
    }
}

#[test]
fn switch_blocks_hold_rules_and_groups() {
    let parsed = parse_block(
        "{ switch (x) { case A: case B: ; default -> {} case C -> throw e; } }",
        27,
    );
    assert_eq!(count(&parsed, SwitchStmt), 1);
    assert_eq!(count(&parsed, SwitchGroup), 1);
    assert_eq!(count(&parsed, SwitchRule), 2);
    assert_eq!(count(&parsed, SwitchLabel), 4);
}

#[test]
fn nested_blocks_below_the_limit() {
    on_small_stack(|| {
        let depth = NESTING_LIMIT as usize - 1;
        let parsed = parse_block(&format!("{}{}", "{".repeat(depth), "}".repeat(depth)), 27);
        assert!(parsed.diagnostics.is_empty());
        assert_eq!(count(&parsed, Block), depth);
    });
}

#[test]
fn nested_blocks_beyond_the_limit() {
    on_small_stack(|| {
        let depth = 100_000;
        let parsed = parse_block(&format!("{}{}", "{".repeat(depth), "}".repeat(depth)), 27);
        assert_eq!(
            diagnostic_kinds(&parsed),
            [ParseDiagnosticKind::NestingTooDeep]
        );
    });
}

#[test]
fn nested_statements_beyond_the_limit() {
    on_small_stack(|| {
        let depth = 20_000;
        let parsed = parse_block(&format!("{{ {} ; }}", "while (a) ".repeat(depth)), 27);
        assert!(diagnostic_kinds(&parsed).contains(&ParseDiagnosticKind::NestingTooDeep));
    });
}

#[test]
fn long_else_if_chains_are_not_nesting() {
    on_small_stack(|| {
        let length = 10_000;
        let parsed = parse_block(
            &format!("{{ if (a) ; {} else ; }}", "else if (a) ; ".repeat(length)),
            27,
        );
        assert_eq!(count(&parsed, IfStmt), length + 1);
        assert!(!diagnostic_kinds(&parsed).contains(&ParseDiagnosticKind::NestingTooDeep));
        // Dropped here, on the small stack.
    });
}
