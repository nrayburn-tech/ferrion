//! Modifiers, annotations, type declarations, and members (`java.grammar`: "Modifiers and
//! annotations", "Type declarations and members"), and what follows the imports in a compilation
//! unit.
//!
//! Where javac's parser rejects a declaration it reports the same mistake at the same position
//! (docs/design/parser-error-recovery.md), learned by running the oracle on small inputs. Some of
//! those checks aren't about syntax (a record's instance fields, a constructor's name), but javac
//! makes them while parsing, so they are made here too.

use ferrion_diagnostics::Span;
use ferrion_syntax::token_sets;

use super::{IDENT, exprs, modules, name, stmts, types};
use crate::SyntaxKind::{self, *};
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::parser::{CompletedMarker, Marker, Parser, TokenSet};

/// Modifier keywords, without the contextual `sealed` and `non-sealed`, whose reading depends on
/// what follows them.
const MODIFIER_KEYWORDS: TokenSet = TokenSet::new(&[
    PublicKw,
    ProtectedKw,
    PrivateKw,
    AbstractKw,
    StaticKw,
    FinalKw,
    TransientKw,
    VolatileKw,
    SynchronizedKw,
    NativeKw,
    StrictfpKw,
    DefaultKw,
]);

/// The modifiers that can follow `sealed` or `non-sealed` for javac to read it as a modifier:
/// those a class or interface can have. Before anything else (`sealed int x`, `sealed record`),
/// javac reads `sealed` as a type name.
const CLASS_MODIFIER_KEYWORDS: TokenSet = TokenSet::new(&[
    PublicKw,
    ProtectedKw,
    PrivateKw,
    AbstractKw,
    StaticKw,
    FinalKw,
    StrictfpKw,
]);

const PRIMITIVE_TYPES: TokenSet = TokenSet::new(token_sets::PRIMITIVE_TYPE_KEYWORD);

/// Tokens a member declaration can start with, besides `{` in a class body.
const MEMBER_START: TokenSet =
    MODIFIER_KEYWORDS
        .union(PRIMITIVE_TYPES)
        .union(IDENT)
        .union(TokenSet::new(&[
            At,
            Lt,
            ClassKw,
            InterfaceKw,
            EnumKw,
            VoidKw,
        ]));

/// Where skipping stops after a token that can't start a member or a top-level declaration:
/// tokens that start one and aren't usually part of a broken one. Names aren't here, so a run of
/// broken code isn't read as a series of fields.
const DECLARATION_RECOVERY: TokenSet =
    MODIFIER_KEYWORDS
        .union(PRIMITIVE_TYPES)
        .union(TokenSet::new(&[
            Semi,
            At,
            ClassKw,
            InterfaceKw,
            EnumKw,
            VoidKw,
        ]));

/// Where skipping stops in a declaration's header, before its body.
const HEADER_RECOVERY: TokenSet = MODIFIER_KEYWORDS.union(TokenSet::new(&[
    LBrace,
    Semi,
    At,
    ClassKw,
    InterfaceKw,
    EnumKw,
]));

/// Where skipping stops in parameters and annotation arguments.
const LIST_RECOVERY: TokenSet = TokenSet::new(&[Comma, LBrace, Semi, ThrowsKw]);

// ----- Compilation units -----

