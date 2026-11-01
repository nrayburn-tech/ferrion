//! Making a summary from a tree: one declaration-only walk.
//!
//! Each declaration is read through its direct children (its modifiers, header, and member
//! list), and bodies are never entered: iterating a node's children jumps over each child's
//! subtree. Member types are handled from an explicit stack rather than by recursion, so ten
//! thousand nested classes don't overflow the stack.

use std::collections::HashMap;

use ferrion_syntax::ast::{self, AstChildren, AstNode};
use ferrion_syntax::{SyntaxKind, SyntaxToken, SyntaxTree};

use super::written::TypeRef;
use super::{
    Annotation, Dim, Directive, EnumConstant, Field, ImplicitMember, Import, ImportKind, Method,
    MethodKind, Modifiers, ModuleSummary, PackageDecl, Parameter, QualifiedName, RecordComponent,
    Summary, TypeKind, TypeParameter, TypeSummary, UnitKind,
};
use crate::Fingerprint;

/// The summary of `tree`. `is_package_info` says whether the file is a `package-info.java`,
/// which only its name tells.
///
/// Any tree gives a summary, whatever errors it has: a missing name is empty, a missing type is
/// [`TypeRef::missing`], and declarations the parser couldn't place in an `Error` node are left
/// out.
pub fn summarize(tree: &SyntaxTree, is_package_info: bool) -> Summary {
    let root = ast::CompilationUnit::cast(tree.root()).expect("the root is a compilation unit");
    let package = root.package_decl().map(|p| PackageDecl {
        name: p.name().map_or_else(QualifiedName::default, qualified),
        annotations: annotations(p.annotations()),
    });
    let imports: Vec<Import> = root.import_decls().filter_map(import).collect();
    let module = root.module_decl().map(module);
    let members: Vec<ast::Member<'_>> = root.members().collect();
    let compact = members
        .iter()
        .any(|m| matches!(m, ast::Member::FieldDecl(_) | ast::Member::MethodDecl(_)));
    let kind = if module.is_some() {
        UnitKind::ModuleInfo
    } else if is_package_info {
        UnitKind::PackageInfo
    } else if compact {
        UnitKind::Compact
    } else {
        UnitKind::Ordinary
    };

    let mut builder = Builder {
        lombok: LombokImports::new(&imports),
        types: Vec::new(),
        pending: Vec::new(),
    };
    if compact {
        // The compilation unit's members are the implicitly declared class's (JLS 7.3).
        builder
            .types
            .push(TypeSummary::new(TypeKind::Implicit, Box::default(), None));
        builder.add_members(0, members.into_iter());
    } else {
        for member in members.into_iter().rev() {
            if let ast::Member::TypeDecl(decl) = member {
                builder.pending.push((decl, None));
            }
        }
    }
    while let Some((decl, outer)) = builder.pending.pop() {
        builder.add_type(decl, outer);
    }
    Summary {
        kind,
        package,
        imports,
        types: builder.types,
        module,
    }
}

struct Builder<'t> {
    lombok: LombokImports,
    types: Vec<TypeSummary>,
    /// Member types still to add, the next last, with their enclosing type's index.
    pending: Vec<(ast::TypeDecl<'t>, Option<u32>)>,
}

/// A type declaration's parts, whatever its kind.
struct Header<'t> {
    kind: TypeKind,
    modifiers: Option<ast::Modifiers<'t>>,
    name: Option<SyntaxToken<'t>>,
    type_parameters: Option<ast::TypeParameters<'t>>,
    extends: Option<ast::ExtendsClause<'t>>,
    implements: Option<ast::ImplementsClause<'t>>,
    permits: Option<ast::PermitsClause<'t>>,
    components: Option<ast::RecordHeader<'t>>,
    constants: Option<ast::EnumBody<'t>>,
    members: Option<AstChildren<'t, ast::Member<'t>>>,
}

