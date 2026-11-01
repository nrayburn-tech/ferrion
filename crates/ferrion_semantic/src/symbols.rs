//! Symbols: keys for types, resolved types, error symbols with their root causes, and the type
//! entries of source and class-file types (docs/design/semantic-names.md#symbols).
//!
//! A type is identified by a [`TypeKey`]: a source type by the source set whose sources declare it,
//! its package, and its path of simple names (`Outer.Inner`), which, unlike a binary name, can't
//! be confused with a top-level type whose name has a `$`; a class-file type by where its class
//! file came from and its binary name. A source set's view maps a name to one key, so two source
//! sets that see `com.acme.Foo` in the same place share its symbol, and a class in a jar or the
//! platform that several source sets use is one symbol.
//!
//! Types resolved from source ([`Ty`]) are flat, like types as written: their parts in pre-order,
//! each with a count of children, so a deeply nested type is never recursed over. A class-file
//! type's types name classes by binary name ([`TyPart::Binary`]), resolved in the view of
//! whoever reads them, as javac resolves a class file's names against the classpath it compiles
//! with ([`crate::Snapshot::resolve_binary`]).

use std::fmt;
use std::sync::Arc;

use crate::summary::{MethodKind, Modifiers, Primitive, TypeKind, WildcardBound};
use crate::{FileId, Interner, Name, Path, SourceSetId};

/// A class source: a platform API or a classpath entry, numbered across the engine, so that a jar
/// several source sets use is one source.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ClassSourceId(pub(crate) u32);

impl ClassSourceId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A type's identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum TypeKey {
    /// A class, interface, enum, record, or annotation interface declared in `set`'s own sources,
    /// top level or a member: `path` is its simple names, outermost first.
    Source {
        set: SourceSetId,
        package: Name,
        path: Path,
    },
    /// A compact compilation unit's implicitly declared class (`path` `None`) or a member of it,
    /// which no other file can name (JLS 7.3).
    Compact {
        set: SourceSetId,
        file: FileId,
        path: Option<Path>,
    },
    /// A class file, by where it came from and its binary name in internal form
    /// (`java/util/Map$Entry`).
    Class { source: ClassSourceId, binary: Name },
}

impl TypeKey {
    pub fn is_source(self) -> bool {
        !matches!(self, TypeKey::Class { .. })
    }

    /// The source set whose view resolves the type's own names: its declaring set for a source
    /// type, and `view` for a class file.
    pub(crate) fn home(self, view: SourceSetId) -> SourceSetId {
        match self {
            TypeKey::Source { set, .. } | TypeKey::Compact { set, .. } => set,
            TypeKey::Class { .. } => view,
        }
    }
}

/// A resolved type: a type as written with its names resolved, or a class file's type with its
/// names as binary names. Flat, its parts in pre-order (see [`crate::summary::TypeRef`]).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Ty {
    pub(crate) parts: Box<[TyPart]>,
}

/// One part of a [`Ty`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TyPart {
    Primitive(Primitive),
    Void,
    /// A class or interface type. Its children are the type it is a member of, when that was
    /// written parameterized (`Outer<String>.Inner`), then its `arguments` type arguments
    /// ([`TyPart::NO_ARGUMENTS`] for a raw type or one written without `<...>`).
    Class {
        ty: TypeKey,
        outer: bool,
        arguments: u32,
    },
    /// A class named in a class file, by binary name, not yet resolved in a view. Children as for
    /// [`TyPart::Class`].
    Binary {
        binary: Name,
        outer: bool,
        arguments: u32,
    },
    /// A type variable: the `index`th type parameter of `owner`.
    TypeVar {
        owner: TypeVarOwner,
        index: u32,
    },
    /// A type variable a class file names that its owner and enclosing classes don't declare.
    UnknownTypeVar {
        name: Name,
    },
    /// An array with `dims` dimensions. One child, the element type.
    Array {
        dims: u32,
    },
    /// A wildcard. One child, the bound, unless `bound` is `None`.
    Wildcard {
        bound: Option<WildcardBound>,
    },
    /// `A & B`. Its children are the `types` types.
    Intersection {
        types: u32,
    },
    /// `A | B`. Its children are the `types` types.
    Union {
        types: u32,
    },
    /// A name that didn't resolve: the error, by its index in the entry's errors. No children;
    /// the type arguments of a class type that didn't resolve are left out.
    Error(u32),
}

