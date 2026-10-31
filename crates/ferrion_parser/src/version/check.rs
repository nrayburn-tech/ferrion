//! The version check: one walk over the tree's node array, which is a loop, and one over its
//! tokens. Nothing here recurses, so any depth of nesting is safe.

use std::borrow::Cow;

use ferrion_diagnostics::Span;
use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeOrToken, SyntaxNode, SyntaxToken, SyntaxTree};

use super::table::{Feature, RestrictedName, Status};
use crate::SyntaxKind::{self, *};
use crate::{ParseDiagnostic, ParseDiagnosticKind as D, Severity};

pub(super) fn check(tree: &SyntaxTree) -> Vec<ParseDiagnostic> {
    let mut checker = Checker {
        tree,
        release: tree.release(),
        out: Vec::new(),
    };
    for node in tree.nodes() {
        checker.node(node);
    }
    for token in tree.tokens() {
        checker.token(token);
    }
    // Stable, so diagnostics at one place keep the order they were found in.
    checker.out.sort_by_key(|d| d.span.start);
    checker.out
}

struct Checker<'t> {
    tree: &'t SyntaxTree,
    release: u32,
    out: Vec<ParseDiagnostic>,
}

impl<'t> Checker<'t> {
    fn node(&mut self, node: SyntaxNode<'t>) {
        match node.kind() {
            CompilationUnit => self.compilation_unit(node),
            ImportDecl => {
                if let Some(module) = ast::ImportDecl::cast(node).and_then(|i| i.module_kw()) {
                    self.feature_at(Feature::ModuleImports, module);
                }
            }
            ModuleDecl => {
                if let Some(module) = direct_token(node, ModuleKw) {
                    self.feature_at(Feature::Modules, module);
                }
            }
            ClassDecl | InterfaceDecl | EnumDecl | AnnotationTypeDecl | RecordDecl => {
                self.type_decl(node);
            }
            TypeParameter => {
                if let Some(name) = ast::TypeParameter::cast(node).and_then(|p| p.name()) {
                    self.declared_type_name(name);
                }
            }
            Modifiers => self.modifiers(node),
            PermitsClause => {
                if let Some(permits) = direct_token(node, PermitsKw)
                    && let Some(before) = permits.prev_non_trivia()
                {
                    let end = before.span().end;
                    self.feature(Feature::PermitsClauses, permits.span(), end);
                }
            }
            FieldDecl | Initializer => self.static_member(node),
            MethodDecl => {
                self.static_member(node);
                self.private_interface_method(node);
            }
            ConstructorDecl => self.constructor(node),
            LocalVarDecl => self.local_var_decl(node),
            ClassType => self.class_type(node),
            LambdaParameters => self.lambda_parameters(node),
            ResourceSpecification => self.resources(node),
            NewExpr => self.new_expr(node),
            SwitchExpr => {
                if let Some(switch) = direct_token(node, SwitchKw) {
                    self.feature_at(Feature::SwitchExpressions, switch);
                }
            }
            SwitchRule => {
                if let Some(arrow) = direct_token(node, Arrow) {
                    self.feature_at(Feature::SwitchRules, arrow);
                }
            }
            SwitchLabel => self.switch_label(node),
            InstanceofExpr => self.instanceof(node),
            TypePattern => self.type_pattern(node),
            RecordPattern => {
                if let Some(paren) = direct_token(node, LParen) {
                    self.feature_at(Feature::RecordPatterns, paren);
                }
            }
            MethodCallExpr => self.method_call(node),
            _ => {}
        }
    }

    fn token(&mut self, token: SyntaxToken<'t>) {
        match token.kind() {
            TextBlock => self.feature_at(Feature::TextBlocks, token),
            StringLiteral | CharLiteral => self.space_escapes(token),
            // A type pattern's `_` is reported with the pattern.
            Underscore if token.parent().kind() != TypePattern => self.underscore(token),
            _ => {}
        }
    }

    // ---------------------------------------------------------------------------------------
    // Reporting.