impl<'t> Header<'t> {
    fn of(decl: ast::TypeDecl<'t>) -> Header<'t> {
        let empty = Header {
            kind: TypeKind::Class,
            modifiers: None,
            name: None,
            type_parameters: None,
            extends: None,
            implements: None,
            permits: None,
            components: None,
            constants: None,
            members: None,
        };
        match decl {
            ast::TypeDecl::ClassDecl(d) => Header {
                kind: TypeKind::Class,
                modifiers: d.modifiers(),
                name: d.name(),
                type_parameters: d.type_parameters(),
                extends: d.extends_clause(),
                implements: d.implements_clause(),
                permits: d.permits_clause(),
                members: d.body().map(|b| b.members()),
                ..empty
            },
            ast::TypeDecl::InterfaceDecl(d) => Header {
                kind: TypeKind::Interface,
                modifiers: d.modifiers(),
                name: d.name(),
                type_parameters: d.type_parameters(),
                extends: d.extends_clause(),
                permits: d.permits_clause(),
                members: d.body().map(|b| b.members()),
                ..empty
            },
            ast::TypeDecl::EnumDecl(d) => Header {
                kind: TypeKind::Enum,
                modifiers: d.modifiers(),
                name: d.name(),
                implements: d.implements_clause(),
                constants: d.body(),
                members: d.body().map(|b| b.members()),
                ..empty
            },
            ast::TypeDecl::RecordDecl(d) => Header {
                kind: TypeKind::Record,
                modifiers: d.modifiers(),
                name: d.name(),
                type_parameters: d.type_parameters(),
                components: d.record_header(),
                implements: d.implements_clause(),
                members: d.body().map(|b| b.members()),
                ..empty
            },
            ast::TypeDecl::AnnotationTypeDecl(d) => Header {
                kind: TypeKind::Annotation,
                modifiers: d.modifiers(),
                name: d.name(),
                type_parameters: d.type_parameters(),
                extends: d.extends_clause(),
                permits: d.permits_clause(),
                members: d.body().map(|b| b.members()),
                ..empty
            },
        }
    }
}

impl<'t> Builder<'t> {
    /// Adds `decl` as a type, and queues its member types.
    fn add_type(&mut self, decl: ast::TypeDecl<'t>, outer: Option<u32>) {
        let header = Header::of(decl);
        let (type_modifiers, type_annotations) = modifiers(header.modifiers);
        let mut ty = TypeSummary::new(
            header.kind,
            header.name.map_or_else(Box::default, name_text),
            outer,
        );
        ty.generated = type_annotations
            .iter()
            .any(|a| a.name.last() == "Generated")
            || outer.is_some_and(|o| self.types[o as usize].generated);
        ty.lombok = type_annotations.iter().any(|a| self.lombok.names(&a.name));
        ty.modifiers = type_modifiers;
        ty.annotations = type_annotations;
        ty.type_parameters = type_parameters(header.type_parameters);
        ty.extends = types(header.extends.map(|c| c.types()));
        ty.implements = types(header.implements.map(|c| c.types()));
        ty.permits = types(header.permits.map(|c| c.types()));
        if let Some(components) = header.components {
            ty.record_components = components
                .components()
                .map(|c| RecordComponent {
                    annotations: modifiers(c.modifiers()).1,
                    ty: TypeRef::from_syntax(c.ty()),
                    varargs: c.varargs().map(|v| annotations(v.annotations())),
                    name: c.name().map_or_else(Box::default, name_text),
                })
                .collect();
        }
        if let Some(body) = header.constants {
            ty.enum_constants = body
                .constants()
                .map(|c| EnumConstant {
                    annotations: modifiers(c.modifiers()).1,
                    name: c.name().map_or_else(Box::default, name_text),
                    has_body: c.body().is_some(),
                })
                .collect();
        }
        let index = self.types.len();
        self.types.push(ty);
        if let Some(members) = header.members {
            self.add_members(index, members);
        }
    }