/// Everything after the imports: a module declaration (`modules::module_decl`), or type
/// declarations and, for an implicitly declared class, methods and fields. Consumes every token
/// up to `Eof`.
pub(crate) fn compilation_unit_items(p: &mut Parser<'_>) {
    // javac reads `module` or `open` as the start of a module declaration only in the first
    // declaration, after any annotations, however it goes on.
    let (after_modifiers, has_keywords) = scan_modifiers(p, 0);
    if modules::at_module_decl(p, after_modifiers) {
        let m = p.start();
        if has_keywords {
            if let (Some(modifiers), _) = modifiers_info(p) {
                let e = modifiers.precede(p);
                error_at_current(p, ParseDiagnosticKind::ModifierNotAllowed);
                e.complete(p, Error);
            }
        } else {
            annotations(p);
        }
        modules::module_decl(p, m);
        if !p.at_eof() {
            p.error_expected(Expected::Token(Eof));
        }
        let mut progress = p.progress();
        while !p.at_eof() {
            p.error_and_bump();
            if !progress.advanced(p) {
                break;
            }
        }
        return;
    }
    let body = Body {
        kind: BodyKind::TopLevel,
        name: None,
    };
    let mut progress = p.progress();
    while !p.at_eof() {
        if p.at(Semi) {
            p.bump();
        } else if modules::at_module_decl_after_types(p) {
            // A module declaration among type declarations: javac reports it at its start.
            error_at_current(p, ParseDiagnosticKind::ExpectedDeclaration(p.current()));
            let e = p.start();
            let m = p.start();
            modules::module_decl(p, m);
            e.complete(p, Error);
        } else if let Some(tokens) = at_misplaced_sealed(p) {
            // javac reports `sealed` where it starts what looks like a declaration but isn't a
            // modifier: `sealed record R() {}`, `sealed int x;`.
            let e = p.start();
            error_at_current(p, ParseDiagnosticKind::ExpectedDeclaration(p.current()));
            p.bump_as(if tokens == 1 { SealedKw } else { NonSealedKw });
            e.complete(p, Error);
        } else if at_member_start(p, &body) {
            member(p, &body);
        } else {
            recover(
                p,
                ParseDiagnosticKind::ExpectedDeclaration(p.current()),
                DECLARATION_RECOVERY,
            );
        }
        if !progress.advanced(p) {
            break;
        }
    }
}

/// At the top level, `sealed` or `non-sealed` that isn't a modifier, followed by a type and a
/// name: how many tokens it is.
fn at_misplaced_sealed(p: &Parser<'_>) -> Option<usize> {
    let tokens = if p.at(NonSealedKw) {
        3
    } else if p.at(SealedKw) {
        1
    } else {
        return None;
    };
    let type_start = p.nth_at_set(tokens, IDENT.union(PRIMITIVE_TYPES)) || p.nth(tokens) == VoidKw;
    (!sealed_is_modifier(p, tokens) && type_start && p.nth_at_set(tokens + 1, IDENT))
        .then_some(tokens)
}

// ----- Modifiers and annotations -----

/// What a declaration's modifiers hold, for the checks javac makes while parsing.
#[derive(Clone, Copy, Default)]
struct ModifierInfo {
    /// Any modifier keyword, as opposed to only annotations.
    keywords: bool,
    /// A keyword other than `final`.
    not_final: bool,
    sealed: bool,
    static_kw: bool,
}

/// Modifier keywords and annotations, as a `Modifiers` node, or `None` if there are none.
pub(crate) fn modifiers(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    modifiers_info(p).0
}

fn modifiers_info(p: &mut Parser<'_>) -> (Option<CompletedMarker>, ModifierInfo) {
    let m = p.start();
    let mut info = ModifierInfo::default();
    let mut seen = TokenSet::EMPTY;
    let mut progress = p.progress();
    loop {
        let kind = if p.at(At) && p.nth(1) != InterfaceKw {
            annotation(p);
            None
        } else if p.at(SealedKw) && sealed_is_modifier(p, 1) {
            Some(SealedKw)
        } else if p.at(NonSealedKw) && sealed_is_modifier(p, 3) {
            Some(NonSealedKw)
        } else if p.at_set(MODIFIER_KEYWORDS) {
            Some(p.current())
        } else {
            break;
        };
        if let Some(kind) = kind {
            if seen.contains(kind) {
                // For `non-sealed`, javac reports its `sealed`.
                let at = if kind == NonSealedKw { 2 } else { 0 };
                let span = p.nth_span(at);
                p.error(ParseDiagnosticKind::RepeatedModifier, span, span.start);
            }
            seen = seen.union(TokenSet::new(&[kind]));
            info.keywords = true;
            info.not_final |= kind != FinalKw;
            info.sealed |= kind == SealedKw;
            info.static_kw |= kind == StaticKw;
            p.bump_as(kind);
        }
        if !progress.advanced(p) {
            break;
        }
    }
    if m.consumed(p) {
        (Some(m.complete(p, Modifiers)), info)
    } else {
        m.abandon(p);
        (None, info)
    }
}

