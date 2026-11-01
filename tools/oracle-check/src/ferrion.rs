//! Ferrion's side of the parser checks: [`Ferrion`] parses a file at a release and gives its
//! errors and tree in javac's terms. The tree mapping is in [`tree`]; the error mapping is here.
//! Both follow docs/design/parser-oracle-normalization.md.

pub mod tree;

use std::sync::Arc;

use ferrion_diagnostics::{SourceText, Span};
use ferrion_lexer::LexErrorKind;
use ferrion_parser::{
    Expected, JavacPhase, Parse, ParseDiagnostic, ParseDiagnosticKind as Kind, ParseOptions,
    Severity, SyntaxKind, SyntaxTree,
};

use crate::syntax::{Parser, SyntaxError, TreeNode};

/// The parser, as the checks compare it.
pub struct Ferrion;

/// A file parsed at a release.
pub struct Parsed {
    parse: Parse,
    release: u32,
}

impl Parser for Ferrion {
    type Parsed = Parsed;

    fn parse(&self, source: &SourceText, release: u32) -> Option<Parsed> {
        let parse = ferrion_parser::parse(Arc::from(source.text()), ParseOptions { release });
        Some(Parsed { parse, release })
    }

    /// In the order javac would meet them: source order, with `premature.eof` last, since javac
    /// reports it on reaching the end of the file, after any lexical error in trailing comments.
    fn errors(&self, parsed: &Parsed) -> Vec<SyntaxError> {
        let tree = &parsed.parse.tree;
        let mut errors: Vec<SyntaxError> = parsed
            .parse
            .diagnostics
            .iter()
            .filter_map(|diagnostic| javac_error(diagnostic, tree, parsed.release))
            .collect();
        let eof = tree.text().len() as u32;
        errors.sort_by_key(|error| {
            if error.code == "compiler.err.premature.eof" {
                eof
            } else {
                error.position
            }
        });
        errors
    }

    fn tree(&self, parsed: &Parsed) -> Vec<TreeNode> {
        tree::map(&parsed.parse.tree, parsed.release)
    }
}

/// A diagnostic as javac reports the same mistake while parsing: its code and position. `None`
/// for warnings and for what javac reports later or never.
pub fn javac_error(
    diagnostic: &ParseDiagnostic,
    tree: &SyntaxTree,
    release: u32,
) -> Option<SyntaxError> {
    if diagnostic.severity != Severity::Error || diagnostic.javac_phase != JavacPhase::Parse {
        return None;
    }
    let (code, position) = match diagnostic.kind {
        Kind::Lexical(kind) => lexical(kind, diagnostic.span, tree),
        kind => match at_end_of_file(tree, diagnostic.anchor) {
            // javac reports any syntax error at the end of the file as `premature.eof`, at the end
            // of the last token.
            Some(end) => ("premature.eof", end),
            None => (syntax_code(kind, release), diagnostic.anchor),
        },
    };
    Some(SyntaxError {
        code: format!("compiler.err.{code}"),
        position,
    })
}