/// What declares a type variable.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum TypeVarOwner {
    Type(TypeKey),
    /// The method at this index in its type's entry.
    Method(TypeKey, u32),
}

impl TyPart {
    pub const NO_ARGUMENTS: u32 = u32::MAX;

    pub fn arity(&self) -> usize {
        match *self {
            TyPart::Class {
                outer, arguments, ..
            }
            | TyPart::Binary {
                outer, arguments, ..
            } => {
                usize::from(outer)
                    + if arguments == TyPart::NO_ARGUMENTS {
                        0
                    } else {
                        arguments as usize
                    }
            }
            TyPart::Array { .. } => 1,
            TyPart::Wildcard { bound } => usize::from(bound.is_some()),
            TyPart::Intersection { types } | TyPart::Union { types } => types as usize,
            TyPart::Primitive(_)
            | TyPart::Void
            | TyPart::TypeVar { .. }
            | TyPart::UnknownTypeVar { .. }
            | TyPart::Error(_) => 0,
        }
    }
}

impl Ty {
    pub fn parts(&self) -> &[TyPart] {
        &self.parts
    }

    pub(crate) fn of(parts: Vec<TyPart>) -> Ty {
        Ty {
            parts: parts.into_boxed_slice(),
        }
    }

    /// The class at the top of the type, if it is a class type: `Map` for `Map<K, V>`.
    pub fn class(&self) -> Option<TypeKey> {
        match self.parts.first() {
            Some(TyPart::Class { ty, .. }) => Some(*ty),
            _ => None,
        }
    }

    /// The error at the top of the type, if its name didn't resolve.
    pub fn error(&self) -> Option<u32> {
        match self.parts.first() {
            Some(TyPart::Error(e)) => Some(*e),
            _ => None,
        }
    }
}

/// Why a name didn't resolve (docs/design/semantic-names.md#errors-and-incompleteness).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Cause {
    /// No type, package, or member of the name is in scope.
    NotFound,
    /// A name qualified by a package no view has (`package nope does not exist`).
    NoPackage { package: Name },
    /// More than one type of the name is in scope at the same level: two on-demand imports, two
    /// module imports, two inherited member types, or a type declared in more than one of a
    /// source set's own files.
    Ambiguous(Box<[TypeKey]>),
    /// The name is in a package the reading module can't see (`package jdk.internal.misc is not
    /// visible`).
    NotVisible { package: Name },
    /// `import module M` of a module the reading module doesn't read.
    ModuleNotRead { module: Name },
    /// `import module M` of a module no view has.
    NoModule { module: Name },
    /// The type is in a cycle of supertypes (`cyclic inheritance involving A`).
    Cycle,
    /// A lookup found nothing in a type whose hierarchy is incomplete, so the name may be one of
    /// the members it would have had.
    Incomplete { ty: TypeKey },
    /// A class file the view lists couldn't be read or is malformed: B1's error.
    BadClass { binary: Name, message: Arc<str> },
    /// The parser found the name missing.
    Syntax,
    /// A qualified name of more segments than are resolved (`resolve::MAX_SEGMENTS`, 1,024).
    TooLong,
}

/// An error symbol: the name that failed and why. Where is known by what holds it: an entry's
/// errors are in its declaration, and a file's names by node. A name whose resolution reads an
/// error symbol (a member of a type that didn't resolve) holds the same error, so that one
/// missing type is one root cause.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ErrorSymbol {
    /// The name as written, dotted if it was qualified.
    pub name: Name,
    pub cause: Cause,
}