/// Whether `sealed` or `non-sealed`, whose next token is `next` tokens ahead, is a modifier:
/// javac reads it as one before a class modifier, another `sealed` or `non-sealed`, an
/// annotation, or `class`, `interface`, or `enum`.
fn sealed_is_modifier(p: &Parser<'_>, next: usize) -> bool {
    match p.nth(next) {
        ClassKw | InterfaceKw | EnumKw | At => true,
        _ => {
            p.nth_at_set(next, CLASS_MODIFIER_KEYWORDS)
                || p.nth_at(next, SealedKw)
                || p.nth_at(next, NonSealedKw)
        }
    }
}

/// Annotations, as direct children of the node being parsed.
fn annotations(p: &mut Parser<'_>) {
    let mut progress = p.progress();
    while p.at(At) && p.nth(1) != InterfaceKw {
        annotation(p);
        if !progress.advanced(p) {
            break;
        }
    }
}

/// An annotation, at `@`: `@A`, `@a.B(1)`, `@A(x = 1, y = {2})`.
pub(crate) fn annotation(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.expect(At);
    if name(p).is_none() {
        p.error_expected(Expected::Identifier);
    }
    if p.at(LParen) {
        // Element values can be annotations, so this recurses.
        p.nested(|p| {
            annotation_arguments(p);
            Some(())
        });
    }
    m.complete(p, Annotation)
}

/// `(...)` after an annotation's name. javac's parser accepts any mix of element values and
/// element-value pairs, and rejects the mix later, so this does too.
fn annotation_arguments(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    if !p.at(RParen) {
        let mut progress = p.progress();
        loop {
            if p.at_set(IDENT) && p.nth(1) == Eq {
                element_value_pair(p);
            } else if element_value(p).is_none() {
                missing_element_value(p);
                break;
            }
            if !p.eat(Comma) || !progress.advanced(p) {
                break;
            }
        }
    }
    if !p.eat(RParen) {
        p.error_expected(Expected::Token(RParen));
        p.skip_until(LIST_RECOVERY.union(HEADER_RECOVERY));
        p.eat(RParen);
    }
    m.complete(p, AnnotationArguments);
}

/// `x = 1`, at the name.
fn element_value_pair(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    p.bump();
    if element_value(p).is_none() {
        missing_element_value(p);
    }
    m.complete(p, ElementValuePair);
}

/// Reports a missing element value at the current token, as javac's `illegal.start.of.expr`,
/// and skips to the next element value or the end of the list.
fn missing_element_value(p: &mut Parser<'_>) {
    error_at_current(p, ParseDiagnosticKind::ExpressionExpected(p.current()));
    p.skip_until(LIST_RECOVERY.union(HEADER_RECOVERY));
}

/// An annotation's element value: an expression, an annotation, or `{...}`. `None`, consuming
/// nothing, if none starts here.
pub(crate) fn element_value(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.current() {
        At => Some(annotation(p)),
        LBrace => p.nested(|p| Some(element_value_array(p))),
        _ => exprs::conditional_expr(p),
    }
}

/// `{1, 2}`, `{}`, or `{,}`, at `{`.
fn element_value_array(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if !p.eat(Comma) {
        let mut progress = p.progress();
        while !p.at(RBrace) && !p.at_eof() {
            if element_value(p).is_none() {
                missing_element_value(p);
            }
            if !p.eat(Comma) || !progress.advanced(p) {
                break;
            }
        }
    }
    p.expect(RBrace);
    m.complete(p, ElementValueArray)
}

// ----- Type declarations -----

/// What kind of body members are parsed in, and the name of its class, for constructors.
struct Body {
    kind: BodyKind,
    name: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    /// The members of an implicitly declared class.
    TopLevel,
    Class,
    Interface,
    Enum,
    Record,
    Annotation,
    /// An anonymous class or an enum constant's body.
    Anonymous,
}

