//! Name resolution in declarations, and the entries built from it: each source type's
//! supertypes, every type's member types, and each source type's entry
//! (docs/design/semantic-names.md#resolving-names).
//!
//! A simple type name resolves to the first of (JLS 6.3, 6.4.1, with javac's choices observed):
//!
//! 1. a type parameter of the enclosing method, for a name in its signature;
//! 2. for each enclosing class, innermost first: a member type it declares or inherits, then
//!    one of its type parameters (a member type named like a type parameter wins in the body);
//!    the class whose header the name is in contributes its type parameters only, since its
//!    members are in scope in its body, not its `extends` clause;
//! 3. a top-level type of the compilation unit, or one a single-type import or single-static
//!    import brings in;
//! 4. a type of the same package;
//! 5. a type an on-demand import brings in, `java.lang.*` included: two different ones are
//!    ambiguous;
//! 6. a type a module import brings in, a compact unit's implicit `import module java.base`
//!    included: two different ones are ambiguous.
//!
//! A qualified name is resolved left to right: the first segment as a type if one is in scope,
//! else as a package; then a member type of a type, or a type or subpackage of a package.
//! Imports name types by canonical name, so their first segment is a package.
//!
//! A type's supertypes are resolved erased first ([`Supertypes`]), before anything that needs its
//! members, as javac does: `class A extends A.B` needs `A`'s members, which need its supertypes,
//! and javac reports it as cyclic, while `class A implements Supplier<A.In>` is fine, since type
//! arguments are resolved with the rest of the entry ([`TypeEntry`]).

use std::collections::HashMap;
use std::sync::Arc;

use crate::engine::derived::{Dep, Memo, Reads};
use crate::summary::{
    Annotation, ImplicitMember, Import, ImportKind, MethodKind, Modifiers, Primitive,
    QualifiedName, Summary, TypeKind, TypePart, TypeRef, UnitKind,
};
use crate::symbols::{
    Cause, ClassEntry, ErrorSymbol, FieldEntry, FieldKind, MemberType, MemberTypes, MethodEntry,
    Supertypes, Ty, TyPart, TypeEntry, TypeKey, TypeParam, TypeResolution, TypeVarOwner,
};
use crate::view::{Found, ModuleImport};
use crate::{Cancelled, FileId, Fingerprint, Interner, Name, Snapshot, SourceSetId};

/// Where a source type is declared.
pub(crate) struct Located {
    pub(crate) file: FileId,
    pub(crate) summary: Arc<Summary>,
    pub(crate) index: usize,
}

use crate::symbols::MemberAccess as Access;

impl Snapshot {
    /// A source type's erased supertypes; `None` for a class file's, which are read from its
    /// entry in a view.
    pub fn supertypes(&self, key: TypeKey) -> Result<Option<Arc<Supertypes>>, Cancelled> {
        self.check()?;
        if let TypeKey::Local { .. } = key {
            return Ok(self.local_type(key)?.map(|t| t.supertypes.clone()));
        }
        if !key.is_source() {
            return Ok(None);
        }
        Ok(self.supertypes_memo(key)?.map(|m| m.value.clone()))
    }

    /// A type's member types, declared and inherited, in `view` (a source type's own set,
    /// whatever `view` is).
    pub fn member_types(
        &self,
        view: SourceSetId,
        key: TypeKey,
    ) -> Result<Arc<MemberTypes>, Cancelled> {
        self.check()?;
        if let TypeKey::Local { .. } = key {
            return Ok(self
                .local_type(key)?
                .map_or_else(Default::default, |t| t.member_types.clone()));
        }
        Ok(self
            .member_types_memo(view, key)?
            .map_or_else(Default::default, |m| m.value.clone()))
    }

    /// A source type's entry, or a local or anonymous class's from its file's body result;
    /// `None` for a class file, whose [`Snapshot::class_entry`] it has.
    pub fn type_entry(&self, key: TypeKey) -> Result<Option<Arc<TypeEntry>>, Cancelled> {
        self.check()?;
        if let TypeKey::Local { .. } = key {
            return Ok(self.local_type(key)?.map(|t| t.entry.clone()));
        }
        if !key.is_source() {
            return Ok(None);
        }
        Ok(Some(self.type_entry_memo(key)?.value.clone()))
    }

    /// Checks that each entry of `file`'s types (its scope, and each type's supertypes, member
    /// types, and entry) carries its value's fingerprint, for the differential check.
    #[doc(hidden)]
    pub fn check_fingerprints(&self, file: FileId) -> Result<(), String> {
        let cancelled = |_| "cancelled with no edit running".to_owned();
        let scope = self.file_scope_memo(file).map_err(cancelled)?;
        if scope.fingerprint != Fingerprint::of(&*scope.value) {
            return Err("its scope has a stale fingerprint".to_owned());
        }
        let body = self.body_memo(file).map_err(cancelled)?;
        if body.fingerprint != Fingerprint::of(&*body.value) {
            return Err("its body names have a stale fingerprint".to_owned());
        }
        for &key in scope.value.keys.iter() {
            if let Some(memo) = self.supertypes_memo(key).map_err(cancelled)?
                && memo.fingerprint != Fingerprint::of(&*memo.value)
            {
                return Err(format!("{key:?}'s supertypes have a stale fingerprint"));
            }
            if let Some(memo) = self
                .member_types_memo(key.home(scope.value.set), key)
                .map_err(cancelled)?
                && memo.fingerprint != Fingerprint::of(&*memo.value)
            {
                return Err(format!("{key:?}'s member types have a stale fingerprint"));
            }
            let memo = self.type_entry_memo(key).map_err(cancelled)?;
            if memo.fingerprint != Fingerprint::of(&*memo.value) {
                return Err(format!("{key:?}'s entry has a stale fingerprint"));
            }
        }
        Ok(())
    }

    /// A class file's entry, as read.
    pub fn class_entry(&self, key: TypeKey) -> Option<Arc<ClassEntry>> {
        let TypeKey::Class { source, binary } = key else {
            return None;
        };
        let shared = self.shared();
        shared.classes.entry(source, binary, &shared.names)
    }

    /// A type by canonical name (`java.util.Map.Entry`) in `set`'s view, as an import names it.
    pub fn resolve_type_name(
        &self,
        set: SourceSetId,
        canonical: &str,
    ) -> Result<TypeResolution, Cancelled> {
        self.check()?;
        let mut reads = Reads::untracked(self);
        let mut resolver = Resolver::new(&mut reads, set);
        let name = QualifiedName(resolver.names.intern(canonical));
        let resolved = resolver.canonical(name)?;
        Ok(resolved
            .last()
            .cloned()
            .unwrap_or(TypeResolution::Error(ErrorSymbol {
                name: name.0,
                cause: Cause::Syntax,
            })))
    }

    pub(crate) fn supertypes_memo(
        &self,
        key: TypeKey,
    ) -> Result<Option<Arc<Memo<Supertypes>>>, Cancelled> {
        self.shared()
            .supertypes
            .get_or_cycle(self, &key, |reads| compute_supertypes(reads, key))
    }

    pub(crate) fn member_types_memo(
        &self,
        view: SourceSetId,
        key: TypeKey,
    ) -> Result<Option<Arc<Memo<MemberTypes>>>, Cancelled> {
        let view = key.home(view);
        self.shared()
            .member_types
            .get_or_cycle(self, &(view, key), |reads| {
                compute_member_types(reads, view, key)
            })
    }

    pub(crate) fn type_entry_memo(&self, key: TypeKey) -> Result<Arc<Memo<TypeEntry>>, Cancelled> {
        self.shared()
            .type_entries
            .get(self, &key, |reads| compute_type_entry(reads, key))
    }
}

