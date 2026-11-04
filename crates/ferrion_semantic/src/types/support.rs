//! What member tables and attribution read of declarations through a context: a type's fields
//! and methods by index, a member's modifiers as inheritance and access need them, and
//! subclassing between classes.

use std::rc::Rc;

use super::decl::{Body, Flags};
use super::{R, Tri, Type, TypeArg, TypeContext, TypeId, Why};
use crate::summary::{Primitive, WildcardBound};
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
    pub is_protected: bool,
    pub is_native: bool,
    pub varargs: bool,
    /// How many parameters and type parameters it declares.
    pub params: usize,
    pub type_params: usize,
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
                        is_protected: f.has(Flags::PROTECTED),
                        is_native: f.has(Flags::NATIVE),
                        varargs: f.has(Flags::VARARGS),
                        params: m.params.len(),
                        type_params: m.type_params.len(),
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
        // A local class's package is the one its top level class is in.
        let top = self.outermost(field.owner)?;
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
                        MemberAccess::Package(crate::resolve::package_of(top, &names))
                    };
                    (f.has(Flags::STATIC) || interface, access)
                }))
            },
        )
    }

    /// Whether a field could be a constant variable, as far as its declaration says: `final`
    /// (an interface's are), and not of an array type.
    pub(crate) fn may_be_constant(&mut self, field: FieldRef) -> Result<bool, Cancelled> {
        self.top(
            |_| false,
            |cx| {
                let Some(decl) = cx.decl(field.owner)? else {
                    return Ok(false);
                };
                Ok(decl.field(field.index as usize).is_some_and(|(_, ty, f)| {
                    (f.has(Flags::FINAL) || decl.is_interface())
                        && !matches!(
                            ty.parts().first(),
                            Some(crate::symbols::TyPart::Array { .. })
                        )
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

    /// The outer type of the member class `key` written with the qualifier `o`
    /// ([`TypeContext::member_outer`]).
    pub(crate) fn outer_of_member(&mut self, key: TypeKey, o: TypeId) -> Result<TypeId, Cancelled> {
        self.top(|_| o, |cx| cx.member_outer(key, o))
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

    /// A method as this view sees it: a table made in another source set's view names a class file
    /// as that view found it, and this view may find the name elsewhere, as the method's
    /// translated types do. `None` if this view's class doesn't have the method.
    pub(crate) fn method_in_view(&mut self, m: MethodRef) -> Result<Option<MethodRef>, Cancelled> {
        let TypeKey::Class { binary, .. } = m.owner else {
            return Ok(Some(m));
        };
        self.top(
            |_| None,
            |cx| {
                let Some(owner) = cx.binary(binary)? else {
                    return Ok(Some(m));
                };
                if owner == m.owner {
                    return Ok(Some(m));
                }
                let (Some(from), Some(to)) = (cx.decl(m.owner)?, cx.decl(owner)?) else {
                    return Ok(None);
                };
                let (Body::Class(from), Body::Class(to)) = (&from.body, &to.body) else {
                    return Ok(None);
                };
                let Some(method) = from.methods.get(m.index as usize) else {
                    return Ok(None);
                };
                Ok(to
                    .methods
                    .iter()
                    .position(|n| n.name == method.name && n.descriptor == method.descriptor)
                    .map(|index| MethodRef {
                        owner,
                        index: index as u32,
                    }))
            },
        )
    }

    /// Whether a type is a raw interface type (JLS 4.8).
    pub(crate) fn is_raw_interface(&mut self, t: TypeId) -> Result<bool, Cancelled> {
        self.top(
            |_| false,
            |cx| {
                let Type::Class { key, .. } = cx.arena.get(t) else {
                    return Ok(false);
                };
                Ok(cx.is_raw(t)? && cx.decl(key)?.is_some_and(|d| d.is_interface()))
            },
        )
    }

    /// Whether an unchecked conversion to a type is one javac doesn't count: to a type all of whose
    /// arguments are unbounded wildcards (observed: a raw `List` passed as `List<?>` or
    /// `Collection<? extends Object>`, `Class[]` as `Class<?>[]`, leaves the method's result as it
    /// is, where `List<? extends Number>` erases it).
    pub(crate) fn takes_raw_freely(&mut self, t: TypeId) -> Result<bool, Cancelled> {
        self.top(|_| false, |cx| cx.takes_raw_freely_(t))
    }

    pub(crate) fn takes_raw_freely_(&mut self, t: TypeId) -> R<bool> {
        let mut t = t;
        while let Type::Array(component) = self.arena.get(t) {
            t = component;
        }
        let args = self.arena.class_args(t).to_vec();
        for arg in args {
            match arg {
                TypeArg::Wildcard(None) => {}
                TypeArg::Wildcard(Some((WildcardBound::Extends, b))) if self.is_object(b)? => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// The outer type of the member class `key` created through an instance of type `o`: `o`'s
    /// supertype whose class declares `key`, if that has arguments (observed: `new From<Integer>()
    /// { }.new To<String>()` is `From<java/lang/Integer>$To<java/lang/String>`); none through a raw
    /// type.
    pub(crate) fn outer_of_created(
        &mut self,
        key: TypeKey,
        o: TypeId,
    ) -> Result<Option<TypeId>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                if cx.is_raw(o)? {
                    return Ok(None);
                }
                let outer = cx.member_outer(key, o)?;
                Ok(cx.has_args(outer).then_some(outer))
            },
        )
    }

    /// Gives a local class the supertypes its entry can't say: an anonymous class made through an
    /// outer instance, whose superclass's outer type only the walk knows.
    pub(crate) fn set_supertypes(&mut self, key: TypeKey, supertypes: &[TypeId]) {
        self.supertypes.insert(key, Rc::from(supertypes));
    }

    /// A class's constructors, by index in its declaration.
    pub(crate) fn constructors(&mut self, key: TypeKey) -> Result<Vec<u32>, Cancelled> {
        self.top(
            |_| Vec::new(),
            |cx| {
                let Some(decl) = cx.decl(key)? else {
                    return Ok(Vec::new());
                };
                Ok((0..decl.method_count())
                    .filter(|&i| {
                        decl.method(i)
                            .is_some_and(|m| m.flags.has(Flags::CONSTRUCTOR))
                    })
                    .map(|i| i as u32)
                    .collect())
            },
        )
    }

    /// Whether a class's constructor is the default one: implicit, a default constructor's or a
    /// record's canonical one.
    pub(crate) fn has_default_constructor(&mut self, key: TypeKey) -> Result<bool, Cancelled> {
        self.top(
            |_| false,
            |cx| {
                Ok(cx.decl(key)?.is_some_and(|d| match &d.body {
                    Body::Entry(e) => e
                        .methods
                        .iter()
                        .any(|m| m.implicit && m.kind != crate::summary::MethodKind::Method),
                    Body::Class(_) => false,
                }))
            },
        )
    }

    /// Whether a source type has an annotation from a `lombok` package, so that Lombok may add
    /// members and constructors (until B4).
    pub(crate) fn is_lombok(&mut self, key: TypeKey) -> Result<bool, Cancelled> {
        self.top(
            |_| false,
            |cx| {
                Ok(cx
                    .decl(key)?
                    .is_some_and(|d| matches!(&d.body, Body::Entry(e) if e.lombok)))
            },
        )
    }

    /// Whether a class or one of its supertypes has a Lombok annotation.
    pub(crate) fn lombok_above(&mut self, key: TypeKey) -> Result<bool, Cancelled> {
        self.top(
            |_| false,
            |cx| {
                let mut seen = Vec::new();
                let mut pending = vec![key];
                while let Some(k) = pending.pop() {
                    if seen.contains(&k) {
                        continue;
                    }
                    seen.push(k);
                    if cx
                        .decl(k)?
                        .is_some_and(|d| matches!(&d.body, Body::Entry(e) if e.lombok))
                    {
                        return Ok(true);
                    }
                    for s in cx.declared_supertypes(k)?.iter() {
                        if let Type::Class { key, .. } = cx.arena.get(*s) {
                            pending.push(key);
                        }
                    }
                }
                Ok(false)
            },
        )
    }

    /// The top level class around a class, through member and local classes alike.
    pub(crate) fn outermost(&mut self, key: TypeKey) -> Result<TypeKey, Cancelled> {
        self.top(
            |_| key,
            |cx| {
                let mut current = key;
                for _ in 0..4096 {
                    match cx.decl(current)?.and_then(|d| d.enclosing.or(d.lexical)) {
                        Some(outer) => current = outer,
                        None => break,
                    }
                }
                Ok(current)
            },
        )
    }

    /// `classes` in the order javac searches a type's supertypes for a member (observation 22 in
    /// docs/design/semantic-attribution.md), the order of an intersection's parts: a class first,
    /// then interfaces deepest first, then by qualified name.
    pub(crate) fn search_order(&mut self, classes: &[TypeKey]) -> Result<Vec<TypeKey>, Cancelled> {
        let types: Vec<TypeId> = classes
            .iter()
            .map(|&key| self.arena.class(key, None, None))
            .collect();
        self.top(
            |_| classes.to_vec(),
            |cx| {
                let ordered = cx.order(types)?;
                Ok(ordered
                    .into_iter()
                    .filter_map(|t| match cx.arena.get(t) {
                        Type::Class { key, .. } => Some(key),
                        _ => None,
                    })
                    .collect())
            },
        )
    }

    /// A type's erasure as a JVM descriptor: `I`, `[Ljava/lang/String;`. A local class is named by
    /// its binary text, as a method's key names it.
    pub(crate) fn erased_descriptor(&mut self, t: TypeId) -> Result<String, Cancelled> {
        let mut out = String::new();
        let mut t = self.erasure(t)?;
        while let Type::Array(component) = self.arena.get(t) {
            out.push('[');
            t = component;
        }
        match self.arena.get(t) {
            Type::Primitive(p) => out.push(match p {
                Primitive::Boolean => 'Z',
                Primitive::Byte => 'B',
                Primitive::Char => 'C',
                Primitive::Short => 'S',
                Primitive::Int => 'I',
                Primitive::Long => 'J',
                Primitive::Float => 'F',
                Primitive::Double => 'D',
            }),
            Type::Void => out.push('V'),
            Type::Class { key, .. } => {
                out.push('L');
                out.push_str(&self.binary_text(key));
                out.push(';');
            }
            _ => out.push('?'),
        }
        Ok(out)
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
