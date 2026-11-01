//! Blocks, statements, and switch blocks (`java.grammar`: "Statements", "Switches").
//!
//! Every statement that contains a statement or a block nests through [`Parser::nested`], and
//! `else if` chains are a loop, so a long chain costs no stack and doesn't count toward the
//! nesting limit (docs/design/parser-deep-nesting.md).
//!
//! Errors stay inside their statement: a statement ends at its own `;` or `}`, and a block or
//! switch block ends at a token that can only start a member, such as `private`, which javac
//! treats the same way (docs/design/parser-error-recovery.md). Each error matches javac's first
//! error for the same mistake, by kind and position; the kinds are documented in
//! `diagnostic.rs`.

#[cfg(test)]
mod tests;

use crate::SyntaxKind::{self, *};
use crate::diagnostic::{Expected, ParseDiagnosticKind};
use crate::grammar::{IDENT, decls, eat_name, exprs, nth_at_name, patterns, types};
use crate::parser::{CompletedMarker, Marker, NodeAnchors, Parser, TokenSet};
use ferrion_syntax::token_sets;

/// Modifiers that only a member can have. In a block, one closes the block, as an unclosed
/// method body is the likeliest cause; javac does the same.
const MEMBER_ONLY: TokenSet = TokenSet::new(&[
    PublicKw,
    ProtectedKw,
    PrivateKw,
    StaticKw,
    NativeKw,
    TransientKw,
    VolatileKw,
]);

/// Modifiers a local class can have but a local variable can't.
const CLASS_ONLY: TokenSet = TokenSet::new(&[AbstractKw, StrictfpKw]);

/// Tokens after `sealed` or `non-sealed` that make it a modifier in a block.
const AFTER_SEALED: TokenSet = TokenSet::new(&[
    ClassKw,
    InterfaceKw,
    AbstractKw,
    FinalKw,
    StrictfpKw,
    StaticKw,
    SealedKw,
    NonSealedKw,
    At,
]);

const LITERAL: TokenSet = TokenSet::new(token_sets::LITERAL);
const PRIMITIVE: TokenSet = TokenSet::new(token_sets::PRIMITIVE_TYPE_KEYWORD);

/// Where a statement is parsed, which decides what it may be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    /// In a block or a switch group: any block statement, declarations included.
    Block,
    /// The body of an `if`, a loop, or a label: a statement, but not a declaration (JLS 14.5).
    Substatement,
}

/// Which construct a switch block belongs to: a rule's body is checked differently in each.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Switch {
    Statement,
    Expression,
}

/// Keywords that start a statement and nothing else. Where a member or a top-level declaration
/// should be, javac parses the statement and reports it (see [`misplaced_statement`]).
pub(crate) const STATEMENT_KEYWORDS: TokenSet = TokenSet::new(&[
    IfKw, ForKw, WhileKw, DoKw, TryKw, SwitchKw, ReturnKw, ThrowKw, BreakKw, ContinueKw, AssertKw,
]);

/// A statement where a member or a top-level declaration should be, at one of
/// [`STATEMENT_KEYWORDS`]: reported at the keyword, as javac's `statement.not.expected`, and
/// parsed into an `Error` node.
pub(crate) fn misplaced_statement(p: &mut Parser<'_>) {
    let m = p.start();
    report_here(p, ParseDiagnosticKind::StatementNotExpected);
    p.nested(|p| {
        statement(p, Context::Block);
        Some(())
    });
    m.complete(p, Error);
}

/// A `Block`, at `{`: a method, constructor, initializer, or lambda body, or a statement.
/// Reports a missing `{` and returns `None` if the current token isn't one.
pub(crate) fn block(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if !p.at(LBrace) {
        p.error_expected(Expected::Token(LBrace));
        return None;
    }
    p.nested(|p| Some(block_at_brace(p)))
}

fn block_at_brace(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    block_statements(p, TokenSet::EMPTY);
    p.expect(RBrace);
    m.complete(p, Block)
}

