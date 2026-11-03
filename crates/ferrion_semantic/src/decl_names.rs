//! Names in declarations: every name outside bodies resolved, keyed by `NodeId` of the file's
//! current tree, so that the javac comparison can start on declarations before bodies are
//! resolved (docs/design/semantic-names.md#names-in-declarations).
//!
//! The names are those of the package declaration, imports, the module declaration, annotations
//! on declarations (and the element names of their arguments), type parameter bounds, `extends`,
//! `implements`, and `permits` clauses, field and record component types, method and
//! constructor signatures, and `throws` clauses. Each `Name` node and each `ClassType` node is
//! one name, so a qualified name gives each of its prefixes: `java.util.List` is the package
//! `java`, the package `java.util`, and the type. Names inside bodies, annotation argument
//! expressions, and enum constants' arguments and bodies are W5's.
//!
//! The walk reads each declaration's direct children and never enters a body, with member types
//! from an explicit stack, as the summary's does; names within one type or annotation are found
//! by a loop over its nodes.

use std::sync::Arc;

use ferrion_classfile::ClassName;
use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeId, SyntaxKind, SyntaxNode};

use crate::classes::ClassSource;
use crate::engine::derived::Reads;
use crate::resolve::{FileScope, Resolver, Scope};
use crate::summary::{Header, ImportKind, QualifiedName, UnitKind, qualified};
use crate::symbols::{
    Cause, ClassSourceId, ErrorSymbol, LeftToB3, MethodRef, SymbolKey, Ty, TyPart, TypeKey,
    TypeResolution, TypeVarOwner,
};
use crate::{Cancelled, FileId, Interner, Name, Snapshot, SourceSetId};

/// A file's names in declarations, in the order of their nodes.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct FileNames {
    pub names: Box<[ResolvedName]>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ResolvedName {
    /// The `Name`, `ClassType`, or `ElementValuePair` node.
    pub node: NodeId,
    /// The name's byte range in the file: the node's, or an element-value pair's name token.
    pub start: u32,
    pub end: u32,
    pub symbol: Symbol,
}

/// What a name is, in a declaration or a body.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Symbol {
    Package(Name),
    Module(Name),
    Type(TypeKey),
    TypeVar(TypeVarOwner, u32),
    /// A single-static import's member that isn't a member type: the fields and methods of the
    /// name, which a use chooses among.
    StaticMember {
        owner: TypeKey,
        name: Name,
    },
    /// An annotation argument's element: the annotation interface's method.
    Element {
        owner: TypeKey,
        name: Name,
    },
    /// A local variable, parameter, exception parameter, resource, or pattern binding of
    /// `file`: the node declaring it, and the offset of its name, valid for one tree.
    Local {
        file: FileId,
        node: NodeId,
        at: u32,
    },
    /// A field or enum constant, by the type declaring it.
    Field {
        owner: TypeKey,
        name: Name,
    },
    /// A method name: the type whose members are searched (JLS 15.12.1), and its member
    /// methods of the name, declared or inherited, among which B3 chooses.
    Methods {
        search: TypeKey,
        name: Name,
        candidates: Arc<[MethodRef]>,
    },
    /// A name whose meaning needs types, which B3 attributes.
    LeftToB3(LeftToB3),
    Error(ErrorSymbol),
}

impl From<TypeResolution> for Symbol {
    fn from(resolution: TypeResolution) -> Symbol {
        match resolution {
            TypeResolution::Type(key) => Symbol::Type(key),
            TypeResolution::TypeVar(owner, index) => Symbol::TypeVar(owner, index),
            TypeResolution::Package(name) => Symbol::Package(name),
            TypeResolution::Error(error) => Symbol::Error(error),
        }
    }
}

impl FileNames {
    /// The name at `node`, if it is one.
    pub fn get(&self, node: NodeId) -> Option<&ResolvedName> {
        let index = self.names.binary_search_by_key(&node, |n| n.node).ok()?;
        Some(&self.names[index])
    }
}

impl Snapshot {
    /// The names in `file`'s declarations, by node of its current tree ([`Snapshot::tree`]).
    pub fn decl_names(&self, file: FileId) -> Result<Arc<FileNames>, Cancelled> {
        self.check()?;
        Ok(self
            .shared()
            .file_names
            .get(self, &file, |reads| compute(reads, file))?
            .value
            .clone())
    }

