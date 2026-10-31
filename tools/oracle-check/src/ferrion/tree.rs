//! Ferrion's tree in javac's terms: the rows of javac's tree, in the pre-order of its
//! `TreeScanner`, built from a [`SyntaxTree`]. Each rule is in
//! docs/design/parser-oracle-normalization.md ("Ferrion's tree in javac's terms"), with the javac
//! behavior it reproduces.
//!
//! Positions are byte offsets into the tree's text; the comparison converts them to javac's
//! UTF-16 offsets.

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken, SyntaxTree};

use crate::syntax::TreeNode;

/// The first release where `var` is a restricted type name: before it, javac's tree has an
/// `IDENTIFIER` named `var`.
const VAR_RELEASE: u32 = 10;

/// The tree's rows.
pub fn map(tree: &SyntaxTree, release: u32) -> Vec<TreeNode> {
    let mut rows = Rows {
        tree,
        release,
        rows: Vec::new(),
        depth: 0,
        shared: false,
        copy: false,
        record_header: None,
    };
    rows.compilation_unit(tree.root());
    rows.rows
}

struct Rows<'t> {
    tree: &'t SyntaxTree,
    release: u32,
    rows: Vec<TreeNode>,
    depth: u32,
    /// Rows are of nodes javac writes a second time: the modifiers and type of a declarator after
    /// the first.
    shared: bool,
    /// Rows are of javac's copies of a record's components, which have no end.
    copy: bool,
    /// The header of the record whose body is being mapped, for its compact constructor.
    record_header: Option<SyntaxNode<'t>>,
}

// ----- Helpers over nodes and tokens -----

fn start(node: SyntaxNode<'_>) -> u32 {
    node.span().start
}

fn end(node: SyntaxNode<'_>) -> u32 {
    node.span().end
}