/// javac's code for a syntax error.
fn syntax_code(kind: Kind, release: u32) -> &'static str {
    match kind {
        Kind::Lexical(_) => unreachable!("lexical errors are mapped separately"),
        Kind::Expected(Expected::Token(SyntaxKind::WithKw)) => "expected.str",
        Kind::Expected(Expected::Token(_) | Expected::Identifier) => "expected",
        Kind::Expected(Expected::OneOf(kinds)) => match kinds.len() {
            0 | 1 => "expected",
            2 => "expected2",
            3 => "expected3",
            _ => "expected4",
        },
        // The parser's generic recovery, where no more specific kind applies yet.
        Kind::UnexpectedToken(_) | Kind::ExpressionExpected(_) => "illegal.start.of.expr",
        Kind::TypeExpected(_) | Kind::ExpectedMember(_) => "illegal.start.of.type",
        Kind::ClassLiteralExpected => "dot.class.expected",
        Kind::ArrayDimensionMissing => "array.dimension.missing",
        Kind::ArrayDimensionsAndInitializer => {
            "illegal.array.creation.both.dimension.and.initialization"
        }
        Kind::MixedLambdaParameters => "invalid.lambda.parameter.declaration",
        Kind::ExpectedDeclaration(_) => match release {
            ..16 => "expected3",
            16..25 => "expected4",
            _ => "class.method.or.field.expected",
        },
        Kind::ExpectedEnumConstant(_) => "enum.constant.expected",
        Kind::ExpectedModuleDirective(_) => "invalid.module.directive",
        Kind::ExpectedModuleKeyword(_) => "expected.module",
        Kind::RepeatedModifier => "repeated.modifier",
        Kind::MissingReturnType => "invalid.meth.decl.ret.type.req",
        Kind::MisplacedEnumConstant => "enum.constant.not.expected",
        Kind::GenericEnum => "enum.cant.be.generic",
        Kind::PermitsWithoutSealed => "invalid.permits.clause",
        Kind::ModifierNotAllowed => "mod.not.allowed.here",
        Kind::RecordComponentModifiers => "record.cant.declare.field.modifiers",
        Kind::RecordComponentDims => "record.component.and.old.array.syntax",
        Kind::InstanceFieldInRecord => "record.cannot.declare.instance.fields",
        Kind::VarargsNotLast => "varargs.must.be.last",
        Kind::VarargsWithDims => "varargs.and.old.array.syntax",
        Kind::MisplacedReceiver => "this.as.identifier",
        Kind::ReceiverWithDims => "array.and.receiver",
        Kind::StatementExpected => "illegal.start.of.stmt",
        Kind::NotAStatement => "not.stmt",
        Kind::LocalVariableNotAllowed => "variable.not.allowed",
        Kind::LocalClassNotAllowed => "class.not.allowed",
        Kind::SealedLocalClass => "sealed.or.non.sealed.local.classes.not.allowed",
        Kind::ElseWithoutIf => "else.without.if",
        Kind::CatchWithoutTry => "catch.without.try",
        Kind::FinallyWithoutTry => "finally.without.try",
        Kind::TryWithoutCatch => "try.without.catch.finally.or.resource.decls",
        Kind::ResourceNotAVariable => "try.with.resources.expr.needs.var",
        Kind::ForEachWithoutDeclaration => "bad.initializer",
        Kind::CaseOutsideSwitch => "orphaned",
        Kind::CaseExpected => "expected3",
        Kind::DefaultNotAllowed => "default.label.not.allowed",
        Kind::GuardNotAllowed => "guard.not.allowed",
        Kind::StatementNotAllowedInRule => "switch.case.unexpected.statement",
        Kind::AnnotatedClassLiteral => "no.annotations.on.dot.class",
        Kind::AnnotatedExpression => "illegal.start.of.expr",
        Kind::RecordPatternAnnotations => "record.patterns.annotations.not.allowed",
        // Never compared (`JavacPhase::Never`).
        Kind::NestingTooDeep => "nesting.too.deep",
        // The version check's kinds, by its own mapping. Only errors get here.
        Kind::FeatureNotInRelease { .. }
        | Kind::PreviewFeature(_)
        | Kind::RestrictedTypeName { .. }
        | Kind::RestrictedType { .. }
        | Kind::VarArray
        | Kind::VarCompound
        | Kind::VarWithDeclaredTypes
        | Kind::UnderscoreIdentifier { .. }
        | Kind::UnderscoreNotAllowed
        | Kind::UnderscoreWithBrackets
        | Kind::YieldCall { .. }
        | Kind::ExtraneousSemicolon { .. } => {
            ferrion_parser::version::javac_diagnostic(kind, Severity::Error)
                .expect("a version diagnostic")
                .code
        }
    }
}

/// When the first token at or after `anchor` is the end of the file: the end of the last token
/// before it, where javac reports `premature.eof` (0 in a file without tokens).
fn at_end_of_file(tree: &SyntaxTree, anchor: u32) -> Option<u32> {
    let tokens = tree.lexer_tokens();
    let first = tokens.partition_point(|token| token.span.start < anchor);
    let next = tokens[first..]
        .iter()
        .find(|token| !SyntaxKind::from_token_kind(token.kind).is_trivia())?;
    if SyntaxKind::from_token_kind(next.kind) != SyntaxKind::Eof {
        return None;
    }
    let last = tokens.iter().rev().find(|token| {
        let kind = SyntaxKind::from_token_kind(token.kind);
        !kind.is_trivia() && kind != SyntaxKind::Eof
    });
    Some(last.map_or(0, |token| token.span.end))
}

