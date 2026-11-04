//! One view over a type's declaration, whether it comes from source (a [`TypeEntry`]) or a class
//! file (a [`ClassEntry`]): its kind, type parameters, generic supertypes, fields, and methods,
//! as the relations read them.

use std::sync::Arc;

use ferrion_classfile::class::AccessFlags as F;

use crate::Name;
use crate::summary::{MethodKind, Modifiers, TypeKind};
use crate::symbols::{ClassEntry, ClassTypeParam, ErrorSymbol, Ty, TypeEntry, TypeKey, TypeParam};

/// Where a declaration was read from.
#[derive(Debug)]
pub(crate) enum Body {
    Entry(Arc<TypeEntry>),
    Class(Arc<ClassEntry>),
}

/// A type's declaration.
#[derive(Debug)]
pub(crate) struct Decl {
    pub key: TypeKey,
    pub kind: TypeKind,
    pub body: Body,
    /// The type it is a member of, if it is a member type.
    pub enclosing: Option<TypeKey>,
    /// A member class with an enclosing instance (JLS 8.1.3): its type has its enclosing class's
    /// type as its outer type.
    pub inner: bool,
    /// For a local or anonymous class, the class whose body declares it.
    pub lexical: Option<TypeKey>,
    /// `<init>`, the name of a class file's constructors.
    pub init: Name,
}

/// A method's or field's modifiers, as far as the relations need them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Flags(u8);

impl Flags {
    pub const STATIC: Flags = Flags(1);
    pub const PRIVATE: Flags = Flags(2);
    pub const PUBLIC: Flags = Flags(4);
    pub const PROTECTED: Flags = Flags(8);
    pub const ABSTRACT: Flags = Flags(16);
    pub const VARARGS: Flags = Flags(32);
    pub const CONSTRUCTOR: Flags = Flags(64);

    pub fn has(self, flag: Flags) -> bool {
        self.0 & flag.0 != 0
    }

    fn with(self, flag: Flags, on: bool) -> Flags {
        if on { Flags(self.0 | flag.0) } else { self }
    }
}

/// A method or constructor of a declaration.
pub(crate) struct MethodView<'a> {
    pub name: Name,
    pub flags: Flags,
    pub type_params: Params<'a>,
    pub params: &'a [Ty],
    /// `None` for `void` and constructors.
    pub result: Option<&'a Ty>,
    pub throws: &'a [Ty],
}

/// Type parameters, from source or a class file.
#[derive(Clone, Copy)]
pub(crate) enum Params<'a> {
    Source(&'a [TypeParam]),
    Class(&'a [ClassTypeParam]),
}

impl<'a> Params<'a> {
    pub fn len(self) -> usize {
        match self {
            Params::Source(p) => p.len(),
            Params::Class(p) => p.len(),
        }
    }

    pub fn bounds(self, index: usize) -> &'a [Ty] {
        match self {
            Params::Source(p) => p.get(index).map_or(&[], |p| &p.bounds),
            Params::Class(p) => p.get(index).map_or(&[], |p| &p.bounds),
        }
    }

    pub fn name(self, index: usize) -> Option<Name> {
        match self {
            Params::Source(p) => p.get(index).map(|p| p.name),
            Params::Class(p) => p.get(index).map(|p| p.name),
        }
    }
}

fn modifier_flags(m: Modifiers) -> Flags {
    Flags::default()
        .with(Flags::STATIC, m.contains(Modifiers::STATIC))
        .with(Flags::PRIVATE, m.contains(Modifiers::PRIVATE))
        .with(Flags::PUBLIC, m.contains(Modifiers::PUBLIC))
        .with(Flags::PROTECTED, m.contains(Modifiers::PROTECTED))
        .with(Flags::ABSTRACT, m.contains(Modifiers::ABSTRACT))
}

fn access_flags(access: u16) -> Flags {
    let f = F(access);
    Flags::default()
        .with(Flags::STATIC, f.contains(F::STATIC))
        .with(Flags::PRIVATE, f.contains(F::PRIVATE))
        .with(Flags::PUBLIC, f.contains(F::PUBLIC))
        .with(Flags::PROTECTED, f.contains(F::PROTECTED))
        .with(Flags::ABSTRACT, f.contains(F::ABSTRACT))
        .with(Flags::VARARGS, f.contains(F::VARARGS))
}