    /// The key the javac comparison matches `symbol` by (semantic.md, "The
    /// javac oracle"): `package java.util`, `module java.base`, `type java/util/Map$Entry class
    /// <entry>` or `... source <file>`, `type local <file>:<offset>`, `typevar <owner> T`,
    /// `member p/A.m`, `field p/A.f`, `method p/A.value()I`, `local <file>:<offset>`, or
    /// `error <cause> <name>`. A method name with several candidates is `methods p/A.m:` and
    /// their keys; one left to B3 is `b3 <why>`.
    pub fn symbol_key(&self, symbol: &Symbol) -> Result<SymbolKey, Cancelled> {
        let names = self.names().clone();
        Ok(SymbolKey(match symbol {
            Symbol::Package(p) => format!("package {}", names.text(*p)),
            Symbol::Module(m) => format!("module {}", names.text(*m)),
            Symbol::Type(key) => self.type_key_text(*key)?,
            Symbol::TypeVar(owner, index) => {
                let (owner_key, name) = match owner {
                    TypeVarOwner::Type(key) => {
                        let name = self.type_param_name(*key, None, *index)?;
                        (self.type_key_text(*key)?, name)
                    }
                    TypeVarOwner::Method(key, method) => {
                        let name = self.type_param_name(*key, Some(*method), *index)?;
                        let method_name = self.method_text(*key, *method)?;
                        (method_name, name)
                    }
                };
                format!("typevar {owner_key} {}", names.text(name))
            }
            Symbol::StaticMember { owner, name } => {
                format!("member {}.{}", self.binary_text(*owner)?, names.text(*name))
            }
            Symbol::Element { owner, name } => {
                let descriptor = self.element_descriptor(*owner, *name)?;
                format!(
                    "method {}.{}{descriptor}",
                    self.binary_text(*owner)?,
                    names.text(*name)
                )
            }
            Symbol::Local { file, at, .. } => {
                format!("local {}:{at}", self.file(*file)?.path.display())
            }
            Symbol::Field { owner, name } => {
                format!("field {}.{}", self.binary_text(*owner)?, names.text(*name))
            }
            Symbol::Methods {
                search,
                name,
                candidates,
            } => match &candidates[..] {
                [one] => self.method_text(one.owner, one.index)?,
                _ => {
                    let mut keys = Vec::with_capacity(candidates.len());
                    for candidate in candidates.iter() {
                        keys.push(self.method_text(candidate.owner, candidate.index)?);
                    }
                    format!(
                        "methods {}.{}: {}",
                        self.binary_text(*search)?,
                        names.text(*name),
                        keys.join(" | ")
                    )
                }
            },
            Symbol::LeftToB3(why) => format!("b3 {}", why.describe()),
            Symbol::Error(error) => {
                format!(
                    "error {} {}",
                    error.cause.describe(&names),
                    names.text(error.name)
                )
            }
        }))
    }

    /// Where a local or anonymous class is declared, as its key names it: `local <file>:<offset>`.
    fn local_text(&self, key: TypeKey) -> Result<String, Cancelled> {
        let TypeKey::Local { file, .. } = key else {
            return Ok(String::new());
        };
        let at = self.local_type(key)?.map_or(u32::MAX, |t| t.at);
        Ok(format!("local {}:{at}", self.file(file)?.path.display()))
    }

    /// A type's binary name in internal form: `java/util/Map$Entry`. A local or anonymous
    /// class's, whose binary name has digits javac chooses, is where it is declared:
    /// `local <file>:<offset>`.
    pub fn binary_text(&self, key: TypeKey) -> Result<String, Cancelled> {
        let names = self.names();
        Ok(match key {
            TypeKey::Local { .. } => self.local_text(key)?,
            TypeKey::Source { package, path, .. } => {
                let path = names.path_text(path, "$");
                if package.is_empty() {
                    path
                } else {
                    format!("{}/{path}", names.text(package).replace('.', "/"))
                }
            }
            TypeKey::Compact { file, path, .. } => {
                let stem = self
                    .file(file)?
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match path {
                    None => stem,
                    Some(path) => format!("{stem}${}", names.path_text(path, "$")),
                }
            }
            TypeKey::Class { binary, .. } => names.text(binary).to_string(),
        })
    }

