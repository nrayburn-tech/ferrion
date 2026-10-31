//! What `parse` reports: lexical errors, syntax errors, and (later) version errors, each with the
//! offset javac reports the same mistake at (docs/design/parser-error-recovery.md).

use ferrion_diagnostics::Span;
use ferrion_lexer::{LexError, LexErrorKind};

use crate::SyntaxKind;

/// A problem `parse` found. The tree still holds every token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub kind: ParseDiagnosticKind,
    pub severity: Severity,
    /// What an editor underlines. It can cover a whole run of unexpected tokens.
    pub span: Span,
    /// The offset javac reports the same mistake at (its diagnostic's `position`), by the rule
    /// documented on each kind.
    pub anchor: u32,
    /// When javac reports the same mistake.
    pub javac_phase: JavacPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    /// javac's parsing-phase warnings, which Ferrion produces only to compare them with javac's
    /// and never shows to users.
    Warning,
}

/// When javac reports a mistake: while parsing, in a later phase, or never. The comparison with
/// javac's parsing phase leaves out the last two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JavacPhase {
    Parse,
    /// While entering or attributing the file: syntax-shaped errors Ferrion reports earlier.
    Later,
    /// A deliberate departure from javac, such as the nesting limit.
    Never,
}

/// What went wrong. Kinds are Ferrion's own, named for its messages; the oracle comparison maps
/// each to javac's codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseDiagnosticKind {
    /// An error the lexer reported. Anchor: the start of its span.
    Lexical(LexErrorKind),
    /// Something the grammar needs here is missing. Anchor: the end of the previous non-trivia
    /// token, as for javac's `expected` errors.
    Expected(Expected),
    /// A token that nothing here can start or continue. Its span covers the run of tokens put in
    /// an `Error` node. Anchor: the start of the first, as for javac's `illegal.start.of.*`.
    UnexpectedToken(SyntaxKind),
    /// An expression is missing: the current token, given here, can't start one. Nothing is
    /// consumed. Anchor: the token's start, as for javac's `illegal.start.of.expr`.
    ExpressionExpected(SyntaxKind),
    /// A type is missing: the current token, given here, can't start one. Nothing is consumed.
    /// Anchor: the token's start, as for javac's `illegal.start.of.type`.
    TypeExpected(SyntaxKind),
    /// A primitive or array type in an expression (`int`, `a[]`) isn't followed by `.class` or
    /// `::`. Anchor: the start of the token after the type, as for javac's `dot.class.expected`.
    ClassLiteralExpected,
    /// An array creation with neither a dimension expression nor an initializer: `new int[]`. Its
    /// span runs from `new` to the token after it. Anchor: the start of that token, as for
    /// javac's `array.dimension.missing`.
    ArrayDimensionMissing,
    /// An array creation with both dimension expressions and an initializer: `new int[1] {2}`.
    /// Anchor: the initializer's `{`, as for javac's
    /// `illegal.array.creation.both.dimension.and.initialization`.
    ArrayDimensionsAndInitializer,
    /// A lambda's parameters mix inferred types (`x`) with declared ones (`int y`, `var z`). Its
    /// span covers the parameters. Anchor: the lambda's `(`, as for javac's
    /// `invalid.lambda.parameter.declaration`.
    MixedLambdaParameters,
    /// More than [`NESTING_LIMIT`](crate::NESTING_LIMIT) constructs nested in each other. javac
    /// has no such limit (docs/design/parser-deep-nesting.md). Anchor: the token that would have
    /// gone deeper.
    NestingTooDeep,
    /// A token that can't start a top-level declaration. Anchor: its start. javac's code depends
    /// on the release: `expected3` (class, interface, or enum) before 16, `expected4` (or record)
    /// from 16 to 24, and `class.method.or.field.expected` from 25.
    ExpectedDeclaration(SyntaxKind),
    /// A token that can't start a member declaration in a class body, or that can't follow a
    /// member's modifiers. Anchor: its start, as for javac's `illegal.start.of.type`.
    ExpectedMember(SyntaxKind),
    /// A token that can't start an enum constant before an enum's `;`. Anchor: its start, as for
    /// javac's `enum.constant.expected`.
    ExpectedEnumConstant(SyntaxKind),
    /// A token that can't start a module directive. Anchor: its start, as for javac's
    /// `invalid.module.directive`.
    ExpectedModuleDirective(SyntaxKind),
    /// `open` not followed by `module`. Anchor: the start of the token after `open`, as for
    /// javac's `expected.module`.
    ExpectedModuleKeyword(SyntaxKind),
    /// A modifier written twice. Anchor: the second one's start (for `non-sealed`, its `sealed`),
    /// as for javac's `repeated.modifier`.
    RepeatedModifier,
    /// A method without a result type: a constructor whose name isn't its class's. Anchor: the
    /// name, as for javac's `invalid.meth.decl.ret.type.req`.
    MissingReturnType,
    /// The same, among an enum's members: `F() {}` after the enum's `;`. Anchor: the name, as for
    /// javac's `enum.constant.not.expected`.
    MisplacedEnumConstant,
    /// Type parameters on an enum. Anchor: the first type parameter, as for javac's
    /// `enum.cant.be.generic`.
    GenericEnum,
    /// A `permits` clause on a class or interface that isn't `sealed`. Anchor: `permits`, as for
    /// javac's `invalid.permits.clause`.
    PermitsWithoutSealed,
    /// A modifier keyword where none is allowed: on a parameter (other than `final`) or a module
    /// declaration. Anchor: the token after the modifiers, as for javac's `mod.not.allowed.here`.
    ModifierNotAllowed,
    /// A modifier keyword on a record component. Anchor: the start of the modifiers, as for
    /// javac's `record.cant.declare.field.modifiers`.
    RecordComponentModifiers,
    /// `[]` after a record component's name. Anchor: the `[`, as for javac's
    /// `record.component.and.old.array.syntax`.
    RecordComponentDims,
    /// A field in a record that isn't `static`. Anchor: the first declarator's name, as for
    /// javac's `record.cannot.declare.instance.fields`.
    InstanceFieldInRecord,
    /// A variable arity parameter or record component followed by another. Anchor: its name, as
    /// for javac's `varargs.must.be.last`.
    VarargsNotLast,
    /// `[]` after a variable arity parameter's name. Anchor: the `[`, as for javac's
    /// `varargs.and.old.array.syntax`.
    VarargsWithDims,
    /// A receiver parameter (`A this`) that isn't the first parameter, or in a record header.
    /// Anchor: `this`, as for javac's `this.as.identifier`.
    MisplacedReceiver,
    /// `[]` after a receiver parameter's `this`. Anchor: the `[`, as for javac's
    /// `array.and.receiver`.
    ReceiverWithDims,
    /// Where a statement must follow (the body of an `if`, a loop, a label, or a switch rule in a
    /// switch statement), a token that can't start one: `}`, `case`, or `default`. Anchor: the
    /// start of the token, as for javac's `illegal.start.of.stmt`.
    StatementExpected,
    /// An expression statement whose expression can't be a statement, such as `a + b;`. Its span
    /// covers the expression. Anchor: javac's position for the expression: its operator for a
    /// binary, conditional, or `instanceof` expression, its `.` or `[` for a field or array
    /// access or a class literal, and otherwise its start, as for javac's `not.stmt`.
    NotAStatement,
    /// A local variable declaration where only a statement is allowed, as the body of an `if`, a
    /// loop, a label, or a switch rule. Anchor: the first declarator's name, as for javac's
    /// `variable.not.allowed`.
    LocalVariableNotAllowed,
    /// A local class, interface, enum, or record where only a statement is allowed. Anchor: its
    /// `class`, `interface`, `enum`, or `record` keyword, as for javac's `class.not.allowed`.
    LocalClassNotAllowed,
    /// `sealed` or `non-sealed` as the first modifier of a local class or interface. Anchor: the
    /// modifier, as for javac's `sealed.or.non.sealed.local.classes.not.allowed`.
    SealedLocalClass,
    /// `else` without an `if` before it. Anchor: the `else`, as for javac's `else.without.if`.
    ElseWithoutIf,
    /// `catch` without a `try` before it. Anchor: the `catch`, as for javac's
    /// `catch.without.try`.
    CatchWithoutTry,
    /// `finally` without a `try` before it. Anchor: the `finally`, as for javac's
    /// `finally.without.try`.
    FinallyWithoutTry,
    /// A `try` with no resources, `catch`, or `finally`. Anchor: the `try`, as for javac's
    /// `try.without.catch.finally.or.resource.decls`.
    TryWithoutCatch,
    /// A resource that is an expression but not a variable or field, as in `try (open()) {}`.
    /// Anchor: as for [`NotAStatement`](Self::NotAStatement), but a method call's is its `(`, as
    /// for javac's `try.with.resources.expr.needs.var`.
    ResourceNotAVariable,
    /// An enhanced `for` whose variable isn't declared: `for (x : xs)`. Anchor: the start of
    /// the variable, as for javac's `bad.initializer`.
    ForEachWithoutDeclaration,
    /// A `case` or `default` label outside a switch block. Anchor: the `case` or `default`, as for
    /// javac's `orphaned`.
    CaseOutsideSwitch,
    /// In a switch block, a token other than `case`, `default`, or `}`. Its span covers the run
    /// of tokens put in an `Error` node. Anchor: the start of the first, as for javac's
    /// `expected3` (case, default, or `}`).
    CaseExpected,
    /// `default` as a case element anywhere but after `null` (`case null, default`). Anchor: the
    /// `default`, as for javac's `default.label.not.allowed`.
    DefaultNotAllowed,
    /// A guard (`when`) on a label without a pattern: `default`, or `case null, default`. Its
    /// span covers the condition. Anchor: the `when`, as for javac's `guard.not.allowed`.
    GuardNotAllowed,
    /// A statement other than an expression statement, a block, or `throw` as the body of a
    /// switch rule in a switch statement. Anchor: the start of the statement, as for javac's
    /// `switch.case.unexpected.statement`.
    StatementNotAllowedInRule,
}

