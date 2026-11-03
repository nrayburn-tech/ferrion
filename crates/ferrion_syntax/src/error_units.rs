//! What a syntax error affects (docs/design/parser-error-recovery.md, "What a syntax error
//! affects"): its **unit**, the innermost statement, member declaration, import, package
//! declaration, or module directive containing it. Range formatting copies each error unit as
//! written and formats around it (docs/design/formatter-completeness.md, "Files with syntax
//! errors"); the linter will run its rules on what no unit covers.
//!
//! The rule for a declaration's header (an error in its modifiers, name, type parameters, or
//! clauses makes the whole declaration the unit) needs no case of its own: the innermost unit
//! containing such an error is the declaration. An error in a body but in none of its statements
//! or members, such as a missing `}`, is the declaration's too, since the body no longer ends
//! where it should.
//!
//! One case is narrower than the innermost unit: an `Error` node that stands in a list of
//! statements or members, such as a stray `)` between two statements, is a unit of its own. It
//! takes an item's place, and the items around it are whole, so the rest of its block or body is
//! unaffected.

use crate::SyntaxKind as K;
use crate::{NodeId, SyntaxKind, SyntaxNode, SyntaxTree};

impl<'t> SyntaxNode<'t> {
    /// The unit an error in this node affects: the innermost of this node and its ancestors
    /// that is a statement, a member declaration (enum constants included), an import, a
    /// package declaration, a module declaration or directive, or an `Error` node in a block,
    /// a `switch` group, a class body, or the compilation unit. The root when there is none,
    /// as for an error between the top-level declarations: the whole file.
    pub fn error_unit(self) -> SyntaxNode<'t> {
        self.ancestors()
            .find(|&node| is_unit(node))
            .unwrap_or_else(|| self.tree().root())
    }
}

impl SyntaxTree {
    /// The units affected by the tree's errors (see [`SyntaxNode::error_unit`]), in source
    /// order, none inside another: the unit of each node with an error of its own, without the
    /// units inside another one. Empty for a tree without errors.
    pub fn error_units(&self) -> Vec<SyntaxNode<'_>> {
        if !self.root().contains_error() {
            return Vec::new();
        }
        let mut units: Vec<SyntaxNode<'_>> = Vec::new();
        let mut next = 0;
        while next < self.node_count() {
            let node = self.node(NodeId::from_index(next));
            if !node.contains_error() {
                next = node.subtree().end.index();
                continue;
            }
            next += 1;
            if !node.has_own_error() || reported_for_error_children(node) {
                continue;
            }
            let unit = node.error_unit();
            // Units nest or don't overlap, and come in pre-order of their errors, so a unit
            // that contains earlier ones replaces them.
            while units.last().is_some_and(|&last| unit.contains(last)) {
                units.pop();
            }
            if !units.last().is_some_and(|&last| last.contains(unit)) {
                units.push(unit);
            }
        }
        units.sort_by_key(|unit| unit.id());
        units
    }
}

/// Whether the error of `list`'s own is one the parser reported for the `Error` nodes among its
/// items, which are units of their own: at a token that can't start a statement, the parser
/// reports the error in the block, then puts the token in an `Error` node. So a list whose own
/// error could only be that, a block, a `switch` group, a class body, or the compilation unit
/// that has an `Error` item, ends with its `}` (it isn't unclosed), and holds no character the
/// lexer couldn't make a token of, doesn't make itself a unit.
fn reported_for_error_children(list: SyntaxNode<'_>) -> bool {
    let closed = match list.kind() {
        // Its own `}`, not one ending its last statement.
        K::Block | K::ClassBody => list
            .children_with_tokens()
            .last()
            .and_then(|last| last.as_token())
            .is_some_and(|t| t.kind() == K::RBrace),
        K::SwitchGroup | K::CompilationUnit => true,
        _ => return false,
    };
    closed
        && list.children().any(|child| child.kind() == K::Error)
        && !list.direct_tokens().any(|t| t.kind() == K::ErrorToken)
}

/// Whether `node` is a unit: what [`SyntaxNode::error_unit`] stops at.
fn is_unit(node: SyntaxNode<'_>) -> bool {
    let kind = node.kind();
    let parent = node.parent().map(|p| p.kind());
    match kind {
        K::ImportDecl
        | K::PackageDecl
        | K::ModuleDecl
        | K::RequiresDirective
        | K::ExportsDirective
        | K::OpensDirective
        | K::UsesDirective
        | K::ProvidesDirective => true,
        _ if is_member(kind) => true,
        _ if is_statement(kind) => parent.is_some_and(holds_statements),
        K::Error => matches!(
            parent,
            Some(K::Block | K::SwitchGroup | K::ClassBody | K::CompilationUnit)
        ),
        _ => false,
    }
}

/// Whether `kind` is a member declaration's, local and nested types included.
fn is_member(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        K::ClassDecl
            | K::InterfaceDecl
            | K::EnumDecl
            | K::RecordDecl
            | K::AnnotationTypeDecl
            | K::FieldDecl
            | K::MethodDecl
            | K::ConstructorDecl
            | K::CompactConstructorDecl
            | K::Initializer
            | K::EnumConstant
    )
}

/// Whether `kind` is a statement's, as java.grammar's `Stmt` lists them, apart from local type
/// declarations, which are members.
fn is_statement(kind: SyntaxKind) -> bool {
    matches!(
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

/// Whether a node of `kind` has statements as children: a statement-kind child elsewhere is a
/// body (a method's or a lambda's `Block`) or part of a header (a `for`'s `LocalVarDecl`), and
/// belongs to what holds it.
fn holds_statements(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        K::Block
            | K::SwitchGroup
            | K::SwitchRule
            | K::LabeledStmt
            | K::IfStmt
            | K::WhileStmt
            | K::DoStmt
            | K::ForStmt
            | K::ForEachStmt
    )
}
