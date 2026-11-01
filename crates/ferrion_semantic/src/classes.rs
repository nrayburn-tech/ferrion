//! Class sources: the platform APIs and classpath entries of every source set, numbered across
//! the engine, with their package listings and class entries cached
//! (docs/design/semantic-names.md#class-file-types).
//!
//! Class sources don't change while the engine runs (a jar rebuilt on disk is a configuration
//! change, which makes a new engine), so what is read from them is cached without dependencies
//! and never needs verifying: a package's listing, and each class file read as a
//! [`ClassEntry`]. A class's names stay binary names, resolved by whoever reads them in their own
//! view, so one class entry serves every source set.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::{Arc, RwLock};

use ferrion_classfile::class::{AccessFlags, Attribute, ClassFile};
use ferrion_classfile::descriptor::{self, BaseType, ElementType, FieldType};
use ferrion_classfile::signature::{
    self, ClassTypeSignature, TypeArgument, TypeParameter as SigTypeParameter, TypeSignature,
};
use ferrion_classfile::{ClassName, PackageListing, PackageName};

use crate::engine::derived::{read, write};
use crate::input::{ClassesInput, Platform};
use crate::summary::{Primitive, WildcardBound};
use crate::symbols::{
    ClassEntry, ClassField, ClassMethod, ClassSourceId, ClassTypeParam, Ty, TyPart, TypeKey,
    TypeVarOwner,
};
use crate::{Interner, Name};

pub(crate) enum ClassSource {
    Platform(Arc<dyn Platform>),
    Entry(ClassesInput),
}

impl ClassSource {
    fn package(&self, name: &PackageName) -> ferrion_classfile::Result<Option<PackageListing>> {
        match self {
            ClassSource::Platform(platform) => platform.package(name),
            ClassSource::Entry(entry) => entry.entry.package(name),
        }
    }

    fn find(&self, name: &ClassName) -> ferrion_classfile::Result<Option<Arc<[u8]>>> {
        Ok(match self {
            ClassSource::Platform(platform) => platform.find(name)?.map(|c| c.bytes),
            ClassSource::Entry(entry) => entry.entry.find(name)?.map(|c| c.bytes),
        })
    }