    fn type_key_text(&self, key: TypeKey) -> Result<String, Cancelled> {
        let binary = self.binary_text(key)?;
        if let TypeKey::Local { .. } = key {
            return Ok(format!("type {binary}"));
        }
        let origin = match key {
            TypeKey::Source { .. } | TypeKey::Compact { .. } => {
                let mut reads = Reads::untracked(self);
                match reads.locate(key)? {
                    Some(located) => {
                        format!("source {}", self.file(located.file)?.path.display())
                    }
                    None => "source ?".to_owned(),
                }
            }
            TypeKey::Class { source, .. } => self.class_source_text(source, &binary),
            TypeKey::Local { .. } => unreachable!("handled above"),
        };
        Ok(format!("type {binary} {origin}"))
    }

    /// Where the class `binary` of `source` came from: a classpath entry, or the platform, or
    /// the entry a `--patch-module` added it to the platform from, which javac names as the
    /// class's origin.
    fn class_source_text(&self, source: ClassSourceId, binary: &str) -> String {
        match self.shared().classes.source(source) {
            ClassSource::Platform(platform) => match platform.patched_from(&ClassName::new(binary)) {
                Some(path) => format!("class {}", path.display()),
                None => "class platform".to_owned(),
            },
            ClassSource::Entry(entry) => format!("class {}", entry.path.display()),
        }
    }

    fn type_param_name(
        &self,
        key: TypeKey,
        method: Option<u32>,
        index: u32,
    ) -> Result<Name, Cancelled> {
        if let Some(entry) = self.type_entry(key)? {
            let params = match method {
                None => &entry.type_parameters,
                Some(m) => match entry.methods.get(m as usize) {
                    Some(m) => &m.type_parameters,
                    None => return Ok(Name::EMPTY),
                },
            };
            return Ok(params.get(index as usize).map_or(Name::EMPTY, |p| p.name));
        }
        let Some(entry) = self.class_entry(key) else {
            return Ok(Name::EMPTY);
        };
        let params = match method {
            None => &entry.type_parameters,
            Some(m) => match entry.methods.get(m as usize) {
                Some(m) => &m.type_parameters,
                None => return Ok(Name::EMPTY),
            },
        };
        Ok(params.get(index as usize).map_or(Name::EMPTY, |p| p.name))
    }

    /// `method p/A.m(I)V` for a method of a type's entry.
    fn method_text(&self, key: TypeKey, method: u32) -> Result<String, Cancelled> {
        let names = self.names().clone();
        let owner = self.binary_text(key)?;
        if let Some(entry) = self.type_entry(key)? {
            let Some(m) = entry.methods.get(method as usize) else {
                return Ok(format!("method {owner}.?"));
            };
            let mut descriptor = String::from("(");
            for parameter in m.parameters.iter() {
                descriptor.push_str(&self.erased_descriptor(parameter)?);
            }
            descriptor.push(')');
            match &m.result {
                Some(result) => descriptor.push_str(&self.erased_descriptor(result)?),
                None => descriptor.push('V'),
            }
            let name = if m.result.is_none() {
                "<init>".to_owned()
            } else {
                names.text(m.name).to_string()
            };
            return Ok(format!("method {owner}.{name}{descriptor}"));
        }
        let entry = self.class_entry(key);
        Ok(
            match entry.as_ref().and_then(|e| e.methods.get(method as usize)) {
                Some(m) => format!(
                    "method {owner}.{}{}",
                    names.text(m.name),
                    names.text(m.descriptor)
                ),
                None => format!("method {owner}.?"),
            },
        )
    }

    /// The descriptor of the annotation element `name` of `owner`: `()I`.
    fn element_descriptor(&self, owner: TypeKey, name: Name) -> Result<String, Cancelled> {
        if let Some(entry) = self.type_entry(owner)? {
            if let Some(m) = entry
                .methods
                .iter()
                .find(|m| m.name == name && m.parameters.is_empty())
                && let Some(result) = &m.result
            {
                return Ok(format!("(){}", self.erased_descriptor(result)?));
            }
            return Ok("()?".to_owned());
        }
        Ok(self
            .class_entry(owner)
            .and_then(|e| {
                e.methods
                    .iter()
                    .find(|m| m.name == name)
                    .map(|m| self.names().text(m.descriptor).to_string())
            })
            .unwrap_or_else(|| "()?".to_owned()))
    }

    /// The descriptor of a resolved type's erasure: `Ljava/lang/String;`, `[I`. A type variable
    /// is erased to its first bound's erasure, following bounds that are type variables
    /// themselves (`<X extends T>` with `T extends Enum<T>` is `Ljava/lang/Enum;`, as javac
    /// erases it) up to [`MAX_BOUNDS`] deep, so that bounds naming each other can't loop.
    pub fn erased_descriptor(&self, ty: &Ty) -> Result<String, Cancelled> {
        self.erasure(ty, 0)
    }