/// A node's direct tokens, without trivia.
fn tokens(node: SyntaxNode<'_>) -> impl Iterator<Item = SyntaxToken<'_>> {
    node.direct_tokens().filter(|token| !token.is_trivia())
}

fn token(node: SyntaxNode<'_>, kind: K) -> Option<SyntaxToken<'_>> {
    tokens(node).find(|token| token.kind() == kind)
}

fn child(node: SyntaxNode<'_>, kind: K) -> Option<SyntaxNode<'_>> {
    node.children().find(|child| child.kind() == kind)
}

fn children(node: SyntaxNode<'_>, kind: K) -> impl Iterator<Item = SyntaxNode<'_>> {
    node.children().filter(move |child| child.kind() == kind)
}

/// The name of a declaration: its first identifier (or `_`) token.
fn name(node: SyntaxNode<'_>) -> Option<SyntaxToken<'_>> {
    tokens(node).find(|token| matches!(token.kind(), K::Identifier | K::Underscore))
}

fn is_type(kind: K) -> bool {
    matches!(
        kind,
        K::PrimitiveType
            | K::ClassType
            | K::ArrayType
            | K::VoidType
            | K::IntersectionType
            | K::UnionType
    )
}

fn is_expr(kind: K) -> bool {
    matches!(
        kind,
        K::LiteralExpr
            | K::NameExpr
            | K::ThisExpr
            | K::SuperExpr
            | K::ParenExpr
            | K::ClassLiteralExpr
            | K::FieldAccessExpr
            | K::ArrayAccessExpr
            | K::MethodCallExpr
            | K::ConstructorCallExpr
            | K::NewExpr
            | K::ArrayCreationExpr
            | K::PrefixExpr
            | K::PostfixExpr
            | K::BinaryExpr
            | K::InstanceofExpr
            | K::ConditionalExpr
            | K::AssignExpr
            | K::CastExpr
            | K::LambdaExpr
            | K::MethodRefExpr
            | K::SwitchExpr
    )
}

fn is_pattern(kind: K) -> bool {
    matches!(kind, K::TypePattern | K::RecordPattern | K::UnnamedPattern)
}

fn is_type_decl(kind: K) -> bool {
    matches!(
        kind,
        K::ClassDecl | K::InterfaceDecl | K::EnumDecl | K::RecordDecl | K::AnnotationTypeDecl
    )
}

fn first_type(node: SyntaxNode<'_>) -> Option<SyntaxNode<'_>> {
    node.children().find(|child| is_type(child.kind()))
}

fn first_expr(node: SyntaxNode<'_>) -> Option<SyntaxNode<'_>> {
    node.children().find(|child| is_expr(child.kind()))
}

fn exprs(node: SyntaxNode<'_>) -> impl Iterator<Item = SyntaxNode<'_>> {
    node.children().filter(|child| is_expr(child.kind()))
}

/// Whether a literal is a string literal or a text block, which javac folds.
fn is_string_literal(node: SyntaxNode<'_>) -> bool {
    node.kind() == K::LiteralExpr
        && tokens(node)
            .next()
            .is_some_and(|token| matches!(token.kind(), K::StringLiteral | K::TextBlock))
}

/// `var` as a type: a `ClassType` that is just the name `var`.
fn is_var(ty: SyntaxNode<'_>) -> bool {
    ty.kind() == K::ClassType
        && ty.children().next().is_none()
        && tokens(ty).count() == 1
        && name(ty).is_some_and(|token| token.text() == "var")
}

/// The operator of a binary or assignment expression: its direct tokens between the operands.
/// The lexer splits `>>` and `>>>=` into runs of `>` tokens.
fn operator(node: SyntaxNode<'_>) -> (Vec<K>, u32) {
    let ops: Vec<SyntaxToken> = tokens(node).collect();
    let at = ops.first().map_or(start(node), |token| token.span().start);
    (ops.iter().map(|token| token.kind()).collect(), at)
}

fn binary_kind(ops: &[K]) -> &'static str {
    match ops {
        [K::PipePipe] => "CONDITIONAL_OR",
        [K::AmpAmp] => "CONDITIONAL_AND",
        [K::Pipe] => "OR",
        [K::Caret] => "XOR",
        [K::Amp] => "AND",
        [K::EqEq] => "EQUAL_TO",
        [K::BangEq] => "NOT_EQUAL_TO",
        [K::Lt] => "LESS_THAN",
        [K::Gt] => "GREATER_THAN",
        [K::LtEq] => "LESS_THAN_EQUAL",
        [K::GtEq] => "GREATER_THAN_EQUAL",
        [K::LtLt] => "LEFT_SHIFT",
        [K::Gt, K::Gt] => "RIGHT_SHIFT",
        [K::Gt, K::Gt, K::Gt] => "UNSIGNED_RIGHT_SHIFT",
        [K::Plus] => "PLUS",
        [K::Minus] => "MINUS",
        [K::Star] => "MULTIPLY",
        [K::Slash] => "DIVIDE",
        [K::Percent] => "REMAINDER",
        _ => "ERRONEOUS",
    }
}

fn assignment_kind(ops: &[K]) -> &'static str {
    match ops {
        [K::Eq] => "ASSIGNMENT",
        [K::PlusEq] => "PLUS_ASSIGNMENT",
        [K::MinusEq] => "MINUS_ASSIGNMENT",
        [K::StarEq] => "MULTIPLY_ASSIGNMENT",
        [K::SlashEq] => "DIVIDE_ASSIGNMENT",
        [K::PercentEq] => "REMAINDER_ASSIGNMENT",
        [K::AmpEq] => "AND_ASSIGNMENT",
        [K::PipeEq] => "OR_ASSIGNMENT",
        [K::CaretEq] => "XOR_ASSIGNMENT",
        [K::LtLtEq] => "LEFT_SHIFT_ASSIGNMENT",
        [K::Gt, K::GtEq] => "RIGHT_SHIFT_ASSIGNMENT",
        [K::Gt, K::Gt, K::GtEq] => "UNSIGNED_RIGHT_SHIFT_ASSIGNMENT",
        _ => "ERRONEOUS",
    }
}

fn literal_kind(kind: K) -> &'static str {
    match kind {
        K::IntLiteral => "INT_LITERAL",
        K::LongLiteral => "LONG_LITERAL",
        K::FloatLiteral => "FLOAT_LITERAL",
        K::DoubleLiteral => "DOUBLE_LITERAL",
        K::CharLiteral => "CHAR_LITERAL",
        K::StringLiteral | K::TextBlock => "STRING_LITERAL",
        K::TrueKw | K::FalseKw => "BOOLEAN_LITERAL",
        K::NullKw => "NULL_LITERAL",
        _ => "ERRONEOUS",
    }
}

/// A layer of an array type: a dimension, with its annotations, as javac nests it.
struct Layer<'t> {
    annotations: Vec<SyntaxNode<'t>>,
    /// The `ANNOTATED_TYPE`'s span end and preferred position, when there are annotations.
    annotated: (u32, u32),
    /// The `ARRAY_TYPE`'s span end and preferred position.
    array: (u32, u32),
}

impl<'t> Layer<'t> {
    /// A `[]` with javac's usual positions: both nodes span to `end`, at the `[`.
    fn dim(dim: SyntaxNode<'t>, end: u32) -> Layer<'t> {
        let bracket = token(dim, K::LBracket).map_or(start(dim), |t| t.span().start);
        Layer {
            annotations: children(dim, K::Annotation).collect(),
            annotated: (end, bracket),
            array: (end, bracket),
        }
    }
}

// ----- Writing rows -----

impl<'t> Rows<'t> {
    /// Opens a row; its children follow until [`Rows::close`]. Returns its index.
    fn open(
        &mut self,
        kind: &'static str,
        start: Option<u32>,
        end: Option<u32>,
        preferred: Option<u32>,
    ) -> usize {
        self.rows.push(TreeNode {
            kind,
            start,
            end: if self.copy { None } else { end },
            depth: self.depth,
            preferred,
            shared: self.shared,
        });
        self.depth += 1;
        self.rows.len() - 1
    }

    fn close(&mut self) {
        self.depth -= 1;
    }

    /// A row spanning `start..end`, preferred at `preferred`.
    fn node(&mut self, kind: &'static str, start: u32, end: u32, preferred: u32) -> usize {
        self.open(kind, Some(start), Some(end), Some(preferred))
    }

    fn leaf(&mut self, kind: &'static str, start: u32, end: u32, preferred: u32) {
        self.node(kind, start, end, preferred);
        self.close();
    }

    /// A row with the span of `node`, preferred at its start.
    fn spanning(&mut self, kind: &'static str, node: SyntaxNode<'_>) -> usize {
        self.node(kind, start(node), end(node), start(node))
    }

    /// Runs `f` with the rows it writes marked as shared, or not.
    fn with_shared(&mut self, shared: bool, f: impl FnOnce(&mut Self)) {
        let saved = self.shared;
        self.shared = shared || saved;
        f(self);
        self.shared = saved;
    }

    // ----- Compilation units -----

    fn compilation_unit(&mut self, unit: SyntaxNode<'t>) {
        let code: Vec<SyntaxToken> = unit
            .tree()
            .tokens()
            .filter(|token| !token.is_trivia() && token.kind() != K::Eof)
            .collect();
        match (code.first(), code.last()) {
            // javac starts a compilation unit without tokens at the end of the file, and ends it
            // at 0.
            (None, _) | (_, None) => {
                let len = self.tree.text().len() as u32;
                self.node("COMPILATION_UNIT", len, 0, len);
            }
            (Some(first), Some(last)) => {
                let at = first.span().start;
                self.node("COMPILATION_UNIT", at, last.span().end, at);
            }
        }
        let items: Vec<NodeOrToken> = unit.children_with_tokens().collect();
        let implicit = items.iter().any(|item| {
            item.as_node()
                .is_some_and(|n| matches!(n.kind(), K::FieldDecl | K::MethodDecl | K::Initializer))
        });
        let has_type = items
            .iter()
            .any(|item| item.as_node().is_some_and(|n| is_type_decl(n.kind())));
        let mut first_member = None;
        for (i, item) in items.iter().enumerate() {
            match *item {
                NodeOrToken::Node(node) => match node.kind() {
                    K::PackageDecl => self.package(node),
                    K::ImportDecl => self.import(node),
                    K::ModuleDecl => self.module(node),
                    K::Error => self.erroneous(node),
                    _ if implicit => {
                        first_member.get_or_insert(i);
                    }
                    _ => self.member(node),
                },
                NodeOrToken::Token(token) if token.kind() == K::Semi && !implicit => {
                    // javac has an `EMPTY_STATEMENT` for a top-level `;`, except before an
                    // import or a module declaration, and in a file without type declarations.
                    let later_import = items[i + 1..].iter().any(|item| {
                        item.as_node()
                            .is_some_and(|n| matches!(n.kind(), K::ImportDecl | K::ModuleDecl))
                    });
                    if has_type && !later_import {
                        let span = token.span();
                        self.leaf("EMPTY_STATEMENT", span.start, span.end, span.start);
                    }
                }
                NodeOrToken::Token(_) => {}
            }
        }
        // An implicitly declared class: javac's synthesized `CLASS` spans from the first member to
        // the last token, and holds every member, type declarations included.
        if let Some(first) = first_member {
            let at = items[first].span().start;
            let last = code.last().map_or(at, |token| token.span().end);
            self.node("CLASS", at, last, at);
            self.empty_modifiers();
            for item in &items[first..] {
                if let NodeOrToken::Node(node) = *item {
                    self.member(node);
                }
            }
            self.close();
        }
        self.close();
    }

    fn package(&mut self, package: SyntaxNode<'t>) {
        let keyword = token(package, K::PackageKw).map_or(start(package), |t| t.span().start);
        self.node("PACKAGE", start(package), end(package), keyword);
        for annotation in children(package, K::Annotation) {
            self.annotation(annotation, "ANNOTATION");
        }
        if let Some(name) = child(package, K::Name) {
            self.name(name);
        }
        self.close();
    }

    fn import(&mut self, import: SyntaxNode<'t>) {
        self.spanning("IMPORT", import);
        if let Some(name) = child(import, K::Name) {
            match token(import, K::Star) {
                // javac's `MEMBER_SELECT` for `.*` names `*`.
                Some(star) => {
                    let dot = star.prev_non_trivia().map_or(end(name), |t| t.span().start);
                    self.node("MEMBER_SELECT", start(name), star.span().end, dot);
                    self.name(name);
                    self.close();
                }
                None => self.name(name),
            }
        }
        self.close();
    }

    /// A qualified name: `MEMBER_SELECT`s over an `IDENTIFIER`.
    fn name(&mut self, name: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| match child(name, K::Name) {
            Some(qualifier) => {
                let dot = token(name, K::Dot).map_or(end(qualifier), |t| t.span().start);
                self.node("MEMBER_SELECT", start(name), end(name), dot);
                self.name(qualifier);
                self.close();
            }
            None => {
                self.spanning("IDENTIFIER", name);
                self.close();
            }
        })
    }

    fn module(&mut self, module: SyntaxNode<'t>) {
        let keyword = token(module, K::ModuleKw)
            .or_else(|| child(module, K::Name).and_then(|n| n.first_token().prev_non_trivia()))
            .map_or(start(module), |t| t.span().start);
        // A module starts at its first annotation, or at `module`: never at `open`.
        let from = child(module, K::Annotation).map_or(keyword, start);
        self.node("MODULE", from, end(module), keyword);
        for annotation in children(module, K::Annotation) {
            self.annotation(annotation, "ANNOTATION");
        }
        if let Some(name) = child(module, K::Name) {
            self.name(name);
        }
        for directive in module.children() {
            let kind = match directive.kind() {
                K::RequiresDirective => "REQUIRES",
                K::ExportsDirective => "EXPORTS",
                K::OpensDirective => "OPENS",
                K::UsesDirective => "USES",
                K::ProvidesDirective => "PROVIDES",
                K::Error => {
                    self.erroneous(directive);
                    continue;
                }
                _ => continue,
            };
            self.spanning(kind, directive);
            for name in children(directive, K::Name) {
                self.name(name);
            }
            self.close();
        }
        self.close();
    }

    // ----- Modifiers and annotations -----

    /// `MODIFIERS`: the modifiers' span, or none (-1..-1), or an empty span at `empty_at`. Its
    /// children are the annotations among the modifiers, then `more`.
    fn modifiers(
        &mut self,
        modifiers: Option<SyntaxNode<'t>>,
        empty_at: Option<u32>,
        more: &[SyntaxNode<'t>],
    ) {
        match (modifiers, empty_at) {
            (Some(modifiers), _) => {
                self.spanning("MODIFIERS", modifiers);
            }
            (None, Some(at)) => {
                self.node("MODIFIERS", at, at, at);
            }
            (None, None) => {
                self.open("MODIFIERS", None, None, None);
            }
        }
        for annotation in modifiers
            .into_iter()
            .flat_map(|m| children(m, K::Annotation))
        {
            self.annotation(annotation, "ANNOTATION");
        }
        for &annotation in more {
            self.annotation(annotation, "ANNOTATION");
        }
        self.close();
    }

    fn empty_modifiers(&mut self) {
        self.open("MODIFIERS", None, None, None);
        self.close();
    }

    /// An annotation, as `kind`: `ANNOTATION` on declarations, `TYPE_ANNOTATION` on types.
    fn annotation(&mut self, annotation: SyntaxNode<'t>, kind: &'static str) {
        ferrion_stack::ensure_stack(|| {
            self.spanning(kind, annotation);
            if let Some(name) = child(annotation, K::Name) {
                self.name(name);
            }
            if let Some(arguments) = child(annotation, K::AnnotationArguments) {
                for argument in arguments.children() {
                    if argument.kind() == K::ElementValuePair {
                        let eq = token(argument, K::Eq).map_or(start(argument), |t| t.span().start);
                        self.node("ASSIGNMENT", start(argument), end(argument), eq);
                        if let Some(name) = name(argument) {
                            let span = name.span();
                            self.leaf("IDENTIFIER", span.start, span.end, span.start);
                        }
                        if let Some(value) = argument.children().next() {
                            self.element_value(value);
                        }
                        self.close();
                    } else {
                        self.element_value(argument);
                    }
                }
            }
            self.close();
        })
    }

    fn element_value(&mut self, value: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| match value.kind() {
            K::Annotation => self.annotation(value, "ANNOTATION"),
            K::ElementValueArray => {
                self.spanning("NEW_ARRAY", value);
                for value in value.children() {
                    self.element_value(value);
                }
                self.close();
            }
            _ => self.expr(value),
        })
    }

    // ----- Declarations -----

    /// A member of a class body, or of a compilation unit.
    fn member(&mut self, member: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| match member.kind() {
            K::ClassDecl
            | K::InterfaceDecl
            | K::EnumDecl
            | K::RecordDecl
            | K::AnnotationTypeDecl => self.type_decl(member),
            K::FieldDecl => self.variables(member, None),
            K::MethodDecl | K::ConstructorDecl => self.method(member),
            K::CompactConstructorDecl => self.compact_constructor(member),
            // An initializer is its block, from `static` if there is one.
            K::Initializer => {
                self.spanning("BLOCK", member);
                if let Some(block) = child(member, K::Block) {
                    self.statements(block);
                }
                self.close();
            }
            _ => self.erroneous(member),
        })
    }

    fn type_decl(&mut self, decl: SyntaxNode<'t>) {
        let (kind, keyword) = match decl.kind() {
            K::ClassDecl => ("CLASS", K::ClassKw),
            K::InterfaceDecl => ("INTERFACE", K::InterfaceKw),
            K::EnumDecl => ("ENUM", K::EnumKw),
            K::RecordDecl => ("RECORD", K::RecordKw),
            _ => ("ANNOTATION_TYPE", K::InterfaceKw),
        };
        // javac's preferred position is the keyword; `record` is found before the name if it
        // wasn't recorded as a keyword.
        let keyword = token(decl, keyword)
            .or_else(|| name(decl).and_then(|n| n.prev_non_trivia()))
            .map_or(start(decl), |t| t.span().start);
        self.node(kind, start(decl), end(decl), keyword);
        let modifiers = child(decl, K::Modifiers);
        if decl.kind() == K::AnnotationTypeDecl {
            // javac's modifiers include the `@` of `@interface`.
            let at = token(decl, K::At).map(|t| t.span());
            match at {
                Some(at) => {
                    let from = modifiers.map_or(at.start, start);
                    self.node("MODIFIERS", from, at.end, from);
                    for annotation in modifiers
                        .into_iter()
                        .flat_map(|m| children(m, K::Annotation))
                    {
                        self.annotation(annotation, "ANNOTATION");
                    }
                    self.close();
                }
                None => self.modifiers(modifiers, None, &[]),
            }
        } else {
            self.modifiers(modifiers, None, &[]);
        }
        if let Some(parameters) = child(decl, K::TypeParameters) {
            self.type_parameters(parameters);
        }
        for clause in decl.children() {
            if matches!(
                clause.kind(),
                K::ExtendsClause | K::ImplementsClause | K::PermitsClause
            ) {
                for ty in clause.children().filter(|c| is_type(c.kind())) {
                    self.ty(ty);
                }
            }
        }
        // A record's components are members, after `implements`.
        let header = child(decl, K::RecordHeader);
        if let Some(header) = header {
            for component in children(header, K::RecordComponent) {
                self.parameter(component);
            }
        }
        let saved = std::mem::replace(&mut self.record_header, header);
        if let Some(body) = child(decl, K::ClassBody) {
            self.class_body(body);
        } else if let Some(body) = child(decl, K::EnumBody) {
            for constant in children(body, K::EnumConstant) {
                self.enum_constant(constant);
            }
            for member in body.children() {
                if member.kind() != K::EnumConstant {
                    self.member(member);
                }
            }
        }
        self.record_header = saved;
        self.close();
    }

    /// The members of a class body. `;` has no node.
    fn class_body(&mut self, body: SyntaxNode<'t>) {
        for member in body.children() {
            self.member(member);
        }
    }

    fn type_parameters(&mut self, parameters: SyntaxNode<'t>) {
        for parameter in children(parameters, K::TypeParameter) {
            self.spanning("TYPE_PARAMETER", parameter);
            for annotation in children(parameter, K::Annotation) {
                self.annotation(annotation, "TYPE_ANNOTATION");
            }
            for bound in parameter.children().filter(|c| is_type(c.kind())) {
                self.ty(bound);
            }
            self.close();
        }
    }

    /// A field or local variable declaration: a `VARIABLE` per declarator. The first ends after
    /// its `,`; the others start at the declaration's start and share its modifiers and type.
    /// `end` replaces the last one's end (a resource's `;`).
    fn variables(&mut self, decl: SyntaxNode<'t>, last_end: Option<u32>) {
        let modifiers = child(decl, K::Modifiers);
        let ty = first_type(decl);
        let declarators: Vec<SyntaxNode> = children(decl, K::VariableDeclarator).collect();
        let commas: Vec<SyntaxToken> = tokens(decl).filter(|t| t.kind() == K::Comma).collect();
        for (i, declarator) in declarators.iter().enumerate() {
            let until = if i + 1 < declarators.len() {
                commas.get(i).map_or(end(*declarator), |t| t.span().end)
            } else {
                last_end.unwrap_or(end(decl))
            };
            let at = name(*declarator).map_or(start(*declarator), |t| t.span().start);
            self.node("VARIABLE", start(decl), until, at);
            let shared = i > 0;
            self.with_shared(shared, |rows| rows.modifiers(modifiers, None, &[]));
            if let Some(ty) = ty {
                let dims: Vec<SyntaxNode> = children(*declarator, K::Dim).collect();
                self.trailing_dims(ty, &dims, |rows| {
                    rows.with_shared(shared, |rows| rows.local_type(ty));
                });
            }
            if let Some(init) = declarator
                .children()
                .find(|c| is_expr(c.kind()) || c.kind() == K::ArrayInitializer)
            {
                self.initializer(init);
            }
            self.close();
        }
    }

    /// A type where javac reads `var` as `VAR_TYPE`: local variables, lambda parameters, and
    /// patterns.
    fn local_type(&mut self, ty: SyntaxNode<'t>) {
        if is_var(ty) && self.release >= VAR_RELEASE {
            self.spanning("VAR_TYPE", ty);
            self.close();
        } else {
            self.ty(ty);
        }
    }

    fn initializer(&mut self, init: SyntaxNode<'t>) {
        if init.kind() == K::ArrayInitializer {
            self.array_initializer(init);
        } else {
            self.expr(init);
        }
    }

    fn array_initializer(&mut self, init: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| {
            self.spanning("NEW_ARRAY", init);
            for element in init.children() {
                self.initializer(element);
            }
            self.close();
        })
    }

    /// C-style dimensions after a name (or a method's parameters) wrap the type `ty`, which
    /// `inner` writes: the first is outermost, and each spans from the type to the last `]`.
    fn trailing_dims(
        &mut self,
        ty: SyntaxNode<'t>,
        dims: &[SyntaxNode<'t>],
        inner: impl FnOnce(&mut Self),
    ) {
        let until = dims.last().map_or(end(ty), |dim| end(*dim));
        let layers: Vec<Layer> = dims.iter().map(|dim| Layer::dim(*dim, until)).collect();
        self.layers(start(ty), layers, inner);
    }

    /// Writes array layers, outermost first, around what `inner` writes.
    fn layers(&mut self, from: u32, layers: Vec<Layer<'t>>, inner: impl FnOnce(&mut Self)) {
        let mut opened = 0;
        for layer in &layers {
            if !layer.annotations.is_empty() {
                self.node("ANNOTATED_TYPE", from, layer.annotated.0, layer.annotated.1);
                for annotation in &layer.annotations {
                    self.annotation(*annotation, "TYPE_ANNOTATION");
                }
                opened += 1;
            }
            self.node("ARRAY_TYPE", from, layer.array.0, layer.array.1);
            opened += 1;
        }
        inner(self);
        for _ in 0..opened {
            self.close();
        }
    }

    fn method(&mut self, method: SyntaxNode<'t>) {
        let at = name(method).map_or(start(method), |t| t.span().start);
        self.node("METHOD", start(method), end(method), at);
        let type_parameters = child(method, K::TypeParameters);
        let annotations: Vec<SyntaxNode> = children(method, K::Annotation).collect();
        let empty_at = type_parameters.map(start);
        self.modifiers(child(method, K::Modifiers), empty_at, &annotations);
        if method.kind() == K::MethodDecl
            && let Some(result) = first_type(method)
        {
            let dims: Vec<SyntaxNode> = children(method, K::Dim).collect();
            self.trailing_dims(result, &dims, |rows| rows.ty(result));
        }
        if let Some(parameters) = type_parameters {
            self.type_parameters(parameters);
        }
        if let Some(parameters) = child(method, K::FormalParameters) {
            for parameter in children(parameters, K::FormalParameter) {
                self.parameter(parameter);
            }
            // javac lists the receiver parameter after the others.
            if let Some(receiver) = child(parameters, K::ReceiverParameter) {
                self.receiver(receiver);
            }
        }
        if let Some(throws) = child(method, K::ThrowsClause) {
            for ty in throws.children().filter(|c| is_type(c.kind())) {
                self.ty(ty);
            }
        }
        if let Some(body) = child(method, K::Block) {
            self.block(body);
        }
        if let Some(value) = child(method, K::DefaultValue).and_then(|d| d.children().next()) {
            self.element_value(value);
        }
        self.close();
    }

    /// A compact constructor: javac gives it copies of the record's components as parameters,
    /// with the components' starts and no ends.
    fn compact_constructor(&mut self, constructor: SyntaxNode<'t>) {
        let at = name(constructor).map_or(start(constructor), |t| t.span().start);
        self.node("METHOD", start(constructor), end(constructor), at);
        self.modifiers(child(constructor, K::Modifiers), None, &[]);
        if let Some(header) = self.record_header {
            let saved = std::mem::replace(&mut self.copy, true);
            for component in children(header, K::RecordComponent) {
                self.parameter(component);
            }
            self.copy = saved;
        }
        if let Some(body) = child(constructor, K::Block) {
            self.block(body);
        }
        self.close();
    }

    /// A formal parameter, record component, or typed lambda parameter: a `VARIABLE` to the end of
    /// its name (and any dimensions after it).
    fn parameter(&mut self, parameter: SyntaxNode<'t>) {
        let at = name(parameter).map_or(start(parameter), |t| t.span().start);
        let row = self.node("VARIABLE", start(parameter), end(parameter), at);
        self.modifiers(child(parameter, K::Modifiers), None, &[]);
        let type_row = self.rows.len();
        if let Some(ty) = first_type(parameter) {
            match child(parameter, K::Varargs) {
                Some(varargs) => self.varargs_type(ty, varargs),
                None => {
                    let dims: Vec<SyntaxNode> = children(parameter, K::Dim).collect();
                    let local = parameter.kind() == K::LambdaParameter;
                    self.trailing_dims(ty, &dims, |rows| {
                        if local {
                            rows.local_type(ty);
                        } else {
                            rows.ty(ty);
                        }
                    });
                }
            }
        }
        // javac's copy of a component is preferred where its type is.
        if self.copy
            && let Some(preferred) = self.rows.get(type_row).and_then(|r| r.preferred)
        {
            self.rows[row].preferred = Some(preferred);
        }
        self.close();
    }

    /// A variable arity parameter's type. javac makes the `...` the innermost dimension, and
    /// positions annotated dimensions its own way (see the normalization document).
    fn varargs_type(&mut self, ty: SyntaxNode<'t>, varargs: SyntaxNode<'t>) {
        let from = start(ty);
        let Some(ellipsis) = token(varargs, K::Ellipsis) else {
            self.ty(ty);
            return;
        };
        let after = ellipsis.span().end;
        let before = ellipsis.prev_non_trivia().map_or(after, |t| t.span().end);
        let (element, dims): (SyntaxNode, Vec<SyntaxNode>) = if ty.kind() == K::ArrayType {
            (first_type(ty).unwrap_or(ty), children(ty, K::Dim).collect())
        } else {
            (ty, Vec::new())
        };
        let mut layers = Vec::new();
        for (i, dim) in dims.iter().enumerate() {
            let mut layer = Layer::dim(*dim, before);
            let first_annotation = layer.annotations.first().map(|a| start(*a));
            if i == 0 {
                match first_annotation {
                    Some(at) => {
                        layer.annotated = (after, at);
                        layer.array = (before, at);
                    }
                    None => layer.array.0 = after,
                }
            }
            layers.push(layer);
        }
        let annotations: Vec<SyntaxNode> = children(varargs, K::Annotation).collect();
        let first_annotation = annotations.first().map(|a| start(*a));
        let ellipsis_at = ellipsis.span().start;
        layers.push(Layer {
            annotated: (after, first_annotation.unwrap_or(ellipsis_at)),
            array: (after, ellipsis_at),
            annotations,
        });
        let stretch = dims.is_empty() && first_annotation.is_some();
        self.layers(from, layers, |rows| {
            let row = rows.rows.len();
            rows.ty(element);
            // With annotations on the `...` alone, the element type ends after them.
            if stretch {
                rows.rows[row].end = Some(before);
            }
        });
    }

    /// `A this` or `A Outer.this`: a `VARIABLE` whose name is an expression.
    fn receiver(&mut self, receiver: SyntaxNode<'t>) {
        let this = token(receiver, K::ThisKw);
        let qualifier =
            tokens(receiver).find(|t| matches!(t.kind(), K::Identifier | K::Underscore));
        let name_start = qualifier
            .or(this)
            .map_or(start(receiver), |t| t.span().start);
        self.node("VARIABLE", start(receiver), end(receiver), name_start);
        self.modifiers(child(receiver, K::Modifiers), None, &[]);
        if let Some(ty) = first_type(receiver) {
            self.ty(ty);
        }
        if let Some(this) = this {
            let span = this.span();
            match qualifier {
                Some(qualifier) => {
                    let dot = token(receiver, K::Dot).map_or(span.start, |t| t.span().start);
                    let q = qualifier.span();
                    self.node("MEMBER_SELECT", q.start, span.end, dot);
                    self.leaf("IDENTIFIER", q.start, q.end, q.start);
                    self.close();
                }
                None => self.leaf("IDENTIFIER", span.start, span.end, span.start),
            }
        }
        self.close();
    }

    /// An enum constant: a `VARIABLE`, preferred at its start (its annotations, if any), whose
    /// type and initializer javac synthesizes without ends.
    fn enum_constant(&mut self, constant: SyntaxNode<'t>) {
        let Some(name) = name(constant) else {
            self.erroneous(constant);
            return;
        };
        let name = name.span();
        self.spanning("VARIABLE", constant);
        self.modifiers(child(constant, K::Modifiers), None, &[]);
        self.open("IDENTIFIER", Some(name.start), None, Some(name.start));
        self.close();
        let arguments = child(constant, K::Arguments);
        let body = child(constant, K::ClassBody);
        let has_arguments = arguments.is_some_and(|a| exprs(a).next().is_some());
        // The `NEW_CLASS` spans the arguments and body; without either (or with only `()`) it
        // starts at the name and has no end.
        let (from, until) = match (arguments, body) {
            (Some(arguments), Some(body)) => (start(arguments), Some(end(body))),
            (None, Some(body)) => (start(body), Some(end(body))),
            (Some(arguments), None) if has_arguments => (start(arguments), Some(end(arguments))),
            _ => (name.start, None),
        };
        self.open("NEW_CLASS", Some(from), until, Some(from));
        self.open("IDENTIFIER", Some(name.start), None, Some(name.start));
        self.close();
        if let Some(arguments) = arguments {
            for argument in exprs(arguments) {
                self.expr(argument);
            }
        }
        if let Some(body) = body {
            self.node("ENUM", name.start, end(body), name.start);
            self.empty_modifiers();
            self.class_body(body);
            self.close();
        }
        self.close();
        self.close();
    }

    // ----- Types -----

    fn ty(&mut self, ty: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| match ty.kind() {
            K::PrimitiveType => {
                let annotations: Vec<SyntaxNode> = children(ty, K::Annotation).collect();
                let keyword = tokens(ty).last().map_or(ty.span(), |t| t.span());
                if let Some(first) = annotations.first() {
                    self.node("ANNOTATED_TYPE", start(ty), end(ty), start(*first));
                    for annotation in &annotations {
                        self.annotation(*annotation, "TYPE_ANNOTATION");
                    }
                    self.leaf("PRIMITIVE_TYPE", keyword.start, keyword.end, keyword.start);
                    self.close();
                } else {
                    self.leaf("PRIMITIVE_TYPE", keyword.start, keyword.end, keyword.start);
                }
            }
            K::VoidType => {
                self.spanning("PRIMITIVE_TYPE", ty);
                self.close();
            }
            K::ClassType => self.class_type(ty),
            K::ArrayType => {
                let Some(element) = first_type(ty) else {
                    self.erroneous(ty);
                    return;
                };
                let until = end(ty);
                let mut layers: Vec<Layer> = children(ty, K::Dim)
                    .map(|dim| Layer::dim(dim, until))
                    .collect();
                // After a class type, an annotated first dimension is preferred at its first
                // annotation, not its `[`.
                if element.kind() != K::PrimitiveType
                    && let Some(first) = layers.first_mut()
                    && let Some(at) = first.annotations.first().map(|a| start(*a))
                {
                    first.annotated.1 = at;
                    first.array.1 = at;
                }
                self.layers(start(ty), layers, |rows| rows.ty(element));
            }
            K::IntersectionType => self.intersection(ty, start(ty)),
            K::UnionType => {
                self.spanning("UNION_TYPE", ty);
                for alternative in ty.children().filter(|c| is_type(c.kind())) {
                    self.ty(alternative);
                }
                self.close();
            }
            K::Wildcard => self.wildcard(ty),
            _ => self.erroneous(ty),
        })
    }

    fn intersection(&mut self, ty: SyntaxNode<'t>, from: u32) {
        self.node("INTERSECTION_TYPE", from, end(ty), from);
        for bound in ty.children().filter(|c| is_type(c.kind())) {
            self.ty(bound);
        }
        self.close();
    }

    /// A class type segment: `PARAMETERIZED_TYPE` around `ANNOTATED_TYPE` around
    /// `MEMBER_SELECT` (or `IDENTIFIER`), each as the segment needs.
    fn class_type(&mut self, ty: SyntaxNode<'t>) {
        let qualifier = child(ty, K::ClassType);
        let annotations: Vec<SyntaxNode> = children(ty, K::Annotation).collect();
        let arguments = child(ty, K::TypeArguments);
        let name = tokens(ty)
            .filter(|t| matches!(t.kind(), K::Identifier | K::Underscore))
            .last();
        let name_end = name.map_or(end(ty), |t| t.span().end);
        if let Some(arguments) = arguments {
            let lt = token(arguments, K::Lt).map_or(start(arguments), |t| t.span().start);
            self.node("PARAMETERIZED_TYPE", start(ty), end(arguments), lt);
        }
        if let Some(first) = annotations.first() {
            self.node("ANNOTATED_TYPE", start(ty), name_end, start(*first));
            for annotation in &annotations {
                self.annotation(*annotation, "TYPE_ANNOTATION");
            }
        }
        match qualifier {
            Some(qualifier) => {
                let dot = token(ty, K::Dot).map_or(end(qualifier), |t| t.span().start);
                self.node("MEMBER_SELECT", start(ty), name_end, dot);
                self.class_type(qualifier);
                self.close();
            }
            None => {
                let span = name.map_or(ty.span(), |t| t.span());
                self.leaf("IDENTIFIER", span.start, span.end, span.start);
            }
        }
        if !annotations.is_empty() {
            self.close();
        }
        if let Some(arguments) = arguments {
            for argument in arguments.children() {
                self.ty(argument);
            }
            self.close();
        }
    }

    fn wildcard(&mut self, wildcard: SyntaxNode<'t>) {
        let annotations: Vec<SyntaxNode> = children(wildcard, K::Annotation).collect();
        if let Some(first) = annotations.first() {
            self.node(
                "ANNOTATED_TYPE",
                start(wildcard),
                end(wildcard),
                start(*first),
            );
            for annotation in &annotations {
                self.annotation(*annotation, "TYPE_ANNOTATION");
            }
        }
        let question = token(wildcard, K::Question).map_or(start(wildcard), |t| t.span().start);
        let kind = if token(wildcard, K::ExtendsKw).is_some() {
            "EXTENDS_WILDCARD"
        } else if token(wildcard, K::SuperKw).is_some() {
            "SUPER_WILDCARD"
        } else {
            "UNBOUNDED_WILDCARD"
        };
        self.node(kind, question, end(wildcard), question);
        if let Some(bound) = first_type(wildcard) {
            self.ty(bound);
        }
        self.close();
        if !annotations.is_empty() {
            self.close();
        }
    }

    /// A type or an expression, where the syntax allows either.
    fn expr_or_type(&mut self, node: SyntaxNode<'t>) {
        if is_type(node.kind()) {
            self.ty(node);
        } else {
            self.expr(node);
        }
    }

    // ----- Expressions -----

    fn expr(&mut self, expr: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| self.expr_inner(expr))
    }

    fn expr_inner(&mut self, e: SyntaxNode<'t>) {
        match e.kind() {
            K::LiteralExpr => {
                let kind = tokens(e)
                    .next()
                    .map_or("ERRONEOUS", |t| literal_kind(t.kind()));
                self.spanning(kind, e);
                self.close();
            }
            K::NameExpr => {
                self.spanning("IDENTIFIER", e);
                self.close();
            }
            K::ThisExpr | K::SuperExpr => match first_expr(e) {
                Some(qualifier) => {
                    self.selected(e, qualifier);
                }
                None => {
                    self.spanning("IDENTIFIER", e);
                    self.close();
                }
            },
            K::ParenExpr => {
                self.spanning("PARENTHESIZED", e);
                if let Some(inner) = first_expr(e) {
                    self.expr(inner);
                }
                self.close();
            }
            K::ClassLiteralExpr => match e.children().next() {
                Some(target) => {
                    let dot = token(e, K::Dot).map_or(start(e), |t| t.span().start);
                    self.node("MEMBER_SELECT", start(e), end(e), dot);
                    self.expr_or_type(target);
                    self.close();
                }
                None => self.erroneous(e),
            },
            K::FieldAccessExpr => match first_expr(e) {
                Some(target) => self.selected(e, target),
                None => self.erroneous(e),
            },
            K::ArrayAccessExpr => {
                let bracket = token(e, K::LBracket).map_or(start(e), |t| t.span().start);
                self.node("ARRAY_ACCESS", start(e), end(e), bracket);
                for operand in exprs(e) {
                    self.expr(operand);
                }
                self.close();
            }
            K::MethodCallExpr | K::ConstructorCallExpr => self.call(e),
            K::NewExpr => self.new_class(e),
            K::ArrayCreationExpr => self.new_array(e),
            K::PrefixExpr => {
                let operand = first_expr(e);
                // javac reads `-` before a decimal integer literal as part of the literal.
                if let Some(literal) = operand.and_then(|o| tokens(o).next())
                    && tokens(e).next().map(|t| t.kind()) == Some(K::Minus)
                    && operand.is_some_and(|o| o.kind() == K::LiteralExpr)
                    && matches!(literal.kind(), K::IntLiteral | K::LongLiteral)
                    && literal.text().starts_with(|c: char| matches!(c, '1'..='9'))
                {
                    self.spanning(literal_kind(literal.kind()), e);
                    self.close();
                    return;
                }
                let kind = match tokens(e).next().map(|t| t.kind()) {
                    Some(K::Plus) => "UNARY_PLUS",
                    Some(K::Minus) => "UNARY_MINUS",
                    Some(K::Bang) => "LOGICAL_COMPLEMENT",
                    Some(K::Tilde) => "BITWISE_COMPLEMENT",
                    Some(K::PlusPlus) => "PREFIX_INCREMENT",
                    Some(K::MinusMinus) => "PREFIX_DECREMENT",
                    _ => "ERRONEOUS",
                };
                self.spanning(kind, e);
                if let Some(operand) = first_expr(e) {
                    self.expr(operand);
                }
                self.close();
            }
            K::PostfixExpr => {
                let op = tokens(e).last();
                let kind = match op.map(|t| t.kind()) {
                    Some(K::PlusPlus) => "POSTFIX_INCREMENT",
                    Some(K::MinusMinus) => "POSTFIX_DECREMENT",
                    _ => "ERRONEOUS",
                };
                let at = op.map_or(start(e), |t| t.span().start);
                self.node(kind, start(e), end(e), at);
                if let Some(operand) = first_expr(e) {
                    self.expr(operand);
                }
                self.close();
            }
            K::BinaryExpr => self.binary(e),
            K::InstanceofExpr => {
                let at = token(e, K::InstanceofKw).map_or(start(e), |t| t.span().start);
                self.node("INSTANCE_OF", start(e), end(e), at);
                for operand in e.children() {
                    if is_pattern(operand.kind()) {
                        self.pattern(operand);
                    } else {
                        self.expr_or_type(operand);
                    }
                }
                self.close();
            }
            K::ConditionalExpr => {
                let at = token(e, K::Question).map_or(start(e), |t| t.span().start);
                self.node("CONDITIONAL_EXPRESSION", start(e), end(e), at);
                for operand in exprs(e) {
                    self.expr(operand);
                }
                self.close();
            }
            K::AssignExpr => {
                let (ops, at) = operator(e);
                self.node(assignment_kind(&ops), start(e), end(e), at);
                for operand in exprs(e) {
                    self.expr(operand);
                }
                self.close();
            }
            K::CastExpr => {
                self.spanning("TYPE_CAST", e);
                for operand in e.children() {
                    match operand.kind() {
                        // javac's intersection type starts at the cast's `(`.
                        K::IntersectionType => self.intersection(operand, start(e)),
                        kind if is_type(kind) => self.ty(operand),
                        _ => self.expr(operand),
                    }
                }
                self.close();
            }
            K::LambdaExpr => {
                self.spanning("LAMBDA_EXPRESSION", e);
                if let Some(parameters) = child(e, K::LambdaParameters) {
                    for parameter in children(parameters, K::LambdaParameter) {
                        self.lambda_parameter(parameter);
                    }
                }
                if let Some(body) = e
                    .children()
                    .filter(|c| c.kind() != K::LambdaParameters)
                    .last()
                {
                    if body.kind() == K::Block {
                        self.block(body);
                    } else {
                        self.expr(body);
                    }
                }
                self.close();
            }
            K::MethodRefExpr => {
                self.spanning("MEMBER_REFERENCE", e);
                if let Some(qualifier) = e.children().next() {
                    self.expr_or_type(qualifier);
                }
                if let Some(arguments) = child(e, K::TypeArguments) {
                    for argument in arguments.children() {
                        self.ty(argument);
                    }
                }
                self.close();
            }
            K::SwitchExpr => self.switch(e, "SWITCH_EXPRESSION", true),
            _ => self.erroneous(e),
        }
    }

    /// `target.name`: a `MEMBER_SELECT` preferred at the `.`.
    fn selected(&mut self, e: SyntaxNode<'t>, target: SyntaxNode<'t>) {
        let dot = token(e, K::Dot).map_or(end(target), |t| t.span().start);
        self.node("MEMBER_SELECT", start(e), end(e), dot);
        self.expr(target);
        self.close();
    }

    /// A method call, or `this(...)` or `super(...)`: a `METHOD_INVOCATION` preferred at the
    /// `(`, with the type arguments, then the method (a `MEMBER_SELECT` spanning the type
    /// arguments, or an `IDENTIFIER`), then the arguments.
    fn call(&mut self, e: SyntaxNode<'t>) {
        let arguments = child(e, K::Arguments);
        let paren = arguments
            .and_then(|a| token(a, K::LParen))
            .map_or(start(e), |t| t.span().start);
        self.node("METHOD_INVOCATION", start(e), end(e), paren);
        let type_arguments = child(e, K::TypeArguments);
        if let Some(type_arguments) = type_arguments {
            for argument in type_arguments.children() {
                self.ty(argument);
            }
        }
        let name = if e.kind() == K::MethodCallExpr {
            tokens(e).find(|t| matches!(t.kind(), K::Identifier | K::Underscore))
        } else {
            tokens(e).find(|t| matches!(t.kind(), K::ThisKw | K::SuperKw))
        };
        let name = name.map_or(e.span(), |t| t.span());
        match first_expr(e) {
            Some(receiver) => {
                let dot = token(e, K::Dot).map_or(name.start, |t| t.span().start);
                self.node("MEMBER_SELECT", start(e), name.end, dot);
                self.expr(receiver);
                self.close();
            }
            // `<T>this()`: the `IDENTIFIER` starts at the type arguments.
            None => {
                let from = type_arguments.map_or(name.start, start);
                self.leaf("IDENTIFIER", from, name.end, from);
            }
        }
        if let Some(arguments) = arguments {
            for argument in exprs(arguments) {
                self.expr(argument);
            }
        }
        self.close();
    }

    fn new_class(&mut self, e: SyntaxNode<'t>) {
        let new = token(e, K::NewKw).map_or(start(e), |t| t.span().start);
        self.node("NEW_CLASS", start(e), end(e), new);
        if let Some(outer) = first_expr(e) {
            self.expr(outer);
        }
        if let Some(ty) = first_type(e) {
            self.ty(ty);
        }
        if let Some(type_arguments) = child(e, K::TypeArguments) {
            for argument in type_arguments.children() {
                self.ty(argument);
            }
        }
        if let Some(arguments) = child(e, K::Arguments) {
            for argument in exprs(arguments) {
                self.expr(argument);
            }
        }
        if let Some(body) = child(e, K::ClassBody) {
            self.spanning("CLASS", body);
            self.empty_modifiers();
            self.class_body(body);
            self.close();
        }
        self.close();
    }

    /// An array creation: the element type, wrapped in its extra dimensions; the dimension
    /// expressions; the initializer's elements; then the annotations of the dimensions that
    /// aren't part of the element type.
    fn new_array(&mut self, e: SyntaxNode<'t>) {
        let new = token(e, K::NewKw).map_or(start(e), |t| t.span().start);
        self.node("NEW_ARRAY", start(e), end(e), new);
        let sizes: Vec<SyntaxNode> = children(e, K::DimExpr).collect();
        let dims: Vec<SyntaxNode> = children(e, K::Dim).collect();
        let (own, extra) = match (sizes.is_empty(), dims.split_first()) {
            (true, Some((first, rest))) => (Some(*first), rest.to_vec()),
            _ => (None, dims.clone()),
        };
        if let Some(element) = first_type(e) {
            let until = dims.last().map_or(end(element), |dim| end(*dim));
            let layers = extra.iter().map(|dim| Layer::dim(*dim, until)).collect();
            self.layers(start(element), layers, |rows| {
                let row = rows.rows.len();
                rows.ty(element);
                // An annotated primitive element type is preferred at its keyword here, not at
                // its first annotation as elsewhere.
                if element.kind() == K::PrimitiveType
                    && let Some(keyword) = tokens(element).last()
                    && rows.rows[row].kind == "ANNOTATED_TYPE"
                {
                    rows.rows[row].preferred = Some(keyword.span().start);
                }
            });
        }
        for size in &sizes {
            if let Some(size) = first_expr(*size) {
                self.expr(size);
            }
        }
        if let Some(init) = child(e, K::ArrayInitializer) {
            for element in init.children() {
                self.initializer(element);
            }
        }
        for dim in sizes.iter().chain(own.as_ref()) {
            for annotation in children(*dim, K::Annotation) {
                self.annotation(annotation, "TYPE_ANNOTATION");
            }
        }
        self.close();
    }

    fn lambda_parameter(&mut self, parameter: SyntaxNode<'t>) {
        if first_type(parameter).is_some() {
            self.parameter(parameter);
            return;
        }
        let span = name(parameter).map_or(parameter.span(), |t| t.span());
        self.node("VARIABLE", span.start, span.end, span.start);
        self.modifiers(child(parameter, K::Modifiers), None, &[]);
        self.close();
    }

    /// A binary expression. javac folds adjacent string literals in a chain of `+` into one
    /// `STRING_LITERAL`, but only in the chain at the top of a binary expression, and then
    /// prefers each `+` of the chain at its right operand.
    fn binary(&mut self, e: SyntaxNode<'t>) {
        let (ops, at) = operator(e);
        let top = e
            .parent()
            .is_none_or(|p| !matches!(p.kind(), K::BinaryExpr | K::InstanceofExpr));
        if top && ops == [K::Plus] {
            let mut operands = Vec::new();
            let mut node = e;
            while node.kind() == K::BinaryExpr && operator(node).0 == [K::Plus] {
                let mut sides = exprs(node);
                let (Some(lhs), Some(rhs)) = (sides.next(), sides.next()) else {
                    break;
                };
                operands.push(rhs);
                node = lhs;
            }
            operands.push(node);
            operands.reverse();
            if operands
                .windows(2)
                .any(|pair| is_string_literal(pair[0]) && is_string_literal(pair[1]))
            {
                self.folded(&operands);
                return;
            }
        }
        self.node(binary_kind(&ops), start(e), end(e), at);
        for operand in exprs(e) {
            self.expr(operand);
        }
        self.close();
    }

    fn folded(&mut self, operands: &[SyntaxNode<'t>]) {
        let mut groups: Vec<Vec<SyntaxNode>> = Vec::new();
        for &operand in operands {
            match groups.last_mut() {
                Some(group)
                    if is_string_literal(operand)
                        && group.last().is_some_and(|last| is_string_literal(*last)) =>
                {
                    group.push(operand);
                }
                _ => groups.push(vec![operand]),
            }
        }
        let from = start(operands[0]);
        for group in groups[1..].iter().rev() {
            let last = group[group.len() - 1];
            self.node("PLUS", from, end(last), start(group[0]));
        }
        for (i, group) in groups.iter().enumerate() {
            match group.as_slice() {
                [single] => self.expr(*single),
                [first, .., last] => {
                    self.leaf("STRING_LITERAL", start(*first), end(*last), start(*first));
                }
                [] => {}
            }
            if i > 0 {
                self.close();
            }
        }
    }

    // ----- Statements -----

    fn block(&mut self, block: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| {
            self.spanning("BLOCK", block);
            self.statements(block);
            self.close();
        })
    }

    fn statements(&mut self, block: SyntaxNode<'t>) {
        for statement in block.children() {
            self.statement(statement);
        }
    }

    /// `(condition)`: the statement's parentheses, as javac's `PARENTHESIZED`.
    fn condition(&mut self, statement: SyntaxNode<'t>) {
        let (Some(open), Some(close)) = (token(statement, K::LParen), token(statement, K::RParen))
        else {
            if let Some(condition) = first_expr(statement) {
                self.expr(condition);
            }
            return;
        };
        let at = open.span().start;
        self.node("PARENTHESIZED", at, close.span().end, at);
        if let Some(condition) = first_expr(statement) {
            self.expr(condition);
        }
        self.close();
    }

    fn statement(&mut self, s: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| self.statement_inner(s))
    }

    fn statement_inner(&mut self, s: SyntaxNode<'t>) {
        let simple = |rows: &mut Self, kind: &'static str| {
            rows.spanning(kind, s);
            for child in s.children() {
                rows.child_statement_or_expr(child);
            }
            rows.close();
        };
        match s.kind() {
            K::Block => self.block(s),
            K::LocalVarDecl => self.variables(s, None),
            kind if is_type_decl(kind) => self.type_decl(s),
            K::EmptyStmt => simple(self, "EMPTY_STATEMENT"),
            K::ExprStmt => simple(self, "EXPRESSION_STATEMENT"),
            K::AssertStmt => simple(self, "ASSERT"),
            K::LabeledStmt => simple(self, "LABELED_STATEMENT"),
            K::BreakStmt => simple(self, "BREAK"),
            K::ContinueStmt => simple(self, "CONTINUE"),
            K::ReturnStmt => simple(self, "RETURN"),
            K::ThrowStmt => simple(self, "THROW"),
            K::YieldStmt => simple(self, "YIELD"),
            K::IfStmt => {
                self.spanning("IF", s);
                self.condition(s);
                for branch in s.children().filter(|c| !is_expr(c.kind())) {
                    self.statement(branch);
                }
                self.close();
            }
            K::WhileStmt => {
                self.spanning("WHILE_LOOP", s);
                self.condition(s);
                for body in s.children().filter(|c| !is_expr(c.kind())) {
                    self.statement(body);
                }
                self.close();
            }
            K::DoStmt => {
                self.spanning("DO_WHILE_LOOP", s);
                for body in s.children().filter(|c| !is_expr(c.kind())) {
                    self.statement(body);
                }
                self.condition(s);
                self.close();
            }
            K::SynchronizedStmt => {
                self.spanning("SYNCHRONIZED", s);
                self.condition(s);
                if let Some(body) = child(s, K::Block) {
                    self.block(body);
                }
                self.close();
            }
            K::ForStmt => {
                self.spanning("FOR_LOOP", s);
                for part in s.children() {
                    match part.kind() {
                        K::ForInit => match child(part, K::LocalVarDecl) {
                            Some(decl) => self.variables(decl, None),
                            None => self.expression_statements(part),
                        },
                        K::ForUpdate => self.expression_statements(part),
                        kind if is_expr(kind) => self.expr(part),
                        _ => self.statement(part),
                    }
                }
                self.close();
            }
            K::ForEachStmt => {
                self.spanning("ENHANCED_FOR_LOOP", s);
                for part in s.children() {
                    match part.kind() {
                        K::LocalVarDecl => self.variables(part, None),
                        kind if is_expr(kind) => self.expr(part),
                        _ => self.statement(part),
                    }
                }
                self.close();
            }
            K::TryStmt => self.try_statement(s),
            K::SwitchStmt => self.switch(s, "SWITCH", false),
            _ => self.erroneous(s),
        }
    }

    /// A child of a simple statement: an expression, or a statement (a label's body).
    fn child_statement_or_expr(&mut self, child: SyntaxNode<'t>) {
        if is_expr(child.kind()) {
            self.expr(child);
        } else {
            self.statement(child);
        }
    }

    /// A `for` header's expressions: an `EXPRESSION_STATEMENT` for each, without a `;`.
    fn expression_statements(&mut self, part: SyntaxNode<'t>) {
        for expr in exprs(part) {
            self.spanning("EXPRESSION_STATEMENT", expr);
            self.expr(expr);
            self.close();
        }
    }

    fn try_statement(&mut self, s: SyntaxNode<'t>) {
        self.spanning("TRY", s);
        if let Some(resources) = child(s, K::ResourceSpecification) {
            for resource in resources.children() {
                // javac's resource ends after the `;` that follows it, if one does.
                let semi = resource
                    .last_token()
                    .next_non_trivia()
                    .filter(|t| t.kind() == K::Semi)
                    .map(|t| t.span().end);
                if resource.kind() == K::LocalVarDecl {
                    self.variables(resource, semi);
                } else {
                    let row = self.rows.len();
                    self.expr(resource);
                    if let (Some(semi), Some(row)) = (semi, self.rows.get_mut(row)) {
                        row.end = Some(semi);
                    }
                }
            }
        }
        for part in s.children() {
            match part.kind() {
                K::Block => self.block(part),
                K::CatchClause => {
                    self.spanning("CATCH", part);
                    if let Some(parameter) = child(part, K::CatchParameter) {
                        let at = name(parameter).map_or(start(parameter), |t| t.span().start);
                        self.node("VARIABLE", start(parameter), end(parameter), at);
                        self.modifiers(child(parameter, K::Modifiers), None, &[]);
                        if let Some(ty) = first_type(parameter) {
                            let dims: Vec<SyntaxNode> = children(parameter, K::Dim).collect();
                            self.trailing_dims(ty, &dims, |rows| rows.ty(ty));
                        }
                        self.close();
                    }
                    if let Some(body) = child(part, K::Block) {
                        self.block(body);
                    }
                    self.close();
                }
                K::FinallyClause => {
                    if let Some(body) = child(part, K::Block) {
                        self.block(body);
                    }
                }
                _ => {}
            }
        }
        self.close();
    }

    /// A switch statement or expression: the selector in parentheses, then a `CASE` per label.
    fn switch(&mut self, s: SyntaxNode<'t>, kind: &'static str, expression: bool) {
        self.spanning(kind, s);
        self.condition(s);
        if let Some(body) = child(s, K::SwitchBlock) {
            for item in body.children() {
                match item.kind() {
                    K::SwitchRule => self.switch_rule(item, expression),
                    K::SwitchGroup => self.switch_group(item),
                    _ => self.erroneous(item),
                }
            }
        }
        self.close();
    }

    /// `case ... -> body`: one `CASE`. An expression body is wrapped in an
    /// `EXPRESSION_STATEMENT` in a switch statement.
    fn switch_rule(&mut self, rule: SyntaxNode<'t>, expression: bool) {
        self.spanning("CASE", rule);
        if let Some(label) = child(rule, K::SwitchLabel) {
            self.case_labels(label);
        }
        for body in rule.children().filter(|c| c.kind() != K::SwitchLabel) {
            match body.kind() {
                K::Block => self.block(body),
                kind if is_expr(kind) && !expression => {
                    let semi = token(rule, K::Semi).map_or(end(body), |t| t.span().end);
                    self.node("EXPRESSION_STATEMENT", start(body), semi, start(body));
                    self.expr(body);
                    self.close();
                }
                kind if is_expr(kind) => self.expr(body),
                _ => self.statement(body),
            }
        }
        self.close();
    }

    /// `case 1: case 2: statements`: a `CASE` per label; the last holds the statements.
    fn switch_group(&mut self, group: SyntaxNode<'t>) {
        let labels: Vec<SyntaxNode> = children(group, K::SwitchLabel).collect();
        let colons: Vec<SyntaxToken> = tokens(group).filter(|t| t.kind() == K::Colon).collect();
        let statements: Vec<SyntaxNode> = group
            .children()
            .filter(|c| c.kind() != K::SwitchLabel)
            .collect();
        for (i, label) in labels.iter().enumerate() {
            let colon = colons.get(i).map_or(end(*label), |t| t.span().end);
            let last = i + 1 == labels.len();
            let until = match statements.last() {
                Some(statement) if last => end(*statement),
                _ => colon,
            };
            self.node("CASE", start(*label), until, start(*label));
            self.case_labels(*label);
            if last {
                for statement in &statements {
                    self.statement(*statement);
                }
            }
            self.close();
        }
    }

    /// A label's elements, each a `..._CASE_LABEL`, then its guard's condition.
    fn case_labels(&mut self, label: SyntaxNode<'t>) {
        let mut guard = None;
        for item in label.children_with_tokens() {
            match item {
                NodeOrToken::Token(token) if token.kind() == K::DefaultKw => {
                    let span = token.span();
                    self.leaf("DEFAULT_CASE_LABEL", span.start, span.end, span.start);
                }
                NodeOrToken::Token(_) => {}
                NodeOrToken::Node(node) if node.kind() == K::Guard => guard = Some(node),
                NodeOrToken::Node(node) if is_pattern(node.kind()) => {
                    self.spanning("PATTERN_CASE_LABEL", node);
                    self.pattern(node);
                    self.close();
                }
                NodeOrToken::Node(node) => {
                    self.spanning("CONSTANT_CASE_LABEL", node);
                    self.expr(node);
                    self.close();
                }
            }
        }
        if let Some(condition) = guard.and_then(first_expr) {
            self.expr(condition);
        }
    }

    // ----- Patterns -----

    fn pattern(&mut self, pattern: SyntaxNode<'t>) {
        ferrion_stack::ensure_stack(|| match pattern.kind() {
            K::TypePattern => {
                self.spanning("BINDING_PATTERN", pattern);
                let at = name(pattern).map_or(start(pattern), |t| t.span().start);
                self.node("VARIABLE", start(pattern), end(pattern), at);
                self.modifiers(child(pattern, K::Modifiers), None, &[]);
                if let Some(ty) = first_type(pattern) {
                    self.local_type(ty);
                }
                self.close();
                self.close();
            }
            K::RecordPattern => {
                self.spanning("DECONSTRUCTION_PATTERN", pattern);
                for part in pattern.children() {
                    if is_pattern(part.kind()) {
                        self.pattern(part);
                    } else {
                        // `var(...)` nested in a record pattern is a `VAR_TYPE`; at the top, javac
                        // rejects it.
                        self.local_type(part);
                    }
                }
                self.close();
            }
            // javac's `ANY_PATTERN` for `_` is empty, at the end of the `_`.
            K::UnnamedPattern => {
                let at = end(pattern);
                self.leaf("ANY_PATTERN", at, at, at);
            }
            _ => self.erroneous(pattern),
        })
    }

    /// Tokens the parser couldn't place: javac's `ERRONEOUS`, a leaf. javac's tree is only
    /// compared for files it accepts, so these only appear where Ferrion reports an error and
    /// javac doesn't.
    fn erroneous(&mut self, node: SyntaxNode<'t>) {
        self.spanning("ERRONEOUS", node);
        self.close();
    }
}
