//! Patterns (`java.grammar`: "Patterns", decision 12), and the lookahead that tells a pattern
//! from a constant in a case label.

use ferrion_diagnostics::Span;

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
        let annotation = record_pattern_annotation(p);
        let found = scan_modifiers(p, 0);
        let modifiers_start = p.nth_span(0);
        let modifiers = decls::modifiers(p).is_some();
        let type_start = p.nth_span(0);
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
            // javac takes `final` on type patterns only, and annotations on neither.
            if let Some(span) = annotation {
                p.error(
                    ParseDiagnosticKind::RecordPatternAnnotations,
                    span,
                    span.start,
                );
            } else if found.not_final {
                p.error(
                    ParseDiagnosticKind::ModifierNotAllowed,
                    type_start,
                    type_start.start,
                );
            } else if found.final_kw {
                p.error(
                    ParseDiagnosticKind::FinalRecordPattern,
                    modifiers_start,
                    modifiers_start.start,
                );
            }
            record_components(p);
            return Some(m.complete(p, RecordPattern));
        }
        if found.not_final {
            p.error(
                ParseDiagnosticKind::ModifierNotAllowed,
                type_start,
                type_start.start,
            );
        }
        if p.at_set(IDENT) {
            p.bump();
        } else {
            p.error_expected(Expected::Identifier);
        }
        Some(m.complete(p, TypePattern))
    })
}

/// If a record pattern starts here, the span of the first annotations before its type or on it
/// (`@A R(var x)`, `a.@A R(var x)`), outside its type arguments: javac rejects them while
/// parsing.
fn record_pattern_annotation(p: &Parser<'_>) -> Option<Span> {
    let mut n = 0;
    loop {
        let next = types::annotations_end(p, n);
        if p.nth(next) == FinalKw {
            n = next + 1;
        } else {
            n = next;
            break;
        }
    }
    let end = types::scan_type(p, n)?;
    if p.nth(end) != LParen {
        return None;
    }
    let mut depth = 0usize;
    for i in 0..end {
        match p.nth(i) {
            Lt => depth += 1,
            Gt => depth = depth.saturating_sub(1),
            At if depth == 0 => {
                let last = types::annotations_end(p, i).max(i + 1) - 1;
                return Some(Span::new(p.nth_span(i).start, p.nth_span(last).end));
            }
            _ => {}
        }
    }
    None
}

/// After `instanceof`, at modifiers followed by a type and no name or `(`: `final String`. javac
/// reads a type with modifiers, which it reports at the type, so it is a `TypePattern` without
/// a name.
pub(crate) fn modified_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    decls::modifiers(p);
    let type_start = p.nth_span(0);
    p.error(
        ParseDiagnosticKind::ModifierNotAllowed,
        type_start,
        type_start.start,
    );
    if types::type_(p).is_none() {
        types::type_expected(p);
    }
    Some(m.complete(p, TypePattern))
}

/// Modifier keywords a pattern can be written with, though only `final` is allowed.
const MODIFIER_KEYWORDS: TokenSet = TokenSet::new(&[
    PublicKw,
    ProtectedKw,
    PrivateKw,
    AbstractKw,
    StaticKw,
    FinalKw,
    TransientKw,
    VolatileKw,
    NativeKw,
    StrictfpKw,
]);

/// What [`scan_modifiers`] found.
pub(crate) struct ModifierScan {
    /// The index of the token after the modifiers.
    pub end: usize,
    pub final_kw: bool,
    /// A modifier keyword other than `final`.
    pub not_final: bool,
}

/// Scans the annotations and modifier keywords at the `n`th token ahead, without consuming them.
pub(crate) fn scan_modifiers(p: &Parser<'_>, mut n: usize) -> ModifierScan {
    let mut scan = ModifierScan {
        end: n,
        final_kw: false,
        not_final: false,
    };
    loop {
        let after = types::annotations_end(p, n);
        if after > n {
            n = after;
        } else if p.nth_at_set(n, MODIFIER_KEYWORDS) {
            if p.nth(n) == FinalKw {
                scan.final_kw = true;
            } else {
                scan.not_final = true;
            }
            n += 1;
        } else {
            scan.end = n;
            return scan;
        }
    }
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
/// - parentheses around a pattern, or around `_`, make a pattern too, which javac reports at the
///   `(`: Java no longer has parenthesized patterns. `(a)` and `(String) s` are constants;
/// - anything else, `_` alone included, is a constant.
pub(crate) fn at_case_pattern(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while p.nth(n) == LParen {
        n += 1;
    }
    if p.nth_at_set(n, MODIFIER_KEYWORDS) || p.nth(n) == At || n > 0 && p.nth(n) == Underscore {
        return true;
    }
    let Some(after) = types::scan_type(p, n) else {
        return false;
    };
    if p.nth_at_set(after, IDENT) {
        return true;
    }
    p.nth(after) == LParen && scan_record_pattern(p, after, n > 0)
}

/// At the `(` of a possible record pattern, the `n`th token ahead: see [`at_case_pattern`].
/// A loop over the tokens, so nested record patterns cost no stack.
fn scan_record_pattern(p: &Parser<'_>, mut n: usize, parenthesized: bool) -> bool {
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
            return found && (p.nth_at_set(n, FOLLOW) || parenthesized && p.nth(n) == RParen);
        }
        at_component = false;
    }
}