/// Whether a local class, interface, enum, record, or annotation type starts at the current
/// token, after any modifiers (for block statements).
pub(crate) fn at_type_decl(p: &Parser<'_>) -> bool {
    match p.current() {
        ClassKw | InterfaceKw | EnumKw => true,
        At => p.nth(1) == InterfaceKw,
        _ => at_record(p),
    }
}

/// `record` read as a keyword: followed by a name. javac reads it so at every release, so even
/// at Java 8 `record r;` is a record declaration missing its header, not a field.
fn at_record(p: &Parser<'_>) -> bool {
    p.at(RecordKw) && p.nth_at_set(1, IDENT)
}

/// A type declaration after its modifiers; `m` was started before them. `None`, abandoning `m`,
/// if none starts here.
pub(crate) fn type_decl(p: &mut Parser<'_>, m: Marker) -> Option<CompletedMarker> {
    type_decl_with(p, m, ModifierInfo::default())
}

fn type_decl_with(p: &mut Parser<'_>, m: Marker, info: ModifierInfo) -> Option<CompletedMarker> {
    let (kind, body_kind) = match p.current() {
        ClassKw => (ClassDecl, BodyKind::Class),
        InterfaceKw => (InterfaceDecl, BodyKind::Interface),
        EnumKw => (EnumDecl, BodyKind::Enum),
        At if p.nth(1) == InterfaceKw => {
            p.bump();
            (AnnotationTypeDecl, BodyKind::Annotation)
        }
        _ if at_record(p) => (RecordDecl, BodyKind::Record),
        _ => {
            m.abandon(p);
            return None;
        }
    };
    if kind == RecordDecl {
        p.bump_as(RecordKw);
    } else {
        p.bump();
    }
    let name = if p.at_set(IDENT) {
        let name = p.nth_text(0).into_owned();
        p.bump();
        Some(name)
    } else {
        p.error_expected(Expected::Identifier);
        None
    };
    if p.at(Lt) {
        if kind == EnumDecl {
            let e = p.start();
            // javac reports it at the first type parameter.
            let span = p.nth_span(1);
            p.error(ParseDiagnosticKind::GenericEnum, span, span.start);
            types::type_parameters(p);
            complete_or_abandon(p, e, Error);
        } else {
            types::type_parameters(p);
        }
    }
    if kind == RecordDecl {
        if p.at(LParen) {
            record_header(p);
        } else {
            p.error_expected(Expected::Token(LParen));
        }
    }
    match kind {
        ClassDecl => {
            if p.at(ExtendsKw) {
                type_list_clause(p, ExtendsKw, ExtendsClause, false);
            }
            if p.at(ImplementsKw) {
                type_list_clause(p, ImplementsKw, ImplementsClause, true);
            }
        }
        InterfaceDecl | AnnotationTypeDecl => {
            if p.at(ExtendsKw) {
                type_list_clause(p, ExtendsKw, ExtendsClause, true);
            }
        }
        _ => {
            if p.at(ImplementsKw) {
                type_list_clause(p, ImplementsKw, ImplementsClause, true);
            }
        }
    }
    if matches!(kind, ClassDecl | InterfaceDecl | AnnotationTypeDecl) && p.at(PermitsKw) {
        if !info.sealed {
            error_at_current(p, ParseDiagnosticKind::PermitsWithoutSealed);
        }
        type_list_clause(p, PermitsKw, PermitsClause, true);
    }
    if !p.at(LBrace) {
        p.error_expected(Expected::Token(LBrace));
        p.skip_until(HEADER_RECOVERY);
    }
    if p.at(LBrace) {
        let body = Body {
            kind: body_kind,
            name,
        };
        if kind == EnumDecl {
            enum_body(p, &body);
        } else {
            class_body_in(p, &body);
        }
    }
    Some(m.complete(p, kind))
}

