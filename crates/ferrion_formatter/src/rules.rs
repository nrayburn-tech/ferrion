//! The rules that turn nodes into document elements (docs/design/formatter.md, "Building the
//! document"): one function per node kind, in a module per area.
//!
//! [`node`] sends each kind to its area's module, and each module's `format` matches the kinds it
//! owns. A kind without a rule of its own yet gets [`generic`], which prints the node's tokens
//! with single spaces, a line break between statements and members, and indented bodies. That
//! keeps the safety properties on every file that parses, so each rule written later replaces a
//! working one. To add a rule, add an arm for the kind to its module's `format`; the area of
//! every kind is fixed here.
//!
//! What more than one area lays out the same way is in a module of its own, which each area's
//! rules call, so that a construct breaks alike wherever it appears:
//!
//! - [`lists`]: delimited lists (arguments, parameters, initializers, annotation arguments, type
//!   arguments), with a [`lists::Shape`] for what differs between them, and undelimited
//!   comma-separated lists;
//! - [`variables`]: `Type name`, which breaks before the name, and the value after an `=` or an
//!   assignment's operator;
//! - [`comments`]: what the rules ask about the comments next to a token.
//!
//! This module also has the helpers for a node's children ([`children`], [`emit`],
//! [`first_token`], [`last_token`]). Rules keep no state of their own: what a rule needs from
//! elsewhere in the file, such as the type-use annotations it imports, or how many capped
//! indents are open, is on the [`Formatter`].

mod comments;
pub mod declarations;
pub mod expressions;
mod lists;
pub mod statements;
pub mod types;
mod variables;

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxKind, SyntaxNode, SyntaxToken};

use crate::document::IndentKind;
use crate::format::Formatter;

/// The module whose rules format a node kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Area {
    Declarations,
    Statements,
    Expressions,
    Types,
}

/// The area of `kind`, a node kind. The `Error` node, which is never in a tree the formatter
/// formats, and token kinds have none.
pub fn area(kind: SyntaxKind) -> Option<Area> {
    Some(match kind {
        K::CompilationUnit
        | K::PackageDecl
        | K::ImportDecl
        | K::Name
        | K::ModuleDecl
        | K::RequiresDirective
        | K::ExportsDirective
        | K::OpensDirective
        | K::UsesDirective
        | K::ProvidesDirective
        | K::Modifiers
        | K::Annotation
        | K::AnnotationArguments
        | K::ElementValuePair
        | K::ElementValueArray
        | K::ClassDecl
        | K::InterfaceDecl
        | K::EnumDecl
        | K::RecordDecl
        | K::AnnotationTypeDecl
        | K::ExtendsClause
        | K::ImplementsClause
        | K::PermitsClause
        | K::ClassBody
        | K::EnumBody
        | K::EnumConstant
        | K::RecordHeader
        | K::RecordComponent
        | K::FieldDecl
        | K::VariableDeclarator
        | K::MethodDecl
        | K::ConstructorDecl
        | K::CompactConstructorDecl
        | K::Initializer
        | K::FormalParameters
        | K::FormalParameter
        | K::ReceiverParameter
        | K::DefaultValue => Area::Declarations,
        K::TypeParameters
        | K::TypeParameter
        | K::PrimitiveType
        | K::VoidType
        | K::ClassType
        | K::ArrayType
        | K::Dim
        | K::IntersectionType
        | K::UnionType
        | K::TypeArguments
        | K::Wildcard
        | K::Varargs
        | K::ThrowsClause => Area::Types,
        K::Block
        | K::LocalVarDecl
        | K::EmptyStmt
        | K::ExprStmt
        | K::IfStmt
        | K::AssertStmt
        | K::SwitchStmt
        | K::WhileStmt
        | K::DoStmt
        | K::ForStmt
        | K::ForInit
        | K::ForUpdate
        | K::ForEachStmt
        | K::LabeledStmt
        | K::BreakStmt
        | K::ContinueStmt
        | K::ReturnStmt
        | K::ThrowStmt
        | K::SynchronizedStmt
        | K::TryStmt
        | K::ResourceSpecification
        | K::CatchClause
        | K::CatchParameter
        | K::FinallyClause
        | K::YieldStmt
        | K::SwitchExpr
        | K::SwitchBlock
        | K::SwitchRule
        | K::SwitchGroup
        | K::SwitchLabel
        | K::Guard => Area::Statements,
        K::LiteralExpr
        | K::NameExpr
        | K::ThisExpr
        | K::SuperExpr
        | K::ParenExpr
        | K::ClassLiteralExpr
        | K::FieldAccessExpr
        | K::ArrayAccessExpr
        | K::MethodCallExpr
        | K::Arguments
        | K::ConstructorCallExpr
        | K::NewExpr
        | K::ArrayCreationExpr
        | K::DimExpr
        | K::ArrayInitializer
        | K::PrefixExpr
        | K::PostfixExpr
        | K::BinaryExpr
        | K::InstanceofExpr
        | K::ConditionalExpr
        | K::AssignExpr
        | K::CastExpr
        | K::LambdaExpr
        | K::LambdaParameters
        | K::LambdaParameter
        | K::MethodRefExpr
        | K::TypePattern
        | K::RecordPattern
        | K::UnnamedPattern => Area::Expressions,
        _ => return None,
    })
}