impl Decl {
    pub fn is_interface(&self) -> bool {
        matches!(self.kind, TypeKind::Interface | TypeKind::Annotation)
    }

    pub fn type_params(&self) -> Params<'_> {
        match &self.body {
            Body::Entry(e) => Params::Source(&e.type_parameters),
            Body::Class(c) => Params::Class(&c.type_parameters),
        }
    }

    /// The type parameters of the class, or of its `method`th method.
    pub fn params_of(&self, method: Option<u32>) -> Params<'_> {
        match method {
            None => self.type_params(),
            Some(m) => match &self.body {
                Body::Entry(e) => Params::Source(
                    e.methods
                        .get(m as usize)
                        .map_or(&[][..], |m| &m.type_parameters),
                ),
                Body::Class(c) => Params::Class(
                    c.methods
                        .get(m as usize)
                        .map_or(&[][..], |m| &m.type_parameters),
                ),
            },
        }
    }

    /// The generic superclass as written or read; `None` when none is written.
    pub fn superclass(&self) -> Option<&Ty> {
        match &self.body {
            Body::Entry(e) => e.superclass.as_ref(),
            Body::Class(c) => c.generic_superclass.as_ref(),
        }
    }

    pub fn interfaces(&self) -> &[Ty] {
        match &self.body {
            Body::Entry(e) => &e.interfaces,
            Body::Class(c) => &c.generic_interfaces,
        }
    }

    /// The errors a source entry's types name by index.
    pub fn errors(&self) -> &[ErrorSymbol] {
        match &self.body {
            Body::Entry(e) => &e.errors,
            Body::Class(_) => &[],
        }
    }

    pub fn field_count(&self) -> usize {
        match &self.body {
            Body::Entry(e) => e.fields.len(),
            Body::Class(c) => c.fields.len(),
        }
    }

    /// A field's name, type, and modifiers; an interface's fields are static.
    pub fn field(&self, index: usize) -> Option<(Name, &Ty, Flags)> {
        let interface = self.is_interface();
        match &self.body {
            Body::Entry(e) => e.fields.get(index).map(|f| {
                let flags = modifier_flags(f.modifiers);
                let enum_constant = f.kind == crate::symbols::FieldKind::EnumConstant;
                (
                    f.name,
                    &f.ty,
                    flags.with(Flags::STATIC, interface || enum_constant),
                )
            }),
            Body::Class(c) => c
                .fields
                .get(index)
                .map(|f| (f.name, &f.ty, access_flags(f.access))),
        }
    }

    pub fn method_count(&self) -> usize {
        match &self.body {
            Body::Entry(e) => e.methods.len(),
            Body::Class(c) => c.methods.len(),
        }
    }

    pub fn method(&self, index: usize) -> Option<MethodView<'_>> {
        let interface = self.is_interface();
        match &self.body {
            Body::Entry(e) => {
                let m = e.methods.get(index)?;
                let constructor = m.kind != MethodKind::Method;
                let mut flags = modifier_flags(m.modifiers)
                    .with(Flags::VARARGS, m.varargs)
                    .with(Flags::CONSTRUCTOR, constructor);
                if interface {
                    // An interface's methods are public, and abstract unless they have a body:
                    // static, default, and private ones do (JLS 9.4).
                    let has_body = m.modifiers.contains(Modifiers::STATIC)
                        || m.modifiers.contains(Modifiers::DEFAULT)
                        || m.modifiers.contains(Modifiers::PRIVATE);
                    flags = flags
                        .with(Flags::PUBLIC, !m.modifiers.contains(Modifiers::PRIVATE))
                        .with(Flags::ABSTRACT, !has_body);
                }
                let result = m
                    .result
                    .as_ref()
                    .filter(|r| !matches!(r.parts().first(), Some(crate::symbols::TyPart::Void)));
                Some(MethodView {
                    name: m.name,
                    flags,
                    type_params: Params::Source(&m.type_parameters),
                    params: &m.parameters,
                    result,
                    throws: &m.throws,
                })
            }
            Body::Class(c) => {
                let m = c.methods.get(index)?;
                Some(MethodView {
                    name: m.name,
                    flags: access_flags(m.access).with(Flags::CONSTRUCTOR, m.name == self.init),
                    type_params: Params::Class(&m.type_parameters),
                    params: &m.parameters,
                    result: m.result.as_ref(),
                    throws: &m.throws,
                })
            }
        }
    }
}
