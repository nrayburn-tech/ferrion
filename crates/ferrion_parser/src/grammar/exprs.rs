//! Expressions (`java.grammar`: "Expressions").
//!
//! Precedence climbing, with a loop for every chain so that a long chain costs no stack
//! (docs/design/parser-deep-nesting.md): binary operators loop in [`binary`], assignments and the
//! else branches of `?:` in [`assignment`] and [`conditional`], prefix operators and casts in
//! [`unary`], and member access, calls, array access, and postfix operators in [`postfix`].
//! Right-nested chains (`a = b = c`, `a ? b : c ? d : e`, `-(int) -x`) open a node per operator
//! and close them all at the end. Recursion is left for real nesting, through [`expr`], which
//! counts it.
//!
//! The readings that need lookahead are decided from tokens alone, never by backtracking:
//!
//! - at `(`: a lambda, a cast, or a parenthesized expression ([`classify_paren`]);
//! - at a name: a type that qualifies a method reference or a class literal (`List<String>::size`,
//!   `int[].class`), or an expression ([`type_qualifier`]). Other names stay expressions
//!   (decision 2), so `a.b::c` and `a.b.class` have expression qualifiers;
//! - after `instanceof`: a type or a pattern.
//!
//! Where javac decides an ambiguous reading by its own lookahead, these follow what the oracle
//! shows it doing, so that the first error matches javac's.

// Functions for areas not written yet.
#![allow(dead_code)]

#[cfg(test)]
mod tests;

use ferrion_diagnostics::Span;
use ferrion_syntax::token_sets;

use crate::SyntaxKind::{self, *};
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::grammar::types::{self, PRIMITIVE};
use crate::grammar::{IDENT, decls, patterns, stmts};
use crate::parser::{CompletedMarker, Marker, Parser, TokenSet};

const LITERAL: TokenSet = TokenSet::new(token_sets::LITERAL);

const PREFIX_OP: TokenSet = TokenSet::new(token_sets::PREFIX_OP);

/// Tokens that can start an expression. `void` can too, before `.class`; see [`at_expr_start`].
const EXPR_START: TokenSet = LITERAL.union(IDENT).union(PRIMITIVE).union(TokenSet::new(&[
    LParen, ThisKw, SuperKw, NewKw, SwitchKw, Lt, Plus, Minus, Bang, Tilde, PlusPlus, MinusMinus,
]));

/// Tokens that can start the operand of a cast to a reference type: JLS 15.16 allows a lambda or
/// a unary expression that doesn't start with `+`, `-`, `++`, or `--`, which would make `(a) - b`
/// a cast.
const CAST_OPERAND_START: TokenSet = LITERAL.union(IDENT).union(PRIMITIVE).union(TokenSet::new(&[
    LParen, ThisKw, SuperKw, NewKw, SwitchKw, Bang, Tilde, VoidKw,
]));

/// Tokens that end recovery inside an expression list: `;`, and keywords that start a statement
/// or a declaration but no expression. Closing delimiters end it too.
const RECOVERY: TokenSet = TokenSet::new(&[
    Semi,
    IfKw,
    ElseKw,
    ForKw,
    WhileKw,
    DoKw,
    TryKw,
    CatchKw,
    FinallyKw,
    ReturnKw,
    ThrowKw,
    BreakKw,
    ContinueKw,
    AssertKw,
    SynchronizedKw,
    CaseKw,
    DefaultKw,
    ClassKw,
    InterfaceKw,
    EnumKw,
    PublicKw,
    ProtectedKw,
    PrivateKw,
    StaticKw,
    AbstractKw,
    NativeKw,
    TransientKw,
    VolatileKw,
    StrictfpKw,
    ImportKw,
    PackageKw,
]);

/// Where to stop skipping a broken argument or array element.
const ITEM_RECOVERY: TokenSet = RECOVERY.union(TokenSet::new(&[Comma]));

/// How many tokens [`type_qualifier`] scans for type arguments, so that `a < b < c < ...`
/// doesn't make the parser quadratic. Type arguments in a method reference's qualifier are far
/// shorter.
const TYPE_ARGUMENTS_BUDGET: usize = 1024;

/// Any expression, assignments and lambdas included. `None`, consuming nothing, if no expression
/// starts here.
pub(crate) fn expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    // Every expression nested in another (in parentheses, arguments, lambda bodies, ...) comes
    // through here, so this is where nesting is counted.
    p.nested(assignment)
}

