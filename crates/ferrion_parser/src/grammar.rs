//! The grammar: recursive descent over the parser core (`parser.rs`), producing the nodes
//! `java.grammar` describes. It is split by area, so each can be written on its own:
//!
//! - this file: compilation units, package and import declarations, and qualified names;
//! - `decls.rs`: modifiers, annotations, type declarations, and members;
//! - `modules.rs`: module declarations;
//! - `types.rs`: types, type arguments, and type parameters;
//! - `exprs.rs`: expressions;
//! - `stmts.rs`: blocks, statements, and switch blocks;
//! - `patterns.rs`: patterns;
//! - `numbers.rs`: numeric literals out of range.
//!
//! Functions other areas call are `pub(crate)`, with the signatures their callers need.

mod decls;
mod exprs;
mod modules;
mod numbers;
mod patterns;
mod stmts;
mod types;

use crate::SyntaxKind::*;
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::parser::{CompletedMarker, Parser, TokenSet};
use ferrion_diagnostics::Span;
use ferrion_syntax::token_sets;

/// An identifier, or `_` (an identifier at Java 8).
pub(crate) const IDENT: TokenSet = TokenSet::new(token_sets::IDENT);

/// What can follow a declaration's name: a variable's, a parameter's, or a method's.
const AFTER_NAME: TokenSet = TokenSet::new(&[Semi, Eq, Comma, LBracket, RParen, LParen, Colon]);

/// Whether the `n`th token ahead is a declaration's name: an identifier, or `assert` or `enum`
/// (and with `this_too`, `this`) followed by what can follow a name. javac reads those keywords
/// as the name and reports them, as `assert` and `enum` were names before Java 1.4 and 5. Before
/// anything else (`int\nenum E {}`), the keyword starts what it usually does.
pub(crate) fn nth_at_name(p: &Parser<'_>, n: usize, this_too: bool) -> bool {
    p.nth_at_set(n, IDENT) || nth_at_keyword_name(p, n, this_too)
}

fn nth_at_keyword_name(p: &Parser<'_>, n: usize, this_too: bool) -> bool {
    let keyword = matches!(p.nth(n), AssertKw | EnumKw) || this_too && p.nth(n) == ThisKw;
    keyword && p.nth_at_set(n + 1, AFTER_NAME)
}

/// A declaration's name (see [`nth_at_name`]), reporting a keyword. Returns `false`, consuming
/// nothing, if there is none.
pub(crate) fn eat_name(p: &mut Parser<'_>, this_too: bool) -> bool {
    if p.at_set(IDENT) {
        p.bump();
        return true;
    }
    if !nth_at_keyword_name(p, 0, this_too) {
        return false;
    }
    let span = p.nth_span(0);
    p.error(
        ParseDiagnosticKind::KeywordAsName(p.current()),
        span,
        span.start,
    );
    p.bump();
    true
}

/// Parses a whole file.
pub(crate) fn compilation_unit(p: &mut Parser<'_>) {
    let m = p.start();
    let package = if p.at(PackageKw) || p.at(At) && at_annotated_package(p) {
        Some(package_decl(p))
    } else {
        None
    };
    let mut progress = p.progress();
    loop {
        if p.at(ImportKw) {
            import_decl(p);
        } else if p.at(Semi) {
            // The version check reports a `;` among the imports by release.
            p.bump();
        } else {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    let implicit_class = decls::compilation_unit_items(p);
    if implicit_class && let Some((decl, keyword)) = package {
        // javac reports it once the file is parsed, after any other error.
        p.error(ParseDiagnosticKind::ImplicitClassPackage, decl, keyword);
    }
    // Whatever the items left, such as unmatched closing delimiters.
    let mut progress = p.progress();
    while !p.at_eof() {
        p.error_and_bump();
        if !progress.advanced(p) {
            break;
        }
    }
    p.bump_eof();
    m.complete(p, CompilationUnit);
}

/// Whether the annotations at the current token are followed by `package`. Scans tokens without
/// consuming them: `@` and a qualified name, each perhaps followed by a parenthesized group.
fn at_annotated_package(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while p.nth(n) == At && p.nth(n + 1) != InterfaceKw {
        n += 1;
        while p.nth_at_set(n, IDENT) {
            n += 1;
            if p.nth(n) != Dot {
                break;
            }
            n += 1;
        }
        if p.nth(n) == LParen {
            let mut depth = 0usize;
            loop {
                match p.nth(n) {
                    LParen => depth += 1,
                    RParen => depth -= 1,
                    Eof => return false,
                    _ => {}
                }
                n += 1;
                if depth == 0 {
                    break;
                }
            }
        }
    }
    p.nth(n) == PackageKw
}

/// `@A package a.b;`. Returns its span and where its `package` starts.
fn package_decl(p: &mut Parser<'_>) -> (Span, u32) {
    let start = p.nth_span(0).start;
    let m = p.start();
    let mut progress = p.progress();
    while p.at(At) {
        decls::annotation(p);
        if !progress.advanced(p) {
            break;
        }
    }
    let keyword = p.nth_span(0).start;
    p.expect(PackageKw);
    if name(p).is_none() {
        p.error_expected(Expected::Identifier);
    }
    p.expect(Semi);
    m.complete(p, PackageDecl);
    (Span::new(start, p.previous_end()), keyword)
}

/// `import a.b.C;`, `import static a.b.C.m;`, `import a.b.*;`, `import module m;`
fn import_decl(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    let mut imports_module = false;
    if !p.eat(StaticKw) && p.at(ModuleKw) && p.nth_at_set(1, IDENT) {
        // `module` followed by a name, not by `.` as in `import module.a.B;`.
        p.bump_as(ModuleKw);
        imports_module = true;
    }
    let qualified = p.nth(1) == Dot;
    match name(p) {
        None => p.error_expected(Expected::Identifier),
        Some(_) if p.at(Dot) && p.nth(1) == Star => {
            p.bump();
            p.bump();
        }
        // A single-type import names a type in a package, so its name is qualified; javac
        // reports `'.' expected` after `import a;`. Module imports take any module name.
        Some(_) if !imports_module && !qualified => p.error_expected(Expected::Token(Dot)),
        Some(_) => {}
    }
    p.expect(Semi);
    m.complete(p, ImportDecl);
}

/// A qualified name, `a.b.c`, as nested `Name` nodes, or `None` if the current token isn't an
/// identifier. Stops before a `.` followed by `*`. A `.` followed by something other than a name
/// is consumed and reported, as javac does.
pub(crate) fn name(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at_set(IDENT) {
        return None;
    }
    let m = p.start();
    p.bump();
    let mut name = m.complete(p, Name);
    let mut progress = p.progress();
    while p.at(Dot) && p.nth(1) != Star {
        let m = name.precede(p);
        p.bump();
        if p.at_set(IDENT) {
            p.bump();
        } else {
            p.error_expected(Expected::Identifier);
        }
        name = m.complete(p, Name);
        if !progress.advanced(p) {
            break;
        }
    }
    Some(name)
}
