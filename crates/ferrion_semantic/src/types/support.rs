//! What member tables and attribution read of declarations through a context: a type's fields
//! and methods by index, a member's modifiers as inheritance and access need them, and
//! subclassing between classes.

use super::decl::{Body, Flags};
use super::{Tri, TypeContext, TypeId, Why};
use crate::symbols::{FieldRef, MemberAccess, MethodRef, TypeKey};
use crate::{Cancelled, Name};

/// What a member table needs of a type's declaration.
pub(crate) struct DeclInfo {
    /// A source type's entry says its hierarchy is incomplete, or it is Lombok-annotated.
    pub incomplete: bool,
    pub interface: bool,
    /// Its fields' names, by index.
    pub fields: Vec<Name>,
    /// Its methods by index: `None` for a constructor.
    pub methods: Vec<Option<DeclMethod>>,
}

pub(crate) struct DeclMethod {
    pub name: Name,
    pub is_abstract: bool,
}

/// What inheritance needs of a method.
pub(crate) struct MethodInfo {
    pub is_private: bool,
    pub is_static: bool,
    pub is_public: bool,
    pub is_abstract: bool,
    /// Neither public, protected, nor private, nor a member of an interface.
    pub package_access: bool,
    /// Declared in an interface.
    pub interface: bool,
}

impl TypeContext<'_> {
    pub(crate) fn decl_info(&mut self, key: TypeKey) -> Result<Option<DeclInfo>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                let Some(decl) = cx.decl(key)? else {
                    return Ok(None);
                };
                let incomplete = match &decl.body {
                    Body::Entry(e) => e.incomplete || e.lombok,
                    Body::Class(_) => false,
                };
                let fields = (0..decl.field_count())
                    .map(|i| decl.field(i).map_or(Name::EMPTY, |f| f.0))
                    .collect();
                let methods = (0..decl.method_count())
                    .map(|i| {
                        decl.method(i)
                            .filter(|m| !m.flags.has(Flags::CONSTRUCTOR))
                            .map(|m| DeclMethod {
                                name: m.name,
                                is_abstract: m.flags.has(Flags::ABSTRACT),
                            })
                    })
                    .collect();
                Ok(Some(DeclInfo {
                    incomplete,
                    interface: decl.is_interface(),
                    fields,
                    methods,
                }))
            },
        )
    }

    /// A class's direct supertypes as declared, in terms of its type variables.
    pub(crate) fn declared_supertype_list(
        &mut self,
        key: TypeKey,
    ) -> Result<Vec<TypeId>, Cancelled> {
        self.top(
            |cx| vec![cx.arena.unknown(Why::Limit)],
            |cx| Ok(cx.declared_supertypes(key)?.to_vec()),
        )
    }

    pub(crate) fn method_info(
        &mut self,
        method: MethodRef,
    ) -> Result<Option<MethodInfo>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                let Some(decl) = cx.decl(method.owner)? else {
                    return Ok(None);
                };
                let interface = decl.is_interface();
                Ok(decl.method(method.index as usize).map(|m| {
                    let f = m.flags;
                    MethodInfo {
                        is_private: f.has(Flags::PRIVATE),
                        is_static: f.has(Flags::STATIC),
                        is_public: f.has(Flags::PUBLIC) || interface,
                        is_abstract: f.has(Flags::ABSTRACT),
                        package_access: !interface
                            && !f.has(Flags::PUBLIC)
                            && !f.has(Flags::PROTECTED)
                            && !f.has(Flags::PRIVATE),
                        interface,
                    }
                }))
            },
        )
    }

    /// A field's modifiers as inheritance and access need them: whether it is static, and who
    /// can inherit it (a member of an interface is public).
    pub(crate) fn field_info(
        &mut self,
        field: FieldRef,
    ) -> Result<Option<(bool, MemberAccess)>, Cancelled> {
        let names = self.names.clone();
        self.top(
            |_| None,
            |cx| {
                let Some(decl) = cx.decl(field.owner)? else {
                    return Ok(None);
                };
                let interface = decl.is_interface();
                Ok(decl.field(field.index as usize).map(|(_, _, f)| {
                    let access = if f.has(Flags::PRIVATE) {
                        MemberAccess::Private
                    } else if interface || f.has(Flags::PUBLIC) || f.has(Flags::PROTECTED) {
                        MemberAccess::Public
                    } else {
                        MemberAccess::Package(crate::resolve::package_of(field.owner, &names))
                    };
                    (f.has(Flags::STATIC) || interface, access)
                }))
            },
        )
    }

    /// Whether a field is inherited by a subclass in `package`: not private, and not of package
    /// access in another package.
    pub(crate) fn inheritable_field(
        &mut self,
        field: FieldRef,
        package: Name,
    ) -> Result<bool, Cancelled> {
        Ok(match self.field_info(field)? {
            Some((_, MemberAccess::Public)) => true,
            Some((_, MemberAccess::Package(p))) => p == package,
            _ => false,
        })
    }

    /// Whether the class `sub` has `sup` among its supertypes, or is it.
    pub(crate) fn is_subclass(&mut self, sub: TypeKey, sup: TypeKey) -> Result<bool, Cancelled> {
        if sub == sup {
            return Ok(true);
        }
        self.top(
            |_| false,
            |cx| Ok(matches!(cx.super_decl(sub, sup)?, Tri::Yes(Some(_)))),
        )
    }

    /// The index of `key`'s field named `name`.
    pub(crate) fn field_named(
        &mut self,
        key: TypeKey,
        name: Name,
    ) -> Result<Option<u32>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                let Some(decl) = cx.decl(key)? else {
                    return Ok(None);
                };
                Ok((0..decl.field_count())
                    .find(|&i| decl.field(i).is_some_and(|f| f.0 == name))
                    .map(|i| i as u32))
            },
        )
    }

    /// Whether a class type, or one it is a member of, has type arguments.
    pub fn has_type_args(&self, t: TypeId) -> bool {
        self.has_args(t)
    }

    /// The outer type an inner class of a generic class has where it is used without a
    /// qualifier inside `site` (JLS 6.5.5.1), if any.
    pub(crate) fn implicit_outer_in(
        &mut self,
        key: TypeKey,
        site: TypeKey,
    ) -> Result<Option<TypeId>, Cancelled> {
        self.top(|_| None, |cx| cx.implicit_outer(key, site))
    }

    /// A type variable's or capture variable's upper bound, the intersection of its bounds; any
    /// other type itself.
    pub fn upper_bound(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        self.top(|cx| cx.arena.unknown(Why::Limit), |cx| cx.upper(t))
    }

    /// The primitive type a type unboxes to, directly or through its bounds; `None` if it
    /// doesn't, or that isn't known.
    pub(crate) fn unboxed_primitive(
        &mut self,
        t: TypeId,
    ) -> Result<Option<crate::summary::Primitive>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                Ok(match cx.unboxed(t)? {
                    Tri::Yes((p, _)) => Some(p),
                    _ => None,
                })
            },
        )
    }

    /// A method's name.
    pub(crate) fn method_name(&mut self, method: MethodRef) -> Result<Option<Name>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                Ok(cx
                    .decl(method.owner)?
                    .and_then(|d| d.method(method.index as usize).map(|m| m.name)))
            },
        )
    }
}