/// `extends A`, `implements A, B`, or `permits A, B`, at the keyword. A class's `extends` takes
/// one type, so a `,` after it ends the clause. As with javac's parser, `extends` and
/// `implements` take any type, even `int[]`, which is rejected after parsing; `permits` takes
/// class types.
fn type_list_clause(p: &mut Parser<'_>, keyword: SyntaxKind, kind: SyntaxKind, several: bool) {
    let m = p.start();
    p.bump_as(keyword);
    let mut progress = p.progress();
    loop {
        let ty = if keyword == PermitsKw {
            types::class_type(p)
        } else {
            types::type_(p)
        };
        if ty.is_none() {
            error_at_current(p, ParseDiagnosticKind::TypeExpected(p.current()));
            break;
        }
        if !several || !p.eat(Comma) || !progress.advanced(p) {
            break;
        }
    }
    m.complete(p, kind);
}

/// `(int x, String... y)`, at `(`.
fn record_header(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    parameter_list(p, true);
    m.complete(p, RecordHeader);
}

/// The parameters or record components after `(`, and the `)`.
fn parameter_list(p: &mut Parser<'_>, components: bool) {
    if !p.at(RParen) {
        let mut first = true;
        let mut progress = p.progress();
        loop {
            let varargs_name = parameter(p, first, components);
            if !p.at(Comma) {
                break;
            }
            if let Some(span) = varargs_name {
                p.error(ParseDiagnosticKind::VarargsNotLast, span, span.start);
            }
            p.bump();
            if p.at(RParen) {
                error_at_current(p, ParseDiagnosticKind::TypeExpected(RParen));
                break;
            }
            first = false;
            if !progress.advanced(p) {
                break;
            }
        }
    }
    if !p.eat(RParen) {
        p.error_expected(Expected::OneOf(&[Comma, RParen, LBracket]));
        p.skip_until(LIST_RECOVERY);
        p.eat(RParen);
    }
}

/// A `FormalParameter`, `ReceiverParameter`, or `RecordComponent`. Returns the name of a
/// variable arity one, for the error if another follows it.
fn parameter(p: &mut Parser<'_>, first: bool, component: bool) -> Option<Span> {
    let m = p.start();
    let modifiers_start = p.nth_span(0);
    let (_, info) = modifiers_info(p);
    if component && info.keywords {
        p.error(
            ParseDiagnosticKind::RecordComponentModifiers,
            modifiers_start,
            modifiers_start.start,
        );
    } else if !component && info.not_final {
        error_at_current(p, ParseDiagnosticKind::ModifierNotAllowed);
    }
    if types::type_(p).is_none() {
        error_at_current(p, ParseDiagnosticKind::TypeExpected(p.current()));
        complete_or_abandon(
            p,
            m,
            if component {
                RecordComponent
            } else {
                FormalParameter
            },
        );
        return None;
    }
    let varargs = at_varargs(p);
    if varargs {
        varargs_node(p);
    }
    if let Some(this) = at_receiver(p) {
        // `A this` or `A Outer.this`. javac accepts any qualified name before `.this`.
        let span = p.nth_span(this);
        if !first || component {
            p.error(ParseDiagnosticKind::MisplacedReceiver, span, span.start);
        }
        for _ in 0..=this {
            p.bump();
        }
        if at_dims(p) {
            error_at_current(p, ParseDiagnosticKind::ReceiverWithDims);
            types::dims(p);
        }
        m.complete(
            p,
            if component {
                RecordComponent
            } else {
                ReceiverParameter
            },
        );
        return None;
    }
    let name = p.nth_span(0);
    if p.at_set(IDENT) {
        p.bump();
    } else {
        p.error_expected(Expected::Identifier);
    }
    if at_dims(p) {
        // C-style dimensions are fine on other parameters.
        if varargs {
            error_at_current(p, ParseDiagnosticKind::VarargsWithDims);
        } else if component {
            error_at_current(p, ParseDiagnosticKind::RecordComponentDims);
        }
        types::dims(p);
    }
    m.complete(
        p,
        if component {
            RecordComponent
        } else {
            FormalParameter
        },
    );
    varargs.then_some(name)
}