    /// Reports `feature` if the release doesn't have it.
    fn feature(&mut self, feature: Feature, span: Span, anchor: u32) {
        if feature.is_in(self.release) {
            return;
        }
        let kind = match feature.status() {
            Status::Final(_) => D::FeatureNotInRelease {
                feature,
                release: self.release,
            },
            Status::Preview => D::PreviewFeature(feature),
        };
        self.out.push(ParseDiagnostic::error(kind, span, anchor));
    }

    fn feature_at(&mut self, feature: Feature, token: SyntaxToken<'_>) {
        self.feature(feature, token.span(), token.span().start);
    }

    fn report(&mut self, kind: D, severity: Severity, token: SyntaxToken<'_>) {
        self.out.push(ParseDiagnostic {
            kind,
            severity,
            span: token.span(),
            anchor: token.span().start,
            javac_phase: kind.javac_phase(),
        });
    }

    /// A warning before `from`, an error from it.
    fn severity_from(&self, from: u32) -> Severity {
        if self.release < from {
            Severity::Warning
        } else {
            Severity::Error
        }
    }

    // ---------------------------------------------------------------------------------------
    // Files and declarations.

    /// Implicitly declared classes, and `;` before an import.
    fn compilation_unit(&mut self, node: SyntaxNode<'t>) {
        if let Some(unit) = ast::CompilationUnit::cast(node)
            && let Some(member) = unit
                .members()
                .find(|m| matches!(m.syntax().kind(), FieldDecl | MethodDecl))
            && let Some(anchor) = after_modifiers(member.syntax())
        {
            self.feature_at(Feature::ImplicitClasses, anchor);
        }

        // javac reports the first `;` of each run of them before an import.
        let last_import = node
            .children()
            .filter(|c| c.kind() == ImportDecl)
            .last()
            .map(|c| c.span().start);
        let Some(last_import) = last_import else {
            return;
        };
        let mut after_semi = false;
        for child in node.children_with_tokens() {
            match child {
                NodeOrToken::Token(token) if token.is_trivia() => {}
                NodeOrToken::Token(token) if token.kind() == Semi => {
                    if !after_semi && token.span().start < last_import {
                        let kind = D::ExtraneousSemicolon {
                            release: self.release,
                        };
                        self.report(kind, self.severity_from(21), token);
                    }
                    after_semi = true;
                }
                _ => after_semi = false,
            }
        }
    }

    fn type_decl(&mut self, node: SyntaxNode<'t>) {
        let name = match node.kind() {
            ClassDecl => ast::ClassDecl::cast(node).and_then(|d| d.name()),
            InterfaceDecl => ast::InterfaceDecl::cast(node).and_then(|d| d.name()),
            EnumDecl => ast::EnumDecl::cast(node).and_then(|d| d.name()),
            AnnotationTypeDecl => ast::AnnotationTypeDecl::cast(node).and_then(|d| d.name()),
            RecordDecl => ast::RecordDecl::cast(node).and_then(|d| d.name()),
            _ => None,
        };
        if node.kind() == RecordDecl
            && let Some(record) = direct_token(node, RecordKw)
        {
            self.feature_at(Feature::Records, record);
        }
        if let Some(name) = name {
            self.declared_type_name(name);
        }
        let local = node
            .parent()
            .is_some_and(|p| matches!(p.kind(), Block | SwitchGroup));
        if local {
            match node.kind() {
                EnumDecl => {
                    if let Some(enum_kw) = direct_token(node, EnumKw) {
                        self.feature_at(Feature::LocalEnums, enum_kw);
                    }
                }
                // A local annotation interface is an error at every release.
                InterfaceDecl => {
                    if let Some(interface) = direct_token(node, InterfaceKw) {
                        self.feature_at(Feature::LocalInterfaces, interface);
                    }
                }
                _ => {}
            }
        } else if matches!(node.kind(), InterfaceDecl | EnumDecl | AnnotationTypeDecl)
            || node.kind() == ClassDecl && has_modifier(node, StaticKw)
        {
            self.static_member(node);
        }
    }

