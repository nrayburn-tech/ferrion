//! File summaries: what other files can see of a file, and nothing that changes when a body or an
//! offset does (docs/design/semantic-names.md#file-summaries).
//!
//! [`summarize`] makes one from a tree, by a declaration-only walk that reads each declaration's
//! direct children and never enters a body. Everything in a summary is owned and holds no
//! offsets or node ids, so it outlives its tree and an edit inside a body, or above a
//! declaration, leaves it equal. Types and annotations are kept as written, unresolved;
//! resolution reads them against the source set's view.

mod build;
mod dump;
#[cfg(test)]
mod tests;
mod written;

use std::fmt;
use std::sync::Arc;

use crate::{Fingerprint, Interner, Name};

pub use build::summarize;
pub(crate) use build::{Header, name_text, qualified};
pub use written::{Primitive, TypeAnnotation, TypePart, TypeRef, WildcardBound};

/// A file's summary.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Summary {
    pub kind: UnitKind,
    /// The package declaration, if any. A file without one is in the unnamed package.
    pub package: Option<PackageDecl>,
    /// In source order.
    pub imports: Vec<Import>,
    /// Every type declared at the top level or as a member of another, in pre-order: a type
    /// comes before its member types, and members in source order. Local and anonymous classes
    /// aren't here; they belong to bodies.
    pub types: Vec<TypeSummary>,
    /// A `module-info.java`'s declaration.
    pub module: Option<ModuleSummary>,
}

/// What kind of compilation unit a file is (JLS 7.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UnitKind {
    /// Type declarations, with an optional package declaration and imports.
    Ordinary,
    /// A compact compilation unit (JEP 512): top-level methods or fields, which with any type
    /// declarations beside them are members of an implicitly declared class
    /// ([`TypeKind::Implicit`], the first type).
    Compact,
    /// A `package-info.java`, whose package declaration may carry annotations.
    PackageInfo,
    /// A `module-info.java`.
    ModuleInfo,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PackageDecl {
    pub name: QualifiedName,
    pub annotations: Box<[Annotation]>,
}

/// A dotted name as written (`java.util.Map`), interned whole, with Unicode escapes in its
/// identifiers translated, so names compare as javac compares them. A segment the parser found
/// missing is empty.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct QualifiedName(pub Name);

impl Default for Name {
    fn default() -> Name {
        Name::EMPTY
    }
}

impl QualifiedName {
    pub fn name(self) -> Name {
        self.0
    }

    pub fn text(self, names: &Interner) -> Arc<str> {
        names.text(self.0)
    }

    /// The segments, each interned.
    pub fn segments(self, names: &Interner) -> Vec<Name> {
        names
            .text(self.0)
            .split('.')
            .map(|segment| names.intern(segment))
            .collect()
    }

    /// The last segment: the simple name.
    pub fn last(self, names: &Interner) -> Name {
        let text = names.text(self.0);
        names.intern(text.rsplit('.').next().unwrap_or_default())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Import {
    pub kind: ImportKind,
    /// The name before any `.*`: `a.b` for `import a.b.*;`, `a.B.m` for `import static a.B.m;`.
    pub name: QualifiedName,
}

/// JLS 7.5.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ImportKind {
    /// `import a.B;`
    SingleType,
    /// `import a.*;`
    OnDemand,
    /// `import static a.B.m;`
    SingleStatic,
    /// `import static a.B.*;`
    StaticOnDemand,
    /// `import module m;` (JLS 7.5.5)
    Module,
}

/// An annotation as written: its name, and a fingerprint of its arguments' tokens (`None` for a
/// marker annotation without parentheses). Arguments are resolved and evaluated from the tree
/// when needed; other files only need to know when they change.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Annotation {
    pub name: QualifiedName,
    pub arguments: Option<Fingerprint>,
}

impl Annotation {
    /// `@Name`, or `@Name(…)` with arguments.
    pub fn display(&self, names: &Interner) -> impl fmt::Display {
        let ellipsis = if self.arguments.is_some() {
            "(…)"
        } else {
            ""
        };
        format!("@{}{ellipsis}", self.name.text(names))
    }
}

/// One `[]` of an array type or a declarator, with its annotations.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Dim {
    pub annotations: Box<[Annotation]>,
}

/// Modifier keywords as written. Implicit modifiers (an interface method's `public abstract`)
/// aren't added; that is resolution's job.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(u16);