/// If a receiver parameter's `this` follows, possibly qualified (`Outer.this`), how many tokens
/// ahead it is.
fn at_receiver(p: &Parser<'_>) -> Option<usize> {
    let mut n = 0;
    while p.nth_at_set(n, IDENT) && p.nth(n + 1) == Dot {
        n += 2;
    }
    (p.nth(n) == ThisKw).then_some(n)
}

/// Whether `...` follows, perhaps after annotations.
fn at_varargs(p: &Parser<'_>) -> bool {
    p.nth(types::annotations_end(p, 0)) == Ellipsis
}

/// `@A ...`
fn varargs_node(p: &mut Parser<'_>) {
    let m = p.start();
    annotations(p);
    p.expect(Ellipsis);
    m.complete(p, Varargs);
}

/// Whether array dimensions follow, perhaps with annotations: `[]` or `@A []`.
fn at_dims(p: &Parser<'_>) -> bool {
    p.nth(types::annotations_end(p, 0)) == LBracket
}

// ----- Bodies -----

/// A class body, at `{`, for anonymous classes and enum constants as well as declarations.
pub(crate) fn class_body(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let body = Body {
        kind: BodyKind::Anonymous,
        name: None,
    };
    class_body_in(p, &body)
}

fn class_body_in(p: &mut Parser<'_>, body: &Body) -> Option<CompletedMarker> {
    if !p.at(LBrace) {
        return None;
    }
    p.nested(|p| {
        let m = p.start();
        p.bump();
        members(p, body);
        p.expect(RBrace);
        Some(m.complete(p, ClassBody))
    })
}

/// `{ A, B(1) { ... }; members }`, at `{`.
fn enum_body(p: &mut Parser<'_>, body: &Body) {
    p.nested(|p| {
        let m = p.start();
        p.bump();
        let mut to_members = false;
        // `{ , }` is allowed: the constants are optional, and so is a trailing comma.
        if !p.eat(Comma) {
            let mut progress = p.progress();
            while !p.at(RBrace) && !p.at(Semi) && !p.at_eof() {
                if !p.at_set(IDENT) && !p.at(At) {
                    // Probably a member before the `;`.
                    error_at_current(p, ParseDiagnosticKind::ExpectedEnumConstant(p.current()));
                    to_members = true;
                    break;
                }
                enum_constant(p);
                if p.eat(Comma) {
                    if p.at(Comma) {
                        error_at_current(p, ParseDiagnosticKind::ExpectedEnumConstant(Comma));
                        p.error_and_bump();
                    }
                } else if !p.at(RBrace) && !p.at(Semi) {
                    p.error_expected(Expected::OneOf(&[Comma, RBrace, Semi]));
                    to_members = true;
                    break;
                }
                if !progress.advanced(p) {
                    break;
                }
            }
        }
        if p.eat(Semi) || to_members {
            members(p, body);
        }
        p.expect(RBrace);
        Some(m.complete(p, EnumBody))
    });
}

/// `@A NAME(args) { body }`
fn enum_constant(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(At) {
        let modifiers = p.start();
        annotations(p);
        complete_or_abandon(p, modifiers, Modifiers);
    }
    if p.at_set(IDENT) {
        p.bump();
    } else {
        p.error_expected(Expected::Identifier);
    }
    if p.at(LParen) {
        exprs::arguments(p);
    }
    if p.at(LBrace) {
        class_body(p);
    }
    complete_or_abandon(p, m, EnumConstant);
}

/// Members up to the body's `}` or the end of the file.
fn members(p: &mut Parser<'_>, body: &Body) {
    let mut progress = p.progress();
    while !p.at(RBrace) && !p.at_eof() {
        if p.at(Semi) {
            p.bump();
        } else if at_member_start(p, body) {
            member(p, body);
        } else {
            recover(
                p,
                ParseDiagnosticKind::ExpectedMember(p.current()),
                DECLARATION_RECOVERY,
            );
        }
        if !progress.advanced(p) {
            break;
        }
    }
}