/// Block statements up to `}`, the end of the file, a token in `stop`, or a token only a member
/// can start, which is reported. Consumes none of those.
fn block_statements(p: &mut Parser<'_>, stop: TokenSet) {
    let mut progress = p.progress();
    while !p.at(RBrace) && !p.at_eof() && !p.at_set(stop) {
        if p.at_set(MEMBER_ONLY) {
            expression_expected(p);
            break;
        }
        statement(p, Context::Block);
        if !progress.advanced(p) {
            break;
        }
    }
}

/// A statement where one must follow, as a body: nested one level deeper. A block body is one
/// level, not two: the block counts its own nesting.
fn substatement(p: &mut Parser<'_>) {
    if p.at(LBrace) {
        block(p);
        return;
    }
    p.nested(|p| {
        statement(p, Context::Substatement);
        Some(())
    });
}

/// One statement. Consumes at least one token unless the current token is `}`, `case`, or
/// `default` in a substatement, a member-only modifier, or the end of the file, each of which is
/// reported.
fn statement(p: &mut Parser<'_>, context: Context) {
    match p.current() {
        LBrace => {
            p.nested(|p| Some(block_at_brace(p)));
        }
        Semi => {
            let m = p.start();
            p.bump();
            m.complete(p, EmptyStmt);
        }
        IfKw => if_stmt(p),
        WhileKw => while_stmt(p),
        DoKw => do_stmt(p),
        ForKw => for_stmt(p),
        TryKw => try_stmt(p),
        SwitchKw => switch_stmt(p),
        SynchronizedKw => synchronized_stmt(p),
        ReturnKw => return_stmt(p),
        ThrowKw => {
            throw_stmt(p);
        }
        BreakKw => jump_stmt(p, BreakStmt),
        ContinueKw => jump_stmt(p, ContinueStmt),
        AssertKw => assert_stmt(p),
        ElseKw => else_without_if(p, context),
        CatchKw => {
            let m = p.start();
            report_here(p, ParseDiagnosticKind::CatchWithoutTry);
            catch_clause(p);
            m.complete(p, Error);
        }
        FinallyKw => {
            let m = p.start();
            report_here(p, ParseDiagnosticKind::FinallyWithoutTry);
            finally_clause(p);
            m.complete(p, Error);
        }
        CaseKw | DefaultKw if context == Context::Block => case_outside_switch(p),
        CaseKw | DefaultKw | RBrace | Eof => report_here(p, ParseDiagnosticKind::StatementExpected),
        _ if p.at_set(MEMBER_ONLY) => expression_expected(p),
        _ if at_yield_stmt(p) => yield_stmt(p),
        _ if p.at_set(IDENT) && p.nth(1) == Colon => labeled_stmt(p),
        _ if at_local_decl(p) => local_decl(p, context),
        _ => expr_stmt(p),
    }
}

// ----- Declarations -----

/// Whether a local class, interface, enum, record, or variable declaration starts here, from
/// its modifiers, its keyword, or a type followed by a name.
fn at_local_decl(p: &Parser<'_>) -> bool {
    match p.current() {
        FinalKw | AbstractKw | StrictfpKw | At | ClassKw | InterfaceKw | EnumKw => true,
        _ if at_sealed_modifier(p, 0) => true,
        _ if decls::at_type_decl(p) => true,
        _ => at_local_var_decl(p, 0),
    }
}

/// Whether `sealed` or `non-sealed` at the `n`th token ahead is a modifier: followed by another
/// modifier or `class` or `interface`.
fn at_sealed_modifier(p: &Parser<'_>, n: usize) -> bool {
    let after = if p.nth_at(n, NonSealedKw) {
        n + 3
    } else if p.nth_at(n, SealedKw) {
        n + 1
    } else {
        return false;
    };
    p.nth_at_set(after, AFTER_SEALED)
}

/// Whether a type followed by a name starts at the `n`th token ahead, as in a local variable
/// declaration without modifiers. `void` never starts one.
fn at_local_var_decl(p: &Parser<'_>, n: usize) -> bool {
    p.nth(n) != VoidKw && types::scan_type(p, n).is_some_and(|after| nth_at_name(p, after, false))
}