/// A conditional expression, or any expression that binds tighter: an annotation's element value
/// (JLS 9.7.1), where an assignment isn't one, so that `@A(x = y = 1)` is an error at the second
/// `=`, as with javac. `None`, consuming nothing, if no expression starts here.
pub(crate) fn conditional_expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    p.nested(conditional)
}

/// A case constant or a guard's condition: a conditional expression (JLS 14.11.1) in which `->`
/// ends the expression instead of starting a lambda, so that `case A ->` and `when (a) ->` read
/// as javac reads them. Lambdas nested in parentheses or arguments are still lambdas. `None`,
/// consuming nothing, if no expression starts here.
pub(crate) fn case_label_expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    p.without_lambdas(conditional)
}

/// Whether an expression can start at the current token.
pub(crate) fn at_expr_start(p: &Parser<'_>) -> bool {
    p.at_set(EXPR_START) || p.at(VoidKw) && p.nth(1) == Dot
}

/// `(a, b)`, at `(`: a call's, a constructor's, or an enum constant's `Arguments`.
pub(crate) fn arguments(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at(LParen) {
        return None;
    }
    let m = p.start();
    p.bump();
    if !p.at(RParen) {
        let mut progress = p.progress();
        loop {
            if expr(p).is_none() {
                expression_expected(p);
                p.skip_until(ITEM_RECOVERY);
            }
            if p.eat(Comma) {
                if !progress.advanced(p) {
                    break;
                }
                continue;
            }
            if p.at(RParen) || p.at_eof() {
                break;
            }
            // An argument followed by something else: `f(a b)`.
            p.error_expected(Expected::OneOf(&[RParen, Comma]));
            p.skip_until(ITEM_RECOVERY);
            if !p.eat(Comma) || !progress.advanced(p) {
                break;
            }
        }
    }
    p.expect(RParen);
    Some(m.complete(p, Arguments))
}

/// A variable's initializer: an expression, or an `ArrayInitializer` at `{`.
pub(crate) fn variable_initializer(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if p.at(LBrace) {
        p.nested(array_initializer)
    } else {
        expr(p)
    }
}

/// `{1, 2,}`, at `{`.
fn array_initializer(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let m = p.start();
    p.bump();
    if p.at(Comma) && p.nth(1) == RBrace {
        // `{,}`, an empty initializer with a trailing comma.
        p.bump();
    } else {
        let mut progress = p.progress();
        while !p.at(RBrace) && !p.at_eof() {
            if variable_initializer(p).is_none() {
                expression_expected(p);
                p.skip_until(ITEM_RECOVERY);
            }
            if !p.eat(Comma) || !progress.advanced(p) {
                break;
            }
        }
    }
    p.expect(RBrace);
    Some(m.complete(p, ArrayInitializer))
}

/// Reports that an expression is missing at the current token, without consuming it.
fn expression_expected(p: &mut Parser<'_>) {
    let span = p.nth_span(0);
    p.error(
        ParseDiagnosticKind::ExpressionExpected(p.current()),
        span,
        span.start,
    );
}

// ----- Operators -----

/// An assignment, or a conditional expression. Assignments are right-associative: `a = b = c`
/// opens an `AssignExpr` per operator, parses the operands in a loop, and closes them at the end.
fn assignment(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let mut open = Vec::new();
    let mut progress = p.progress();
    let mut last = None;
    loop {
        let Some(lhs) = conditional(p) else {
            if !open.is_empty() {
                expression_expected(p);
            }
            break;
        };
        let Some(len) = assignment_operator(p) else {
            last = Some(lhs);
            break;
        };
        let m = lhs.precede(p);
        for _ in 0..len {
            p.bump();
        }
        open.push(m);
        if !progress.advanced(p) {
            break;
        }
    }
    close_all(p, open, AssignExpr, last)
}

/// Completes `open`, innermost (last) first, as `kind`; returns the outermost, or `inner` if
/// there are none.
fn close_all(
    p: &mut Parser<'_>,
    mut open: Vec<Marker>,
    kind: SyntaxKind,
    inner: Option<CompletedMarker>,
) -> Option<CompletedMarker> {
    let mut result = inner;
    while let Some(m) = open.pop() {
        result = Some(m.complete(p, kind));
    }
    result
}