fn at_member_start(p: &Parser<'_>, body: &Body) -> bool {
    p.at_set(MEMBER_START) || p.at(LBrace) && body.kind != BodyKind::TopLevel
}

// ----- Members -----

/// A member declaration, at a token [`at_member_start`] accepts.
fn member(p: &mut Parser<'_>, body: &Body) {
    let top_level = body.kind == BodyKind::TopLevel;
    if !top_level && (p.at(LBrace) || p.at(StaticKw) && p.nth(1) == LBrace) {
        let m = p.start();
        p.eat(StaticKw);
        stmts::block(p);
        m.complete(p, Initializer);
        return;
    }
    let m = p.start();
    let (modifiers, info) = modifiers_info(p);
    if at_type_decl(p) {
        type_decl_with(p, m, info);
        return;
    }
    let constructors = matches!(
        body.kind,
        BodyKind::TopLevel
            | BodyKind::Class
            | BodyKind::Enum
            | BodyKind::Record
            | BodyKind::Anonymous
    );
    let generic = p.at(Lt);
    if generic {
        types::type_parameters(p);
        annotations(p);
    }
    if constructors && p.at_set(IDENT) && p.nth(1) == LParen {
        constructor_name(p, body);
        method_rest(p, m, ConstructorDecl, false);
    } else if body.kind == BodyKind::Record && !generic && p.at_set(IDENT) && p.nth(1) == LBrace {
        constructor_name(p, body);
        stmts::block(p);
        m.complete(p, CompactConstructorDecl);
    } else if at_type_start(p) {
        let ty = types::type_(p);
        let void = ty.is_some_and(|ty| ty.kind() == VoidType);
        if generic || p.at(LParen) || p.at_set(IDENT) && p.nth(1) == LParen {
            if p.at_set(IDENT) {
                p.bump();
            } else {
                p.error_expected(Expected::Identifier);
            }
            method_rest(p, m, MethodDecl, !void);
        } else {
            if body.kind == BodyKind::Record && !info.static_kw && p.at_set(IDENT) {
                error_at_current(p, ParseDiagnosticKind::InstanceFieldInRecord);
            }
            variable_declarators(p);
            p.expect(Semi);
            m.complete(p, FieldDecl);
        }
    } else {
        // Modifiers or type parameters with no declaration after them.
        let kind = if top_level {
            ParseDiagnosticKind::ExpectedDeclaration(p.current())
        } else {
            ParseDiagnosticKind::ExpectedMember(p.current())
        };
        error_at_current(p, kind);
        if modifiers.is_none() && !generic {
            // Not reached: whatever `at_member_start` accepts starts something above.
            m.abandon(p);
            p.error_and_bump();
        } else {
            m.complete(p, Error);
        }
    }
}

/// Whether a type can start here, after a member's modifiers.
fn at_type_start(p: &Parser<'_>) -> bool {
    p.at_set(IDENT) || p.at_set(PRIMITIVE_TYPES) || p.at(VoidKw)
}

/// A constructor's name, checked against its class's, as javac does while parsing.
fn constructor_name(p: &mut Parser<'_>, body: &Body) {
    let matches = body.name.as_deref() == Some(&*p.nth_text(0));
    if !matches {
        let kind = if body.kind == BodyKind::Enum {
            ParseDiagnosticKind::MisplacedEnumConstant
        } else {
            ParseDiagnosticKind::MissingReturnType
        };
        error_at_current(p, kind);
    }
    p.bump();
}