/// Whether a local class or interface (after its modifiers) starts here.
fn at_local_type_decl(p: &Parser<'_>) -> bool {
    matches!(p.current(), ClassKw | InterfaceKw | EnumKw)
        || p.at(At) && p.nth(1) == InterfaceKw
        || decls::at_type_decl(p)
}

/// A local class, interface, enum, record, or variable declaration.
fn local_decl(p: &mut Parser<'_>, context: Context) {
    let (sealed, class_only) = scan_modifiers(p);
    let m = p.start();
    if let Some(sealed) = sealed {
        p.error(ParseDiagnosticKind::SealedLocalClass, sealed, sealed.start);
    }
    decls::modifiers(p);
    if at_local_type_decl(p) {
        if context != Context::Block {
            let keyword = if p.at(At) {
                p.nth_span(1)
            } else {
                p.nth_span(0)
            };
            p.error(
                ParseDiagnosticKind::LocalClassNotAllowed,
                keyword,
                keyword.start,
            );
        }
        decls::type_decl(p, m);
        return;
    }
    if class_only {
        // javac expects a class here.
        report_here(p, ParseDiagnosticKind::ExpectedDeclaration(p.current()));
    }
    local_var_decl_rest(p, m, context, Terminator::Semi);
}

/// Scans the modifier keywords and simple annotations at the current token, without consuming
/// them, for a modifier only a class can have, and for `sealed` or `non-sealed` as the first
/// token (its span): javac reports a sealed local class only then, not after another modifier.
fn scan_modifiers(p: &Parser<'_>) -> (Option<ferrion_diagnostics::Span>, bool) {
    const KEYWORDS: TokenSet = TokenSet::new(token_sets::MODIFIER);
    let mut sealed = None;
    let mut class_only = false;
    let mut n = 0;
    loop {
        if at_sealed_modifier(p, n) {
            if n == 0 {
                sealed = Some(p.nth_span(n));
            }
            n += if p.nth_at(n, NonSealedKw) { 3 } else { 1 };
        } else if p.nth(n) == At && p.nth(n + 1) != InterfaceKw {
            // `@a.B`, without arguments: enough for the usual cases.
            n += 1;
            while p.nth_at_set(n, IDENT) {
                n += 1;
                if p.nth(n) != Dot {
                    break;
                }
                n += 1;
            }
        } else if p.nth_at_set(n, KEYWORDS) && p.nth(n) != DefaultKw {
            class_only |= p.nth_at_set(n, CLASS_ONLY);
            n += 1;
        } else {
            return (sealed, class_only);
        }
    }
}

/// What ends a local variable declaration.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Terminator {
    /// A statement: its `;`.
    Semi,
    /// A `for` header, an enhanced `for`, or a resource: the enclosing construct's token.
    None,
}

/// The rest of a local variable declaration after its modifiers, which `m` was started before:
/// its type, declarators, and `;`.
fn local_var_decl_rest(
    p: &mut Parser<'_>,
    m: Marker,
    context: Context,
    terminator: Terminator,
) -> Option<CompletedMarker> {
    if types::type_(p).is_none() {
        type_expected(p);
    } else if context == Context::Substatement && p.at_set(IDENT) {
        let name = p.nth_span(0);
        p.error(
            ParseDiagnosticKind::LocalVariableNotAllowed,
            name,
            name.start,
        );
    }
    decls::variable_declarators(p);
    if terminator == Terminator::Semi {
        p.expect(Semi);
    }
    complete_or_abandon(p, m, LocalVarDecl)
}

// ----- Expression statements -----

/// An expression statement, or if no expression starts here, the current token in an `Error`
/// node (unless it is `}`).
fn expr_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    if statement_expr(p).is_none() {
        m.abandon(p);
        // Reported as a missing expression, as javac does; the token isn't one.
        expression_expected(p);
        if !p.at(RBrace) {
            p.error_and_bump();
        }
        return;
    }
    p.expect(Semi);
    m.complete(p, ExprStmt);
}