    /// A type declaration's or type parameter's name.
    fn declared_type_name(&mut self, name: SyntaxToken<'t>) {
        if let Some(restricted) = self.restricted(name) {
            let kind = D::RestrictedTypeName {
                name: restricted,
                release: self.release,
            };
            self.report(kind, self.severity_from(restricted.restricted_from()), name);
        }
    }

    /// `sealed` and `non-sealed`.
    fn modifiers(&mut self, node: SyntaxNode<'t>) {
        let mut previous = None;
        for token in node.direct_tokens() {
            match token.kind() {
                SealedKw => self.feature_at(Feature::SealedClasses, token),
                // The three tokens of `non-sealed` all have this kind: report the first.
                NonSealedKw if previous != Some(NonSealedKw) => {
                    self.feature_at(Feature::SealedClasses, token);
                }
                _ => {}
            }
            if !token.is_trivia() {
                previous = Some(token.kind());
            }
        }
    }

    fn private_interface_method(&mut self, node: SyntaxNode<'t>) {
        let in_interface = node
            .parent()
            .filter(|p| p.kind() == ClassBody)
            .and_then(|p| p.parent())
            .is_some_and(|p| p.kind() == InterfaceDecl);
        if in_interface
            && has_modifier(node, PrivateKw)
            && let Some(parameters) =
                ast::MethodDecl::cast(node).and_then(|m| m.formal_parameters())
        {
            self.feature_at(
                Feature::PrivateInterfaceMethods,
                parameters.syntax().first_token(),
            );
        }
    }

    /// A static member of an inner class, before 16. `node` is a member of a class body.
    fn static_member(&mut self, node: SyntaxNode<'t>) {
        if Feature::StaticMembersOfInnerClasses.is_in(self.release) {
            return;
        }
        let Some(body) = node.parent().filter(|p| p.kind() == ClassBody) else {
            return;
        };
        if !is_inner_class_body(body) {
            return;
        }
        let anchor = match node.kind() {
            FieldDecl if has_modifier(node, StaticKw) && !is_constant_field(node) => {
                ast::FieldDecl::cast(node)
                    .and_then(|f| f.declarators().next())
                    .and_then(|d| d.name())
            }
            MethodDecl if has_modifier(node, StaticKw) => {
                ast::MethodDecl::cast(node).and_then(|m| m.name())
            }
            Initializer => ast::Initializer::cast(node).and_then(|i| i.static_kw()),
            ClassDecl => direct_token(node, ClassKw),
            InterfaceDecl | AnnotationTypeDecl => direct_token(node, InterfaceKw),
            EnumDecl => direct_token(node, EnumKw),
            _ => None,
        };
        if let Some(anchor) = anchor {
            self.feature_at(Feature::StaticMembersOfInnerClasses, anchor);
        }
    }