    fn erasure(&self, ty: &Ty, depth: u32) -> Result<String, Cancelled> {
        let mut out = String::new();
        let mut parts: &[TyPart] = ty.parts();
        while let Some(TyPart::Array { dims }) = parts.first() {
            for _ in 0..*dims {
                out.push('[');
            }
            parts = &parts[1..];
        }
        match parts.first() {
            Some(TyPart::Primitive(p)) => out.push(p.descriptor()),
            Some(TyPart::Void) => out.push('V'),
            Some(TyPart::Class { ty, .. }) => {
                out.push_str(&format!("L{};", self.binary_text(*ty)?))
            }
            Some(TyPart::Binary { binary, .. }) => {
                out.push_str(&format!("L{};", self.names().text(*binary)))
            }
            Some(TyPart::TypeVar { owner, index }) if depth < MAX_BOUNDS => {
                match self.first_bound(*owner, *index)? {
                    Some(bound) => out.push_str(&self.erasure(&bound, depth + 1)?),
                    None => out.push_str("Ljava/lang/Object;"),
                }
            }
            _ => out.push_str("Ljava/lang/Object;"),
        }
        Ok(out)
    }

    fn first_bound(&self, owner: TypeVarOwner, index: u32) -> Result<Option<Ty>, Cancelled> {
        let (key, method) = match owner {
            TypeVarOwner::Type(key) => (key, None),
            TypeVarOwner::Method(key, m) => (key, Some(m)),
        };
        if let Some(entry) = self.type_entry(key)? {
            let params = match method {
                None => &entry.type_parameters,
                Some(m) => match entry.methods.get(m as usize) {
                    Some(m) => &m.type_parameters,
                    None => return Ok(None),
                },
            };
            return Ok(params
                .get(index as usize)
                .and_then(|p| p.bounds.first().cloned()));
        }
        Ok(self.class_entry(key).and_then(|entry| {
            let params = match method {
                None => entry.type_parameters.clone(),
                Some(m) => entry.methods.get(m as usize)?.type_parameters.clone(),
            };
            params.get(index as usize)?.bounds.first().cloned()
        }))
    }
}

/// How many type variables' bounds erasure follows, one through the next.
const MAX_BOUNDS: u32 = 64;

struct Walker<'r, 's> {
    r: Resolver<'r, 's>,
    names: Arc<Interner>,
    set: SourceSetId,
    out: Vec<ResolvedName>,
    /// The summary index the next type declaration has: the walk takes them in the summary's
    /// order.
    next_index: usize,
}

/// A type declaration still to walk, with the index of the type it is a member of.
struct Pending<'t> {
    decl: ast::TypeDecl<'t>,
    outer: Option<usize>,
}

fn compute(reads: &mut Reads<'_>, file: FileId) -> Result<FileNames, Cancelled> {
    let (tree, _) = reads.tree(file)?;
    let set = reads.snapshot.file(file)?.set;
    let file_scope = reads.file_scope(file)?;
    let names = reads.snapshot.names().clone();
    let root = ast::CompilationUnit::cast(tree.root()).expect("the root is a compilation unit");
    let compact = file_scope.summary.kind == UnitKind::Compact;
    let mut w = Walker {
        r: Resolver::new(reads, set),
        names: names.clone(),
        set,
        out: Vec::new(),
        next_index: 0,
    };
    let top = Scope::top(&file_scope);
    if let Some(package) = root.package_decl() {
        for annotation in package.annotations() {
            w.annotation(&top, annotation)?;
        }
        if let Some(name) = package.name() {
            w.packages(name)?;
        }
    }
    for import in root.import_decls() {
        w.import(import)?;
    }
    if let Some(module) = root.module_decl() {
        w.module(&top, module)?;
    }
    let mut pending: Vec<Pending<'_>> = Vec::new();
    if compact {
        // The implicitly declared class is the summary's first type.
        w.next_index = 1;
        let members: Vec<ast::Member<'_>> = root.members().collect();
        w.body(&file_scope, 0, members, &mut pending)?;
    } else {
        for member in root.members().collect::<Vec<_>>().into_iter().rev() {
            if let ast::Member::TypeDecl(decl) = member {
                pending.push(Pending { decl, outer: None });
            }
        }
    }
    while let Some(next) = pending.pop() {
        w.type_decl(&file_scope, next, &mut pending)?;
    }
    let mut names_out = w.out;
    names_out.sort_by_key(|n| n.node);
    names_out.dedup_by_key(|n| n.node);
    Ok(FileNames {
        names: names_out.into_boxed_slice(),
    })
}