/// Formats `node` with its kind's rule. Every rule recurses through here, which grows the stack
/// when it runs low (docs/design/parser-deep-nesting.md).
pub fn node<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    ferrion_stack::ensure_stack(|| match area(node.kind()) {
        Some(Area::Declarations) => declarations::format(f, node),
        Some(Area::Statements) => statements::format(f, node),
        Some(Area::Expressions) => expressions::format(f, node),
        Some(Area::Types) => types::format(f, node),
        None => generic(f, node),
    });
}

/// The first rule, for any node: its child nodes and tokens in order, with a line break around
/// each statement, member, and directive, a body's contents indented on their own lines, and
/// otherwise a space between two tokens unless [`space_between`] says none. Blank lines between
/// statements and members are kept, at most one.
pub fn generic<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parent = node.kind();
    let mut previous: Option<NodeOrToken<'t>> = None;
    for element in node.children_with_tokens() {
        let first = match element {
            NodeOrToken::Token(token) if token.is_trivia() => continue,
            NodeOrToken::Token(token) => token,
            NodeOrToken::Node(child) => child.first_token(),
        };
        if first.kind() == K::RBrace && matches!(element, NodeOrToken::Token(_)) {
            // An empty body stays `{}`; otherwise its comments go inside, then the indent ends.
            let empty = previous.is_some_and(|p| p.kind() == K::LBrace)
                && first
                    .prev_token()
                    .is_some_and(|t| t.kind() == K::LBrace || only_whitespace_before(t));
            if !empty {
                f.comments_before(first.id());
                f.end();
                f.hard_line();
            } else {
                f.end();
            }
            f.token(first);
            previous = Some(element);
            continue;
        }
        if let Some(previous) = previous {
            if previous.kind() == K::LBrace {
                f.line_break_before(first);
            } else if first.kind() == K::Eof {
            } else if is_line_item(parent, previous) || is_line_item(parent, element) {
                f.line_break_before(first);
            } else if space_between(last_token(previous), first) {
                f.space();
            }
        }
        match element {
            NodeOrToken::Token(token) => f.token(token),
            NodeOrToken::Node(child) => f.node(child),
        }
        if first.kind() == K::LBrace && matches!(element, NodeOrToken::Token(_)) {
            f.start_indent(IndentKind::Block);
        }
        previous = Some(element);
    }
}

/// Whether `token` and the trivia before it, back to the previous token that isn't trivia, are
/// all whitespace and line terminators.
fn only_whitespace_before(token: SyntaxToken<'_>) -> bool {
    let mut token = Some(token);
    while let Some(t) = token
        && t.is_trivia()
    {
        if !matches!(t.kind(), K::Whitespace | K::LineTerminator) {
            return false;
        }
        token = t.prev_token();
    }
    true
}

/// Emits a child: a token with [`Formatter::token`], a node with its kind's rule.
pub(crate) fn emit<'t>(f: &mut Formatter<'t>, child: NodeOrToken<'t>) {
    match child {
        NodeOrToken::Token(token) => f.token(token),
        NodeOrToken::Node(node) => f.node(node),
    }
}

/// The children of `node` that aren't trivia, in order.
pub(crate) fn children<'t>(node: SyntaxNode<'t>) -> impl Iterator<Item = NodeOrToken<'t>> {
    node.children_with_tokens()
        .filter(|child| !matches!(child, NodeOrToken::Token(token) if token.is_trivia()))
}

pub(crate) fn first_token(element: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match element {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.first_token(),
    }
}