    /// Statements before `super(...)` or `this(...)`.
    fn constructor(&mut self, node: SyntaxNode<'t>) {
        let Some(body) = ast::ConstructorDecl::cast(node).and_then(|c| c.body()) else {
            return;
        };
        for (i, statement) in body.statements().enumerate() {
            let call = ast::ExprStmt::cast(statement.syntax())
                .and_then(|s| s.expr())
                .and_then(|e| ast::ConstructorCallExpr::cast(e.syntax()));
            if let Some(call) = call {
                if i > 0
                    && let Some(arguments) = call.arguments()
                {
                    let paren = arguments.syntax().first_token();
                    self.feature_at(Feature::FlexibleConstructorBodies, paren);
                }
                return;
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // Types and restricted names.

    /// A local variable declared with a restricted name as its type. javac warns at the first
    /// variable's name, and from the release that restricts the name reports `var` in an array
    /// or with several variables, and the others anywhere, at the type.
    fn local_var_decl(&mut self, node: SyntaxNode<'t>) {
        let Some(decl) = ast::LocalVarDecl::cast(node) else {
            return;
        };
        let Some((class_type, array)) = decl.ty().and_then(|ty| local_type(ty.syntax())) else {
            return;
        };
        let Some(name_token) = simple_name(class_type) else {
            return;
        };
        let Some(restricted) = self.restricted(name_token) else {
            return;
        };
        let first_name = decl.declarators().next().and_then(|d| d.name());
        if self.release < restricted.restricted_from() {
            if let Some(first_name) = first_name {
                let kind = D::RestrictedType {
                    name: restricted,
                    release: self.release,
                };
                self.report(kind, Severity::Warning, first_name);
            }
        } else if restricted == RestrictedName::Var {
            if array || decl.declarators().any(|d| d.dims().next().is_some()) {
                self.report(D::VarArray, Severity::Error, name_token);
            } else if decl.declarators().nth(1).is_some() {
                self.report(D::VarCompound, Severity::Error, name_token);
            }
        } else {
            let kind = D::RestrictedType {
                name: restricted,
                release: self.release,
            };
            self.report(kind, Severity::Error, name_token);
        }
    }

    /// A restricted name used as a type anywhere but the places handled elsewhere: a local
    /// variable's type, a lambda parameter's `var`, `var` in or as a record pattern, and `new`.
    fn class_type(&mut self, node: SyntaxNode<'t>) {
        let Some(name) = simple_name(node) else {
            return;
        };
        let Some(restricted) = self.restricted(name) else {
            return;
        };
        let Some(parent) = node.parent() else {
            return;
        };
        // javac reports `var<T>` after parsing.
        let with_arguments =
            ast::ClassType::cast(node).is_some_and(|t| t.type_arguments().is_some());
        let handled = match parent.kind() {
            _ if with_arguments && restricted == RestrictedName::Var => true,
            // A qualifier, or a segment of one.
            ClassType => true,
            NewExpr | ArrayCreationExpr => true,
            LocalVarDecl => true,
            ArrayType => parent.parent().is_some_and(|p| p.kind() == LocalVarDecl),
            LambdaParameter => restricted == RestrictedName::Var,
            // javac reports `var(...)` as a nested record pattern's type after parsing, and
            // at the top level while parsing.
            RecordPattern => {
                restricted == RestrictedName::Var
                    && parent.parent().is_some_and(|p| p.kind() == RecordPattern)
            }
            TypePattern => {
                restricted == RestrictedName::Var
                    && parent.parent().is_some_and(|p| p.kind() == RecordPattern)
            }
            _ => false,
        };
        if handled {
            return;
        }
        let kind = D::RestrictedType {
            name: restricted,
            release: self.release,
        };
        self.report(kind, self.severity_from(restricted.restricted_from()), name);
    }

    /// `var` lambda parameters: a type's name below 10, a feature from 11, and from 11 not
    /// together with declared types.
    fn lambda_parameters(&mut self, node: SyntaxNode<'t>) {
        let Some(parameters) = ast::LambdaParameters::cast(node) else {
            return;
        };
        let mut with_var = false;
        let mut with_type = false;
        let mut inferred = false;
        for parameter in parameters.parameters() {
            let var = parameter
                .ty()
                .and_then(|ty| simple_name(ty.syntax()))
                .filter(|&name| self.restricted(name) == Some(RestrictedName::Var));
            match (var, parameter.ty()) {
                (Some(var), _) => {
                    with_var = true;
                    let kind = D::RestrictedType {
                        name: RestrictedName::Var,
                        release: self.release,
                    };
                    let bracketed = parameter.dims().next().is_some();
                    match parameter.name() {
                        _ if self.release < RestrictedName::Var.restricted_from() => {
                            self.report(kind, Severity::Warning, var);
                        }
                        // `var x[]`: javac reports the brackets, at the name, instead.
                        Some(name) if bracketed && self.release == 10 => {
                            self.report(kind, Severity::Error, name);
                        }
                        Some(name) if bracketed => {
                            self.report(D::VarArray, Severity::Error, name);
                        }
                        Some(name) => self.feature_at(Feature::VarLambdaParameters, name),
                        None => {}
                    }
                }
                (None, Some(_)) => with_type = true,
                (None, None) => inferred = true,
            }
        }
        if with_var
            && with_type
            && !inferred
            && Feature::VarLambdaParameters.is_in(self.release)
            && let Some(paren) = direct_token(node, LParen)
        {
            self.report(D::VarWithDeclaredTypes, Severity::Error, paren);
        }
    }

    // ---------------------------------------------------------------------------------------
    // Statements and expressions.

    /// Variables as resources.
    fn resources(&mut self, node: SyntaxNode<'t>) {
        for resource in node.children().filter(|c| ast::Expr::can_cast(c.kind())) {
            if let Some(after) = resource.last_token().next_non_trivia() {
                self.feature_at(Feature::ResourceVariables, after);
            }
        }
    }

    /// `<>` with a class body.
    fn new_expr(&mut self, node: SyntaxNode<'t>) {
        let Some(new) = ast::NewExpr::cast(node) else {
            return;
        };
        if new.body().is_none() {
            return;
        }
        let diamond = new
            .ty()
            .and_then(|ty| ast::ClassType::cast(ty.syntax()))
            .and_then(|ty| ty.type_arguments())
            .filter(|args| args.arguments().next().is_none());
        if let Some(diamond) = diamond {
            let lt = diamond.syntax().first_token();
            self.feature_at(Feature::DiamondWithAnonymousClasses, lt);
        }
    }

    /// Multiple labels, patterns, `case null`, and `case null, default`.
    fn switch_label(&mut self, node: SyntaxNode<'t>) {
        if direct_token(node, CaseKw).is_none() {
            return;
        }
        // The elements and `default`, in order, each with the `,` before it.
        let mut items: Vec<(Option<SyntaxNode<'t>>, SyntaxToken<'t>)> = Vec::new();
        let mut comma = None;
        let in_switch_expression = node
            .ancestors()
            .find(|a| matches!(a.kind(), SwitchExpr | SwitchStmt))
            .is_some_and(|a| a.kind() == SwitchExpr);
        for child in node.children_with_tokens() {
            let (element, start) = match child {
                NodeOrToken::Node(n) if n.kind() == Guard => break,
                NodeOrToken::Node(n) => (Some(n), n.first_token()),
                NodeOrToken::Token(t) if t.kind() == DefaultKw => (None, t),
                NodeOrToken::Token(t) if t.kind() == Comma => {
                    comma = Some(t);
                    continue;
                }
                NodeOrToken::Token(_) => continue,
            };
            if items.len() == 1 {
                // javac points at the `,` in a switch expression, and at the second label in a
                // switch statement.
                let anchor = match comma {
                    Some(comma) if in_switch_expression => comma,
                    _ => start,
                };
                self.feature_at(Feature::MultipleCaseLabels, anchor);
            }
            items.push((element, start));
        }
        for (element, start) in items {
            match element {
                Some(element) if is_pattern(element) => {
                    self.feature_at(Feature::PatternsInSwitch, start);
                }
                Some(element) => {
                    if element.kind() == LiteralExpr && start.kind() == NullKw {
                        self.feature_at(Feature::CaseNull, start);
                    }
                }
                None => self.feature_at(Feature::PatternsInSwitch, start),
            }
        }
    }

    fn instanceof(&mut self, node: SyntaxNode<'t>) {
        let Some(instanceof) = ast::InstanceofExpr::cast(node) else {
            return;
        };
        if let Some(ty) = instanceof.ty()
            && ty.syntax().kind() == PrimitiveType
        {
            self.feature_at(Feature::PrimitivePatterns, ty.syntax().first_token());
        }
    }

    /// A type pattern's `_`, a pattern in `instanceof`, and a primitive type in a top-level
    /// pattern. The `_` comes first, as javac reports it first.
    fn type_pattern(&mut self, node: SyntaxNode<'t>) {
        let Some(pattern) = ast::TypePattern::cast(node) else {
            return;
        };
        let name = pattern.name();
        if let Some(name) = name.filter(|n| n.kind() == Underscore) {
            self.underscore(name);
        }
        let parent = node.parent().map(|p| p.kind());
        if parent == Some(InstanceofExpr)
            && let Some(name) = name
        {
            self.feature_at(Feature::InstanceofPatterns, name);
        }
        if matches!(parent, Some(InstanceofExpr | SwitchLabel))
            && let Some(ty) = pattern.ty()
            && ty.syntax().kind() == PrimitiveType
        {
            self.feature_at(Feature::PrimitivePatterns, ty.syntax().first_token());
        }
    }

    /// An unqualified call to a method named `yield`.
    fn method_call(&mut self, node: SyntaxNode<'t>) {
        let Some(call) = ast::MethodCallExpr::cast(node) else {
            return;
        };
        if call.receiver().is_some() || call.type_arguments().is_some() {
            return;
        }
        if let Some(name) = call.name()
            && self.restricted(name) == Some(RestrictedName::Yield)
        {
            let kind = D::YieldCall {
                release: self.release,
            };
            self.report(kind, self.severity_from(14), name);
        }
    }

    // ---------------------------------------------------------------------------------------
    // Tokens.

    /// `\s` in a string or character literal. Unicode escapes are translated first, so
    /// `\u005cs` is one too; the anchor is the `s` as written.
    fn space_escapes(&mut self, token: SyntaxToken<'t>) {
        if Feature::SpaceEscape.is_in(self.release) {
            return;
        }
        let text = self.tree.text();
        let span = token.span();
        let mut escapes = ferrion_lexer::unicode_escapes(text, span).peekable();
        // The literal's characters after translation, each with its span as written.
        let mut chars = Vec::new();
        let mut pos = span.start as usize;
        while pos < span.end as usize {
            if let Some(escape) = escapes.next_if(|e| e.start as usize == pos) {
                if let Some(translated) = escape.ch {
                    chars.push((translated, pos as u32, escape.end));
                }
                pos = escape.end as usize;
            } else {
                let c = text[pos..].chars().next().unwrap_or('\0');
                let end = pos + c.len_utf8().max(1);
                chars.push((c, pos as u32, end as u32));
                pos = end;
            }
        }
        let mut i = 0;
        while i < chars.len() {
            if chars[i].0 == '\\' {
                if let Some(&(c, start, end)) = chars.get(i + 1)
                    && c == 's'
                {
                    let kind = D::FeatureNotInRelease {
                        feature: Feature::SpaceEscape,
                        release: self.release,
                    };
                    self.out.push(ParseDiagnostic::error(
                        kind,
                        Span::new(chars[i].1, end),
                        start,
                    ));
                }
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    /// `_` as a name: an unnamed variable where one is allowed, and otherwise an identifier.
    fn underscore(&mut self, token: SyntaxToken<'t>) {
        let parent = token.parent();
        // From 22, `None` where `_` names an unnamed variable, and otherwise the error. javac
        // reads a variable after the first in a field, or one with `[]` after its name, as an
        // unnamed variable that isn't allowed there.
        let from_22 = match parent.kind() {
            Error => return,
            // A nested `_` pattern is no identifier at any release. javac points just after it.
            UnnamedPattern => {
                let end = token.span().end;
                self.feature(Feature::UnnamedVariables, token.span(), end);
                return;
            }
            VariableDeclarator => {
                let declarator = ast::VariableDeclarator::cast(parent);
                let bracketed = declarator.is_some_and(|d| d.dims().next().is_some());
                let later_field = parent
                    .parent()
                    .and_then(ast::FieldDecl::cast)
                    .and_then(|f| f.declarators().next())
                    .is_some_and(|first| first.syntax() != parent);
                match parent.parent().map(|p| p.kind()) {
                    Some(LocalVarDecl) if bracketed => Some(D::UnderscoreWithBrackets),
                    Some(LocalVarDecl) => None,
                    Some(FieldDecl) if later_field => Some(D::UnderscoreNotAllowed),
                    _ => Some(D::UnderscoreIdentifier {
                        release: self.release,
                    }),
                }
            }
            LambdaParameter | CatchParameter | TypePattern => None,
            _ => Some(D::UnderscoreIdentifier {
                release: self.release,
            }),
        };
        let unnamed = !matches!(from_22, Some(D::UnderscoreIdentifier { .. }));
        if self.release < 9 {
            let kind = D::UnderscoreIdentifier {
                release: self.release,
            };
            self.report(kind, Severity::Warning, token);
        } else if !unnamed || self.release >= 22 {
            if let Some(kind) = from_22 {
                self.report(kind, Severity::Error, token);
            }
        } else {
            self.feature_at(Feature::UnnamedVariables, token);
        }
    }

    /// The restricted name `token` spells, compared after Unicode escapes are translated.
    fn restricted(&self, token: SyntaxToken<'_>) -> Option<RestrictedName> {
        if token.kind() != Identifier {
            return None;
        }
        let lexed = self.tree.lexer_tokens()[token.id().index()];
        let text: Cow<'_, str> = ferrion_lexer::cooked_text(self.tree.text(), lexed);
        RestrictedName::from_text(&text)
    }
}

// -------------------------------------------------------------------------------------------
// Tree helpers.

fn direct_token<'t>(node: SyntaxNode<'t>, kind: SyntaxKind) -> Option<SyntaxToken<'t>> {
    node.direct_tokens().find(|t| t.kind() == kind)
}

fn has_modifier(node: SyntaxNode<'_>, kind: SyntaxKind) -> bool {
    node.children()
        .find(|c| c.kind() == Modifiers)
        .is_some_and(|m| direct_token(m, kind).is_some())
}

/// The first token of a member after its modifiers.
fn after_modifiers(member: SyntaxNode<'_>) -> Option<SyntaxToken<'_>> {
    match member.first_child() {
        Some(modifiers) if modifiers.kind() == Modifiers => {
            modifiers.last_token().next_non_trivia()
        }
        _ => Some(member.first_token()),
    }
}

/// The name of a `ClassType` without a qualifier.
fn simple_name(node: SyntaxNode<'_>) -> Option<SyntaxToken<'_>> {
    let ty = ast::ClassType::cast(node)?;
    if ty.qualifier().is_some() {
        return None;
    }
    ty.name()
}

/// A local variable's `ClassType`, and whether it is an array's element type.
fn local_type(ty: SyntaxNode<'_>) -> Option<(SyntaxNode<'_>, bool)> {
    match ty.kind() {
        ClassType => Some((ty, false)),
        ArrayType => ast::ArrayType::cast(ty)?
            .element_type()
            .map(|e| e.syntax())
            .filter(|e| e.kind() == ClassType)
            .map(|e| (e, true)),
        _ => None,
    }
}

fn is_pattern(node: SyntaxNode<'_>) -> bool {
    matches!(node.kind(), TypePattern | RecordPattern)
}

/// Whether a class body is an inner class's: an anonymous class's, or a class's that isn't
/// `static` and isn't a member of an interface (local classes included).
fn is_inner_class_body(body: SyntaxNode<'_>) -> bool {
    let Some(owner) = body.parent() else {
        return false;
    };
    match owner.kind() {
        NewExpr | EnumConstant => true,
        ClassDecl => {
            if has_modifier(owner, StaticKw) {
                return false;
            }
            let Some(container) = owner.parent() else {
                return false;
            };
            match container.kind() {
                Block | SwitchGroup => true,
                ClassBody => container
                    .parent()
                    .is_some_and(|c| !matches!(c.kind(), InterfaceDecl | AnnotationTypeDecl)),
                EnumBody => true,
                _ => false,
            }
        }
        _ => false,
    }
}

/// Whether a field may be a constant variable, which inner classes could always declare: `static
/// final` with an initializer, of a primitive type or `String`. Whether the initializer is a
/// constant expression needs names resolved, so this errs on the side of a constant.
fn is_constant_field(node: SyntaxNode<'_>) -> bool {
    let Some(field) = ast::FieldDecl::cast(node) else {
        return false;
    };
    let primitive_or_string = field.ty().is_some_and(|ty| {
        ty.syntax().kind() == PrimitiveType
            || simple_name(ty.syntax()).is_some_and(|name| name.text() == "String")
    });
    has_modifier(node, FinalKw)
        && primitive_or_string
        && field.declarators().all(|d| d.initializer().is_some())
}
