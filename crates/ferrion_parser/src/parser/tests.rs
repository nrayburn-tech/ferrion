//! The core on its own, driven by small grammars written for the tests.

use std::sync::Arc;

use ferrion_syntax::SyntaxKind::{self as K, *};

use super::*;
use crate::builder::Builder;
use crate::diagnostic::ParseDiagnosticKind as D;
use crate::{JavacPhase, Parse, invariants};

/// Parses `text` with `grammar` inside a compilation unit, and checks the result.
fn run(text: &str, grammar: impl FnOnce(&mut Parser<'_>)) -> Parse {
    let text: Arc<str> = Arc::from(text);
    let lexed = ferrion_lexer::lex(&text);
    let mut p = Parser::new(&text, &lexed.tokens, 21);
    let m = p.start();
    grammar(&mut p);
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
    let errors: Vec<_> = lexed.errors.iter().map(|e| e.span).collect();
    let tree = Builder::new(text.clone(), lexed.tokens, &errors, 21).build(events);
    let parsed = Parse { tree, diagnostics };
    invariants::check(&text, 21, &parsed).unwrap();
    parsed
}

fn kinds(parsed: &Parse) -> Vec<SyntaxKind> {
    parsed.tree.nodes().map(|n| n.kind()).collect()
}

fn diagnostics(parsed: &Parse) -> Vec<(D, u32)> {
    parsed
        .diagnostics
        .iter()
        .map(|d| (d.kind, d.anchor))
        .collect()
}

/// `(((x)))`, recursing through `nested` for each parenthesis.
fn parens(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if p.at(Identifier) {
        let m = p.start();
        p.bump();
        return Some(m.complete(p, NameExpr));
    }
    if !p.at(LParen) {
        p.error_expected(Expected::Identifier);
        return None;
    }
    let m = p.start();
    p.bump();
    p.nested(parens);
    p.expect(RParen);
    Some(m.complete(p, ParenExpr))
}

#[test]
fn lookahead_skips_trivia() {
    run("a /* c */ . // d\n b", |p| {
        assert_eq!(p.current(), Identifier);
        assert_eq!(p.nth(1), Dot);
        assert_eq!(p.nth(2), Identifier);
        assert_eq!(p.nth(3), Eof);
        assert_eq!(p.nth(100), Eof);
        assert!(!p.nth_is_adjacent(1));
        assert_eq!(p.nth_span(2), Span::new(18, 19));
        assert_eq!(p.nth_text(2), "b");
        p.bump();
        p.bump();
        p.bump();
        assert!(p.at_eof());
    });
}

#[test]
fn contextual_keywords_are_recorded_where_read() {
    let parsed = run("module \\u006dodule record yield", |p| {
        assert!(p.at(ModuleKw) && p.at(Identifier));
        assert!(p.at_set(TokenSet::new(&[ModuleKw])));
        assert!(!p.at(RecordKw));
        let m = p.start();
        assert!(p.eat(ModuleKw));
        // Escapes are translated before the comparison.
        assert!(p.at(ModuleKw));
        p.bump();
        assert!(p.eat(RecordKw));
        p.expect(YieldKw);
        m.complete(p, Unparsed);
    });
    let tokens: Vec<_> = parsed
        .tree
        .tokens()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind())
        .collect();
    assert_eq!(tokens, [ModuleKw, Identifier, RecordKw, YieldKw, Eof]);
}

#[test]
fn var_is_never_a_keyword() {
    run("var", |p| {
        assert_eq!(p.significant(0).contextual, None);
        p.bump();
    });
}

#[test]
fn non_sealed_is_three_adjacent_tokens() {
    let parsed = run("non-sealed non - sealed", |p| {
        assert!(p.at(NonSealedKw));
        assert!(!p.nth_at(3, NonSealedKw));
        let m = p.start();
        p.bump_as(NonSealedKw);
        assert!(p.at(Identifier) && !p.at(NonSealedKw));
        p.bump();
        p.bump();
        p.bump();
        m.complete(p, Modifiers);
    });
    let tokens: Vec<_> = parsed
        .tree
        .tokens()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind())
        .collect();
    assert_eq!(
        tokens,
        [
            NonSealedKw,
            NonSealedKw,
            NonSealedKw,
            Identifier,
            Minus,
            Identifier,
            Eof
        ]
    );
}

#[test]
fn expect_reports_after_the_previous_token() {
    let parsed = run("a b", |p| {
        let m = p.start();
        p.bump();
        assert!(!p.expect(Semi));
        m.complete(p, ExprStmt);
        assert!(p.at(Identifier));
    });
    assert_eq!(
        diagnostics(&parsed),
        [(D::Expected(Expected::Token(Semi)), 1)]
    );
    assert_eq!(parsed.diagnostics[0].span, Span::empty(1));
    assert!(parsed.tree.root().first_child().unwrap().contains_error());
    assert_eq!(parsed.diagnostics[0].message(), "';' expected");
}