/// What a name in a type context resolved to.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeResolution {
    Type(TypeKey),
    TypeVar(TypeVarOwner, u32),
    Package(Name),
    Error(ErrorSymbol),
}

/// A source type's or class file's direct supertypes, erased: what the hierarchy is made of,
/// resolved before anything that needs its members.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Supertypes {
    /// `None` for `java.lang.Object`, interfaces' implicit `Object` aside.
    pub superclass: Option<Result<TypeKey, ErrorSymbol>>,
    pub interfaces: Box<[Result<TypeKey, ErrorSymbol>]>,
    /// For a type in a cycle of supertypes, the source types in it, sorted: every supertype
    /// above is then an error with [`Cause::Cycle`]. Empty otherwise.
    pub cycle: Box<[TypeKey]>,
}

impl Supertypes {
    pub fn cyclic(&self) -> bool {
        !self.cycle.is_empty()
    }
}

/// A type's member types by simple name, declared and inherited (JLS 8.5, 9.5): a declared one
/// hides every inherited one of its name, private ones aren't inherited, and one inherited from
/// two different supertypes is ambiguous.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct MemberTypes {
    /// Sorted by name id, each with who can inherit it.
    pub(crate) members: Box<[(Name, (MemberType, MemberAccess))]>,
    /// Some supertype didn't resolve, is in a cycle, or has an incomplete hierarchy itself, so the
    /// type may have members this doesn't list.
    pub incomplete: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MemberType {
    One(TypeKey),
    /// Inherited from two or more supertypes as different types.
    Ambiguous(Box<[TypeKey]>),
}

/// Who can inherit a member type (JLS 8.5: the subclasses it is accessible to).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MemberAccess {
    Public,
    /// Package access: only subclasses in this package inherit it.
    Package(Name),
    Private,
}