/// javac's code and position for a lexical error. The lexer's spans point at the mistake; javac's
/// positions follow its scanner (see the normalization document).
fn lexical(kind: LexErrorKind, span: Span, tree: &SyntaxTree) -> (&'static str, u32) {
    // The token holding the error: the last one starting at or before it.
    let tokens = tree.lexer_tokens();
    let token = tokens[..tokens.partition_point(|token| token.span.start <= span.start)]
        .last()
        .map_or(span, |token| token.span);
    match kind {
        // javac has its own code for a digit that isn't ASCII.
        LexErrorKind::IllegalCharacter
            if first_char(&tree.text()[span.range()])
                .is_some_and(|c| !c.is_ascii() && c.is_numeric()) =>
        {
            ("illegal.nonascii.digit", span.start)
        }
        LexErrorKind::IllegalCharacter => ("illegal.char", span.start),
        // At the character that isn't a hexadecimal digit.
        LexErrorKind::InvalidUnicodeEscape => ("illegal.unicode.esc", span.end),
        // At the character after the backslash.
        LexErrorKind::InvalidEscape => ("illegal.esc.char", span.start + 1),
        LexErrorKind::UnterminatedComment => ("unclosed.comment", span.start),
        LexErrorKind::UnterminatedCharLiteral | LexErrorKind::MultipleCharsInCharLiteral => {
            ("unclosed.char.lit", token.start)
        }
        LexErrorKind::UnterminatedStringLiteral => ("unclosed.str.lit", token.start),
        LexErrorKind::UnterminatedTextBlock => ("unclosed.text.block", token.start),
        LexErrorKind::EmptyCharLiteral => ("empty.char.lit", token.start),
        LexErrorKind::LineTerminatorInCharLiteral => ("illegal.line.end.in.char.lit", token.start),
        LexErrorKind::TextBlockOpeningNotOnOwnLine => ("illegal.text.block.open", span.start),
        LexErrorKind::MalformedNumber => malformed_number(tree.text(), span, token),
    }
}

/// The first character of `text`, with a Unicode escape decoded.
fn first_char(text: &str) -> Option<char> {
    match text.strip_prefix('\\') {
        Some(escape) if escape.starts_with('u') => {
            let digits = escape.trim_start_matches('u').get(..4)?;
            char::from_u32(u32::from_str_radix(digits, 16).ok()?)
        }
        _ => text.chars().next(),
    }
}

/// javac has a code for each way a number can be malformed; the lexer has one kind, with a span
/// on the mistake: underscores, a digit that doesn't belong, or the whole literal.
fn malformed_number(text: &str, span: Span, token: Span) -> (&'static str, u32) {
    let bad = &text[span.range()];
    let literal = text[token.range()].to_ascii_lowercase();
    if !bad.is_empty() && bad.bytes().all(|byte| byte == b'_') {
        // At the last underscore.
        return ("illegal.underscore", span.end - 1);
    }
    if span.start > token.start && bad.len() == 1 {
        if literal.starts_with("0b") {
            // javac counts the binary digit's position without the `0b`.
            return (
                "illegal.digit.in.binary.literal",
                span.start.saturating_sub(2),
            );
        }
        if literal.starts_with('0') {
            return ("illegal.digit.in.octal.literal", span.start);
        }
    }
    if let Some(digits) = literal.strip_prefix("0x") {
        let has_digit = digits
            .chars()
            .take_while(|c| *c != '.' && *c != 'p')
            .any(|c| c.is_ascii_hexdigit());
        if !has_digit {
            return ("invalid.hex.number", token.start);
        }
    } else if literal
        .strip_prefix("0b")
        .is_some_and(|digits| !digits.contains(['0', '1']))
    {
        return ("invalid.binary.number", token.start);
    }
    ("malformed.fp.lit", token.start)
}

#[cfg(test)]
mod tests;