impl Reads<'_> {
    /// The file and summary declaring a source type, or `None` if no file of its set declares
    /// it or more than one does. Records the package entry and the summary.
    pub(crate) fn locate(&mut self, key: TypeKey) -> Result<Option<Located>, Cancelled> {
        let file = match key {
            TypeKey::Source { set, package, path } => {
                let top = self.snapshot.names().path_names(path)[0];
                let Some(declarations) = self.package_type(set, package, top)? else {
                    return Ok(None);
                };
                if declarations.len() > 1 {
                    return Ok(None);
                }
                declarations[0]
            }
            TypeKey::Compact { file, .. } => {
                if !self.snapshot.file_exists(file)? {
                    return Ok(None);
                }
                file
            }
            TypeKey::Class { .. } | TypeKey::Local { .. } => return Ok(None),
        };
        let scope = self.file_scope(file)?;
        Ok(scope.indexes.get(&key).map(|&index| Located {
            file,
            summary: scope.summary.clone(),
            index,
        }))
    }

    /// A source type's erased supertypes, or a class file's resolved in `view`. `None` if asking
    /// is a cycle. Records the entry, or for a class file what resolving its names read.
    pub(crate) fn supertypes(
        &mut self,
        view: SourceSetId,
        key: TypeKey,
    ) -> Result<Option<Arc<Supertypes>>, Cancelled> {
        if key.is_source() {
            let memo = self.snapshot.supertypes_memo(key)?;
            self.record(
                Dep::Supertypes(key),
                memo.as_ref().map_or(Fingerprint::CYCLE, |m| m.fingerprint),
            );
            return Ok(memo.map(|m| m.value.clone()));
        }
        let Some(entry) = self.snapshot.class_entry(key) else {
            return Ok(Some(Arc::new(Supertypes {
                superclass: None,
                interfaces: Box::default(),
                cycle: Arc::from([]),
            })));
        };
        let resolve = |reads: &mut Self, binary: Name| -> Result<_, Cancelled> {
            Ok(reads.find_binary(view, binary)?.ok_or(ErrorSymbol {
                name: binary,
                cause: Cause::NotFound,
            }))
        };
        let superclass = match entry.superclass {
            Some(binary) => Some(resolve(self, binary)?),
            None => None,
        };
        let mut interfaces = Vec::new();
        for &binary in entry.interfaces.iter() {
            interfaces.push(resolve(self, binary)?);
        }
        Ok(Some(Arc::new(Supertypes {
            superclass,
            interfaces: interfaces.into_boxed_slice(),
            cycle: Arc::from([]),
        })))
    }

    /// A type's member types in `view`, or `None` if asking is a cycle. Records the entry.
    pub(crate) fn member_types(
        &mut self,
        view: SourceSetId,
        key: TypeKey,
    ) -> Result<Option<Arc<MemberTypes>>, Cancelled> {
        let memo = self.snapshot.member_types_memo(view, key)?;
        self.record(
            Dep::MemberTypes(key.home(view), key),
            memo.as_ref().map_or(Fingerprint::CYCLE, |m| m.fingerprint),
        );
        Ok(memo.map(|m| m.value.clone()))
    }

    /// A source type's entry. Records it.
    pub(crate) fn type_entry(&mut self, key: TypeKey) -> Result<Arc<TypeEntry>, Cancelled> {
        let memo = self.snapshot.type_entry_memo(key)?;
        self.record(Dep::TypeEntry(key), memo.fingerprint);
        Ok(memo.value.clone())
    }
}

/// What a file's names are resolved against, a derived entry per file: its summary, each of its
/// types' keys (made once, so a type nested ten thousand deep costs one step more than its outer
/// type), and its imports. Imports are resolved when a name needs them, so that an import nothing
/// uses can't make a cycle.
pub(crate) struct FileScope {
    pub(crate) set: SourceSetId,
    pub(crate) file: FileId,
    pub(crate) summary: Arc<Summary>,
    /// Each of the summary's types' keys, by index.
    pub(crate) keys: Box<[TypeKey]>,
    /// Each import's last segment, by index.
    import_last: Box<[Name]>,
    /// Each type's index by key, the first if two types of the file have the same path.
    indexes: HashMap<TypeKey, usize>,
    /// The top-level types' indexes by name, the first of each name.
    top_level: HashMap<Name, usize>,
    /// Each type's member types' indexes.
    members: Box<[Box<[u32]>]>,
    /// The summary's fingerprint, which everything else follows from.
    fingerprint: Fingerprint,
}

/// The scope follows from the summary, so its fingerprint is the summary's.
impl std::hash::Hash for FileScope {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.set.hash(state);
        self.file.hash(state);
        self.fingerprint.hash(state);
    }
}

impl FileScope {
    pub(crate) fn package(&self) -> Name {
        self.summary.package_name()
    }

    /// Whether the file is a compact compilation unit, whose types are the implicitly declared
    /// class's members.
    pub(crate) fn is_compact(&self) -> bool {
        self.summary.kind == UnitKind::Compact
    }

    /// The top-level type `name` the file declares.
    fn own(&self, name: Name) -> Option<TypeKey> {
        self.top_level.get(&name).map(|&i| self.keys[i])
    }

    /// The indexes of the member types of the type at `index`, in order.
    pub(crate) fn members(&self, index: usize) -> &[u32] {
        &self.members[index]
    }

    fn imports(&self) -> &[Import] {
        &self.summary.imports
    }

    /// The last segment of the import at `index`.
    fn import_last(&self, index: usize) -> Name {
        self.import_last[index]
    }
}

fn compute_file_scope(reads: &mut Reads<'_>, file: FileId) -> Result<FileScope, Cancelled> {
    let set = reads.snapshot.file(file)?.set;
    let memo = reads.summary_memo_recorded(file)?;
    let summary = memo.value.summary.clone();
    let names = reads.snapshot.names().clone();
    let compact = summary.kind == UnitKind::Compact;
    let package = summary.package_name();
    let mut paths: Vec<Option<crate::Path>> = Vec::with_capacity(summary.types.len());
    let mut keys = Vec::with_capacity(summary.types.len());
    for (index, ty) in summary.types.iter().enumerate() {
        let outer = ty.outer.and_then(|o| paths[o as usize]);
        let path = if compact && index == 0 {
            None
        } else {
            Some(names.path(outer, ty.name))
        };
        paths.push(path);
        keys.push(if compact {
            TypeKey::Compact { set, file, path }
        } else {
            TypeKey::Source {
                set,
                package,
                path: path.expect("a type of an ordinary unit has a path"),
            }
        });
    }
    let import_last = summary
        .imports
        .iter()
        .map(|i| i.name.last(&names))
        .collect();
    let mut indexes = HashMap::with_capacity(keys.len());
    for (index, &key) in keys.iter().enumerate() {
        indexes.entry(key).or_insert(index);
    }
    let mut top_level = HashMap::new();
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); summary.types.len()];
    for (index, ty) in summary.types.iter().enumerate() {
        match ty.outer {
            Some(outer) => members[outer as usize].push(index as u32),
            None if ty.kind != TypeKind::Implicit => {
                top_level.entry(ty.name).or_insert(index);
            }
            None => {}
        }
    }
    let members = members.into_iter().map(Vec::into_boxed_slice).collect();
    Ok(FileScope {
        set,
        file,
        import_last,
        indexes,
        top_level,
        members,
        summary,
        keys: keys.into_boxed_slice(),
        fingerprint: memo.fingerprint,
    })
}

impl Snapshot {
    pub(crate) fn file_scope_memo(&self, file: FileId) -> Result<Arc<Memo<FileScope>>, Cancelled> {
        self.shared()
            .file_scopes
            .get(self, &file, |reads| compute_file_scope(reads, file))
    }