#[test]
fn one_diagnostic_per_mistake() {
    let parsed = run("a b c;", |p| {
        let m = p.start();
        p.bump();
        p.expect(Semi);
        // Still recovering: not reported, but marked.
        p.expect(Semi);
        p.bump();
        p.bump();
        // A token consumed normally ends recovery.
        p.expect(Comma);
        m.complete(p, ExprStmt);
        p.bump();
    });
    assert_eq!(
        diagnostics(&parsed),
        [
            (D::Expected(Expected::Token(Semi)), 1),
            (D::Expected(Expected::Token(Comma)), 5),
        ]
    );
}

#[test]
fn skipping_takes_delimited_groups_whole() {
    let parsed = run("x ( ; [ ] ) ; y } z", |p| {
        let stop = TokenSet::new(&[Semi]);
        let error = p.skip_until(stop).unwrap();
        assert_eq!(error.kind(), K::Error);
        // The `;` inside the parentheses didn't stop it.
        assert!(p.at(Semi));
        p.bump();
        assert!(p.skip_until(stop).is_some());
        // An unmatched closing delimiter stops it.
        assert!(p.at(RBrace));
        assert!(p.skip_until(stop).is_none());
    });
    assert_eq!(
        kinds(&parsed),
        [CompilationUnit, K::Error, K::Error, K::Error, K::Error]
    );
    let first = parsed.tree.root().first_child().unwrap();
    assert_eq!(first.text(), "x ( ; [ ] )");
    assert_eq!(
        diagnostics(&parsed),
        [
            (D::UnexpectedToken(Identifier), 0),
            (D::UnexpectedToken(Identifier), 14),
        ]
    );
}

#[test]
fn precede_and_change_kind() {
    let parsed = run("a.b", |p| {
        let m = p.start();
        p.bump();
        let mut inner = m.complete(p, NameExpr);
        inner.change_kind(p, Name);
        assert_eq!(inner.kind(), Name);
        let m = inner.precede(p);
        p.bump();
        p.bump();
        m.complete(p, Name);
    });
    assert_eq!(kinds(&parsed), [CompilationUnit, Name, Name]);
}

#[test]
fn abandoned_markers_leave_their_tokens_to_the_parent() {
    let parsed = run("a", |p| {
        let outer = p.start();
        let inner = p.start();
        p.bump();
        inner.abandon(p);
        outer.complete(p, NameExpr);
        let empty = p.start();
        empty.abandon(p);
    });
    assert_eq!(kinds(&parsed), [CompilationUnit, NameExpr]);
}

#[test]
fn nesting_below_the_limit() {
    let depth = NESTING_LIMIT as usize;
    // The name inside counts as a level too.
    let text = format!("{}x{}", "(".repeat(depth - 1), ")".repeat(depth - 1));
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let parsed = run(&text, |p| {
                p.nested(parens);
            });
            assert!(parsed.diagnostics.is_empty());
            assert_eq!(parsed.tree.node_count(), depth + 1);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn nesting_too_deep_on_a_small_stack() {
    let depth = 100_000;
    let text = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let parsed = run(&text, |p| {
                p.nested(parens);
            });
            let limit = NESTING_LIMIT as usize;
            assert_eq!(diagnostics(&parsed), [(D::NestingTooDeep, limit as u32)]);
            assert_eq!(parsed.diagnostics[0].javac_phase, JavacPhase::Never);
            // The parentheses at the limit and inside it are one Error node.
            let error = parsed
                .tree
                .node(ferrion_syntax::NodeId::from_index(limit + 1));
            assert_eq!(error.kind(), K::Error);
            assert_eq!(error.text().len(), 2 * (depth - limit) + 1);
            assert_eq!(parsed.tree.node_count(), limit + 2);
            // The tree is dropped here, on the small stack.
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn nesting_too_deep_without_a_group() {
    let parsed = run("a b c;", |p| {
        p.depth = NESTING_LIMIT;
        assert!(
            p.nested(|p| {
                p.bump();
                Some(())
            })
            .is_none()
        );
        assert!(p.at(Semi));
        p.depth = 0;
        p.bump();
    });
    assert_eq!(diagnostics(&parsed), [(D::NestingTooDeep, 0)]);
    assert_eq!(parsed.tree.root().first_child().unwrap().text(), "a b c");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "made no progress")]
fn a_stalled_loop_is_a_bug() {
    run("a", |p| {
        let mut progress = p.progress();
        progress.advanced(p);
    });
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "neither completed nor abandoned")]
fn a_dropped_marker_is_a_bug() {
    run("a", |p| {
        let _ = p.start();
    });
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "an empty NameExpr")]
fn an_empty_node_is_a_bug() {
    run("a", |p| {
        let m = p.start();
        m.complete(p, NameExpr);
    });
}