/// The number of tokens of the assignment operator here, if there is one: `>>=` is `>` `>=`,
/// and `>>>=` is `>` `>` `>=`, with nothing between them.
fn assignment_operator(p: &Parser<'_>) -> Option<usize> {
    match p.current() {
        Eq | PlusEq | MinusEq | StarEq | SlashEq | PercentEq | AmpEq | PipeEq | CaretEq
        | LtLtEq => Some(1),
        Gt if p.nth_is_adjacent(1) && p.nth(1) == GtEq => Some(2),
        Gt if p.nth_is_adjacent(1)
            && p.nth(1) == Gt
            && p.nth_is_adjacent(2)
            && p.nth(2) == GtEq =>
        {
            Some(3)
        }
        _ => None,
    }
}

/// `a ? b : c`. The else branch loops: `a ? b : c ? d : e` opens a `ConditionalExpr` per `?`.
fn conditional(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let mut condition = binary(p, 1)?;
    if !p.at(Question) {
        return Some(condition);
    }
    let mut open = Vec::new();
    let mut progress = p.progress();
    loop {
        let m = condition.precede(p);
        p.bump();
        open.push(m);
        if expr(p).is_none() {
            expression_expected(p);
        }
        if !p.expect(Colon) {
            break;
        }
        match binary(p, 1) {
            None => {
                expression_expected(p);
                break;
            }
            Some(next) if p.at(Question) => condition = next,
            Some(_) => break,
        }
        if !progress.advanced(p) {
            break;
        }
    }
    close_all(p, open, ConditionalExpr, None)
}

/// Binary operators and `instanceof` binding at least as tightly as `min_power`, by precedence
/// climbing: operators of one level loop, and only a tighter operator on the right recurses, at
/// most once per level.
fn binary(p: &mut Parser<'_>, min_power: u8) -> Option<CompletedMarker> {
    let mut lhs = unary(p)?;
    let mut progress = p.progress();
    while let Some((power, len)) = binary_operator(p) {
        if power < min_power {
            break;
        }
        let m = lhs.precede(p);
        if p.at(InstanceofKw) {
            p.bump();
            instanceof_rest(p);
            lhs = m.complete(p, InstanceofExpr);
        } else {
            for _ in 0..len {
                p.bump();
            }
            if binary(p, power + 1).is_none() {
                expression_expected(p);
            }
            lhs = m.complete(p, BinaryExpr);
        }
        if !progress.advanced(p) {
            break;
        }
    }
    Some(lhs)
}

/// The binary operator here, if there is one: its binding power (higher binds tighter) and its
/// number of tokens. `>>` and `>>>` are runs of `>` with nothing between them (decision 15).
fn binary_operator(p: &Parser<'_>) -> Option<(u8, usize)> {
    let adjacent = |n: usize, kind: SyntaxKind| p.nth_is_adjacent(n) && p.nth(n) == kind;
    Some(match p.current() {
        PipePipe => (1, 1),
        AmpAmp => (2, 1),
        Pipe => (3, 1),
        Caret => (4, 1),
        Amp => (5, 1),
        EqEq | BangEq => (6, 1),
        Lt | LtEq | GtEq | InstanceofKw => (7, 1),
        Gt if adjacent(1, GtEq) => return None,
        Gt if adjacent(1, Gt) => {
            if adjacent(2, GtEq) {
                return None;
            } else if adjacent(2, Gt) {
                (8, 3)
            } else {
                (8, 2)
            }
        }
        Gt => (7, 1),
        LtLt => (8, 1),
        Plus | Minus => (9, 1),
        Star | Slash | Percent => (10, 1),
        _ => return None,
    })
}

/// After `instanceof`: a type, or a pattern (`T t`, `final T t`, `R(...)`).
fn instanceof_rest(p: &mut Parser<'_>) {
    let pattern = p.nth(types::annotations_end(p, 0)) == FinalKw
        || types::scan_type_info(p, 0, usize::MAX)
            .is_some_and(|scan| p.nth_at_set(scan.end, IDENT) || p.nth(scan.end) == LParen);
    let parsed = if pattern {
        patterns::pattern(p)
    } else {
        types::type_(p)
    };
    if parsed.is_none() {
        types::type_expected(p);
    }
}