/// A method's or constructor's parameters, dimensions, `throws`, default value, and body, after
/// its name. A method returning `void` and a constructor can't have dimensions.
fn method_rest(p: &mut Parser<'_>, m: Marker, kind: SyntaxKind, dims: bool) {
    if p.at(LParen) {
        let params = p.start();
        p.bump();
        parameter_list(p, false);
        params.complete(p, FormalParameters);
    } else {
        p.error_expected(Expected::Token(LParen));
    }
    if dims && at_dims(p) {
        types::dims(p);
    }
    if p.at(ThrowsKw) {
        let throws = p.start();
        p.bump();
        let mut progress = p.progress();
        loop {
            if throws_type(p).is_none() {
                p.error_expected(Expected::Identifier);
                break;
            }
            if !p.eat(Comma) || !progress.advanced(p) {
                break;
            }
        }
        throws.complete(p, ThrowsClause);
    }
    if p.at(DefaultKw) && kind == MethodDecl {
        let default = p.start();
        p.bump();
        if element_value(p).is_none() {
            error_at_current(p, ParseDiagnosticKind::ExpressionExpected(p.current()));
        }
        default.complete(p, DefaultValue);
        p.expect(Semi);
    } else if p.at(LBrace) {
        stmts::block(p);
    } else if !p.eat(Semi) {
        // javac accepts a constructor without a body while parsing, and rejects it later.
        p.error_expected(Expected::OneOf(&[LBrace, Semi]));
    }
    m.complete(p, kind);
}

/// A type in a `throws` clause. javac's parser takes a qualified name there, with annotations but
/// without type arguments or dimensions, so `throws E<T>` and `throws E[]` end the clause at
/// the `<` or `[`.
fn throws_type(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at_set(IDENT) && !(p.at(At) && p.nth(1) != InterfaceKw) {
        return None;
    }
    let m = p.start();
    types::annotations(p);
    if p.at_set(IDENT) {
        p.bump();
    } else {
        p.error_expected(Expected::Identifier);
    }
    let mut ty = m.complete(p, ClassType);
    let mut progress = p.progress();
    // Each segment can be annotated: `throws a.@A E`.
    while p.at(Dot) && p.nth_at_set(types::annotations_end(p, 1), IDENT) {
        let m = ty.precede(p);
        p.bump();
        types::annotations(p);
        p.bump();
        ty = m.complete(p, ClassType);
        if !progress.advanced(p) {
            break;
        }
    }
    Some(ty)
}

/// `a = 1, b[] = {2}`: one or more `VariableDeclarator`s separated by commas, for fields and
/// local variables.
pub(crate) fn variable_declarators(p: &mut Parser<'_>) {
    let mut progress = p.progress();
    loop {
        variable_declarator(p);
        if !p.eat(Comma) || !progress.advanced(p) {
            break;
        }
    }
}

fn variable_declarator(p: &mut Parser<'_>) {
    if !p.at_set(IDENT) {
        p.error_expected(Expected::Identifier);
        return;
    }
    let m = p.start();
    p.bump();
    types::dims(p);
    if p.eat(Eq) && exprs::variable_initializer(p).is_none() {
        error_at_current(p, ParseDiagnosticKind::ExpressionExpected(p.current()));
    }
    m.complete(p, VariableDeclarator);
}

// ----- Helpers -----

/// Reports `kind` at the current token.
fn error_at_current(p: &mut Parser<'_>, kind: ParseDiagnosticKind) {
    let span = p.nth_span(0);
    p.error(kind, span, span.start);
}

/// Reports `kind` at the current token, then puts it and the tokens after it in an `Error` node,
/// up to one in `stop`.
fn recover(p: &mut Parser<'_>, kind: ParseDiagnosticKind, stop: TokenSet) {
    error_at_current(p, kind);
    if p.skip_until(stop).is_none() {
        // A closing delimiter, or a token in `stop` that can't start what's expected.
        p.error_and_bump();
    }
}

fn complete_or_abandon(p: &mut Parser<'_>, m: Marker, kind: SyntaxKind) {
    if m.consumed(p) {
        m.complete(p, kind);
    } else {
        m.abandon(p);
    }
}

/// The index of the token after the annotations and modifier keywords starting `n` tokens ahead,
/// and whether there are keywords among them.
fn scan_modifiers(p: &Parser<'_>, mut n: usize) -> (usize, bool) {
    let mut keywords = false;
    loop {
        let next = types::annotations_end(p, n);
        if p.nth_at_set(next, MODIFIER_KEYWORDS) {
            keywords = true;
            n = next + 1;
        } else {
            return (next, keywords);
        }
    }
}