impl Modifiers {
    pub const PUBLIC: Modifiers = Modifiers(1 << 0);
    pub const PROTECTED: Modifiers = Modifiers(1 << 1);
    pub const PRIVATE: Modifiers = Modifiers(1 << 2);
    pub const ABSTRACT: Modifiers = Modifiers(1 << 3);
    pub const STATIC: Modifiers = Modifiers(1 << 4);
    pub const FINAL: Modifiers = Modifiers(1 << 5);
    pub const TRANSIENT: Modifiers = Modifiers(1 << 6);
    pub const VOLATILE: Modifiers = Modifiers(1 << 7);
    pub const SYNCHRONIZED: Modifiers = Modifiers(1 << 8);
    pub const NATIVE: Modifiers = Modifiers(1 << 9);
    pub const STRICTFP: Modifiers = Modifiers(1 << 10);
    pub const DEFAULT: Modifiers = Modifiers(1 << 11);
    pub const SEALED: Modifiers = Modifiers(1 << 12);
    pub const NON_SEALED: Modifiers = Modifiers(1 << 13);

    /// Each modifier and its keyword, in the order JLS 8.1.1 and 8.4.3 recommend.
    pub const KEYWORDS: [(Modifiers, &str); 14] = [
        (Modifiers::PUBLIC, "public"),
        (Modifiers::PROTECTED, "protected"),
        (Modifiers::PRIVATE, "private"),
        (Modifiers::ABSTRACT, "abstract"),
        (Modifiers::DEFAULT, "default"),
        (Modifiers::STATIC, "static"),
        (Modifiers::SEALED, "sealed"),
        (Modifiers::NON_SEALED, "non-sealed"),
        (Modifiers::FINAL, "final"),
        (Modifiers::TRANSIENT, "transient"),
        (Modifiers::VOLATILE, "volatile"),
        (Modifiers::SYNCHRONIZED, "synchronized"),
        (Modifiers::NATIVE, "native"),
        (Modifiers::STRICTFP, "strictfp"),
    ];

    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn insert(&mut self, other: Modifiers) {
        self.0 |= other.0;
    }
}

impl fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keywords: Vec<&str> = Modifiers::KEYWORDS
            .iter()
            .filter(|(m, _)| self.contains(*m))
            .map(|(_, k)| *k)
            .collect();
        write!(f, "Modifiers({})", keywords.join(" "))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TypeKind {
    Class,
    Interface,
    Enum,
    Record,
    /// `@interface`
    Annotation,
    /// The implicitly declared class of a compact compilation unit (JLS 7.3). It has no name in
    /// the summary: its name comes from the file's, and no other file can refer to it.
    Implicit,
}