    /// Adds the fields and methods of `members` to the type at `index`, queues its member types
    /// in order, and adds the implicit members it then lacks.
    fn add_members(&mut self, index: usize, members: impl Iterator<Item = ast::Member<'t>>) {
        let kind = self.types[index].kind;
        let mut member_types = Vec::new();
        for member in members {
            match member {
                ast::Member::TypeDecl(decl) => member_types.push(decl),
                ast::Member::FieldDecl(decl) => {
                    let fields = fields(decl, kind);
                    let ty = &mut self.types[index];
                    ty.lombok |= fields
                        .iter()
                        .any(|f| f.annotations.iter().any(|a| self.lombok.names(&a.name)));
                    ty.fields.extend(fields);
                }
                ast::Member::MethodDecl(decl) => self.types[index].methods.push(method(decl)),
                ast::Member::ConstructorDecl(decl) => {
                    self.types[index].methods.push(constructor(decl))
                }
                ast::Member::CompactConstructorDecl(decl) => {
                    let (modifiers, annotations) = modifiers(decl.modifiers());
                    self.types[index].methods.push(Method {
                        kind: MethodKind::CompactConstructor,
                        modifiers,
                        annotations,
                        type_parameters: Box::default(),
                        result: None,
                        name: decl.name().map_or_else(Box::default, name_text),
                        parameters: Box::default(),
                        dims: Box::default(),
                        throws: Box::default(),
                        has_body: decl.body().is_some(),
                        default_value: None,
                    });
                }
                // Initializers are bodies.
                ast::Member::Initializer(_) => {}
            }
        }
        let ty = &mut self.types[index];
        ty.implicit = implicit_members(ty);
        let outer = Some(index as u32);
        self.pending
            .extend(member_types.into_iter().rev().map(|decl| (decl, outer)));
    }
}

impl TypeSummary {
    fn new(kind: TypeKind, name: Box<str>, outer: Option<u32>) -> TypeSummary {
        TypeSummary {
            kind,
            name,
            outer,
            modifiers: Modifiers::default(),
            annotations: Box::default(),
            type_parameters: Box::default(),
            extends: Box::default(),
            implements: Box::default(),
            permits: Box::default(),
            fields: Vec::new(),
            methods: Vec::new(),
            enum_constants: Vec::new(),
            record_components: Vec::new(),
            implicit: Vec::new(),
            generated: false,
            lombok: false,
        }
    }
}

/// The members JLS 8.8.9, 8.9, and 8.10.3 declare for `ty` that it doesn't declare itself.
fn implicit_members(ty: &TypeSummary) -> Vec<ImplicitMember> {
    let has_constructor = ty.methods.iter().any(|m| m.kind != MethodKind::Method);
    let mut implicit = Vec::new();
    match ty.kind {
        TypeKind::Class | TypeKind::Implicit if !has_constructor => {
            implicit.push(ImplicitMember::DefaultConstructor);
        }
        TypeKind::Enum => {
            if !has_constructor {
                implicit.push(ImplicitMember::DefaultConstructor);
            }
            // Declaring either is an error (8.9.3), so they are always there.
            implicit.push(ImplicitMember::EnumValues);
            implicit.push(ImplicitMember::EnumValueOf);
        }
        TypeKind::Record => {
            // The canonical constructor's parameters have the components' types (8.10.4). Types
            // as written are compared by shape, which is all that's known before resolution.
            let shapes: Vec<TypeRef> = ty
                .record_components
                .iter()
                .map(|c| c.ty.shape(usize::from(c.varargs.is_some())))
                .collect();
            let canonical = ty.methods.iter().any(|m| match m.kind {
                MethodKind::CompactConstructor => true,
                MethodKind::Constructor => {
                    m.parameters.len() == shapes.len()
                        && m.parameters.iter().zip(&shapes).all(|(p, shape)| {
                            p.ty.shape(p.dims.len() + usize::from(p.varargs.is_some())) == *shape
                        })
                }
                MethodKind::Method => false,
            });
            if !canonical {
                implicit.push(ImplicitMember::CanonicalConstructor);
            }
            for (i, component) in ty.record_components.iter().enumerate() {
                let declared = ty.methods.iter().any(|m| {
                    m.kind == MethodKind::Method
                        && m.parameters.is_empty()
                        && m.name == component.name
                });
                if !declared {
                    implicit.push(ImplicitMember::Accessor(i as u32));
                }
            }
            for i in 0..ty.record_components.len() {
                implicit.push(ImplicitMember::ComponentField(i as u32));
            }
        }
        _ => {}
    }
    implicit
}

fn fields(decl: ast::FieldDecl<'_>, owner: TypeKind) -> Vec<Field> {
    let (modifiers, annotations) = modifiers(decl.modifiers());
    let ty = TypeRef::from_syntax(decl.ty());
    // An interface's fields are implicitly final (JLS 9.3).
    let is_final = modifiers.contains(Modifiers::FINAL)
        || matches!(owner, TypeKind::Interface | TypeKind::Annotation);
    decl.declarators()
        .map(|d| Field {
            modifiers,
            annotations: annotations.clone(),
            ty: ty.clone(),
            name: d.name().map_or_else(Box::default, name_text),
            dims: d.dims().map(Dim::from_syntax).collect(),
            constant: d
                .initializer()
                .filter(|_| is_final)
                .map(|i| Fingerprint::of_tokens(i.syntax())),
        })
        .collect()
}

fn method(decl: ast::MethodDecl<'_>) -> Method {
    let (modifiers, mut annotations) = modifiers(decl.modifiers());
    if decl.annotations().next().is_some() {
        let mut all = annotations.into_vec();
        all.extend(decl.annotations().map(annotation));
        annotations = all.into_boxed_slice();
    }
    Method {
        kind: MethodKind::Method,
        modifiers,
        annotations,
        type_parameters: type_parameters(decl.type_parameters()),
        result: Some(TypeRef::from_syntax(decl.result())),
        name: decl.name().map_or_else(Box::default, name_text),
        parameters: parameters(decl.formal_parameters()),
        dims: decl.dims().map(Dim::from_syntax).collect(),
        throws: types(decl.throws_clause().map(|c| c.types())),
        has_body: decl.body().is_some(),
        default_value: decl
            .default_value()
            .map(|d| Fingerprint::of_tokens(d.syntax())),
    }
}

fn constructor(decl: ast::ConstructorDecl<'_>) -> Method {
    let (modifiers, mut annotations) = modifiers(decl.modifiers());
    if decl.annotations().next().is_some() {
        let mut all = annotations.into_vec();
        all.extend(decl.annotations().map(annotation));
        annotations = all.into_boxed_slice();
    }
    Method {
        kind: MethodKind::Constructor,
        modifiers,
        annotations,
        type_parameters: type_parameters(decl.type_parameters()),
        result: None,
        name: decl.name().map_or_else(Box::default, name_text),
        parameters: parameters(decl.formal_parameters()),
        dims: Box::default(),
        throws: types(decl.throws_clause().map(|c| c.types())),
        has_body: decl.body().is_some(),
        default_value: None,
    }
}

fn parameters(params: Option<ast::FormalParameters<'_>>) -> Box<[Parameter]> {
    let Some(params) = params else {
        return Box::default();
    };
    params
        .parameters()
        .map(|p| {
            let (modifiers, annotations) = modifiers(p.modifiers());
            Parameter {
                modifiers,
                annotations,
                ty: TypeRef::from_syntax(p.ty()),
                varargs: p.varargs().map(|v| self::annotations(v.annotations())),
                name: p.name().map_or_else(Box::default, name_text),
                dims: p.dims().map(Dim::from_syntax).collect(),
            }
        })
        .collect()
}

fn type_parameters(params: Option<ast::TypeParameters<'_>>) -> Box<[TypeParameter]> {
    let Some(params) = params else {
        return Box::default();
    };
    params
        .parameters()
        .map(|p| TypeParameter {
            annotations: annotations(p.annotations()),
            name: p.name().map_or_else(Box::default, name_text),
            bounds: p.bounds().map(|b| TypeRef::from_syntax(Some(b))).collect(),
        })
        .collect()
}

fn types(types: Option<AstChildren<'_, ast::Type<'_>>>) -> Box<[TypeRef]> {
    types.map_or_else(Box::default, |types| {
        types.map(|t| TypeRef::from_syntax(Some(t))).collect()
    })
}

fn modifiers(node: Option<ast::Modifiers<'_>>) -> (Modifiers, Box<[Annotation]>) {
    let Some(node) = node else {
        return (Modifiers::default(), Box::default());
    };
    let mut modifiers = Modifiers::default();
    for keyword in node.keywords() {
        modifiers.insert(match keyword.kind() {
            SyntaxKind::PublicKw => Modifiers::PUBLIC,
            SyntaxKind::ProtectedKw => Modifiers::PROTECTED,
            SyntaxKind::PrivateKw => Modifiers::PRIVATE,
            SyntaxKind::AbstractKw => Modifiers::ABSTRACT,
            SyntaxKind::StaticKw => Modifiers::STATIC,
            SyntaxKind::FinalKw => Modifiers::FINAL,
            SyntaxKind::TransientKw => Modifiers::TRANSIENT,
            SyntaxKind::VolatileKw => Modifiers::VOLATILE,
            SyntaxKind::SynchronizedKw => Modifiers::SYNCHRONIZED,
            SyntaxKind::NativeKw => Modifiers::NATIVE,
            SyntaxKind::StrictfpKw => Modifiers::STRICTFP,
            SyntaxKind::DefaultKw => Modifiers::DEFAULT,
            SyntaxKind::SealedKw => Modifiers::SEALED,
            // `non-sealed` is three tokens, each of this kind.
            SyntaxKind::NonSealedKw => Modifiers::NON_SEALED,
            _ => continue,
        });
    }
    (modifiers, annotations(node.annotations()))
}

pub(super) fn annotations(nodes: AstChildren<'_, ast::Annotation<'_>>) -> Box<[Annotation]> {
    nodes.map(annotation).collect()
}

fn annotation(node: ast::Annotation<'_>) -> Annotation {
    Annotation {
        name: node.name().map_or_else(QualifiedName::default, qualified),
        arguments: node
            .annotation_arguments()
            .map(|a| Fingerprint::of_tokens(a.syntax())),
    }
}

impl Dim {
    pub(super) fn from_syntax(dim: ast::Dim<'_>) -> Dim {
        Dim {
            annotations: annotations(dim.annotations()),
        }
    }
}

fn import(node: ast::ImportDecl<'_>) -> Option<Import> {
    let name = qualified(node.name()?);
    let kind = match (
        node.static_kw().is_some(),
        node.module_kw().is_some(),
        node.star().is_some(),
    ) {
        (_, true, _) => ImportKind::Module,
        (false, false, false) => ImportKind::SingleType,
        (false, false, true) => ImportKind::OnDemand,
        (true, false, false) => ImportKind::SingleStatic,
        (true, false, true) => ImportKind::StaticOnDemand,
    };
    Some(Import { kind, name })
}

fn module(node: ast::ModuleDecl<'_>) -> ModuleSummary {
    let name = |n: Option<ast::Name<'_>>| n.map_or_else(QualifiedName::default, qualified);
    let directives = node
        .directives()
        .map(|d| match d {
            ast::ModuleDirective::RequiresDirective(d) => Directive::Requires {
                transitive: d.modifiers().any(|m| m.kind() == SyntaxKind::TransitiveKw),
                is_static: d.modifiers().any(|m| m.kind() == SyntaxKind::StaticKw),
                module: name(d.name()),
            },
            ast::ModuleDirective::ExportsDirective(d) => Directive::Exports {
                package: name(d.package_name()),
                to: d.modules().map(qualified).collect(),
            },
            ast::ModuleDirective::OpensDirective(d) => Directive::Opens {
                package: name(d.package_name()),
                to: d.modules().map(qualified).collect(),
            },
            ast::ModuleDirective::UsesDirective(d) => Directive::Uses {
                service: name(d.service()),
            },
            ast::ModuleDirective::ProvidesDirective(d) => Directive::Provides {
                service: name(d.service()),
                with: d.implementations().map(qualified).collect(),
            },
        })
        .collect();
    ModuleSummary {
        annotations: annotations(node.annotations()),
        open: node.open_kw().is_some(),
        name: name(node.name()),
        directives,
    }
}

/// A name's identifiers and dots, without trivia, with escapes translated. A loop over its
/// tokens, though the `Name` nodes nest a level per segment.
fn qualified(name: ast::Name<'_>) -> QualifiedName {
    let tree = name.syntax().tree();
    let mut text = String::new();
    for token in name.syntax().tokens() {
        if !token.is_trivia() {
            let lexed = tree.lexer_tokens()[token.id().index()];
            text.push_str(&ferrion_lexer::cooked_text(tree.text(), lexed));
        }
    }
    QualifiedName::new(text)
}

/// An identifier's name: its text with escapes translated.
pub(super) fn name_text(token: SyntaxToken<'_>) -> Box<str> {
    let tree = token.tree();
    let lexed = tree.lexer_tokens()[token.id().index()];
    ferrion_lexer::cooked_text(tree.text(), lexed).into()
}

/// Recognizes Lombok's annotations without resolving them, from how the file names them: a
/// name qualified by `lombok.`, or one whose first segment a single-type import brings in from
/// a `lombok` package (`@Data` with `import lombok.Data;`, `@Builder.Default` with `import
/// lombok.Builder;`), or, when the file imports a `lombok` package on demand, any name no
/// single-type import covers except `java.lang`'s annotations. Resolution (B2's views) can
/// replace this with the annotation's real origin.
struct LombokImports {
    /// Each single-type import's simple name, and whether it is from Lombok.
    single: HashMap<Box<str>, bool>,
    on_demand: bool,
}

impl LombokImports {
    fn new(imports: &[Import]) -> LombokImports {
        let from_lombok = |name: &str| name == "lombok" || name.starts_with("lombok.");
        let mut single = HashMap::new();
        let mut on_demand = false;
        for import in imports {
            match import.kind {
                ImportKind::SingleType => {
                    single.insert(import.name.last().into(), from_lombok(import.name.as_str()));
                }
                ImportKind::OnDemand => on_demand |= from_lombok(import.name.as_str()),
                _ => {}
            }
        }
        LombokImports { single, on_demand }
    }

    fn names(&self, name: &QualifiedName) -> bool {
        const JAVA_LANG: &[&str] = &[
            "Deprecated",
            "FunctionalInterface",
            "Override",
            "SafeVarargs",
            "SuppressWarnings",
        ];
        if name.as_str().starts_with("lombok.") {
            return true;
        }
        let first = name.segments().next().unwrap_or_default();
        match self.single.get(first) {
            Some(&from_lombok) => from_lombok,
            None => self.on_demand && !name.as_str().contains('.') && !JAVA_LANG.contains(&first),
        }
    }
}
