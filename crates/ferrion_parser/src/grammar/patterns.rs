//! Patterns (`java.grammar`: "Patterns", decision 12), and the lookahead that tells a pattern
//! from a constant in a case label.

use crate::SyntaxKind::*;
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::grammar::{IDENT, decls, types};
use crate::parser::{CompletedMarker, Parser, TokenSet};

/// A pattern: a `TypePattern` (`final String s`, `int i`, `var x`) or a `RecordPattern`
/// (`Point(var x, _)`), for `instanceof` and case labels. Inside a record pattern, `_` is an
/// `UnnamedPattern`. `None`, consuming nothing, if neither modifiers nor a type start here.
pub(crate) fn pattern(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    p.nested(|p| {
        let m = p.start();
        if p.at(Underscore) && p.nth_at(1, WhenKw) {
            // `case _ when x`: javac reads an unnamed pattern with a guard, not a type pattern
            // whose variable is named `when`.
            p.bump();
            return Some(m.complete(p, UnnamedPattern));
        }
        let modifiers = decls::modifiers(p).is_some();
        if types::type_(p).is_none() {
            if !modifiers {
                m.abandon(p);
                return None;
            }
            // `case final 1`: javac reports an illegal start of a type.
            let span = p.nth_span(0);
            p.error(
                ParseDiagnosticKind::TypeExpected(p.current()),
                span,
                span.start,
            );
            return Some(m.complete(p, TypePattern));
        }
        if p.at(LParen) {
            record_components(p);
            return Some(m.complete(p, RecordPattern));
        }
        if p.at_set(IDENT) {
            p.bump();
        } else {
            p.error_expected(Expected::Identifier);
        }
        Some(m.complete(p, TypePattern))
    })
}

/// A record pattern's `(...)`: nested patterns, each `_` an `UnnamedPattern`.
fn record_components(p: &mut Parser<'_>) {
    p.bump();
    let mut progress = p.progress();
    let mut first = true;
    while !p.at_eof() {
        if p.at(RParen) {
            if !first {
                // `R(var x, )`: javac reports an illegal start of a type.
                let span = p.nth_span(0);
                p.error(ParseDiagnosticKind::TypeExpected(RParen), span, span.start);
            }
            break;
        }
        first = false;
        if p.at(Underscore) {
            let m = p.start();
            p.bump();
            m.complete(p, UnnamedPattern);
        } else if pattern(p).is_none() {
            // `R(var x, )`: javac reports an illegal start of a type.
            let span = p.nth_span(0);
            p.error(
                ParseDiagnosticKind::TypeExpected(p.current()),
                span,
                span.start,
            );
            break;
        }
        if !p.eat(Comma) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    p.expect(RParen);
}

/// Whether a case label's element at the current token is a pattern rather than a constant,
/// decided from tokens alone, as javac's observed readings are:
///
/// - modifiers (`final`, an annotation) start a pattern;
/// - a type followed by a name is a type pattern: `A a`, `a.b.C<D> c`, `int[] i`;
/// - a type followed by `(...)` is a record pattern if the parentheses are empty or any
///   component in them, at any depth, is a pattern or `_`, and the `)` is followed by `->`,
///   `:`, `,`, or `when`. So `Foo()` and `Foo(x, _)` are patterns, but `foo(1)`, `Foo(x)`, and
///   `Foo(var x) y` are constants;
/// - `_` followed by `when` is an unnamed pattern with a guard;
/// - anything else, `_` alone included, is a constant.
pub(crate) fn at_case_pattern(p: &Parser<'_>) -> bool {
    if p.at(FinalKw) || p.at(At) {
        return true;
    }
    let Some(after) = types::scan_type(p, 0) else {
        return false;
    };
    if p.nth_at_set(after, IDENT) {
        return true;
    }
    p.nth(after) == LParen && scan_record_pattern(p, after)
}

/// At the `(` of a possible record pattern, the `n`th token ahead: see [`at_case_pattern`].
/// A loop over the tokens, so nested record patterns cost no stack.
fn scan_record_pattern(p: &Parser<'_>, mut n: usize) -> bool {
    const FOLLOW: TokenSet = TokenSet::new(&[Arrow, Colon, Comma, WhenKw]);
    // Open record patterns' parentheses.
    let mut depth = 1usize;
    let mut found = false;
    n += 1;
    if p.nth(n) == RParen {
        found = true;
    }
    // At the start of a component, or (after `skip`) of what follows one.
    let mut at_component = !found;
    loop {
        if at_component {
            if matches!(p.nth(n), Underscore | FinalKw | At) {
                found = true;
            } else if let Some(after) = types::scan_type(p, n) {
                if p.nth_at_set(after, IDENT) {
                    found = true;
                    n = after + 1;
                } else if p.nth(after) == LParen {
                    if p.nth(after + 1) == RParen {
                        found = true;
                        n = after + 2;
                    } else {
                        depth += 1;
                        n = after + 1;
                        continue;
                    }
                }
            }
        }
        // Skip the rest of the component, up to its `,` or the `)` that closes the record
        // pattern, skipping delimited groups whole.
        let mut groups = 0usize;
        loop {
            match p.nth(n) {
                LParen | LBracket | LBrace => groups += 1,
                RParen | RBracket | RBrace if groups > 0 => groups -= 1,
                Comma | RParen if groups == 0 => break,
                Eof | Semi | RBracket | RBrace => return false,
                _ => {}
            }
            n += 1;
        }
        if p.nth(n) == Comma {
            n += 1;
            at_component = true;
            continue;
        }
        // A `)` closing a record pattern.
        n += 1;
        depth -= 1;
        if depth == 0 {
            return found && p.nth_at_set(n, FOLLOW);
        }
        at_component = false;
    }
}