pub(crate) fn last_token(element: NodeOrToken<'_>) -> SyntaxToken<'_> {
    match element {
        NodeOrToken::Token(token) => token,
        NodeOrToken::Node(node) => node.last_token(),
    }
}

/// Whether `element`, a child of a `parent` node, goes on its own lines: a statement, member,
/// import, or directive in the construct that lists them.
fn is_line_item(parent: SyntaxKind, element: NodeOrToken<'_>) -> bool {
    let kind = element.kind();
    match parent {
        K::CompilationUnit => kind != K::Eof,
        K::ClassBody => element.as_node().is_some() || kind == K::Semi,
        K::EnumBody => is_member(kind),
        K::Block | K::SwitchGroup => is_statement(kind),
        K::SwitchBlock => matches!(kind, K::SwitchRule | K::SwitchGroup),
        K::ModuleDecl => matches!(
            kind,
            K::RequiresDirective
                | K::ExportsDirective
                | K::OpensDirective
                | K::UsesDirective
                | K::ProvidesDirective
        ),
        _ => false,
    }
}

/// Whether `kind` is a member of a class body.
pub fn is_member(kind: SyntaxKind) -> bool {
    is_type_decl(kind)
        || matches!(
            kind,
            K::FieldDecl
                | K::MethodDecl
                | K::ConstructorDecl
                | K::CompactConstructorDecl
                | K::Initializer
        )
}

pub fn is_type_decl(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        K::ClassDecl | K::InterfaceDecl | K::EnumDecl | K::RecordDecl | K::AnnotationTypeDecl
    )
}

/// Whether `kind` is a statement's, as the `Stmt` enum lists them.
pub fn is_statement(kind: SyntaxKind) -> bool {
    is_type_decl(kind)
        || matches!(
            kind,
            K::Block
                | K::LocalVarDecl
                | K::EmptyStmt
                | K::ExprStmt
                | K::IfStmt
                | K::AssertStmt
                | K::SwitchStmt
                | K::WhileStmt
                | K::DoStmt
                | K::ForStmt
                | K::ForEachStmt
                | K::LabeledStmt
                | K::BreakStmt
                | K::ContinueStmt
                | K::ReturnStmt
                | K::ThrowStmt
                | K::SynchronizedStmt
                | K::TryStmt
                | K::YieldStmt
        )
}

/// Whether the first rule puts a space between `previous` and `next`, two tokens that aren't
/// trivia, printed side by side. None inside an operator the lexer splits (`>>`, `non-sealed`),
/// before `,`, `;`, `)`, `]`, `.`, `::`, and `...`, after `(`, `[`, `.`, `::`, and `@`, inside
/// type arguments' and parameters' angle brackets, between a name and the `(` or `[` after it,
/// and between a unary operator and its operand. Two tokens printed side by side that would run
/// together get a space from the printer anyway.
pub fn space_between(previous: SyntaxToken<'_>, next: SyntaxToken<'_>) -> bool {
    let (p, n) = (previous.kind(), next.kind());
    let adjacent = next.id().index() == previous.id().index() + 1;
    if adjacent && p == K::Gt && matches!(n, K::Gt | K::GtEq) || p == K::NonSealedKw {
        return false;
    }
    if matches!(
        n,
        K::Comma | K::Semi | K::RParen | K::RBracket | K::Dot | K::ColonColon | K::Ellipsis
    ) || matches!(p, K::LParen | K::LBracket | K::Dot | K::ColonColon | K::At)
    {
        return false;
    }
    let angle = |token: SyntaxToken<'_>| {
        matches!(token.parent().kind(), K::TypeArguments | K::TypeParameters)
    };
    if p == K::Lt && angle(previous) || n == K::Gt && angle(next) {
        return false;
    }
    if n == K::Lt && angle(next) && matches!(p, K::Identifier | K::Dot | K::ColonColon) {
        return false;
    }
    if p == K::Gt
        && angle(previous)
        && (matches!(n, K::LParen | K::LBracket | K::Gt)
            || matches!(
                previous.parent().parent().map(|n| n.kind()),
                Some(K::MethodCallExpr | K::ConstructorCallExpr | K::MethodRefExpr)
            ))
    {
        return false;
    }
    if matches!(n, K::LParen | K::LBracket)
        && matches!(p, K::Identifier | K::ThisKw | K::SuperKw | K::RBracket)
    {
        return false;
    }
    if n == K::LBracket && matches!(p, K::Gt | K::RParen) || n == K::LBracket && is_type_end(p) {
        return false;
    }
    // A unary operator is the only token of its node, outside the operand.
    if previous.parent().kind() == K::PrefixExpr || next.parent().kind() == K::PostfixExpr {
        return false;
    }
    if n == K::Colon && matches!(next.parent().kind(), K::SwitchGroup | K::LabeledStmt) {
        return false;
    }
    true
}

/// Whether `kind` can end a type before its dimensions: a primitive type's keyword.
fn is_type_end(kind: SyntaxKind) -> bool {
    ferrion_syntax::token_sets::PRIMITIVE_TYPE_KEYWORD.contains(&kind)
}