/// An expression that must be a statement expression (JLS 14.8): an assignment, an increment
/// or decrement, a method or constructor call, or a class instance creation. Reports any other.
/// `None`, consuming and reporting nothing, if no expression starts here.
fn statement_expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let increment = p.at(PlusPlus) || p.at(MinusMinus);
    p.mark_statement_expr_start();
    let expr = exprs::expr(p)?;
    let allowed = match expr.kind() {
        AssignExpr | PostfixExpr | MethodCallExpr | ConstructorCallExpr | NewExpr | Error => true,
        PrefixExpr => increment,
        _ => false,
    };
    if !allowed {
        report_at_expr(p, expr, ParseDiagnosticKind::NotAStatement);
    }
    Some(expr)
}

/// Reports `kind` about the expression `expr`, just parsed, at javac's position for it.
fn report_at_expr(p: &mut Parser<'_>, expr: CompletedMarker, kind: ParseDiagnosticKind) {
    let anchors = p.last_node_anchors(expr);
    let span = ferrion_diagnostics::Span::new(anchors.start.start, anchors.end);
    p.error(kind, span, javac_position(expr.kind(), anchors));
}

/// Where javac reports an error about an expression of `kind`: its preferred position, which is
/// its operator, `.`, or `[` for the nodes that have one, a call's `(`, and otherwise its start
/// (docs/design/parser-oracle-normalization.md, "Positions").
fn javac_position(kind: SyntaxKind, anchors: NodeAnchors) -> u32 {
    let direct = match kind {
        BinaryExpr | ConditionalExpr | InstanceofExpr | FieldAccessExpr | ArrayAccessExpr
        | ClassLiteralExpr | AssignExpr | PostfixExpr | ThisExpr | SuperExpr => {
            anchors.first_direct_token
        }
        MethodCallExpr => anchors.after_last_direct_token,
        _ => None,
    };
    direct.unwrap_or(anchors.start).start
}

/// An expression, or if none starts here, a report at the current token, which isn't consumed.
fn expr_or_report(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let expr = exprs::expr(p);
    if expr.is_none() {
        expression_expected(p);
    }
    expr
}

/// `(expr)`, the condition of `if`, `while`, `do`, `switch`, and `synchronized`.
fn parenthesized(p: &mut Parser<'_>) {
    p.expect(LParen);
    expr_or_report(p);
    p.expect(RParen);
}

// ----- Statements -----

/// `if`, with any `else if` chain as a loop: one `IfStmt` is opened per `if`, and all are closed
/// at the end, each the else branch of the one before.
fn if_stmt(p: &mut Parser<'_>) {
    let mut open = Vec::new();
    let mut progress = p.progress();
    loop {
        let m = p.start();
        p.bump();
        parenthesized(p);
        substatement(p);
        open.push(m);
        if !p.eat(ElseKw) {
            break;
        }
        if !p.at(IfKw) {
            substatement(p);
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    while let Some(m) = open.pop() {
        m.complete(p, IfStmt);
    }
}

fn while_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    parenthesized(p);
    substatement(p);
    m.complete(p, WhileStmt);
}

fn do_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    substatement(p);
    if p.expect(WhileKw) {
        parenthesized(p);
    }
    p.expect(Semi);
    m.complete(p, DoStmt);
}

/// A basic or enhanced `for`, told apart after the declaration or expressions before `;` or
/// `:`.
fn for_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    p.expect(LParen);
    let mut enhanced = false;
    if at_for_decl(p) {
        // `T x :` is an enhanced `for`; anything else, a basic one's `ForInit`.
        let init = p.start();
        let decl = p.start();
        decls::modifiers(p);
        enhanced = at_for_each_variable(p);
        local_var_decl_rest(p, decl, Context::Block, Terminator::None);
        if enhanced {
            init.abandon(p);
        } else {
            complete_or_abandon(p, init, ForInit);
        }
    } else if !p.at(Semi) {
        let init = p.start();
        if let Some(first) = for_expressions(p).filter(|_| p.at(Colon)) {
            // `for (x : xs)`: javac's "bad initializer" replaces "not a statement". Other
            // expressions before `:` are a basic `for` missing its `;`, as for javac.
            let span = p.last_node_anchors(first).start;
            p.error(
                ParseDiagnosticKind::ForEachWithoutDeclaration,
                span,
                span.start,
            );
            enhanced = true;
            complete_or_abandon(p, init, Error);
        } else {
            complete_or_abandon(p, init, ForInit);
        }
    }
    if enhanced {
        p.expect(Colon);
        expr_or_report(p);
    } else {
        p.expect(Semi);
        if !p.at(Semi) {
            expr_or_report(p);
        }
        p.expect(Semi);
        if !p.at(RParen) {
            let update = p.start();
            for_expressions(p);
            complete_or_abandon(p, update, ForUpdate);
        }
    }
    p.expect(RParen);
    substatement(p);
    m.complete(p, if enhanced { ForEachStmt } else { ForStmt });
}