/// Prefix operators and casts, then the operand. A run of them (`- -x`, `(A) (B) x`) opens a
/// node for each, parses the operand once, and closes them at the end.
fn unary(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let mut open: Vec<(Marker, SyntaxKind)> = Vec::new();
    let mut progress = p.progress();
    loop {
        if p.at_set(PREFIX_OP) {
            let m = p.start();
            p.bump();
            open.push((m, PrefixExpr));
        } else if p.at(LParen) && classify_paren(p) == ParenKind::Cast {
            let m = p.start();
            p.bump();
            if types::intersection_type(p).is_none() {
                types::type_expected(p);
            }
            p.expect(RParen);
            open.push((m, CastExpr));
        } else {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    let mut result = postfix(p);
    if result.is_none() && !open.is_empty() {
        expression_expected(p);
    }
    while let Some((m, kind)) = open.pop() {
        result = Some(m.complete(p, kind));
    }
    result
}

/// A primary, then member access, calls, array access, method references, and postfix `++`
/// and `--`, in a loop.
fn postfix(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let mut lhs = primary(p)?;
    // javac takes nothing after a switch expression or a lambda but binary operators.
    if matches!(lhs.kind(), SwitchExpr | LambdaExpr) {
        return Some(lhs);
    }
    let mut progress = p.progress();
    loop {
        match p.current() {
            Dot => lhs = dot_selector(p, lhs),
            LBracket => {
                let m = lhs.precede(p);
                p.bump();
                if expr(p).is_none() {
                    expression_expected(p);
                }
                p.expect(RBracket);
                lhs = m.complete(p, ArrayAccessExpr);
            }
            ColonColon => {
                lhs = method_reference(p, lhs);
                break;
            }
            _ => break,
        }
        if !progress.advanced(p) {
            break;
        }
    }
    let mut progress = p.progress();
    while p.at(PlusPlus) || p.at(MinusMinus) {
        let m = lhs.precede(p);
        p.bump();
        lhs = m.complete(p, PostfixExpr);
        if !progress.advanced(p) {
            break;
        }
    }
    Some(lhs)
}

/// At the `.` after `target`: a field access, a call, `.this`, `.super`, `.class`, or `.new`.
fn dot_selector(p: &mut Parser<'_>, target: CompletedMarker) -> CompletedMarker {
    let m = target.precede(p);
    p.bump();
    if p.at_set(IDENT) {
        p.bump();
        if p.at(LParen) {
            arguments(p);
            return m.complete(p, MethodCallExpr);
        }
        return m.complete(p, FieldAccessExpr);
    }
    match p.current() {
        Lt => {
            types::type_arguments(p);
            if p.at_set(IDENT) {
                p.bump();
                // javac reports a missing argument list at the token after the name.
                if arguments(p).is_none() {
                    expression_expected(p);
                }
                m.complete(p, MethodCallExpr)
            } else if (p.at(ThisKw) || p.at(SuperKw)) && p.nth(1) == LParen {
                p.bump();
                arguments(p);
                m.complete(p, ConstructorCallExpr)
            } else {
                p.error_expected(Expected::Identifier);
                m.complete(p, MethodCallExpr)
            }
        }
        ThisKw => {
            p.bump();
            m.complete(p, ThisExpr)
        }
        SuperKw => {
            p.bump();
            if p.at(LParen) {
                arguments(p);
                return m.complete(p, ConstructorCallExpr);
            }
            if !p.at(Dot) && !p.at(ColonColon) {
                p.error_expected(Expected::Token(Dot));
            }
            m.complete(p, SuperExpr)
        }
        ClassKw => {
            p.bump();
            m.complete(p, ClassLiteralExpr)
        }
        NewKw => new_expr(p, m, true),
        _ => {
            p.error_expected(Expected::Identifier);
            m.complete(p, FieldAccessExpr)
        }
    }
}

/// At the `::` after `qualifier`: `::m`, `::<T>m`, or `::new`.
fn method_reference(p: &mut Parser<'_>, qualifier: CompletedMarker) -> CompletedMarker {
    let m = qualifier.precede(p);
    p.bump();
    types::type_arguments(p);
    if p.at_set(IDENT) || p.at(NewKw) {
        p.bump();
    } else {
        p.error_expected(Expected::Identifier);
    }
    m.complete(p, MethodRefExpr)
}

// ----- Primaries -----

fn primary(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if p.at_set(LITERAL) {
        let m = p.start();
        p.bump();
        return Some(m.complete(p, LiteralExpr));
    }
    if p.at_set(IDENT) {
        if p.nth(1) == Arrow && p.lambdas_allowed() {
            return Some(lambda(p));
        }
        if let Some(qualified) = type_qualifier(p) {
            return Some(qualified);
        }
        let m = p.start();
        p.bump();
        if p.at(LParen) {
            arguments(p);
            return Some(m.complete(p, MethodCallExpr));
        }
        return Some(m.complete(p, NameExpr));
    }
    if p.at_set(PRIMITIVE) {
        let ty = types::type_(p)?;
        return Some(type_qualifier_rest(p, ty));
    }
    Some(match p.current() {
        LParen if p.lambdas_allowed() && classify_paren(p) == ParenKind::Lambda => lambda(p),
        LParen => {
            let m = p.start();
            p.bump();
            if expr(p).is_none() {
                expression_expected(p);
            }
            p.expect(RParen);
            m.complete(p, ParenExpr)
        }
        ThisKw | SuperKw => {
            let keyword = p.current();
            let m = p.start();
            p.bump();
            if p.at(LParen) {
                arguments(p);
                m.complete(p, ConstructorCallExpr)
            } else if keyword == ThisKw {
                m.complete(p, ThisExpr)
            } else {
                if !p.at(Dot) && !p.at(ColonColon) {
                    p.error_expected(Expected::Token(Dot));
                }
                m.complete(p, SuperExpr)
            }
        }
        NewKw => {
            let m = p.start();
            new_expr(p, m, false)
        }
        SwitchKw => switch_expr(p),
        Lt => {
            // `<T>this(...)` or `<T>super(...)`, an explicit constructor call.
            let m = p.start();
            types::type_arguments(p);
            if (p.at(ThisKw) || p.at(SuperKw)) && p.nth(1) == LParen {
                p.bump();
                arguments(p);
                m.complete(p, ConstructorCallExpr)
            } else {
                expression_expected(p);
                m.complete(p, Error)
            }
        }
        VoidKw if p.nth(1) == Dot && p.nth(2) == ClassKw => {
            let m = p.start();
            p.bump();
            let void = m.complete(p, VoidType);
            let m = void.precede(p);
            p.bump();
            p.bump();
            m.complete(p, ClassLiteralExpr)
        }
        _ => return None,
    })
}

/// At a name: if the syntax makes it the start of a type that qualifies a method reference or a
/// class literal (`List<String>::size`, `a.b[]::new`, `String[].class`), that expression.
/// Plain names stay expressions (decision 2), and a name followed by `<` that isn't a type of
/// that kind is a comparison.
fn type_qualifier(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !matches!(p.nth(1), Lt | LBracket | Dot | At) {
        return None;
    }
    let scan = types::scan_type_info(p, 0, TYPE_ARGUMENTS_BUDGET)?;
    let next = p.nth(scan.end);
    // A generic type followed by `.` can't be a comparison (`a < b > .c`), so javac reads it as
    // a type, and reports `List<String>.class` after the `.`.
    let is_type = !scan.name_only
        && (scan.dims || next == ColonColon || scan.ends_with_type_arguments && next == Dot);
    if !is_type {
        return None;
    }
    let ty = types::type_(p)?;
    Some(type_qualifier_rest(p, ty))
}

/// After a type in an expression: `::` and a method reference, or `.class`. Anything else is
/// reported after the type.
fn type_qualifier_rest(p: &mut Parser<'_>, ty: CompletedMarker) -> CompletedMarker {
    if p.at(ColonColon) {
        return method_reference(p, ty);
    }
    if p.at(Dot) {
        let m = ty.precede(p);
        p.bump();
        p.expect(ClassKw);
        return m.complete(p, ClassLiteralExpr);
    }
    let span = p.nth_span(0);
    p.error(ParseDiagnosticKind::ClassLiteralExpected, span, span.start);
    ty
}

/// At `new`, with `m` started before it, or for `outer.new`, before the outer expression
/// (`qualified`): a `NewExpr`, or for an array type, an `ArrayCreationExpr`.
fn new_expr(p: &mut Parser<'_>, m: Marker, qualified: bool) -> CompletedMarker {
    let new_start = p.nth_span(0).start;
    p.bump();
    types::type_arguments(p);
    let ty = p.start();
    types::annotations(p);
    if !qualified && p.at_set(PRIMITIVE) {
        p.bump();
        ty.complete(p, PrimitiveType);
        return array_creation_rest(p, m, new_start);
    }
    if !p.at_set(IDENT) {
        if ty.consumed(p) {
            ty.complete(p, ClassType);
        } else {
            ty.abandon(p);
        }
        p.error_expected(Expected::Identifier);
        return m.complete(p, NewExpr);
    }
    types::class_type_rest(p, ty, true);
    if p.at(LParen) {
        arguments(p);
        if p.at(LBrace) {
            // The body counts its own nesting.
            decls::class_body(p);
        }
        return m.complete(p, NewExpr);
    }
    if !qualified && p.nth(types::annotations_end(p, 0)) == LBracket {
        return array_creation_rest(p, m, new_start);
    }
    if qualified {
        types::expected_at_next(p, Expected::Token(LParen));
    } else {
        types::expected_at_next(p, Expected::OneOf(&[LParen, LBracket]));
    }
    m.complete(p, NewExpr)
}

/// After an array creation's element type: `[n]`s, then `[]`s, then an initializer when there
/// are no `[n]`s.
fn array_creation_rest(p: &mut Parser<'_>, m: Marker, new_start: u32) -> CompletedMarker {
    let mut sized = false;
    let mut progress = p.progress();
    loop {
        let bracket = types::annotations_end(p, 0);
        if p.nth(bracket) != LBracket || p.nth(bracket + 1) == RBracket {
            break;
        }
        let dim = p.start();
        types::annotations(p);
        p.bump();
        if expr(p).is_none() {
            expression_expected(p);
        }
        p.expect(RBracket);
        dim.complete(p, DimExpr);
        sized = true;
        if !progress.advanced(p) {
            break;
        }
    }
    let unsized_dims = p.at(LBracket) || p.at(At);
    types::dims(p);
    if p.at(LBrace) {
        if sized {
            let brace = p.nth_span(0);
            p.error(
                ParseDiagnosticKind::ArrayDimensionsAndInitializer,
                Span::new(new_start, brace.end),
                brace.start,
            );
        }
        p.nested(array_initializer);
    } else if !sized {
        let next = p.nth_span(0).start;
        if unsized_dims {
            p.error(
                ParseDiagnosticKind::ArrayDimensionMissing,
                Span::new(new_start, next),
                next,
            );
        } else {
            // `new int` with nothing after it.
            p.error_expected(Expected::Token(LBracket));
        }
    }
    m.complete(p, ArrayCreationExpr)
}

/// `switch (x) { ... }`, at `switch`.
fn switch_expr(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if p.expect(LParen) {
        if expr(p).is_none() {
            expression_expected(p);
        }
        p.expect(RParen);
    }
    if p.at(LBrace) {
        // The switch block counts its own nesting.
        stmts::switch_block(p);
    } else {
        p.error_expected(Expected::Token(LBrace));
    }
    m.complete(p, SwitchExpr)
}

// ----- Parentheses and lambdas -----

/// What a `(` starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ParenKind {
    Lambda,
    Cast,
    Parenthesized,
}

/// At `(`: whether it starts a lambda, a cast, or a parenthesized expression, from the tokens
/// after it. Where the reading is ambiguous, this follows javac's, as observed with the oracle:
///
/// - `()`, `(a, b`, `(T a`, `(final`, and `(T... a` start lambdas, as does `(a) ->`;
/// - `(a<b> c)` is a comparison unless `->` follows it, since `a < b > c` is an expression;
/// - `(T) x`, `(int) -x`, and `(A & B) x` are casts. A cast to a type that could be an
///   expression (`(a)`, `(a.b)`, `(a & b)`) needs an operand that can't continue an expression,
///   so `(a) - b` is a subtraction.
fn classify_paren(p: &Parser<'_>) -> ParenKind {
    use ParenKind::*;
    if p.nth(1) == RParen {
        return Lambda;
    }
    if p.nth_at_set(1, IDENT) {
        match p.nth(2) {
            RParen if p.nth(3) == Arrow => return Lambda,
            RParen => return cast_if_operand(p, 3),
            Comma => return implicit_parameters(p),
            _ => {}
        }
    }
    if p.nth(types::annotations_end(p, 1)) == FinalKw {
        return Lambda;
    }
    let Some(scan) = types::scan_type_info(p, 1, usize::MAX) else {
        return Parenthesized;
    };
    let next = scan.end;
    if p.nth_at_set(next, IDENT) {
        if scan.ends_with_type_arguments && p.nth(next + 1) == RParen && p.nth(next + 2) != Arrow {
            return Parenthesized;
        }
        return Lambda;
    }
    if p.nth(types::annotations_end(p, next)) == Ellipsis {
        return Lambda;
    }
    match p.nth(next) {
        RParen if scan.name_only => cast_if_operand(p, next + 1),
        RParen => Cast,
        Amp => {
            let mut names_only = scan.name_only;
            let mut n = next;
            while p.nth(n) == Amp {
                let Some(bound) = types::scan_type_info(p, n + 1, usize::MAX) else {
                    return Parenthesized;
                };
                names_only &= bound.name_only;
                n = bound.end;
            }
            if p.nth(n) != RParen {
                Parenthesized
            } else if names_only {
                cast_if_operand(p, n + 1)
            } else {
                Cast
            }
        }
        _ => Parenthesized,
    }
}

/// A cast if the `n`th token ahead can start a cast's operand.
fn cast_if_operand(p: &Parser<'_>, n: usize) -> ParenKind {
    if p.nth_at_set(n, CAST_OPERAND_START) {
        ParenKind::Cast
    } else {
        ParenKind::Parenthesized
    }
}

/// At `(a,`: a lambda's inferred parameters, unless a `,` is followed by `)` (`(a, )`), which
/// javac reads as a parenthesized expression.
fn implicit_parameters(p: &Parser<'_>) -> ParenKind {
    let mut n = 1;
    while p.nth_at_set(n, IDENT) && p.nth(n + 1) == Comma {
        n += 2;
    }
    if p.nth(n) == RParen {
        ParenKind::Parenthesized
    } else {
        ParenKind::Lambda
    }
}

/// A lambda, at its parameters: a name, or `(`.
fn lambda(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    let parameters = p.start();
    if p.at_set(IDENT) {
        let parameter = p.start();
        p.bump();
        parameter.complete(p, LambdaParameter);
    } else {
        let open = p.nth_span(0);
        p.bump();
        let mut inferred = false;
        let mut declared = false;
        if !p.at(RParen) {
            let mut progress = p.progress();
            loop {
                match lambda_parameter(p) {
                    Some(true) => declared = true,
                    Some(false) => inferred = true,
                    None => {}
                }
                if !p.eat(Comma) || !progress.advanced(p) {
                    break;
                }
            }
        }
        let close = p.nth_span(0);
        if p.eat(RParen) {
            if inferred && declared {
                p.error(
                    ParseDiagnosticKind::MixedLambdaParameters,
                    Span::new(open.start, close.end),
                    open.start,
                );
            }
        } else {
            // After a parameter, javac expects more of the list, or the C-style dimensions of a
            // declared one.
            p.error_expected(Expected::OneOf(&[Comma, RParen, LBracket]));
        }
    }
    parameters.complete(p, LambdaParameters);
    p.expect(Arrow);
    if p.at(LBrace) {
        // The block counts its own nesting.
        stmts::block(p);
    } else if expr(p).is_none() {
        expression_expected(p);
    }
    m.complete(p, LambdaExpr)
}

/// A parenthesized lambda's parameter: an inferred one (just a name), or a declared one with
/// modifiers, a type (`var` too), varargs, a name, and C-style dimensions. Returns whether it is
/// declared, or `None` if there is none here.
fn lambda_parameter(p: &mut Parser<'_>) -> Option<bool> {
    if p.at_set(IDENT) && matches!(p.nth(1), Comma | RParen) {
        let m = p.start();
        p.bump();
        m.complete(p, LambdaParameter);
        return Some(false);
    }
    let m = p.start();
    decls::modifiers(p);
    if types::type_(p).is_none() {
        types::type_expected(p);
        if !m.consumed(p) {
            m.abandon(p);
            return None;
        }
    }
    if p.nth(types::annotations_end(p, 0)) == Ellipsis {
        let varargs = p.start();
        types::annotations(p);
        p.bump();
        varargs.complete(p, Varargs);
    }
    if p.at_set(IDENT) {
        p.bump();
    } else {
        p.error_expected(Expected::Identifier);
    }
    types::dims(p);
    m.complete(p, LambdaParameter);
    Some(true)
}