/// What an [`Expected`](ParseDiagnosticKind::Expected) error expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expected {
    /// A specific token, such as `;`.
    Token(SyntaxKind),
    /// A name.
    Identifier,
    /// One of several tokens, such as `')'` or `','` after an argument: javac's `expected2` and
    /// `expected3`.
    OneOf(&'static [SyntaxKind]),
}

impl ParseDiagnosticKind {
    pub fn javac_phase(self) -> JavacPhase {
        match self {
            ParseDiagnosticKind::Lexical(_)
            | ParseDiagnosticKind::Expected(_)
            | ParseDiagnosticKind::UnexpectedToken(_)
            | ParseDiagnosticKind::ExpressionExpected(_)
            | ParseDiagnosticKind::TypeExpected(_)
            | ParseDiagnosticKind::ClassLiteralExpected
            | ParseDiagnosticKind::ArrayDimensionMissing
            | ParseDiagnosticKind::ArrayDimensionsAndInitializer
            | ParseDiagnosticKind::MixedLambdaParameters
            | ParseDiagnosticKind::ExpectedDeclaration(_)
            | ParseDiagnosticKind::ExpectedMember(_)
            | ParseDiagnosticKind::ExpectedEnumConstant(_)
            | ParseDiagnosticKind::ExpectedModuleDirective(_)
            | ParseDiagnosticKind::ExpectedModuleKeyword(_)
            | ParseDiagnosticKind::RepeatedModifier
            | ParseDiagnosticKind::MissingReturnType
            | ParseDiagnosticKind::MisplacedEnumConstant
            | ParseDiagnosticKind::GenericEnum
            | ParseDiagnosticKind::PermitsWithoutSealed
            | ParseDiagnosticKind::ModifierNotAllowed
            | ParseDiagnosticKind::RecordComponentModifiers
            | ParseDiagnosticKind::RecordComponentDims
            | ParseDiagnosticKind::InstanceFieldInRecord
            | ParseDiagnosticKind::VarargsNotLast
            | ParseDiagnosticKind::VarargsWithDims
            | ParseDiagnosticKind::MisplacedReceiver
            | ParseDiagnosticKind::ReceiverWithDims
            | ParseDiagnosticKind::StatementExpected
            | ParseDiagnosticKind::NotAStatement
            | ParseDiagnosticKind::LocalVariableNotAllowed
            | ParseDiagnosticKind::LocalClassNotAllowed
            | ParseDiagnosticKind::SealedLocalClass
            | ParseDiagnosticKind::ElseWithoutIf
            | ParseDiagnosticKind::CatchWithoutTry
            | ParseDiagnosticKind::FinallyWithoutTry
            | ParseDiagnosticKind::TryWithoutCatch
            | ParseDiagnosticKind::ResourceNotAVariable
            | ParseDiagnosticKind::ForEachWithoutDeclaration
            | ParseDiagnosticKind::CaseOutsideSwitch
            | ParseDiagnosticKind::CaseExpected
            | ParseDiagnosticKind::DefaultNotAllowed
            | ParseDiagnosticKind::GuardNotAllowed
            | ParseDiagnosticKind::StatementNotAllowedInRule => JavacPhase::Parse,
            ParseDiagnosticKind::NestingTooDeep => JavacPhase::Never,
        }
    }
}