/// Whether a `for` header's declaration starts here.
fn at_for_decl(p: &Parser<'_>) -> bool {
    matches!(p.current(), FinalKw | At) || at_local_var_decl(p, 0)
}

/// After a `for` declaration's modifiers: whether its type is followed by one name, perhaps
/// with dimensions, and `:`.
fn at_for_each_variable(p: &Parser<'_>) -> bool {
    let Some(mut n) = types::scan_type(p, 0) else {
        return false;
    };
    if !p.nth_at_set(n, IDENT) {
        return false;
    }
    n += 1;
    while p.nth(n) == LBracket && p.nth(n + 1) == RBracket {
        n += 2;
    }
    p.nth(n) == Colon
}

/// Statement expressions separated by commas, in a `for` header. For a name followed by `:`,
/// the variable of an enhanced `for` that wasn't declared, returns the name instead, which isn't
/// reported as "not a statement".
fn for_expressions(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    if at_for_each_name(p) {
        return exprs::expr(p);
    }
    let mut progress = p.progress();
    loop {
        if statement_expr(p).is_none() {
            expression_expected(p);
            break;
        }
        if !p.eat(Comma) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    None
}

/// Whether a name, `a.b.c`, followed by `:` starts here: an enhanced `for` without a declared
/// variable.
fn at_for_each_name(p: &Parser<'_>) -> bool {
    let mut n = 0;
    loop {
        if !p.nth_at_set(n, IDENT) {
            return false;
        }
        n += 1;
        match p.nth(n) {
            Dot => n += 1,
            Colon => return true,
            _ => return false,
        }
    }
}

fn labeled_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    p.bump();
    substatement(p);
    m.complete(p, LabeledStmt);
}

/// `break` or `continue`, with an optional label.
fn jump_stmt(p: &mut Parser<'_>, kind: SyntaxKind) {
    let m = p.start();
    p.bump();
    if p.at_set(IDENT) {
        p.bump();
    }
    p.expect(Semi);
    m.complete(p, kind);
}

fn return_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    if !p.at(Semi) {
        expr_or_report(p);
    }
    p.expect(Semi);
    m.complete(p, ReturnStmt);
}

fn throw_stmt(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    expr_or_report(p);
    p.expect(Semi);
    m.complete(p, ThrowStmt)
}

fn assert_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    expr_or_report(p);
    if p.eat(Colon) {
        expr_or_report(p);
    }
    p.expect(Semi);
    m.complete(p, AssertStmt);
}

fn synchronized_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    parenthesized(p);
    block(p);
    m.complete(p, SynchronizedStmt);
}

/// Whether a `yield` statement starts here: from release 14, `yield` followed by a token that
/// can start an expression, or by `;`. `yield = 1`, `yield.f()`, `yield++;`, `yield: ...`, and
/// `yield<T> x` keep reading `yield` as a name, as javac's do. At 13 and below, `yield` is
/// always a name (docs/design/parser-language-versions.md).
fn at_yield_stmt(p: &Parser<'_>) -> bool {
    if p.release() < 14 || !p.at(YieldKw) {
        return false;
    }
    match p.nth(1) {
        // `yield++;` increments a variable named `yield`; `yield ++x;` yields.
        PlusPlus | MinusMinus => p.nth(2) != Semi,
        Semi | Plus | Minus | Bang | Tilde | LParen | At | ThisKw | SuperKw | NewKw | SwitchKw
        | VoidKw => true,
        _ => p.nth_at_set(1, IDENT) || p.nth_at_set(1, LITERAL) || p.nth_at_set(1, PRIMITIVE),
    }
}