    /// Whether `other` is the same platform or entry, so that it is numbered once.
    fn same(&self, other: &ClassSource) -> bool {
        match (self, other) {
            (ClassSource::Platform(a), ClassSource::Platform(b)) => {
                std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
            }
            (ClassSource::Entry(a), ClassSource::Entry(b)) => {
                match (a.entry.as_jar(), b.entry.as_jar()) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => a.path == b.path,
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

/// A package of a class source: its classes' names after the package (`Map$Entry` included), or
/// why it couldn't be listed.
pub(crate) type Listing = Result<Option<PackageListing>, Arc<str>>;

pub(crate) struct ClassSources {
    sources: Box<[ClassSource]>,
    listings: Cache<(ClassSourceId, Name), Arc<Listing>>,
    entries: Cache<(ClassSourceId, Name), Option<Arc<ClassEntry>>>,
}

impl ClassSources {
    pub(crate) fn new() -> ClassSources {
        ClassSources {
            sources: Box::default(),
            listings: Cache::new(),
            entries: Cache::new(),
        }
    }

    /// Numbers `source`, or finds its number if it is one already.
    pub(crate) fn add(sources: &mut Vec<ClassSource>, source: ClassSource) -> ClassSourceId {
        if let Some(index) = sources.iter().position(|s| s.same(&source)) {
            return ClassSourceId(index as u32);
        }
        sources.push(source);
        ClassSourceId(sources.len() as u32 - 1)
    }

    pub(crate) fn with_sources(sources: Vec<ClassSource>) -> ClassSources {
        ClassSources {
            sources: sources.into_boxed_slice(),
            ..ClassSources::new()
        }
    }

    pub(crate) fn source(&self, id: ClassSourceId) -> &ClassSource {
        &self.sources[id.index()]
    }

    /// The listing of `package` (dotted, as interned) in `source`.
    pub(crate) fn listing(
        &self,
        source: ClassSourceId,
        package: Name,
        names: &Interner,
    ) -> Arc<Listing> {
        self.listings.get_or_insert((source, package), || {
            let internal = names.text(package).replace('.', "/");
            Arc::new(
                self.source(source)
                    .package(&PackageName::new(&internal))
                    .map_err(|e| Arc::from(e.to_string())),
            )
        })
    }

    /// Whether `source` has a class file named `simple` (after the package) in `package`.
    pub(crate) fn has(
        &self,
        source: ClassSourceId,
        package: Name,
        simple: &str,
        names: &Interner,
    ) -> Result<bool, Arc<str>> {
        match &*self.listing(source, package, names) {
            Ok(Some(listing)) => Ok(listing.contains(simple)),
            Ok(None) => Ok(false),
            Err(e) => Err(e.clone()),
        }
    }

    /// The class `binary` (internal form) of `source`, read once. `None` if the source has no
    /// such class file; an entry with a `problem` if it couldn't be read.
    pub(crate) fn entry(
        &self,
        source: ClassSourceId,
        binary: Name,
        names: &Interner,
    ) -> Option<Arc<ClassEntry>> {
        self.entries.get_or_insert((source, binary), || {
            let key = TypeKey::Class { source, binary };
            match self
                .source(source)
                .find(&ClassName::new(&names.text(binary)))
            {
                Ok(None) => None,
                Ok(Some(bytes)) => Some(Arc::new(read_class(key, binary, bytes, names))),
                Err(e) => Some(Arc::new(broken(binary, &e.to_string(), names))),
            }
        })
    }

    /// How many class entries have been read, for measuring memory.
    pub(crate) fn entries_read(&self) -> usize {
        self.entries.len()
    }
}

/// A map filled once per key and never invalidated, sharded like the memo tables.
pub(crate) struct Cache<K, V> {
    shards: Box<[RwLock<HashMap<K, V>>]>,
    hasher: RandomState,
}

impl<K: Clone + Eq + Hash, V: Clone> Cache<K, V> {
    pub(crate) fn new() -> Self {
        Cache {
            shards: (0..64).map(|_| RwLock::default()).collect(),
            hasher: RandomState::new(),
        }
    }

    /// The value for `key`, from `make` the first time. Two threads may both make it; the first
    /// stored is kept.
    pub(crate) fn get_or_insert(&self, key: K, make: impl FnOnce() -> V) -> V {
        let shard = &self.shards[self.hasher.hash_one(&key) as usize % self.shards.len()];
        if let Some(value) = read(shard).get(&key) {
            return value.clone();
        }
        let value = make();
        write(shard).entry(key).or_insert(value).clone()
    }

    pub(crate) fn len(&self) -> usize {
        self.shards.iter().map(|s| read(s).len()).sum()
    }
}

/// An entry for a class that couldn't be read at all.
fn broken(binary: Name, message: &str, names: &Interner) -> ClassEntry {
    let text = names.text(binary);
    ClassEntry {
        binary,
        access: AccessFlags::PUBLIC,
        simple: names.intern(text.rsplit('/').next().unwrap_or_default()),
        outer: None,
        local: false,
        record: false,
        type_parameters: Box::default(),
        superclass: None,
        interfaces: Box::default(),
        generic_superclass: None,
        generic_interfaces: Box::default(),
        member_types: Box::default(),
        fields: Box::default(),
        methods: Box::default(),
        problem: Some(Arc::from(message)),
    }
}

/// Reads a class file as an entry. A part that is malformed is left out, and the first problem
/// is kept; the class's own name must be readable.
fn read_class(key: TypeKey, binary: Name, bytes: Arc<[u8]>, names: &Interner) -> ClassEntry {
    let class = match ClassFile::parse(bytes) {
        Ok(class) => class,
        Err(e) => return broken(binary, &e.to_string(), names),
    };
    let mut problem: Option<Arc<str>> = None;
    let mut note = |e: ferrion_classfile::Error| {
        problem.get_or_insert_with(|| Arc::from(e.to_string()));
    };
    let binary_text = names.text(binary);
    let mut entry = broken(binary, "", names);
    entry.problem = None;
    entry.access = class.access().0;
    match class.super_class() {
        Ok(name) => entry.superclass = name.and_then(|n| n.to_str().ok().map(|t| names.intern(&t))),
        Err(e) => note(e),
    }
    let mut interfaces = Vec::new();
    for interface in class.interfaces() {
        match interface.and_then(|n| n.to_str().map(|t| names.intern(&t))) {
            Ok(name) => interfaces.push(name),
            Err(e) => note(e),
        }
    }
    entry.interfaces = interfaces.into_boxed_slice();
    let mut member_types = Vec::new();
    let mut is_record = false;
    let mut signature_text = None;
    for info in class.attributes() {
        let Ok(name) = info.name() else { continue };
        if name.eq_str("InnerClasses") {
            match info.decode() {
                Ok(Attribute::InnerClasses(inner)) => {
                    for item in inner {
                        let item = match item {
                            Ok(item) => item,
                            Err(e) => {
                                note(e);
                                continue;
                            }
                        };
                        let Ok(inner_name) = item.inner.to_str() else {
                            continue;
                        };
                        let simple = item
                            .name
                            .and_then(|n| n.to_str().ok().map(|t| names.intern(&t)));
                        let outer = item
                            .outer
                            .and_then(|n| n.to_str().ok().map(|t| names.intern(&t)));
                        if *inner_name == *binary_text {
                            // This class's own entry: a member, local, or anonymous class.
                            entry.access = item.access.0 | (entry.access & AccessFlags::SUPER);
                            entry.outer = outer;
                            entry.local = outer.is_none();
                            if let Some(simple) = simple {
                                entry.simple = simple;
                            }
                        } else if outer == Some(binary)
                            && let Some(simple) = simple
                            && !item.access.contains(AccessFlags::SYNTHETIC)
                        {
                            member_types.push((simple, names.intern(&inner_name), item.access.0));
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => note(e),
            }
        } else if name.eq_str("Record") {
            is_record = true;
        } else if name.eq_str("Signature") {
            match info.decode() {
                Ok(Attribute::Signature(text)) => signature_text = text.to_str().ok(),
                Ok(_) => {}
                Err(e) => note(e),
            }
        }
    }
    entry.member_types = member_types.into_boxed_slice();
    entry.record = is_record;

    let owner = TypeVarOwner::Type(key);
    let mut class_vars: Vec<Name> = Vec::new();
    let erased = |name: Name| {
        Ty::of(vec![TyPart::Binary {
            binary: name,
            outer: false,
            arguments: TyPart::NO_ARGUMENTS,
        }])
    };
    if let Some(text) = &signature_text {
        match signature::parse_class(text) {
            Ok(sig) => {
                class_vars = sig
                    .type_parameters
                    .iter()
                    .map(|p| names.intern(p.name))
                    .collect();
                let vars = Vars {
                    class: (owner, &class_vars),
                    method: None,
                };
                entry.type_parameters = type_params(&sig.type_parameters, &vars, names);
                entry.generic_superclass = Some(class_ty(&sig.superclass, &vars, names));
                entry.generic_interfaces = sig
                    .interfaces
                    .iter()
                    .map(|i| class_ty(i, &vars, names))
                    .collect();
            }
            Err(e) => note(e),
        }
    }
    if signature_text.is_none() || entry.generic_superclass.is_none() {
        entry.generic_superclass = entry.superclass.map(erased);
        entry.generic_interfaces = entry.interfaces.iter().map(|&i| erased(i)).collect();
    }

    let mut fields = Vec::new();
    for field in class.fields() {
        let access = field.access();
        if access.contains(AccessFlags::SYNTHETIC) {
            continue;
        }
        let read = || -> ferrion_classfile::Result<ClassField> {
            let name = names.intern(&field.name()?.to_str()?);
            let vars = Vars {
                class: (owner, &class_vars),
                method: None,
            };
            let ty = match field.signature()? {
                Some(sig) => {
                    let text = sig.to_str()?;
                    flatten(&signature::parse_field(&text)?, &vars, names)
                }
                None => {
                    let text = field.descriptor()?.to_str()?;
                    field_type(descriptor::parse_field(&text)?, names)
                }
            };
            Ok(ClassField {
                name,
                access: access.0,
                ty,
            })
        };
        match read() {
            Ok(field) => fields.push(field),
            Err(e) => note(e),
        }
    }
    entry.fields = fields.into_boxed_slice();

    let mut methods = Vec::new();
    for method in class.methods() {
        let access = method.access();
        if access.contains(AccessFlags::SYNTHETIC) || access.contains(AccessFlags::BRIDGE) {
            continue;
        }
        let index = methods.len() as u32;
        let read = || -> ferrion_classfile::Result<ClassMethod> {
            let name = names.intern(&method.name()?.to_str()?);
            let descriptor_text = method.descriptor()?.to_str()?.into_owned();
            let parsed = descriptor::parse_method(&descriptor_text)?;
            let mut result = ClassMethod {
                name,
                access: access.0,
                descriptor: names.intern(&descriptor_text),
                type_parameters: Box::default(),
                parameters: parsed
                    .parameters
                    .iter()
                    .map(|&p| field_type(p, names))
                    .collect(),
                result: parsed.result.map(|r| field_type(r, names)),
                throws: Box::default(),
            };
            let mut throws: Vec<Ty> = Vec::new();
            for attribute in method.attributes() {
                if let Ok(Attribute::Exceptions(refs)) = attribute.decode() {
                    for name in refs {
                        throws.push(erased(names.intern(&name?.to_str()?)));
                    }
                }
            }
            result.throws = throws.into_boxed_slice();
            if let Some(sig) = method.signature()? {
                let text = sig.to_str()?;
                let sig = signature::parse_method(&text)?;
                let method_vars: Vec<Name> = sig
                    .type_parameters
                    .iter()
                    .map(|p| names.intern(p.name))
                    .collect();
                let vars = Vars {
                    class: (owner, &class_vars),
                    method: Some((TypeVarOwner::Method(key, index), &method_vars)),
                };
                result.type_parameters = type_params(&sig.type_parameters, &vars, names);
                // A signature may leave out synthetic leading parameters (an inner class's outer
                // instance); the descriptor's count is kept then.
                if sig.parameters.len() == result.parameters.len() {
                    result.parameters = sig
                        .parameters
                        .iter()
                        .map(|p| flatten(p, &vars, names))
                        .collect();
                }
                result.result = sig.result.as_ref().map(|r| flatten(r, &vars, names));
                if !sig.throws.is_empty() {
                    result.throws = sig
                        .throws
                        .iter()
                        .map(|t| flatten(t, &vars, names))
                        .collect();
                }
            }
            Ok(result)
        };
        match read() {
            Ok(method) => methods.push(method),
            Err(e) => note(e),
        }
    }
    entry.methods = methods.into_boxed_slice();
    entry.problem = problem;
    entry
}

/// The type variables in scope in a signature: the class's, and the method's.
struct Vars<'a> {
    class: (TypeVarOwner, &'a [Name]),
    method: Option<(TypeVarOwner, &'a [Name])>,
}

impl Vars<'_> {
    fn part(&self, name: Name) -> TyPart {
        for (owner, vars) in self.method.iter().chain([&self.class]) {
            if let Some(index) = vars.iter().position(|&v| v == name) {
                return TyPart::TypeVar {
                    owner: *owner,
                    index: index as u32,
                };
            }
        }
        TyPart::UnknownTypeVar { name }
    }
}

fn type_params(
    params: &[SigTypeParameter<'_>],
    vars: &Vars<'_>,
    names: &Interner,
) -> Box<[ClassTypeParam]> {
    params
        .iter()
        .map(|p| ClassTypeParam {
            name: names.intern(p.name),
            bounds: p
                .class_bound
                .iter()
                .chain(&p.interface_bounds)
                .map(|b| flatten(b, vars, names))
                .collect(),
        })
        .collect()
}

fn primitive(base: BaseType) -> Primitive {
    match base {
        BaseType::Byte => Primitive::Byte,
        BaseType::Char => Primitive::Char,
        BaseType::Double => Primitive::Double,
        BaseType::Float => Primitive::Float,
        BaseType::Int => Primitive::Int,
        BaseType::Long => Primitive::Long,
        BaseType::Short => Primitive::Short,
        BaseType::Boolean => Primitive::Boolean,
    }
}

fn field_type(ty: FieldType<'_>, names: &Interner) -> Ty {
    let mut parts = Vec::with_capacity(2);
    if ty.dimensions > 0 {
        parts.push(TyPart::Array {
            dims: u32::from(ty.dimensions),
        });
    }
    parts.push(match ty.element {
        ElementType::Base(base) => TyPart::Primitive(primitive(base)),
        ElementType::Object(name) => TyPart::Binary {
            binary: names.intern(name),
            outer: false,
            arguments: TyPart::NO_ARGUMENTS,
        },
    });
    Ty::of(parts)
}

fn class_ty(sig: &ClassTypeSignature<'_>, vars: &Vars<'_>, names: &Interner) -> Ty {
    flatten(&TypeSignature::Class(sig.clone()), vars, names)
}

/// A signature's type as flat parts, from an explicit stack: signatures nest up to
/// `signature::MAX_NESTING` deep.
fn flatten(sig: &TypeSignature<'_>, vars: &Vars<'_>, names: &Interner) -> Ty {
    enum Work<'s, 'a> {
        Sig(&'s TypeSignature<'a>),
        Arg(&'s TypeArgument<'a>),
        /// The class type made of the segments up to and including this index.
        Segments(&'s ClassTypeSignature<'a>, usize),
    }
    let mut parts = Vec::new();
    let mut stack = vec![Work::Sig(sig)];
    while let Some(work) = stack.pop() {
        match work {
            Work::Sig(TypeSignature::Base(base)) => parts.push(TyPart::Primitive(primitive(*base))),
            Work::Sig(TypeSignature::TypeVariable(name)) => {
                parts.push(vars.part(names.intern(name)))
            }
            Work::Sig(TypeSignature::Array {
                dimensions,
                element,
            }) => {
                parts.push(TyPart::Array {
                    dims: u32::from(*dimensions),
                });
                stack.push(Work::Sig(element));
            }
            Work::Sig(TypeSignature::Class(class)) if class.segments.is_empty() => {
                parts.push(TyPart::Error(0))
            }
            Work::Sig(TypeSignature::Class(class)) => {
                stack.push(Work::Segments(class, class.segments.len() - 1))
            }
            Work::Arg(TypeArgument::Any) => parts.push(TyPart::Wildcard { bound: None }),
            Work::Arg(TypeArgument::Extends(t)) => {
                parts.push(TyPart::Wildcard {
                    bound: Some(WildcardBound::Extends),
                });
                stack.push(Work::Sig(t));
            }
            Work::Arg(TypeArgument::Super(t)) => {
                parts.push(TyPart::Wildcard {
                    bound: Some(WildcardBound::Super),
                });
                stack.push(Work::Sig(t));
            }
            Work::Arg(TypeArgument::Exact(t)) => stack.push(Work::Sig(t)),
            Work::Segments(class, last) => {
                let mut binary = String::new();
                for (i, segment) in class.segments[..=last].iter().enumerate() {
                    if i > 0 {
                        binary.push('$');
                    }
                    binary.push_str(segment.name);
                }
                let outer = class.segments[..last]
                    .iter()
                    .any(|s| !s.arguments.is_empty());
                let arguments = &class.segments[last].arguments;
                parts.push(TyPart::Binary {
                    binary: names.intern(&binary),
                    outer,
                    arguments: if arguments.is_empty() {
                        TyPart::NO_ARGUMENTS
                    } else {
                        arguments.len() as u32
                    },
                });
                for argument in arguments.iter().rev() {
                    stack.push(Work::Arg(argument));
                }
                if outer {
                    stack.push(Work::Segments(class, last - 1));
                }
            }
        }
    }
    Ty::of(parts)
}