impl ParseDiagnostic {
    /// An error of `kind`, with javac's phase for it.
    pub fn error(kind: ParseDiagnosticKind, span: Span, anchor: u32) -> ParseDiagnostic {
        ParseDiagnostic {
            kind,
            severity: Severity::Error,
            span,
            anchor,
            javac_phase: kind.javac_phase(),
        }
    }

    pub(crate) fn lexical(error: &LexError) -> ParseDiagnostic {
        ParseDiagnostic::error(
            ParseDiagnosticKind::Lexical(error.kind),
            error.span,
            error.span.start,
        )
    }

    /// The message shown to users.
    pub fn message(&self) -> String {
        match self.kind {
            ParseDiagnosticKind::Lexical(kind) => lexical_message(kind).to_owned(),
            ParseDiagnosticKind::Expected(Expected::Token(kind)) => {
                format!("{} expected", describe(kind))
            }
            ParseDiagnosticKind::Expected(Expected::Identifier) => "name expected".to_owned(),
            ParseDiagnosticKind::Expected(Expected::OneOf(kinds)) => {
                format!("{} expected", one_of(kinds))
            }
            ParseDiagnosticKind::UnexpectedToken(kind) => format!("unexpected {}", describe(kind)),
            ParseDiagnosticKind::ExpressionExpected(kind) => {
                format!("expected an expression, found {}", describe(kind))
            }
            ParseDiagnosticKind::TypeExpected(kind) => {
                format!("expected a type, found {}", describe(kind))
            }
            ParseDiagnosticKind::ClassLiteralExpected => {
                "a type in an expression must be followed by '.class' or '::'".to_owned()
            }
            ParseDiagnosticKind::ArrayDimensionMissing => {
                "an array creation needs a dimension or an initializer".to_owned()
            }
            ParseDiagnosticKind::ArrayDimensionsAndInitializer => {
                "an array creation can't have both dimensions and an initializer".to_owned()
            }
            ParseDiagnosticKind::MixedLambdaParameters => {
                "a lambda's parameters must all have types or all be inferred".to_owned()
            }
            ParseDiagnosticKind::ExpectedDeclaration(kind) => {
                format!("unexpected {}: expected a declaration", describe(kind))
            }
            ParseDiagnosticKind::ExpectedMember(kind) => {
                format!(
                    "unexpected {}: expected a member declaration",
                    describe(kind)
                )
            }
            ParseDiagnosticKind::ExpectedEnumConstant(kind) => {
                format!("unexpected {}: expected an enum constant", describe(kind))
            }
            ParseDiagnosticKind::ExpectedModuleDirective(kind) => {
                format!("unexpected {}: expected a module directive", describe(kind))
            }
            ParseDiagnosticKind::ExpectedModuleKeyword(kind) => {
                format!(
                    "unexpected {}: 'module' expected after 'open'",
                    describe(kind)
                )
            }
            ParseDiagnosticKind::RepeatedModifier => "this modifier is already given".to_owned(),
            ParseDiagnosticKind::MissingReturnType => {
                "a method needs a result type; a constructor takes its class's name".to_owned()
            }
            ParseDiagnosticKind::MisplacedEnumConstant => {
                "enum constants come before the enum's ';'; a method needs a result type".to_owned()
            }
            ParseDiagnosticKind::GenericEnum => "an enum can't have type parameters".to_owned(),
            ParseDiagnosticKind::PermitsWithoutSealed => {
                "only a sealed class or interface has a 'permits' clause".to_owned()
            }
            ParseDiagnosticKind::ModifierNotAllowed => "modifiers aren't allowed here".to_owned(),
            ParseDiagnosticKind::RecordComponentModifiers => {
                "a record component can't have modifiers".to_owned()
            }
            ParseDiagnosticKind::RecordComponentDims => {
                "a record component's array dimensions go on its type".to_owned()
            }
            ParseDiagnosticKind::InstanceFieldInRecord => {
                "a record's fields must be static; its components are its instance fields"
                    .to_owned()
            }
            ParseDiagnosticKind::VarargsNotLast => {
                "only the last parameter can be variable arity".to_owned()
            }
            ParseDiagnosticKind::VarargsWithDims => {
                "a variable arity parameter's array dimensions go on its type".to_owned()
            }
            ParseDiagnosticKind::MisplacedReceiver => {
                "a receiver parameter ('this') can only be a method's first parameter".to_owned()
            }
            ParseDiagnosticKind::ReceiverWithDims => {
                "a receiver parameter can't have array dimensions".to_owned()
            }
            ParseDiagnosticKind::StatementExpected => "statement expected".to_owned(),
            ParseDiagnosticKind::NotAStatement => {
                "this expression can't be used as a statement".to_owned()
            }
            ParseDiagnosticKind::LocalVariableNotAllowed => {
                "a variable can't be declared here; put the statement in a block".to_owned()
            }
            ParseDiagnosticKind::LocalClassNotAllowed => {
                "a class can't be declared here; put the statement in a block".to_owned()
            }
            ParseDiagnosticKind::SealedLocalClass => {
                "a local class can't be sealed or non-sealed".to_owned()
            }
            ParseDiagnosticKind::ElseWithoutIf => "'else' without 'if'".to_owned(),
            ParseDiagnosticKind::CatchWithoutTry => "'catch' without 'try'".to_owned(),
            ParseDiagnosticKind::FinallyWithoutTry => "'finally' without 'try'".to_owned(),
            ParseDiagnosticKind::TryWithoutCatch => {
                "'try' needs resources, a 'catch', or a 'finally'".to_owned()
            }
            ParseDiagnosticKind::ResourceNotAVariable => {
                "a resource must be a declaration, a variable, or a field".to_owned()
            }
            ParseDiagnosticKind::ForEachWithoutDeclaration => {
                "the variable of an enhanced 'for' must be declared here".to_owned()
            }
            ParseDiagnosticKind::CaseOutsideSwitch => "case label outside a switch".to_owned(),
            ParseDiagnosticKind::CaseExpected => "'case', 'default', or '}' expected".to_owned(),
            ParseDiagnosticKind::DefaultNotAllowed => {
                "'default' can only follow 'null' in a case label".to_owned()
            }
            ParseDiagnosticKind::GuardNotAllowed => {
                "only a case label with a pattern can have a guard".to_owned()
            }
            ParseDiagnosticKind::StatementNotAllowedInRule => {
                "a switch rule's body must be an expression, a block, or 'throw'".to_owned()
            }
            ParseDiagnosticKind::NestingTooDeep => format!(
                "too deeply nested: more than {} levels",
                crate::NESTING_LIMIT
            ),
        }
    }
}