fn yield_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump_as(YieldKw);
    expr_or_report(p);
    p.expect(Semi);
    m.complete(p, YieldStmt);
}

/// `else` without an `if`: in an `Error` node. As a substatement, the statement after it takes
/// its place, as with javac.
fn else_without_if(p: &mut Parser<'_>, context: Context) {
    let mut progress = p.progress();
    while p.at(ElseKw) {
        let m = p.start();
        report_here(p, ParseDiagnosticKind::ElseWithoutIf);
        p.bump();
        m.complete(p, Error);
        if !progress.advanced(p) {
            break;
        }
    }
    if context == Context::Substatement {
        statement(p, context);
    }
}

/// A case label in a block, in an `Error` node with its `:` or `->`.
fn case_outside_switch(p: &mut Parser<'_>) {
    let m = p.start();
    report_here(p, ParseDiagnosticKind::CaseOutsideSwitch);
    switch_label(p);
    if !p.eat(Colon) && !p.eat(Arrow) {
        p.error_expected(Expected::OneOf(&[Colon, Arrow]));
    }
    m.complete(p, Error);
}

// ----- try -----

fn try_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    let keyword = p.nth_span(0);
    p.bump();
    let resources = p.at(LParen);
    if resources {
        resource_specification(p);
    }
    if !p.at(LBrace) {
        // javac doesn't attach the catches to a try without a block, so reports it as without
        // them first.
        if !resources {
            p.error(ParseDiagnosticKind::TryWithoutCatch, keyword, keyword.start);
        }
        p.error_expected(Expected::Token(LBrace));
        m.complete(p, TryStmt);
        return;
    }
    block(p);
    let mut handled = false;
    let mut progress = p.progress();
    while p.at(CatchKw) {
        catch_clause(p);
        handled = true;
        if !progress.advanced(p) {
            break;
        }
    }
    if p.at(FinallyKw) {
        finally_clause(p);
        handled = true;
    }
    if !handled && !resources {
        p.error(ParseDiagnosticKind::TryWithoutCatch, keyword, keyword.start);
    }
    m.complete(p, TryStmt);
}

/// `(R r = open(); s)`, at `(`.
fn resource_specification(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    if p.at(RParen) {
        expression_expected(p);
    }
    let mut progress = p.progress();
    while !p.at(RParen) && !p.at_eof() {
        resource(p);
        if !p.eat(Semi) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    p.expect(RParen);
    m.complete(p, ResourceSpecification);
}

/// A declared resource, or from Java 9 a variable or field.
fn resource(p: &mut Parser<'_>) {
    if at_for_decl(p) {
        let m = p.start();
        decls::modifiers(p);
        // A declared resource needs an initializer; javac reports `=` missing after the name.
        let needs_initializer = types::scan_type(p, 0).is_some_and(|after| {
            p.nth_at_set(after, IDENT) && !matches!(p.nth(after + 1), Eq | LBracket)
        });
        local_var_decl_rest(p, m, Context::Block, Terminator::None);
        if needs_initializer {
            p.error_expected(Expected::Token(Eq));
        }
        return;
    }
    let Some(expr) = expr_or_report(p) else {
        return;
    };
    if !matches!(expr.kind(), NameExpr | FieldAccessExpr | Error) {
        report_at_expr(p, expr, ParseDiagnosticKind::ResourceNotAVariable);
    }
}

fn catch_clause(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    p.expect(LParen);
    catch_parameter(p);
    p.expect(RParen);
    block(p);
    m.complete(p, CatchClause);
}

/// `final A | B e`: modifiers, a type or a `UnionType`, and a name.
fn catch_parameter(p: &mut Parser<'_>) {
    let m = p.start();
    decls::modifiers(p);
    if types::union_type(p).is_none() {
        type_expected(p);
    }
    if eat_name(p, false) {
        types::dims(p);
    } else {
        p.error_expected(Expected::Identifier);
    }
    complete_or_abandon(p, m, CatchParameter);
}

fn finally_clause(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    block(p);
    m.complete(p, FinallyClause);
}

// ----- switch -----

fn switch_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    parenthesized(p);
    switch_block_of(p, Switch::Statement);
    m.complete(p, SwitchStmt);
}