impl<'t> Walker<'_, '_> {
    fn record(&mut self, node: SyntaxNode<'_>, symbol: Symbol) {
        let span = node.span();
        self.out.push(ResolvedName {
            node: node.id(),
            start: span.start,
            end: span.end,
            symbol,
        });
    }

    /// The `Name` nodes of a dotted name, first segment first, with each segment's name.
    fn name_chain<'n>(&self, name: ast::Name<'n>) -> Vec<(SyntaxNode<'n>, Name)> {
        let mut chain = Vec::new();
        let mut next = Some(name);
        while let Some(n) = next {
            let segment = n
                .name()
                .map_or(Name::EMPTY, |t| crate::summary::name_text(t, &self.names));
            chain.push((n.syntax(), segment));
            next = n.qualifier();
        }
        chain.reverse();
        chain
    }

    /// A package declaration's name: every prefix is a package.
    fn packages(&mut self, name: ast::Name<'_>) -> Result<(), Cancelled> {
        let chain = self.name_chain(name);
        let mut package = Name::EMPTY;
        for (i, (node, segment)) in chain.iter().enumerate() {
            let symbol = if i < crate::resolve::MAX_SEGMENTS {
                package = if i == 0 {
                    *segment
                } else {
                    self.names.intern(&format!(
                        "{}.{}",
                        self.names.text(package),
                        self.names.text(*segment)
                    ))
                };
                Symbol::Package(package)
            } else {
                Symbol::Error(ErrorSymbol {
                    name: qualified(name, &self.names).name(),
                    cause: Cause::TooLong,
                })
            };
            self.record(*node, symbol);
        }
        Ok(())
    }

    fn import(&mut self, import: ast::ImportDecl<'_>) -> Result<(), Cancelled> {
        let Some(name) = import.name() else {
            return Ok(());
        };
        let full = qualified(name, &self.names);
        let chain = self.name_chain(name);
        let kind = match (
            import.static_kw().is_some(),
            import.module_kw().is_some(),
            import.star().is_some(),
        ) {
            (_, true, _) => ImportKind::Module,
            (false, false, false) => ImportKind::SingleType,
            (false, false, true) => ImportKind::OnDemand,
            (true, false, false) => ImportKind::SingleStatic,
            (true, false, true) => ImportKind::StaticOnDemand,
        };
        if kind == ImportKind::Module {
            let symbol = match self.r.module_import(full.name())? {
                None => Symbol::Module(full.name()),
                Some(cause) => Symbol::Error(ErrorSymbol {
                    name: full.name(),
                    cause,
                }),
            };
            self.record(name.syntax(), symbol);
            return Ok(());
        }
        let (resolved_name, member) = match kind {
            ImportKind::SingleStatic if chain.len() > 1 => {
                let text = self.names.text(full.name());
                let owner = text.rsplit_once('.').map_or("", |(o, _)| o);
                (
                    QualifiedName(self.names.intern(owner)),
                    chain.last().copied(),
                )
            }
            _ => (full, None),
        };
        let mut resolved = self.r.canonical(resolved_name)?;
        // The last segment of a single-type or static import must be a type.
        let needs_type = matches!(
            kind,
            ImportKind::SingleType | ImportKind::SingleStatic | ImportKind::StaticOnDemand
        );
        if let Some(TypeResolution::Package(_)) = resolved.last()
            && (needs_type || kind == ImportKind::OnDemand)
        {
            let n = resolved.len();
            let package = if n >= 2 {
                match &resolved[n - 2] {
                    TypeResolution::Package(p) => *p,
                    _ => Name::EMPTY,
                }
            } else {
                Name::EMPTY
            };
            if needs_type {
                resolved[n - 1] = self.r.missing_in_package(resolved_name.name(), package)?;
            } else if let TypeResolution::Package(p) = resolved[n - 1] {
                if self.r.reads.platform_package_visible(self.set, p)? == Some(false) {
                    resolved[n - 1] = TypeResolution::Error(ErrorSymbol {
                        name: p,
                        cause: Cause::NotVisible { package: p },
                    });
                } else if !self.r.reads.package_exists(self.set, p)? {
                    resolved[n - 1] = TypeResolution::Error(ErrorSymbol {
                        name: p,
                        cause: Cause::NoPackage { package: p },
                    });
                }
            }
        }
        for ((node, _), resolution) in chain.iter().zip(resolved.iter()) {
            self.record(*node, resolution.clone().into());
        }
        if let Some((node, simple)) = member {
            let symbol = match resolved.last() {
                Some(TypeResolution::Type(owner)) => {
                    let members = self.r.reads.member_types(self.set, *owner)?;
                    match members.as_ref().and_then(|m| m.get(simple)) {
                        Some((crate::symbols::MemberType::One(key), _)) => Symbol::Type(*key),
                        Some((crate::symbols::MemberType::Ambiguous(keys), _)) => {
                            Symbol::Error(ErrorSymbol {
                                name: simple,
                                cause: Cause::Ambiguous(keys.clone()),
                            })
                        }
                        None => Symbol::StaticMember {
                            owner: *owner,
                            name: simple,
                        },
                    }
                }
                Some(TypeResolution::Error(error)) => Symbol::Error(error.clone()),
                _ => Symbol::Error(ErrorSymbol {
                    name: full.name(),
                    cause: Cause::NotFound,
                }),
            };
            self.record(node, symbol);
        }
        Ok(())
    }

    fn module(&mut self, scope: &Scope<'_>, module: ast::ModuleDecl<'_>) -> Result<(), Cancelled> {
        for annotation in module.annotations() {
            self.annotation(scope, annotation)?;
        }
        if let Some(name) = module.name() {
            let name_text = qualified(name, &self.names).name();
            self.record(name.syntax(), Symbol::Module(name_text));
        }
        let module_name = |w: &mut Self, name: ast::Name<'_>| {
            let text = qualified(name, &w.names).name();
            w.record(name.syntax(), Symbol::Module(text));
        };
        for directive in module.directives() {
            match directive {
                ast::ModuleDirective::RequiresDirective(d) => {
                    if let Some(name) = d.name() {
                        module_name(self, name);
                    }
                }
                ast::ModuleDirective::ExportsDirective(d) => {
                    if let Some(name) = d.package_name() {
                        self.packages(name)?;
                    }
                    for name in d.modules() {
                        module_name(self, name);
                    }
                }
                ast::ModuleDirective::OpensDirective(d) => {
                    if let Some(name) = d.package_name() {
                        self.packages(name)?;
                    }
                    for name in d.modules() {
                        module_name(self, name);
                    }
                }
                ast::ModuleDirective::UsesDirective(d) => {
                    if let Some(name) = d.service() {
                        self.type_name(scope, name)?;
                    }
                }
                ast::ModuleDirective::ProvidesDirective(d) => {
                    for name in d.service().into_iter().chain(d.implementations()) {
                        self.type_name(scope, name)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A `Name` used as a type name (a module's `uses`), resolved as written in `scope`.
    fn type_name(&mut self, scope: &Scope<'_>, name: ast::Name<'_>) -> Result<(), Cancelled> {
        let chain = self.name_chain(name);
        let segments: Vec<Name> = chain.iter().map(|(_, n)| *n).collect();
        let mut resolved = self.r.qualified(scope, &segments)?;
        if let Some(TypeResolution::Package(_)) = resolved.last() {
            let n = resolved.len();
            let package = if n >= 2 {
                self.written(&segments[..n - 1])
            } else {
                Name::EMPTY
            };
            let written = self.written(&segments);
            resolved[n - 1] = self.r.missing_in_package(written, package)?;
        }
        for ((node, _), resolution) in chain.iter().zip(resolved) {
            self.record(*node, resolution.into());
        }
        Ok(())
    }

    fn written(&self, segments: &[Name]) -> Name {
        let texts: Vec<Arc<str>> = segments.iter().map(|&n| self.names.text(n)).collect();
        self.names.intern(&texts.join("."))
    }

    /// An annotation's name and its elements' names, and those of annotations nested in its
    /// arguments.
    fn annotation(
        &mut self,
        scope: &Scope<'_>,
        annotation: ast::Annotation<'_>,
    ) -> Result<(), Cancelled> {
        self.annotation_only(scope, annotation)?;
        if let Some(arguments) = annotation.annotation_arguments() {
            for node in arguments.syntax().descendants().skip(1) {
                if let Some(nested) = ast::Annotation::cast(node) {
                    self.annotation_only(scope, nested)?;
                }
            }
        }
        Ok(())
    }

    fn annotation_only(
        &mut self,
        scope: &Scope<'_>,
        annotation: ast::Annotation<'_>,
    ) -> Result<(), Cancelled> {
        let Some(name) = annotation.name() else {
            return Ok(());
        };
        self.type_name(scope, name)?;
        let owner = match self.out.last().map(|n| &n.symbol) {
            Some(Symbol::Type(key)) => Some(*key),
            _ => None,
        };
        let Some(arguments) = annotation.annotation_arguments() else {
            return Ok(());
        };
        for child in arguments.syntax().children() {
            let Some(pair) = ast::ElementValuePair::cast(child) else {
                continue;
            };
            let Some(token) = pair.name() else { continue };
            let element = crate::summary::name_text(token, &self.names);
            let symbol = match owner {
                Some(owner) => Symbol::Element {
                    owner,
                    name: element,
                },
                None => Symbol::Error(ErrorSymbol {
                    name: element,
                    cause: Cause::NotFound,
                }),
            };
            let span = token.span();
            self.out.push(ResolvedName {
                node: pair.syntax().id(),
                start: span.start,
                end: span.end,
                symbol,
            });
        }
        Ok(())
    }

    /// Every type name and annotation in `node`, a type or a declaration's part without a body.
    fn types_in(&mut self, scope: &Scope<'_>, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        for d in node.descendants() {
            match d.kind() {
                SyntaxKind::ClassType => {
                    let is_qualifier = d.parent().is_some_and(|p| {
                        ast::ClassType::cast(p)
                            .and_then(|p| p.qualifier())
                            .is_some_and(|q| q.syntax() == d)
                    });
                    if !is_qualifier {
                        self.class_type(scope, ast::ClassType::cast(d).unwrap())?;
                    }
                }
                SyntaxKind::Annotation => {
                    self.annotation_only(scope, ast::Annotation::cast(d).unwrap())?
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A class type and each of its qualifiers.
    fn class_type(&mut self, scope: &Scope<'_>, ty: ast::ClassType<'_>) -> Result<(), Cancelled> {
        let mut chain = Vec::new();
        let mut next = Some(ty);
        while let Some(t) = next {
            let segment = t
                .name()
                .map_or(Name::EMPTY, |n| crate::summary::name_text(n, &self.names));
            chain.push((t.syntax(), segment));
            next = t.qualifier();
        }
        chain.reverse();
        let segments: Vec<Name> = chain.iter().map(|(_, n)| *n).collect();
        let mut resolved = self.r.qualified(scope, &segments)?;
        if let Some(TypeResolution::Package(_)) = resolved.last() {
            let n = resolved.len();
            let package = if n >= 2 {
                self.written(&segments[..n - 1])
            } else {
                Name::EMPTY
            };
            let written = self.written(&segments);
            resolved[n - 1] = self.r.missing_in_package(written, package)?;
        }
        for ((node, _), resolution) in chain.iter().zip(resolved) {
            self.record(*node, resolution.into());
        }
        Ok(())
    }

    fn modifiers(
        &mut self,
        scope: &Scope<'_>,
        modifiers: Option<ast::Modifiers<'_>>,
    ) -> Result<(), Cancelled> {
        for annotation in modifiers.into_iter().flat_map(|m| m.annotations()) {
            self.annotation(scope, annotation)?;
        }
        Ok(())
    }

    fn type_decl(
        &mut self,
        file_scope: &FileScope,
        pending: Pending<'t>,
        queue: &mut Vec<Pending<'t>>,
    ) -> Result<(), Cancelled> {
        let header = Header::of(pending.decl);
        let index = self.next_index;
        self.next_index += 1;
        // The summary has the type at this index, unless the tree and summary disagree, which
        // they can't for the same tree; the names are left out if they do.
        let name = header
            .name
            .map_or(Name::EMPTY, |n| crate::summary::name_text(n, &self.names));
        let matches = file_scope
            .summary
            .types
            .get(index)
            .is_some_and(|t| t.name == name && t.outer.map(|o| o as usize) == pending.outer);
        if !matches {
            return Ok(());
        }
        let header_scope = Scope::of_type(file_scope, index, false);
        let around = header_scope.around();
        self.modifiers(&around, header.modifiers)?;
        if let Some(params) = header.type_parameters {
            self.types_in(&header_scope, params.syntax())?;
        }
        for clause in [
            header.extends.map(|c| c.syntax()),
            header.implements.map(|c| c.syntax()),
            header.permits.map(|c| c.syntax()),
        ]
        .into_iter()
        .flatten()
        {
            self.types_in(&header_scope, clause)?;
        }
        let body = Scope::of_type(file_scope, index, true);
        if let Some(components) = header.components {
            self.types_in(&body, components.syntax())?;
        }
        if let Some(constants) = header.constants {
            for constant in constants.constants() {
                self.modifiers(&body, constant.modifiers())?;
            }
        }
        let members: Vec<ast::Member<'t>> = header.members.into_iter().flatten().collect();
        self.body(file_scope, index, members, queue)
    }

    /// The members of the type at `index`: their signatures, and member types queued.
    fn body(
        &mut self,
        file_scope: &FileScope,
        index: usize,
        members: Vec<ast::Member<'t>>,
        queue: &mut Vec<Pending<'t>>,
    ) -> Result<(), Cancelled> {
        let key = file_scope.keys[index];
        let body = Scope::of_type(file_scope, index, true);
        let mut member_types = Vec::new();
        let mut method_index = 0u32;
        for member in members {
            self.r.reads.check()?;
            match member {
                ast::Member::TypeDecl(decl) => member_types.push(decl),
                ast::Member::FieldDecl(decl) => {
                    self.modifiers(&body, decl.modifiers())?;
                    if let Some(ty) = decl.ty() {
                        self.types_in(&body, ty.syntax())?;
                    }
                    for declarator in decl.declarators() {
                        for dim in declarator.dims() {
                            self.types_in(&body, dim.syntax())?;
                        }
                    }
                }
                ast::Member::MethodDecl(decl) => {
                    let scope = self.method_scope(&body, key, method_index, decl.type_parameters());
                    method_index += 1;
                    self.modifiers(&scope, decl.modifiers())?;
                    for annotation in decl.annotations() {
                        self.annotation(&scope, annotation)?;
                    }
                    if let Some(params) = decl.type_parameters() {
                        self.types_in(&scope, params.syntax())?;
                    }
                    if let Some(result) = decl.result() {
                        self.types_in(&scope, result.syntax())?;
                    }
                    self.parameters(&scope, decl.formal_parameters())?;
                    for dim in decl.dims() {
                        self.types_in(&scope, dim.syntax())?;
                    }
                    if let Some(throws) = decl.throws_clause() {
                        self.types_in(&scope, throws.syntax())?;
                    }
                }
                ast::Member::ConstructorDecl(decl) => {
                    let scope = self.method_scope(&body, key, method_index, decl.type_parameters());
                    method_index += 1;
                    self.modifiers(&scope, decl.modifiers())?;
                    for annotation in decl.annotations() {
                        self.annotation(&scope, annotation)?;
                    }
                    if let Some(params) = decl.type_parameters() {
                        self.types_in(&scope, params.syntax())?;
                    }
                    self.parameters(&scope, decl.formal_parameters())?;
                    if let Some(throws) = decl.throws_clause() {
                        self.types_in(&scope, throws.syntax())?;
                    }
                }
                ast::Member::CompactConstructorDecl(decl) => {
                    method_index += 1;
                    self.modifiers(&body, decl.modifiers())?;
                }
                ast::Member::Initializer(_) => {}
            }
        }
        for decl in member_types.into_iter().rev() {
            queue.push(Pending {
                decl,
                outer: Some(index),
            });
        }
        Ok(())
    }

    fn method_scope<'f>(
        &self,
        body: &Scope<'f>,
        key: TypeKey,
        index: u32,
        params: Option<ast::TypeParameters<'_>>,
    ) -> Scope<'f> {
        let mut scope = body.clone();
        let names: Vec<Name> = params
            .iter()
            .flat_map(|p| p.parameters())
            .map(|p| {
                p.name()
                    .map_or(Name::EMPTY, |n| crate::summary::name_text(n, &self.names))
            })
            .collect();
        scope.method = Some((TypeVarOwner::Method(key, index), names));
        scope
    }

    fn parameters(
        &mut self,
        scope: &Scope<'_>,
        params: Option<ast::FormalParameters<'_>>,
    ) -> Result<(), Cancelled> {
        let Some(params) = params else {
            return Ok(());
        };
        // Receiver parameters too: their types are names in the signature.
        self.types_in(scope, params.syntax())
    }
}