/// A class, interface, enum, record, or annotation interface, top level or a member.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TypeSummary {
    pub kind: TypeKind,
    /// The simple name; empty for [`TypeKind::Implicit`], or if the parser found it missing.
    pub name: Name,
    /// The enclosing type's index in [`Summary::types`]; `None` at the top level.
    pub outer: Option<u32>,
    pub modifiers: Modifiers,
    pub annotations: Box<[Annotation]>,
    pub type_parameters: Box<[TypeParameter]>,
    /// A class's superclass, or an interface's superinterfaces.
    pub extends: Box<[TypeRef]>,
    pub implements: Box<[TypeRef]>,
    pub permits: Box<[TypeRef]>,
    /// One per declarator: `int a, b;` is two fields.
    pub fields: Vec<Field>,
    /// Methods, constructors, and annotation interface elements, in source order.
    pub methods: Vec<Method>,
    pub enum_constants: Vec<EnumConstant>,
    pub record_components: Vec<RecordComponent>,
    /// The members the language declares when the source doesn't.
    pub implicit: Vec<ImplicitMember>,
    /// Annotated with an annotation whose simple name is `Generated`, or a member of such a type
    /// (docs/design/semantic-names.md#generated-sources). Resolved or not.
    pub generated: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TypeParameter {
    pub annotations: Box<[Annotation]>,
    pub name: Name,
    pub bounds: Box<[TypeRef]>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Field {
    pub modifiers: Modifiers,
    pub annotations: Box<[Annotation]>,
    pub ty: TypeRef,
    pub name: Name,
    /// The declarator's own dimensions: `int a[];`.
    pub dims: Box<[Dim]>,
    /// For a field that is `final` (explicitly, or as a field of an interface) and has an
    /// initializer, a fingerprint of the initializer's tokens, from which B3 folds constants.
    /// Other initializers are bodies, and left out.
    pub constant: Option<Fingerprint>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MethodKind {
    Method,
    Constructor,
    /// A record's constructor without a parameter list (JLS 8.10.4.2).
    CompactConstructor,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Method {
    pub kind: MethodKind,
    pub modifiers: Modifiers,
    /// The modifiers' annotations, then those between the type parameters and the result.
    pub annotations: Box<[Annotation]>,
    pub type_parameters: Box<[TypeParameter]>,
    /// The result type; `None` for constructors.
    pub result: Option<TypeRef>,
    pub name: Name,
    /// The formal parameters. A receiver parameter isn't one, and isn't here.
    pub parameters: Box<[Parameter]>,
    /// Dimensions after the parameters: `int m()[]`.
    pub dims: Box<[Dim]>,
    pub throws: Box<[TypeRef]>,
    pub has_body: bool,
    /// An annotation interface element's `default` value, as a fingerprint of its tokens.
    pub default_value: Option<Fingerprint>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Parameter {
    /// `final`, or nothing.
    pub modifiers: Modifiers,
    pub annotations: Box<[Annotation]>,
    pub ty: TypeRef,
    /// For a variable arity parameter, the annotations on its `...`.
    pub varargs: Option<Box<[Annotation]>>,
    pub name: Name,
    /// The parameter's own dimensions: `int x[]`.
    pub dims: Box<[Dim]>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EnumConstant {
    pub annotations: Box<[Annotation]>,
    pub name: Name,
    /// Whether it has a class body, making it an anonymous subclass.
    pub has_body: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RecordComponent {
    pub annotations: Box<[Annotation]>,
    pub ty: TypeRef,
    pub varargs: Option<Box<[Annotation]>>,
    pub name: Name,
}

/// A member the language declares implicitly, because the source doesn't declare it. Their
/// signatures follow from the rule that declares them, so only the rule is recorded.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ImplicitMember {
    /// A class's default constructor (JLS 8.8.9), or an enum's private one (8.9.2).
    DefaultConstructor,
    /// An enum's `static E[] values()` (8.9.3).
    EnumValues,
    /// An enum's `static E valueOf(String)` (8.9.3).
    EnumValueOf,
    /// A record's canonical constructor, when neither it nor a compact constructor is declared
    /// (8.10.4).
    CanonicalConstructor,
    /// The accessor of the record component at this index, when no method of its name without
    /// parameters is declared (8.10.3).
    Accessor(u32),
    /// The private final field of the record component at this index (8.10.3).
    ComponentField(u32),
    /// A record's `public final boolean equals(Object)`, when not declared (8.10.3).
    RecordEquals,
    /// A record's `public final int hashCode()`, when not declared (8.10.3).
    RecordHashCode,
    /// A record's `public final String toString()`, when not declared (8.10.3).
    RecordToString,
}

/// A `module-info.java`'s module declaration (JLS 7.7).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ModuleSummary {
    pub annotations: Box<[Annotation]>,
    pub open: bool,
    pub name: QualifiedName,
    pub directives: Vec<Directive>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Directive {
    Requires {
        transitive: bool,
        is_static: bool,
        module: QualifiedName,
    },
    Exports {
        package: QualifiedName,
        to: Box<[QualifiedName]>,
    },
    Opens {
        package: QualifiedName,
        to: Box<[QualifiedName]>,
    },
    Uses {
        service: QualifiedName,
    },
    Provides {
        service: QualifiedName,
        with: Box<[QualifiedName]>,
    },
}

impl Summary {
    /// The type at `index`'s path: its enclosing types' names and its own, joined by `.`
    /// (`Outer.Inner`). An implicitly declared class's path is empty, and its members' paths
    /// start with `.`.
    pub fn path(&self, index: usize, names: &Interner) -> String {
        self.path_names(index)
            .iter()
            .map(|&n| names.text(n))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// The names on the type at `index`'s path, outermost first.
    pub fn path_names(&self, index: usize) -> Vec<Name> {
        let mut path = Vec::new();
        let mut next = Some(index);
        while let Some(i) = next {
            path.push(self.types[i].name);
            next = self.types[i].outer.map(|o| o as usize);
        }
        path.reverse();
        path
    }

    /// The index of the type at `path` (simple names, outermost first), if the file declares one.
    /// The first if more than one has the path, which is an error javac reports.
    pub fn find(&self, path: &[Name]) -> Option<usize> {
        self.find_in(None, path)
    }

    /// As [`Summary::find`], with `path` starting at the members of the type at `outer`.
    pub fn find_in(&self, outer: Option<u32>, path: &[Name]) -> Option<usize> {
        let mut outer = outer;
        let mut found = None;
        for &name in path {
            let index = self
                .types
                .iter()
                .position(|t| t.outer == outer && t.name == name && t.kind != TypeKind::Implicit)?;
            outer = Some(index as u32);
            found = Some(index);
        }
        found
    }

    /// The member types of the type at `index`, in order.
    pub fn members_of(&self, index: usize) -> impl Iterator<Item = (usize, &TypeSummary)> {
        let outer = Some(index as u32);
        self.types
            .iter()
            .enumerate()
            .skip(index + 1)
            .filter(move |(_, t)| t.outer == outer)
    }

    /// The types declared at the top level, which other files can name by package: none for a
    /// compact compilation unit, whose class can't be named.
    pub fn top_level_types(&self) -> impl Iterator<Item = &TypeSummary> {
        self.types
            .iter()
            .filter(|t| t.outer.is_none() && t.kind != TypeKind::Implicit)
    }

    /// The package name, empty for the unnamed package.
    pub fn package_name(&self) -> Name {
        self.package.as_ref().map_or(Name::EMPTY, |p| p.name.0)
    }
}