/// A `SwitchBlock`, at `{`, for a switch expression; switch statements use the same grammar.
/// Reports a missing `{` and returns `None` if the current token isn't one.
pub(crate) fn switch_block(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    switch_block_of(p, Switch::Expression)
}

fn switch_block_of(p: &mut Parser<'_>, switch: Switch) -> Option<CompletedMarker> {
    if !p.at(LBrace) {
        p.error_expected(Expected::Token(LBrace));
        return None;
    }
    p.nested(|p| {
        const LABELS: TokenSet = TokenSet::new(&[CaseKw, DefaultKw]);
        let m = p.start();
        p.bump();
        let mut progress = p.progress();
        while !p.at(RBrace) && !p.at_eof() && !p.at_set(MEMBER_ONLY) {
            if p.at_set(LABELS) {
                switch_item(p, switch);
            } else {
                report_here(p, ParseDiagnosticKind::CaseExpected);
                if p.skip_until(LABELS.union(MEMBER_ONLY)).is_none() {
                    p.error_and_bump();
                }
            }
            if !progress.advanced(p) {
                break;
            }
        }
        if p.at_set(MEMBER_ONLY) {
            expression_expected(p);
        }
        p.expect(RBrace);
        Some(m.complete(p, SwitchBlock))
    })
}