    /// The key of the type at `index` in `file`'s summary.
    pub fn type_key(&self, file: FileId, index: usize) -> Result<TypeKey, Cancelled> {
        Ok(self.file_scope_memo(file)?.value.keys[index])
    }
}

impl Reads<'_> {
    /// `file`'s scope. Records it.
    pub(crate) fn file_scope(&mut self, file: FileId) -> Result<Arc<FileScope>, Cancelled> {
        let memo = self.snapshot.file_scope_memo(file)?;
        self.record(Dep::FileScope(file), memo.fingerprint);
        Ok(memo.value.clone())
    }
}

/// The scope of a name in a declaration: the file's, and the types enclosing the name, found from
/// the innermost by the summary's links, so a scope is made in O(1) however deep it is.
#[derive(Clone)]
pub(crate) struct Scope<'f> {
    pub(crate) file: &'f FileScope,
    /// The index of the innermost enclosing type, if any.
    pub(crate) innermost: Option<usize>,
    /// Whether the name is in the innermost type's header, where its members aren't in scope.
    pub(crate) header: bool,
    /// The type parameters of the method whose signature the name is in.
    pub(crate) method: Option<(TypeVarOwner, Vec<Name>)>,
    /// For a name in a body, what the body declares around it, which is innermost.
    pub(crate) body: Option<&'f dyn BodyTypes>,
}

/// The types a body declares around a name in it: local and anonymous classes, their member
/// types and type parameters, and the type parameters of the methods around it. A scope in a
/// body asks it first, then its enclosing class's scope (implemented by [`crate::body`]).
pub(crate) trait BodyTypes {
    /// The innermost type `name` declared in the body around the name, if any.
    fn simple(
        &self,
        r: &mut Resolver<'_, '_>,
        name: Name,
    ) -> Result<Option<TypeResolution>, Cancelled>;

    /// The member type `name` of the local or anonymous class `owner`, declared or inherited.
    fn member_type(
        &self,
        r: &mut Resolver<'_, '_>,
        owner: TypeKey,
        name: Name,
    ) -> Result<MemberLookup, Cancelled>;
}

impl<'f> Scope<'f> {
    /// The scope of the file's top level.
    pub(crate) fn top(file: &'f FileScope) -> Scope<'f> {
        Scope {
            file,
            innermost: None,
            header: false,
            method: None,
            body: None,
        }
    }

    /// The scope of the declarations of the type at `index`: its header (`body` false) or its
    /// body.
    pub(crate) fn of_type(file: &'f FileScope, index: usize, body: bool) -> Scope<'f> {
        Scope {
            file,
            innermost: Some(index),
            header: !body,
            method: None,
            body: None,
        }
    }

    /// The scope around a type: its enclosing types' bodies, for its annotations.
    pub(crate) fn around(&self) -> Scope<'f> {
        let outer = self
            .innermost
            .and_then(|i| self.file.summary.types[i].outer.map(|o| o as usize));
        Scope {
            file: self.file,
            innermost: outer,
            header: false,
            method: None,
            body: self.body,
        }
    }
}

pub(crate) enum MemberLookup {
    Found(TypeKey),
    Ambiguous(Box<[TypeKey]>),
    Cycle,
    None { incomplete: bool },
}

/// Resolves names for one computation, recording what it reads, and collects the errors found.
pub(crate) struct Resolver<'r, 's> {
    pub(crate) reads: &'r mut Reads<'s>,
    pub(crate) set: SourceSetId,
    pub(crate) names: Arc<Interner>,
    pub(crate) errors: Vec<ErrorSymbol>,
    /// Imports resolved so far, by name as written.
    canonical: HashMap<Name, Vec<TypeResolution>>,
    /// Simple type names resolved in the classes around a scope: by file, innermost class,
    /// whether in its header, and name.
    simple_cache: HashMap<(FileId, Option<usize>, bool, Name), TypeResolution>,
    module_packages: HashMap<Name, Result<Vec<Name>, Cause>>,
    pub(crate) java_lang: Name,
    java_base: Name,
}

impl<'r, 's> Resolver<'r, 's> {
    pub(crate) fn new(reads: &'r mut Reads<'s>, set: SourceSetId) -> Resolver<'r, 's> {
        let names = reads.snapshot.names().clone();
        Resolver {
            java_lang: names.intern("java.lang"),
            java_base: names.intern("java.base"),
            reads,
            set,
            names,
            errors: Vec::new(),
            canonical: HashMap::new(),
            simple_cache: HashMap::new(),
            module_packages: HashMap::new(),
        }
    }

    pub(crate) fn error(name: Name, cause: Cause) -> TypeResolution {
        TypeResolution::Error(ErrorSymbol { name, cause })
    }

    /// `prefix.segment`, interned.
    fn join(&self, prefix: Name, segment: Name) -> Name {
        if prefix.is_empty() {
            return segment;
        }
        self.names.intern(&format!(
            "{}.{}",
            self.names.text(prefix),
            self.names.text(segment)
        ))
    }

    /// The member type `name` of `owner`, a local or anonymous class's from `body`.
    pub(crate) fn member_type(
        &mut self,
        owner: TypeKey,
        name: Name,
        body: Option<&dyn BodyTypes>,
    ) -> Result<MemberLookup, Cancelled> {
        if let TypeKey::Local { .. } = owner {
            return match body {
                Some(body) => body.member_type(self, owner, name),
                None => Ok(MemberLookup::None { incomplete: true }),
            };
        }
        let Some(members) = self.reads.member_types(self.set, owner)? else {
            return Ok(MemberLookup::Cycle);
        };
        Ok(match members.get(name) {
            Some((MemberType::One(key), _)) => MemberLookup::Found(*key),
            Some((MemberType::Ambiguous(keys), _)) => MemberLookup::Ambiguous(keys.clone()),
            None => MemberLookup::None {
                incomplete: members.incomplete,
            },
        })
    }

    /// The type a lookup by package found, as a resolution.
    pub(crate) fn found(
        &mut self,
        found: Found,
        name: Name,
        package: Name,
    ) -> Option<TypeResolution> {
        match found {
            Found::Type(key) => Some(TypeResolution::Type(key)),
            Found::Ambiguous(keys) => Some(Self::error(name, Cause::Ambiguous(keys))),
            Found::NotVisible => Some(Self::error(name, Cause::NotVisible { package })),
            Found::Bad(message) => Some(Self::error(
                name,
                Cause::BadClass {
                    binary: name,
                    message,
                },
            )),
            Found::None => None,
        }
    }

    /// A simple type name in `scope`.
    pub(crate) fn simple(
        &mut self,
        scope: &Scope<'_>,
        name: Name,
    ) -> Result<TypeResolution, Cancelled> {
        self.reads.check()?;
        if name.is_empty() {
            return Ok(Self::error(name, Cause::Syntax));
        }
        if let Some(body) = scope.body
            && let Some(found) = body.simple(self, name)?
        {
            return Ok(found);
        }
        if let Some((owner, params)) = &scope.method
            && let Some(index) = params.iter().position(|&p| p == name)
        {
            return Ok(TypeResolution::TypeVar(*owner, index as u32));
        }
        // What's left depends only on the classes around and the file, so a name looked up
        // again in the same place is answered at once, and what it read is recorded once.
        let key = (scope.file.file, scope.innermost, scope.header, name);
        if let Some(found) = self.simple_cache.get(&key) {
            return Ok(found.clone());
        }
        let found = self.simple_in_classes(scope, name)?;
        self.simple_cache.insert(key, found.clone());
        Ok(found)
    }

