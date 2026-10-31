//! Module declarations (`java.grammar`: "Compilation units, packages, imports, and modules").
//!
//! The module keywords are contextual (docs/design/parser-language-versions.md): `module` and
//! `open` start a module declaration in the first declaration after the imports, and the
//! directive keywords are keywords where a directive starts.

use super::{IDENT, name};
use crate::SyntaxKind::{self, *};
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::parser::{CompletedMarker, Marker, Parser, TokenSet};

const DIRECTIVE_START: TokenSet =
    TokenSet::new(&[RequiresKw, ExportsKw, OpensKw, UsesKw, ProvidesKw]);

/// Whether a module declaration starts `n` tokens ahead, after its annotations: `module` or
/// `open`. As with javac, either one starts a module declaration when it comes first, whatever
/// follows: `module m;` there is a module declaration missing its `{`.
pub(super) fn at_module_decl(p: &Parser<'_>, n: usize) -> bool {
    p.nth_at(n, ModuleKw) || p.nth_at(n, OpenKw)
}

/// Whether a module declaration starts at the current token after other declarations, where it
/// is an error: `module` and a name followed by `{`, or `open module`. Anything else is read as a
/// member of an implicitly declared class (`module m;` is a field), as javac does.
pub(super) fn at_module_decl_after_types(p: &Parser<'_>) -> bool {
    if p.at(OpenKw) {
        return p.nth_at(1, ModuleKw);
    }
    if !p.at(ModuleKw) || !p.nth_at_set(1, IDENT) {
        return false;
    }
    let mut n = 2;
    while p.nth(n) == Dot && p.nth_at_set(n + 1, IDENT) {
        n += 2;
    }
    p.nth(n) == LBrace
}

/// A module declaration after its annotations; `m` was started before them.
pub(crate) fn module_decl(p: &mut Parser<'_>, m: Marker) -> Option<CompletedMarker> {
    if p.at(OpenKw) {
        p.bump_as(OpenKw);
        if !p.at(ModuleKw) {
            let span = p.nth_span(0);
            p.error(
                ParseDiagnosticKind::ExpectedModuleKeyword(p.current()),
                span,
                span.start,
            );
        }
    }
    p.eat(ModuleKw);
    if name(p).is_none() {
        p.error_expected(Expected::Identifier);
    }
    if !p.at(LBrace) {
        p.error_expected(Expected::Token(LBrace));
        p.skip_until(TokenSet::new(&[LBrace]));
    }
    if p.at(LBrace) {
        p.bump();
        directives(p);
        p.expect(RBrace);
    }
    if m.consumed(p) {
        Some(m.complete(p, ModuleDecl))
    } else {
        m.abandon(p);
        None
    }
}

/// Directives up to the module's `}`.
fn directives(p: &mut Parser<'_>) {
    let mut progress = p.progress();
    while !p.at(RBrace) && !p.at_eof() {
        if p.at(RequiresKw) {
            requires(p);
        } else if p.at(ExportsKw) {
            exports_or_opens(p, ExportsKw, ExportsDirective);
        } else if p.at(OpensKw) {
            exports_or_opens(p, OpensKw, OpensDirective);
        } else if p.at(UsesKw) {
            let m = p.start();
            p.bump_as(UsesKw);
            module_name(p);
            p.expect(Semi);
            m.complete(p, UsesDirective);
        } else if p.at(ProvidesKw) {
            provides(p);
        } else {
            if p.at_set(IDENT) {
                let span = p.nth_span(0);
                p.error(
                    ParseDiagnosticKind::ExpectedModuleDirective(p.current()),
                    span,
                    span.start,
                );
            } else {
                // javac expects the module's `}` here.
                p.error_expected(Expected::Token(RBrace));
            }
            // Skip to the next directive, taking the broken one's `;` with it.
            let skipped = p
                .skip_until(DIRECTIVE_START.union(TokenSet::new(&[Semi])))
                .is_some();
            if !skipped || p.at(Semi) {
                p.error_and_bump();
            }
        }
        if !progress.advanced(p) {
            break;
        }
    }
}

/// `requires transitive static a.b;`. `transitive` is a modifier when a name or `static` follows
/// it, so `requires transitive;` requires a module named `transitive`.
fn requires(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump_as(RequiresKw);
    let mut seen = TokenSet::EMPTY;
    let mut progress = p.progress();
    loop {
        let kind = if p.at(StaticKw) {
            StaticKw
        } else if p.at(TransitiveKw) && (p.nth_at_set(1, IDENT) || p.nth(1) == StaticKw) {
            TransitiveKw
        } else {
            break;
        };
        if seen.contains(kind) {
            let span = p.nth_span(0);
            p.error(ParseDiagnosticKind::RepeatedModifier, span, span.start);
        }
        seen = seen.union(TokenSet::new(&[kind]));
        p.bump_as(kind);
        if !progress.advanced(p) {
            break;
        }
    }
    module_name(p);
    p.expect(Semi);
    m.complete(p, RequiresDirective);
}

/// `exports a.b to m, n;` or the same with `opens`.
fn exports_or_opens(p: &mut Parser<'_>, keyword: SyntaxKind, kind: SyntaxKind) {
    let m = p.start();
    p.bump_as(keyword);
    module_name(p);
    if p.at(Dot) {
        // `exports a.*;`: javac expects a name after the `.`.
        p.bump();
        p.error_expected(Expected::Identifier);
    }
    if p.at(ToKw) {
        p.bump_as(ToKw);
        name_list(p);
    }
    p.expect(Semi);
    m.complete(p, kind);
}

/// `provides a.S with b.I, c.J;`
fn provides(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump_as(ProvidesKw);
    module_name(p);
    if p.at(WithKw) {
        p.bump_as(WithKw);
        name_list(p);
    } else {
        p.error_expected(Expected::Token(WithKw));
    }
    p.expect(Semi);
    m.complete(p, ProvidesDirective);
}

/// Names separated by commas.
fn name_list(p: &mut Parser<'_>) {
    let mut progress = p.progress();
    loop {
        if !module_name(p) || !p.eat(Comma) || !progress.advanced(p) {
            break;
        }
    }
}

/// A qualified name, or an error if there's none. Returns whether there was one.
fn module_name(p: &mut Parser<'_>) -> bool {
    let found = name(p).is_some();
    if !found {
        p.error_expected(Expected::Identifier);
    }
    found
}