/// A `SwitchRule` or a `SwitchGroup`, at `case` or `default`. A group takes every label that
/// ends with `:`, then its statements; a label that ends with `->` starts a rule.
fn switch_item(p: &mut Parser<'_>, switch: Switch) {
    let m = p.start();
    if label_ends_with_arrow(p) {
        switch_label(p);
        if !p.eat(Arrow) {
            p.error_expected(Expected::OneOf(&[Colon, Arrow]));
        }
        rule_body(p, switch);
        m.complete(p, SwitchRule);
        return;
    }
    let mut progress = p.progress();
    loop {
        switch_label(p);
        if !p.eat(Colon) {
            p.error_expected(Expected::OneOf(&[Colon, Arrow]));
        }
        if !(p.at(CaseKw) || p.at(DefaultKw)) || label_ends_with_arrow(p) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    block_statements(p, TokenSet::new(&[CaseKw, DefaultKw]));
    m.complete(p, SwitchGroup);
}

/// Whether the case label at the current token ends with `->` rather than `:`. Scans to the
/// first `->` or `:` outside delimiters, past the `:` of any `?:`.
fn label_ends_with_arrow(p: &Parser<'_>) -> bool {
    let mut depth = 0usize;
    let mut conditionals = 0usize;
    let mut n = 1;
    loop {
        match p.nth(n) {
            LParen | LBracket | LBrace => depth += 1,
            RParen | RBracket | RBrace if depth == 0 => return false,
            RParen | RBracket | RBrace => depth -= 1,
            Arrow if depth == 0 => return true,
            Question if depth == 0 => conditionals += 1,
            Colon if depth == 0 && conditionals > 0 => conditionals -= 1,
            Colon | Semi | Eof if depth == 0 => return false,
            Eof => return false,
            _ => {}
        }
        n += 1;
    }
}

/// A switch rule's body after `->`: a block, a `throw`, or an expression and `;`. In a switch
/// statement, the expression must be a statement expression, and another statement is reported
/// and parsed.
fn rule_body(p: &mut Parser<'_>, switch: Switch) {
    match p.current() {
        LBrace => {
            p.nested(|p| Some(block_at_brace(p)));
        }
        ThrowKw => {
            throw_stmt(p);
        }
        _ if switch == Switch::Expression => {
            expr_or_report(p);
            p.expect(Semi);
        }
        // A missing body, as for javac: `case 1 -> }`.
        RBrace | CaseKw | DefaultKw | Eof => {
            report_here(p, ParseDiagnosticKind::StatementExpected);
        }
        _ if at_statement_keyword(p) => {
            report_here(p, ParseDiagnosticKind::StatementNotAllowedInRule);
            substatement(p);
        }
        _ if at_local_decl(p) => {
            // Not nested itself: a local class's body and an initializer count their own.
            local_decl(p, Context::Substatement);
        }
        _ => {
            if statement_expr(p).is_none() {
                expression_expected(p);
            }
            p.expect(Semi);
        }
    }
}

/// Whether a statement other than an expression statement or a declaration starts here.
fn at_statement_keyword(p: &Parser<'_>) -> bool {
    matches!(
        p.current(),
        Semi | IfKw
            | WhileKw
            | DoKw
            | ForKw
            | TryKw
            | SwitchKw
            | SynchronizedKw
            | ReturnKw
            | BreakKw
            | ContinueKw
            | AssertKw
    ) || at_yield_stmt(p)
        || p.at_set(IDENT) && p.nth(1) == Colon
}

/// A `SwitchLabel`, at `case` or `default`: `default`, or `case` and its elements (constants,
/// patterns, `null`, and `default` after `null`) and an optional guard.
fn switch_label(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    if p.eat(DefaultKw) {
        if p.at(WhenKw) {
            guard(p, true);
        }
        return m.complete(p, SwitchLabel);
    }
    p.expect(CaseKw);
    let mut patterns = false;
    let mut null_default = false;
    let mut elements = 0;
    let mut after_null = false;
    let mut progress = p.progress();
    loop {
        if p.at(DefaultKw) {
            if after_null && elements == 1 {
                null_default = true;
            } else {
                report_here(p, ParseDiagnosticKind::DefaultNotAllowed);
            }
            p.bump();
        } else if patterns::at_case_pattern(p) {
            patterns::pattern(p);
            patterns = true;
        } else {
            after_null = elements == 0 && p.at(NullKw) && p.nth(1) == Comma;
            if exprs::case_label_expr(p).is_none() {
                expression_expected(p);
                break;
            }
        }
        elements += 1;
        if !p.eat(Comma) {
            break;
        }
        if !progress.advanced(p) {
            break;
        }
    }
    if p.at(WhenKw) && (patterns || null_default) {
        guard(p, !patterns);
    }
    m.complete(p, SwitchLabel)
}

/// `when condition`, at `when`. A guard on a label without a pattern is reported.
fn guard(p: &mut Parser<'_>, not_allowed: bool) {
    let m = p.start();
    let keyword = p.nth_span(0);
    p.bump_as(WhenKw);
    if not_allowed {
        p.error(ParseDiagnosticKind::GuardNotAllowed, keyword, keyword.start);
    }
    if exprs::case_label_expr(p).is_none() {
        expression_expected(p);
    }
    m.complete(p, Guard);
}

// ----- Helpers -----

/// Reports `kind` at the current token, without consuming it.
fn report_here(p: &mut Parser<'_>, kind: ParseDiagnosticKind) {
    let span = p.nth_span(0);
    p.error(kind, span, span.start);
}

/// Reports that an expression is missing at the current token, without consuming it: javac's
/// "illegal start of expression".
fn expression_expected(p: &mut Parser<'_>) {
    report_here(p, ParseDiagnosticKind::ExpressionExpected(p.current()));
}

/// Reports that a type is missing at the current token, without consuming it: javac's "illegal
/// start of type".
fn type_expected(p: &mut Parser<'_>) {
    report_here(p, ParseDiagnosticKind::TypeExpected(p.current()));
}

fn complete_or_abandon(p: &mut Parser<'_>, m: Marker, kind: SyntaxKind) -> Option<CompletedMarker> {
    if m.consumed(p) {
        Some(m.complete(p, kind))
    } else {
        m.abandon(p);
        None
    }
}