    /// A simple type name in the classes around `scope` and its file, after a method's type
    /// parameters.
    fn simple_in_classes(
        &mut self,
        scope: &Scope<'_>,
        name: Name,
    ) -> Result<TypeResolution, Cancelled> {
        let mut incomplete = None;
        let file = scope.file;
        let mut next = scope.innermost;
        let mut members = !scope.header;
        while let Some(index) = next {
            self.reads.check()?;
            let key = file.keys[index];
            let ty = &file.summary.types[index];
            if members {
                match self.member_type(key, name, None)? {
                    MemberLookup::Found(key) => return Ok(TypeResolution::Type(key)),
                    MemberLookup::Ambiguous(keys) => {
                        return Ok(Self::error(name, Cause::Ambiguous(keys)));
                    }
                    MemberLookup::Cycle => return Ok(Self::error(name, Cause::Cycle)),
                    MemberLookup::None { incomplete: true } => {
                        incomplete.get_or_insert(key);
                    }
                    MemberLookup::None { incomplete: false } => {}
                }
            }
            if let Some(i) = ty.type_parameters.iter().position(|p| p.name == name) {
                return Ok(TypeResolution::TypeVar(TypeVarOwner::Type(key), i as u32));
            }
            next = ty.outer.map(|o| o as usize);
            members = true;
        }
        if let Some(key) = file.own(name) {
            return Ok(TypeResolution::Type(key));
        }
        for (i, import) in file.imports().iter().enumerate() {
            if import.kind == ImportKind::SingleType && file.import_last(i) == name {
                let resolved = self.canonical(import.name)?;
                if let Some(last) = resolved.last() {
                    return self.single_import_result(import.name, &resolved, last.clone());
                }
            }
        }
        for (i, import) in file.imports().iter().enumerate() {
            if import.kind == ImportKind::SingleStatic && file.import_last(i) == name {
                let owner_name = self.owner_of(import.name);
                if let Some(TypeResolution::Type(owner)) =
                    self.canonical(QualifiedName(owner_name))?.last().cloned()
                    && let MemberLookup::Found(key) = self.member_type(owner, name, None)?
                {
                    return Ok(TypeResolution::Type(key));
                }
            }
        }
        let package = file.package();
        let found = self.reads.find_top_level(self.set, package, name)?;
        if let Some(resolution) = self.found(found, name, package) {
            return Ok(resolution);
        }

        // On demand: every import that has the name, and `java.lang`.
        let mut candidates: Vec<TypeKey> = Vec::new();
        let mut first_error = None;
        for import in file.imports() {
            let target = match import.kind {
                ImportKind::OnDemand | ImportKind::StaticOnDemand => self
                    .canonical(import.name)?
                    .last()
                    .cloned()
                    .unwrap_or(TypeResolution::Package(Name::EMPTY)),
                _ => continue,
            };
            match target {
                TypeResolution::Package(package) if import.kind == ImportKind::OnDemand => {
                    let found = self.reads.find_top_level(self.set, package, name)?;
                    match self.found(found, name, package) {
                        Some(TypeResolution::Type(key)) => candidates.push(key),
                        Some(error) => {
                            first_error.get_or_insert(error);
                        }
                        None => {}
                    }
                }
                TypeResolution::Type(owner) => match self.member_type(owner, name, None)? {
                    MemberLookup::Found(key) => candidates.push(key),
                    MemberLookup::Ambiguous(keys) => {
                        first_error.get_or_insert(Self::error(name, Cause::Ambiguous(keys)));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        let found = self.reads.find_top_level(self.set, self.java_lang, name)?;
        if let Some(TypeResolution::Type(key)) = self.found(found, name, self.java_lang) {
            candidates.push(key);
        }
        candidates.sort();
        candidates.dedup();
        match candidates.len() {
            0 => {}
            1 => return Ok(TypeResolution::Type(candidates[0])),
            _ => return Ok(Self::error(name, Cause::Ambiguous(candidates.into()))),
        }
        if let Some(error) = first_error {
            return Ok(error);
        }

        // Module imports, shadowed by everything above.
        let mut modules: Vec<Name> = file
            .imports()
            .iter()
            .filter(|i| i.kind == ImportKind::Module)
            .map(|i| i.name.0)
            .collect();
        if file.is_compact() {
            modules.push(self.java_base);
        }
        for module in modules {
            let Ok(packages) = self.module_packages(module)? else {
                continue;
            };
            for package in packages {
                let found = self.reads.find_top_level(self.set, package, name)?;
                if let Some(TypeResolution::Type(key)) = self.found(found, name, package) {
                    candidates.push(key);
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        match candidates.len() {
            0 => {}
            1 => return Ok(TypeResolution::Type(candidates[0])),
            _ => return Ok(Self::error(name, Cause::Ambiguous(candidates.into()))),
        }
        Ok(Self::error(
            name,
            match incomplete {
                Some(ty) => Cause::Incomplete { ty },
                None => Cause::NotFound,
            },
        ))
    }

    /// What a single-type import gives a name: its type, or an error.
    fn single_import_result(
        &mut self,
        import: QualifiedName,
        resolved: &[TypeResolution],
        last: TypeResolution,
    ) -> Result<TypeResolution, Cancelled> {
        Ok(match last {
            TypeResolution::Package(_) => {
                let package = match resolved.len() {
                    0 | 1 => Name::EMPTY,
                    n => match &resolved[n - 2] {
                        TypeResolution::Package(p) => *p,
                        _ => Name::EMPTY,
                    },
                };
                self.missing_in_package(import.0, package)?
            }
            other => other,
        })
    }

    /// The error for a type `name` not found in `package`: no such package, or no such type.
    pub(crate) fn missing_in_package(
        &mut self,
        name: Name,
        package: Name,
    ) -> Result<TypeResolution, Cancelled> {
        if package.is_empty() || self.reads.package_exists(self.set, package)? {
            Ok(Self::error(name, Cause::NotFound))
        } else {
            Ok(Self::error(name, Cause::NoPackage { package }))
        }
    }

    /// `a.B` for `a.B.m`.
    pub(crate) fn owner_of(&self, name: QualifiedName) -> Name {
        let text = self.names.text(name.0);
        self.names
            .intern(text.rsplit_once('.').map_or("", |(owner, _)| owner))
    }

    /// The packages `import module M` imports, cached for the computation.
    fn module_packages(&mut self, module: Name) -> Result<Result<Vec<Name>, Cause>, Cancelled> {
        if let Some(found) = self.module_packages.get(&module) {
            return Ok(found.clone());
        }
        let found = match self.reads.module_import(self.set, module)? {
            ModuleImport::Packages(packages) => Ok(packages),
            ModuleImport::NotRead => Err(Cause::ModuleNotRead { module }),
            ModuleImport::NoModule => Err(Cause::NoModule { module }),
        };
        self.module_packages.insert(module, found.clone());
        Ok(found)
    }

    /// Checks an `import module` for errors.
    pub(crate) fn module_import(&mut self, module: Name) -> Result<Option<Cause>, Cancelled> {
        Ok(self.module_packages(module)?.err())
    }

    /// A canonical name, as an import has it, resolved left to right: one resolution per
    /// segment, each of the name up to it.
    pub(crate) fn canonical(
        &mut self,
        name: QualifiedName,
    ) -> Result<Vec<TypeResolution>, Cancelled> {
        if let Some(found) = self.canonical.get(&name.0) {
            return Ok(found.clone());
        }
        let segments = name.segments(&self.names);
        let first = TypeResolution::Package(segments[0]);
        let resolved = self.steps(first, &segments, None)?;
        self.canonical.insert(name.0, resolved.clone());
        Ok(resolved)
    }

    /// A type name as written in a declaration, resolved left to right in `scope`: one
    /// resolution per segment.
    pub(crate) fn qualified(
        &mut self,
        scope: &Scope<'_>,
        segments: &[Name],
    ) -> Result<Vec<TypeResolution>, Cancelled> {
        let Some(&first) = segments.first() else {
            return Ok(Vec::new());
        };
        let mut start = self.simple(scope, first)?;
        if segments.len() > 1
            && matches!(
                &start,
                TypeResolution::Error(ErrorSymbol {
                    cause: Cause::NotFound | Cause::Incomplete { .. },
                    ..
                })
            )
        {
            start = TypeResolution::Package(first);
        }
        self.steps(start, segments, scope.body)
    }

    /// The resolutions of each segment after the first, from the first's, with the member
    /// types of local and anonymous classes from `body`.
    pub(crate) fn steps(
        &mut self,
        first: TypeResolution,
        segments: &[Name],
        body: Option<&dyn BodyTypes>,
    ) -> Result<Vec<TypeResolution>, Cancelled> {
        let mut resolved = Vec::with_capacity(segments.len());
        resolved.push(first);
        let mut written = segments[0];
        for (i, &segment) in segments.iter().enumerate().skip(1) {
            self.reads.check()?;
            if i >= MAX_SEGMENTS {
                // Each prefix has its own name, so resolving them all would cost the square of
                // the name's length.
                let error = Self::error(self.written(segments), Cause::TooLong);
                resolved.resize(segments.len(), error);
                break;
            }
            written = self.join(written, segment);
            let next = match resolved.last().unwrap().clone() {
                TypeResolution::Package(package) => {
                    let found = self.reads.find_top_level(self.set, package, segment)?;
                    match self.found(found, written, package) {
                        Some(resolution) => resolution,
                        None => TypeResolution::Package(self.join(package, segment)),
                    }
                }
                TypeResolution::Type(owner) => match self.member_type(owner, segment, body)? {
                    MemberLookup::Found(key) => TypeResolution::Type(key),
                    MemberLookup::Ambiguous(keys) => Self::error(written, Cause::Ambiguous(keys)),
                    MemberLookup::Cycle => Self::error(written, Cause::Cycle),
                    MemberLookup::None { incomplete: true } => {
                        Self::error(written, Cause::Incomplete { ty: owner })
                    }
                    MemberLookup::None { incomplete: false } => {
                        Self::error(written, Cause::NotFound)
                    }
                },
                // Members of a type variable are its bounds', which B3 resolves.
                TypeResolution::TypeVar(..) => Self::error(written, Cause::NotFound),
                error @ TypeResolution::Error(_) => error,
            };
            resolved.push(next);
        }
        Ok(resolved)
    }

    /// The class a type as written names, erased: for a supertype, before members are known.
    pub(crate) fn erased(
        &mut self,
        scope: &Scope<'_>,
        ty: &TypeRef,
    ) -> Result<Result<TypeKey, ErrorSymbol>, Cancelled> {
        let parts = ty.parts();
        let ends = subtree_ends(parts);
        if !matches!(parts[0], TypePart::Class { .. }) {
            return Ok(Err(ErrorSymbol {
                name: Name::EMPTY,
                cause: Cause::Syntax,
            }));
        }
        let segments: Vec<Name> = children(&ends, 0)
            .map(|i| match parts[i] {
                TypePart::Segment { name, .. } => name,
                _ => Name::EMPTY,
            })
            .collect();
        let resolved = self.qualified(scope, &segments)?;
        Ok(match self.as_type(&segments, resolved.last().cloned())? {
            TypeResolution::Type(key) => Ok(key),
            TypeResolution::Error(error) => Err(error),
            _ => Err(ErrorSymbol {
                name: Name::EMPTY,
                cause: Cause::NotFound,
            }),
        })
    }

    /// The last segment's resolution as a type: a package where a type should be is an error.
    fn as_type(
        &mut self,
        segments: &[Name],
        last: Option<TypeResolution>,
    ) -> Result<TypeResolution, Cancelled> {
        let written = self.written(segments);
        Ok(match last {
            Some(TypeResolution::Package(_)) => {
                let package = match segments.len() {
                    0 | 1 => Name::EMPTY,
                    n => self.written(&segments[..n - 1]),
                };
                self.missing_in_package(written, package)?
            }
            Some(other) => other,
            None => Self::error(written, Cause::Syntax),
        })
    }

    pub(crate) fn written(&self, segments: &[Name]) -> Name {
        let texts: Vec<Arc<str>> = segments.iter().map(|&n| self.names.text(n)).collect();
        self.names.intern(&texts.join("."))
    }

    /// The index of `error` in the errors, adding it.
    fn error_index(&mut self, error: ErrorSymbol) -> u32 {
        if let Some(index) = self.errors.iter().position(|e| *e == error) {
            return index as u32;
        }
        self.errors.push(error);
        self.errors.len() as u32 - 1
    }

    /// A type as written, resolved in `scope`, with `extra` more array dimensions (a declarator's
    /// own, or a variable arity parameter's).
    pub(crate) fn ty(
        &mut self,
        scope: &Scope<'_>,
        ty: &TypeRef,
        extra: u32,
    ) -> Result<Ty, Cancelled> {
        enum Work {
            Node(usize),
            /// The class type made of the class at `class`'s segments up to `segment`, whose
            /// resolutions are in `classes[class]`.
            Class(usize, usize),
        }
        let parts = ty.parts();
        let ends = subtree_ends(parts);
        let mut out = Vec::with_capacity(parts.len() + 1);
        if extra > 0 {
            match parts[0] {
                TypePart::Array { dims } => {
                    out.push(TyPart::Array { dims: dims + extra });
                }
                _ => out.push(TyPart::Array { dims: extra }),
            }
        }
        // Each class part's segment indexes and resolutions.
        let mut classes: Vec<(Vec<usize>, Vec<TypeResolution>)> = Vec::new();
        let start = if extra > 0 && matches!(parts[0], TypePart::Array { .. }) {
            1
        } else {
            0
        };
        let mut stack = vec![Work::Node(start)];
        while let Some(work) = stack.pop() {
            self.reads.check()?;
            let (class, segment) = match work {
                Work::Class(class, segment) => (class, segment),
                Work::Node(i) => {
                    match parts[i] {
                        TypePart::Primitive(p) => out.push(TyPart::Primitive(p)),
                        TypePart::Void => out.push(TyPart::Void),
                        TypePart::Missing | TypePart::Segment { .. } => {
                            let index = self.error_index(ErrorSymbol {
                                name: Name::EMPTY,
                                cause: Cause::Syntax,
                            });
                            out.push(TyPart::Error(index));
                        }
                        TypePart::Array { dims } => {
                            out.push(TyPart::Array { dims });
                            stack.push(Work::Node(i + 1));
                        }
                        TypePart::Wildcard { bound } => {
                            out.push(TyPart::Wildcard { bound });
                            if bound.is_some() {
                                stack.push(Work::Node(i + 1));
                            }
                        }
                        TypePart::Intersection { types } | TypePart::Union { types } => {
                            out.push(match parts[i] {
                                TypePart::Intersection { .. } => TyPart::Intersection { types },
                                _ => TyPart::Union { types },
                            });
                            let kids: Vec<usize> = children(&ends, i).collect();
                            for &kid in kids.iter().rev() {
                                stack.push(Work::Node(kid));
                            }
                        }
                        TypePart::Class { .. } => {
                            let segments: Vec<usize> = children(&ends, i).collect();
                            let written: Vec<Name> = segments
                                .iter()
                                .map(|&s| match parts[s] {
                                    TypePart::Segment { name, .. } => name,
                                    _ => Name::EMPTY,
                                })
                                .collect();
                            let mut resolved = self.qualified(scope, &written)?;
                            let last = self.as_type(&written, resolved.pop())?;
                            resolved.push(last.clone());
                            match last {
                                TypeResolution::Type(_) => {
                                    let last = segments.len() - 1;
                                    classes.push((segments, resolved));
                                    stack.push(Work::Class(classes.len() - 1, last));
                                }
                                TypeResolution::TypeVar(owner, index) => {
                                    out.push(TyPart::TypeVar { owner, index })
                                }
                                TypeResolution::Error(error) => {
                                    let index = self.error_index(error);
                                    out.push(TyPart::Error(index));
                                }
                                TypeResolution::Package(_) => unreachable!("as_type"),
                            }
                        }
                    }
                    continue;
                }
            };
            let (segments, resolved) = &classes[class];
            let TypeResolution::Type(key) = resolved[segment] else {
                unreachable!("a class part's segments from its first type are types")
            };
            let seg = segments[segment];
            let arguments = parts[seg].arguments();
            // An enclosing type written with type arguments is kept as this one's first child.
            let outer = segment > 0
                && matches!(resolved[segment - 1], TypeResolution::Type(_))
                && (0..segment).any(|s| {
                    matches!(resolved[s], TypeResolution::Type(_))
                        && parts[segments[s]].arguments().is_some()
                });
            out.push(TyPart::Class {
                ty: key,
                outer,
                arguments: arguments.unwrap_or(TyPart::NO_ARGUMENTS),
            });
            let kids: Vec<usize> = children(&ends, seg).collect();
            for &kid in kids.iter().rev() {
                stack.push(Work::Node(kid));
            }
            if outer {
                stack.push(Work::Class(class, segment - 1));
            }
        }
        Ok(Ty::of(out))
    }

    /// An annotation's type, in `scope`.
    pub(crate) fn annotation(
        &mut self,
        scope: &Scope<'_>,
        annotation: &Annotation,
    ) -> Result<TypeResolution, Cancelled> {
        let segments = annotation.name.segments(&self.names);
        let mut resolved = self.qualified(scope, &segments)?;
        self.as_type(&segments, resolved.pop())
    }
}

/// The most segments of a qualified name that are resolved: each prefix of a name is a name of
/// its own (`a`, `a.b`, `a.b.c`), so resolving every prefix of a name costs the square of its
/// length. Longer names are error symbols; no Java code has packages that deep.
pub(crate) const MAX_SEGMENTS: usize = 1024;

/// For each part of a flat type, the index just past its subtree. Children of part `i` are at
/// `i + 1`, then at each child's end, up to `i`'s end.
pub(crate) fn subtree_ends(parts: &[TypePart]) -> Vec<usize> {
    let mut ends = vec![0; parts.len()];
    for i in (0..parts.len()).rev() {
        let mut end = i + 1;
        for _ in 0..parts[i].arity() {
            end = if end < parts.len() { ends[end] } else { end };
        }
        ends[i] = end;
    }
    ends
}

/// The indexes of part `i`'s children.
pub(crate) fn children(ends: &[usize], i: usize) -> impl Iterator<Item = usize> + '_ {
    let end = ends[i];
    let mut next = i + 1;
    std::iter::from_fn(move || {
        if next >= end {
            return None;
        }
        let child = next;
        next = ends[child];
        Some(child)
    })
}

/// A source type's erased supertypes.
fn compute_supertypes(reads: &mut Reads<'_>, key: TypeKey) -> Result<Supertypes, Cancelled> {
    let names = reads.snapshot.names().clone();
    let set = key.home(SourceSetId::from_index(0));
    let Some(located) = reads.locate(key)? else {
        return Ok(Supertypes {
            superclass: None,
            interfaces: Box::default(),
            cycle: Arc::from([]),
        });
    };
    let file_scope = reads.file_scope(located.file)?;
    let ty = &located.summary.types[located.index];
    let scope = Scope::of_type(&file_scope, located.index, false);
    let mut resolver = Resolver::new(reads, set);
    let lang = |resolver: &mut Resolver<'_, '_>, simple: &str| -> Result<_, Cancelled> {
        let name = names.intern(simple);
        let found = resolver
            .reads
            .find_top_level(set, resolver.java_lang, name)?;
        Ok(match found {
            Found::Type(key) => Ok(key),
            _ => Err(ErrorSymbol {
                name: names.intern(&format!("java.lang.{simple}")),
                cause: Cause::NotFound,
            }),
        })
    };
    let mut interfaces = Vec::new();
    let superclass = match ty.kind {
        TypeKind::Class | TypeKind::Implicit => match ty.extends.first() {
            Some(written) => Some(resolver.erased(&scope, written)?),
            None => {
                let object = lang(&mut resolver, "Object")?;
                // `java.lang.Object` itself has no superclass.
                (object != Ok(key)).then_some(object)
            }
        },
        TypeKind::Enum => Some(lang(&mut resolver, "Enum")?),
        TypeKind::Record => Some(lang(&mut resolver, "Record")?),
        TypeKind::Interface | TypeKind::Annotation => None,
    };
    let written: &[TypeRef] = match ty.kind {
        TypeKind::Interface => &ty.extends,
        TypeKind::Annotation => &[],
        _ => &ty.implements,
    };
    for written in written {
        interfaces.push(resolver.erased(&scope, written)?);
    }
    if ty.kind == TypeKind::Annotation {
        let name = names.intern("Annotation");
        let package = names.intern("java.lang.annotation");
        interfaces.push(match resolver.reads.find_top_level(set, package, name)? {
            Found::Type(key) => Ok(key),
            _ => Err(ErrorSymbol {
                name: names.intern("java.lang.annotation.Annotation"),
                cause: Cause::NotFound,
            }),
        });
    }
    // A cycle through the supertypes alone (`class P extends Q`, `class Q extends P`) doesn't
    // read any member types, so the source supertypes' own supertypes are read to find it. A
    // supertype found in a cycle with this type, made earlier, says so: the cycle is the same
    // whichever of its types was asked for first.
    // The sets are shared where they are the same, so a long cycle's types are kept once.
    let mut cycle: Option<Arc<[TypeKey]>> = None;
    let mut add = |set: Arc<[TypeKey]>| match &cycle {
        None => cycle = Some(set),
        Some(current) if set.iter().all(|k| current.binary_search(k).is_ok()) => {}
        Some(current) => {
            let mut merged: Vec<TypeKey> = current.iter().chain(set.iter()).copied().collect();
            merged.sort();
            merged.dedup();
            cycle = Some(Arc::from(merged));
        }
    };
    for supertype in superclass.iter().chain(&interfaces) {
        if let Ok(supertype) = supertype
            && supertype.is_source()
            && let Some(theirs) = reads.supertypes(set, *supertype)?
            && theirs.cycle.binary_search(&key).is_ok()
        {
            add(theirs.cycle.clone());
        }
    }
    for found in reads.cycles() {
        add(found.source_types());
    }
    if let Some(cycle) = cycle.filter(|c| !c.is_empty()) {
        let error = |r: &Result<TypeKey, ErrorSymbol>| -> Result<TypeKey, ErrorSymbol> {
            Err(ErrorSymbol {
                name: match r {
                    Ok(TypeKey::Source { path, .. }) => names.path_last(*path),
                    Ok(TypeKey::Compact { path, .. }) => {
                        path.map_or(Name::EMPTY, |p| names.path_last(p))
                    }
                    Ok(TypeKey::Class { binary, .. }) => *binary,
                    Ok(TypeKey::Local { .. }) => Name::EMPTY,
                    Err(e) => e.name,
                },
                cause: Cause::Cycle,
            })
        };
        return Ok(Supertypes {
            superclass: superclass.as_ref().map(error),
            interfaces: interfaces.iter().map(error).collect(),
            cycle,
        });
    }
    Ok(Supertypes {
        superclass,
        interfaces: interfaces.into_boxed_slice(),
        cycle: Arc::from([]),
    })
}

/// The package of a type, for package access.
pub(crate) fn package_of(key: TypeKey, names: &Interner) -> Name {
    match key {
        TypeKey::Source { package, .. } => package,
        TypeKey::Compact { .. } | TypeKey::Local { .. } => Name::EMPTY,
        TypeKey::Class { binary, .. } => {
            let text = names.text(binary);
            names.intern(
                &text
                    .rsplit_once('/')
                    .map_or("", |(p, _)| p)
                    .replace('/', "."),
            )
        }
    }
}

fn compute_member_types(
    reads: &mut Reads<'_>,
    view: SourceSetId,
    key: TypeKey,
) -> Result<MemberTypes, Cancelled> {
    let names = reads.snapshot.names().clone();
    let mut declared: Vec<(Name, TypeKey, Access)> = Vec::new();
    let package = package_of(key, &names);
    let mut incomplete = false;
    match key {
        TypeKey::Class { source, .. } => {
            if let Some(entry) = reads.snapshot.class_entry(key) {
                use ferrion_classfile::class::AccessFlags as F;
                let interface = F(entry.access).contains(F::INTERFACE);
                for &(simple, binary, flags) in entry.member_types.iter() {
                    let member = reads
                        .find_binary(view, binary)?
                        .unwrap_or(TypeKey::Class { source, binary });
                    let flags = F(flags);
                    let access = if flags.contains(F::PRIVATE) {
                        Access::Private
                    } else if interface || flags.contains(F::PUBLIC) || flags.contains(F::PROTECTED)
                    {
                        Access::Public
                    } else {
                        Access::Package(package)
                    };
                    declared.push((simple, member, access));
                }
                incomplete |= entry.problem.is_some();
            }
        }
        _ => {
            let Some(located) = reads.locate(key)? else {
                return Ok(MemberTypes::default());
            };
            let summary = &located.summary;
            let interface = matches!(
                summary.types[located.index].kind,
                TypeKind::Interface | TypeKind::Annotation
            );
            let file_scope = reads.file_scope(located.file)?;
            for &i in file_scope.members(located.index) {
                let i = i as usize;
                let member = &summary.types[i];
                let access = if interface
                    || member.modifiers.contains(Modifiers::PUBLIC)
                    || member.modifiers.contains(Modifiers::PROTECTED)
                {
                    Access::Public
                } else if member.modifiers.contains(Modifiers::PRIVATE) {
                    Access::Private
                } else {
                    Access::Package(package)
                };
                declared.push((member.name, file_scope.keys[i], access));
            }
        }
    }
    let mut members: HashMap<Name, (MemberType, Access)> = HashMap::new();
    for &(name, member, access) in &declared {
        // Two member types of one name is an error javac reports; the first is kept.
        members
            .entry(name)
            .or_insert((MemberType::One(member), access));
    }
    let supertypes = reads.supertypes(view, key)?;
    match &supertypes {
        None => incomplete = true,
        Some(supertypes) => {
            incomplete |= supertypes.cyclic();
            let mut inherited: HashMap<Name, (MemberType, Access)> = HashMap::new();
            for supertype in supertypes
                .superclass
                .iter()
                .chain(supertypes.interfaces.iter())
            {
                let Ok(supertype) = supertype else {
                    incomplete = true;
                    continue;
                };
                let Some(theirs) = reads.member_types(view, *supertype)? else {
                    incomplete = true;
                    continue;
                };
                incomplete |= theirs.incomplete;
                for (name, (member, access)) in theirs.members.iter() {
                    let inheritable = match access {
                        Access::Public => true,
                        Access::Package(p) => *p == package,
                        Access::Private => false,
                    };
                    if !inheritable || members.contains_key(name) {
                        continue;
                    }
                    match inherited.get_mut(name) {
                        None => {
                            inherited.insert(*name, (member.clone(), *access));
                        }
                        Some((existing, _)) if existing == member => {}
                        Some((existing, _)) => {
                            let mut keys: Vec<TypeKey> = Vec::new();
                            for m in [&*existing, member] {
                                match m {
                                    MemberType::One(k) => keys.push(*k),
                                    MemberType::Ambiguous(ks) => keys.extend(ks.iter()),
                                }
                            }
                            keys.sort();
                            keys.dedup();
                            *existing = MemberType::Ambiguous(keys.into_boxed_slice());
                        }
                    }
                }
            }
            members.extend(inherited);
        }
    }
    // A member type entry in a cycle reads a supertype's member types that are being made: they
    // are left out, and the entry is incomplete. Its type's supertypes are then cyclic too,
    // unless the cycle is among class files, so nothing is inherited either way.
    incomplete |= reads.in_cycle();
    let mut members: Vec<(Name, (MemberType, Access))> = members.into_iter().collect();
    members.sort_by_key(|(name, _)| *name);
    Ok(MemberTypes {
        members: members.into_boxed_slice(),
        incomplete,
    })
}

fn compute_type_entry(reads: &mut Reads<'_>, key: TypeKey) -> Result<TypeEntry, Cancelled> {
    let names = reads.snapshot.names().clone();
    let set = key.home(SourceSetId::from_index(0));
    let empty = |errors: Vec<ErrorSymbol>| TypeEntry {
        key,
        kind: TypeKind::Class,
        modifiers: Modifiers::default(),
        annotations: Box::default(),
        type_parameters: Box::default(),
        superclass: None,
        interfaces: Box::default(),
        permits: Box::default(),
        fields: Box::default(),
        methods: Box::default(),
        incomplete: true,
        lombok: false,
        generated: false,
        errors: errors.into_boxed_slice(),
    };
    let Some(located) = reads.locate(key)? else {
        return Ok(empty(Vec::new()));
    };
    let file_scope = reads.file_scope(located.file)?;
    let ty = &located.summary.types[located.index];
    let generated = ty.generated || reads.snapshot.file(located.file)?.generated;
    let header = Scope::of_type(&file_scope, located.index, false);
    let around = header.around();
    let body = Scope::of_type(&file_scope, located.index, true);
    let member_types = reads.member_types(set, key)?;
    let supertypes = reads.supertypes(set, key)?;
    let mut r = Resolver::new(reads, set);

    let mut lombok = false;
    let mut annotations = Vec::new();
    for annotation in ty.annotations.iter() {
        let resolved = r.annotation(&around, annotation)?;
        lombok |= is_lombok(&resolved, &names);
        if let TypeResolution::Error(error) = &resolved {
            r.error_index(error.clone());
        }
        annotations.push(resolved);
    }
    let type_parameters = type_params(&mut r, &header, &ty.type_parameters)?;
    let superclass = match (ty.kind, ty.extends.first()) {
        (TypeKind::Class, Some(written)) => Some(r.ty(&header, written, 0)?),
        _ => None,
    };
    let written_interfaces: &[TypeRef] = match ty.kind {
        TypeKind::Interface => &ty.extends,
        TypeKind::Annotation => &[],
        _ => &ty.implements,
    };
    let mut interfaces = Vec::new();
    for written in written_interfaces {
        interfaces.push(r.ty(&header, written, 0)?);
    }
    let mut permits = Vec::new();
    for written in ty.permits.iter() {
        permits.push(r.ty(&header, written, 0)?);
    }

    let (fields, methods) = members_of(&mut r, key, ty, &body, &mut lombok)?;
    let errors = std::mem::take(&mut r.errors);
    let cyclic = supertypes.as_ref().is_none_or(|s| s.cyclic());
    let incomplete = member_types.as_ref().is_none_or(|m| m.incomplete) || cyclic || lombok;
    Ok(TypeEntry {
        key,
        kind: ty.kind,
        modifiers: ty.modifiers,
        annotations: annotations.into_boxed_slice(),
        type_parameters,
        superclass,
        interfaces: interfaces.into_boxed_slice(),
        permits: permits.into_boxed_slice(),
        fields: fields.into_boxed_slice(),
        methods: methods.into_boxed_slice(),
        incomplete,
        lombok,
        generated,
        errors: errors.into_boxed_slice(),
    })
}

/// A type's fields and methods, declared and implicit, resolved in `body`, the scope of its
/// body: what its entry lists after its header. For source types, and for local and anonymous
/// classes, whose summaries the body walk makes.
pub(crate) fn members_of(
    r: &mut Resolver<'_, '_>,
    key: TypeKey,
    ty: &crate::summary::TypeSummary,
    body: &Scope<'_>,
    lombok: &mut bool,
) -> Result<(Vec<FieldEntry>, Vec<MethodEntry>), Cancelled> {
    let names = r.names.clone();
    let set = r.set;
    let mut fields = Vec::new();
    for field in &ty.fields {
        for annotation in field.annotations.iter() {
            *lombok |= is_lombok(&r.annotation(body, annotation)?, &names);
        }
        fields.push(FieldEntry {
            name: field.name,
            ty: r.ty(body, &field.ty, field.dims.len() as u32)?,
            modifiers: field.modifiers,
            kind: FieldKind::Field,
        });
    }
    let own = |dims: u32| -> Ty {
        let mut parts = Vec::new();
        if dims > 0 {
            parts.push(TyPart::Array { dims });
        }
        parts.push(TyPart::Class {
            ty: key,
            outer: false,
            arguments: TyPart::NO_ARGUMENTS,
        });
        Ty::of(parts)
    };
    for constant in &ty.enum_constants {
        fields.push(FieldEntry {
            name: constant.name,
            ty: own(0),
            modifiers: Modifiers::PUBLIC,
            kind: FieldKind::EnumConstant,
        });
    }
    let mut components = Vec::new();
    for component in &ty.record_components {
        components.push(r.ty(body, &component.ty, u32::from(component.varargs.is_some()))?);
    }
    for (component, ty) in ty.record_components.iter().zip(&components) {
        fields.push(FieldEntry {
            name: component.name,
            ty: ty.clone(),
            modifiers: Modifiers::PRIVATE,
            kind: FieldKind::Component,
        });
    }

    let mut methods = Vec::new();
    for (index, method) in ty.methods.iter().enumerate() {
        let owner = TypeVarOwner::Method(key, index as u32);
        let mut scope = body.clone();
        scope.method = Some((
            owner,
            method.type_parameters.iter().map(|p| p.name).collect(),
        ));
        let type_parameters = type_params(r, &scope, &method.type_parameters)?;
        let mut parameters = Vec::new();
        let varargs = method
            .parameters
            .last()
            .is_some_and(|p| p.varargs.is_some());
        for parameter in method.parameters.iter() {
            let extra = parameter.dims.len() as u32 + u32::from(parameter.varargs.is_some());
            parameters.push(r.ty(&scope, &parameter.ty, extra)?);
        }
        if method.kind == MethodKind::CompactConstructor {
            parameters = components.clone();
        }
        let result = match &method.result {
            Some(result) => Some(r.ty(&scope, result, method.dims.len() as u32)?),
            None => None,
        };
        let mut throws = Vec::new();
        for thrown in method.throws.iter() {
            throws.push(r.ty(&scope, thrown, 0)?);
        }
        methods.push(MethodEntry {
            kind: method.kind,
            name: method.name,
            modifiers: method.modifiers,
            type_parameters,
            parameters: parameters.into_boxed_slice(),
            varargs: varargs
                || (method.kind == MethodKind::CompactConstructor
                    && ty
                        .record_components
                        .last()
                        .is_some_and(|c| c.varargs.is_some())),
            result,
            throws: throws.into_boxed_slice(),
            implicit: false,
        });
    }
    let lang = |r: &mut Resolver<'_, '_>, simple: &str| -> Result<Ty, Cancelled> {
        let name = names.intern(simple);
        let found = r.reads.find_top_level(set, r.java_lang, name)?;
        Ok(match found {
            Found::Type(ty) => Ty::of(vec![TyPart::Class {
                ty,
                outer: false,
                arguments: TyPart::NO_ARGUMENTS,
            }]),
            _ => {
                let index = r.error_index(ErrorSymbol {
                    name: names.intern(&format!("java.lang.{simple}")),
                    cause: Cause::NotFound,
                });
                Ty::of(vec![TyPart::Error(index)])
            }
        })
    };
    let primitive = |p: Primitive| Ty::of(vec![TyPart::Primitive(p)]);
    for implicit in &ty.implicit {
        let method = |name: &str, parameters: Vec<Ty>, result: Option<Ty>, modifiers| MethodEntry {
            kind: if result.is_some() {
                MethodKind::Method
            } else {
                MethodKind::Constructor
            },
            name: names.intern(name),
            modifiers,
            type_parameters: Box::default(),
            parameters: parameters.into_boxed_slice(),
            varargs: false,
            result,
            throws: Box::default(),
            implicit: true,
        };
        let public = Modifiers::PUBLIC;
        methods.push(match *implicit {
            ImplicitMember::DefaultConstructor => {
                let mut m = method("<init>", Vec::new(), None, Modifiers::default());
                m.name = ty.name;
                m
            }
            ImplicitMember::EnumValues => method("values", Vec::new(), Some(own(1)), public),
            ImplicitMember::EnumValueOf => {
                let string = lang(r, "String")?;
                method("valueOf", vec![string], Some(own(0)), public)
            }
            ImplicitMember::CanonicalConstructor => {
                let mut m = method("<init>", components.clone(), None, public);
                m.name = ty.name;
                m.varargs = ty
                    .record_components
                    .last()
                    .is_some_and(|c| c.varargs.is_some());
                m
            }
            ImplicitMember::Accessor(i) => {
                let component = &ty.record_components[i as usize];
                let mut m = method("", Vec::new(), Some(components[i as usize].clone()), public);
                m.name = component.name;
                m
            }
            // The component fields are with the fields.
            ImplicitMember::ComponentField(_) => continue,
            ImplicitMember::RecordEquals => {
                let object = lang(r, "Object")?;
                method(
                    "equals",
                    vec![object],
                    Some(primitive(Primitive::Boolean)),
                    public,
                )
            }
            ImplicitMember::RecordHashCode => method(
                "hashCode",
                Vec::new(),
                Some(primitive(Primitive::Int)),
                public,
            ),
            ImplicitMember::RecordToString => {
                let string = lang(r, "String")?;
                method("toString", Vec::new(), Some(string), public)
            }
        });
    }
    Ok((fields, methods))
}

pub(crate) fn type_params(
    r: &mut Resolver<'_, '_>,
    scope: &Scope<'_>,
    params: &[crate::summary::TypeParameter],
) -> Result<Box<[TypeParam]>, Cancelled> {
    let mut out = Vec::with_capacity(params.len());
    for param in params {
        let mut bounds = Vec::with_capacity(param.bounds.len());
        for bound in param.bounds.iter() {
            bounds.push(r.ty(scope, bound, 0)?);
        }
        out.push(TypeParam {
            name: param.name,
            bounds: bounds.into_boxed_slice(),
        });
    }
    Ok(out.into_boxed_slice())
}

/// Whether an annotation resolved to a type in a `lombok` package.
pub(crate) fn is_lombok(resolved: &TypeResolution, names: &Interner) -> bool {
    let package = match resolved {
        TypeResolution::Type(TypeKey::Source { package, .. }) => names.text(*package).to_string(),
        TypeResolution::Type(TypeKey::Class { binary, .. }) => {
            let text = names.text(*binary);
            text.rsplit_once('/')
                .map_or(String::new(), |(p, _)| p.replace('/', "."))
        }
        _ => return false,
    };
    package == "lombok" || package.starts_with("lombok.")
}