/// `')' or ','`, or `',', ')', or '['`.
fn one_of(kinds: &[SyntaxKind]) -> String {
    let described: Vec<_> = kinds.iter().map(|&kind| describe(kind)).collect();
    match described.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// `';'` for tokens with a fixed spelling, and a description for the others.
fn describe(kind: SyntaxKind) -> String {
    if let Some(text) = kind.fixed_text() {
        return format!("'{text}'");
    }
    match kind {
        SyntaxKind::Identifier => "name",
        SyntaxKind::Eof => "end of file",
        SyntaxKind::ErrorToken => "character",
        SyntaxKind::TextBlock => "text block",
        SyntaxKind::StringLiteral => "string literal",
        SyntaxKind::CharLiteral => "character literal",
        _ => "literal",
    }
    .to_owned()
}

fn lexical_message(kind: LexErrorKind) -> &'static str {
    match kind {
        LexErrorKind::IllegalCharacter => "illegal character",
        LexErrorKind::InvalidUnicodeEscape => "invalid Unicode escape",
        LexErrorKind::InvalidEscape => "invalid escape sequence",
        LexErrorKind::UnterminatedComment => "unterminated comment",
        LexErrorKind::UnterminatedCharLiteral => "unterminated character literal",
        LexErrorKind::UnterminatedStringLiteral => "unterminated string literal",
        LexErrorKind::UnterminatedTextBlock => "unterminated text block",
        LexErrorKind::EmptyCharLiteral => "empty character literal",
        LexErrorKind::MultipleCharsInCharLiteral => {
            "more than one character in a character literal"
        }
        LexErrorKind::LineTerminatorInCharLiteral => "line break in a character literal",
        LexErrorKind::TextBlockOpeningNotOnOwnLine => {
            "a text block's opening delimiter must end its line"
        }
        LexErrorKind::MalformedNumber => "malformed number",
    }
}