impl MemberTypes {
    pub fn get(&self, name: Name) -> Option<&(MemberType, MemberAccess)> {
        let index = self.members.binary_search_by(|(n, _)| n.cmp(&name)).ok()?;
        Some(&self.members[index].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Name, &MemberType)> {
        self.members.iter().map(|(n, (m, _))| (*n, m))
    }
}

/// A source type's resolved header: what other files and bodies need of it
/// (docs/design/semantic-incremental.md#project-symbols).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TypeEntry {
    pub key: TypeKey,
    pub kind: TypeKind,
    /// As written; implicit modifiers aren't added.
    pub modifiers: Modifiers,
    pub annotations: Box<[TypeResolution]>,
    pub type_parameters: Box<[TypeParam]>,
    /// The generic supertypes: `None` when no `extends` clause is written.
    pub superclass: Option<Ty>,
    pub interfaces: Box<[Ty]>,
    pub permits: Box<[Ty]>,
    /// Fields, then enum constants, then a record's component fields, in order.
    pub fields: Box<[FieldEntry]>,
    /// Methods and constructors, then the implicit ones.
    pub methods: Box<[MethodEntry]>,
    /// The type's supertypes or theirs didn't all resolve, or are cyclic, or it is
    /// Lombok-annotated: rules treat its members, overrides, and subtyping as unknown.
    pub incomplete: bool,
    /// Annotated with an annotation from a `lombok` package, on the type or a field, so Lombok
    /// may add members (B4).
    pub lombok: bool,
    /// Generated: under a generated-source root, or annotated `@Generated`.
    pub generated: bool,
    /// The errors in the header, by index from [`TyPart::Error`]; the root causes B5 reports.
    pub errors: Box<[ErrorSymbol]>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TypeParam {
    pub name: Name,
    pub bounds: Box<[Ty]>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FieldEntry {
    pub name: Name,
    pub ty: Ty,
    pub modifiers: Modifiers,
    pub kind: FieldKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FieldKind {
    Field,
    EnumConstant,
    /// A record component's implicit field.
    Component,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MethodEntry {
    pub kind: MethodKind,
    pub name: Name,
    pub modifiers: Modifiers,
    pub type_parameters: Box<[TypeParam]>,
    pub parameters: Box<[Ty]>,
    pub varargs: bool,
    /// `None` for a constructor.
    pub result: Option<Ty>,
    pub throws: Box<[Ty]>,
    /// Declared by the language rather than the source: a default constructor, `values`, an
    /// accessor.
    pub implicit: bool,
}

/// A class file's type, as read: its names unresolved binary names, the same for every view.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ClassEntry {
    pub binary: Name,
    /// The class's own access flags, with an `InnerClasses` entry's for a member class.
    pub access: u16,
    /// The simple name: from `InnerClasses` for a member class, else after the package.
    pub simple: Name,
    /// The class it is a member of, from `InnerClasses`.
    pub outer: Option<Name>,
    /// Local or anonymous, so no other code can name it.
    pub local: bool,
    /// A record: a final class extending `java.lang.Record`, with a `Record` attribute.
    pub record: bool,
    pub type_parameters: Box<[ClassTypeParam]>,
    pub superclass: Option<Name>,
    pub interfaces: Box<[Name]>,
    /// The generic supertypes from `Signature`, or the erased ones.
    pub generic_superclass: Option<Ty>,
    pub generic_interfaces: Box<[Ty]>,
    /// The member classes `InnerClasses` lists as members of this one, with their simple names and
    /// flags.
    pub member_types: Box<[(Name, Name, u16)]>,
    pub fields: Box<[ClassField]>,
    /// Without synthetic and bridge methods.
    pub methods: Box<[ClassMethod]>,
    /// What went wrong reading it, if anything; what could be read is kept.
    pub problem: Option<Arc<str>>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ClassTypeParam {
    pub name: Name,
    pub bounds: Box<[Ty]>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ClassField {
    pub name: Name,
    pub access: u16,
    pub ty: Ty,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ClassMethod {
    pub name: Name,
    pub access: u16,
    /// The descriptor, for the method's key.
    pub descriptor: Name,
    pub type_parameters: Box<[ClassTypeParam]>,
    pub parameters: Box<[Ty]>,
    /// `None` for `void`.
    pub result: Option<Ty>,
    pub throws: Box<[Ty]>,
}

impl ClassEntry {
    pub fn kind(&self) -> TypeKind {
        use ferrion_classfile::class::AccessFlags as F;
        let flags = F(self.access);
        if flags.contains(F::ANNOTATION) {
            TypeKind::Annotation
        } else if flags.contains(F::INTERFACE) {
            TypeKind::Interface
        } else if flags.contains(F::ENUM) {
            TypeKind::Enum
        } else if self.record {
            TypeKind::Record
        } else {
            TypeKind::Class
        }
    }
}

/// A symbol's key for the javac comparison (docs/design/semantic.md, "The javac oracle"), as
/// text: `package java.util`, `type java/util/Map$Entry class <entry>`, `typevar <owner> T`,
/// `field p/A.f`, `method p/A.m(I)V`, `module java.base`, or `error <why> <name>`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SymbolKey(pub String);

impl fmt::Display for SymbolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Cause {
    /// A word for the kind of cause, for keys and dumps.
    pub fn describe(&self, names: &Interner) -> String {
        match self {
            Cause::NotFound => "not-found".to_owned(),
            Cause::NoPackage { package } => format!("no-package {}", names.text(*package)),
            Cause::Ambiguous(types) => format!("ambiguous {}", types.len()),
            Cause::NotVisible { package } => format!("not-visible {}", names.text(*package)),
            Cause::ModuleNotRead { module } => format!("not-read {}", names.text(*module)),
            Cause::NoModule { module } => format!("no-module {}", names.text(*module)),
            Cause::Cycle => "cycle".to_owned(),
            Cause::Incomplete { .. } => "incomplete".to_owned(),
            Cause::BadClass { binary, .. } => format!("bad-class {}", names.text(*binary)),
            Cause::Syntax => "syntax".to_owned(),
            Cause::TooLong => "too-long".to_owned(),
        }
    }
}
