//! Subtyping, containment, sameness, capture, erasure, supertypes, member types, and method
//! signatures (JLS 4.3.4, 4.5.1, 4.6, 4.8, 4.10, 5.1.10, 8.4.2, 8.4.5, 8.4.8.1).

use std::rc::Rc;

use super::decl::Flags;
use super::{Active, R, Slot, Tri, Type, TypeArg, TypeContext, TypeId, Why, YES};
use crate::Cancelled;
use crate::summary::{Primitive, TypeKind, WildcardBound};
use crate::symbols::{TypeKey, TypeVarOwner};

/// A substitution of type variables.
#[derive(Clone, Default, Debug)]
pub(crate) struct Subst(Vec<((TypeVarOwner, u32), TypeId)>);

impl Subst {
    pub fn get(&self, owner: TypeVarOwner, index: u32) -> Option<TypeId> {
        self.0
            .iter()
            .find(|(v, _)| *v == (owner, index))
            .map(|(_, t)| *t)
    }

    pub fn push(&mut self, owner: TypeVarOwner, index: u32, t: TypeId) {
        self.0.push(((owner, index), t));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A field or method of a class, by its index in the class's entry or class file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Member {
    Field(TypeKey, u32),
    Method(TypeKey, u32),
}

/// A method's or constructor's type: its type parameters (as type variables), parameter types,
/// result (`void` for none), and thrown types.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MethodType {
    pub type_params: Vec<TypeId>,
    pub params: Vec<TypeId>,
    pub result: TypeId,
    pub throws: Vec<TypeId>,
}

/// A capture variable just made: the index of the argument it captured, the variable, and the
/// wildcard's bound.
type NewCapture = (u32, TypeId, Option<(WildcardBound, TypeId)>);

/// How a member is seen from a type.
enum View {
    /// As declared: a static member, or one of the type itself.
    Declared,
    /// Erased: a member of a raw type (JLS 4.8).
    Erased,
    Substituted(Subst),
}

/// Whether `a` widens to `b` by primitive subtyping (JLS 4.10.1): `double > float > long > int
/// > char`, `int > short > byte`.
pub(crate) fn primitive_subtype(a: Primitive, b: Primitive) -> bool {
    use Primitive::*;
    if a == b {
        return true;
    }
    let rank = |p: Primitive| match p {
        Byte => Some(0),
        Short => Some(1),
        Char => Some(1),
        Int => Some(2),
        Long => Some(3),
        Float => Some(4),
        Double => Some(5),
        Boolean => None,
    };
    match (rank(a), rank(b)) {
        // `char` and `short` are not subtypes of each other, nor `byte` of `char`.
        (Some(_), Some(_)) if (a == Char && b == Short) || (a == Short && b == Char) => false,
        (Some(_), Some(_)) if a == Byte && b == Char => false,
        (Some(x), Some(y)) => x < y,
        _ => false,
    }
}

impl TypeContext<'_> {
    // Supertypes.

    /// A class's direct supertypes as declared, in terms of its own type variables: its
    /// superclass, written or implied (`Object`, `Enum<E>`, `Record`), then its interfaces
    /// (an annotation interface's is `Annotation`). An interface has no superclass here.
    pub(crate) fn declared_supertypes(&mut self, key: TypeKey) -> R<Rc<[TypeId]>> {
        if let Some(supers) = self.supertypes.get(&key) {
            return Ok(supers.clone());
        }
        let query = Active::Supertypes(key);
        if !self.enter(query) {
            return Ok(Rc::from([self.arena.unknown(Why::Limit)]));
        }
        let cycles = self.cycles;
        let result = self.load_supertypes(key);
        self.leave(query);
        let supers: Rc<[TypeId]> = Rc::from(result?);
        if self.cycles == cycles {
            self.supertypes.insert(key, supers.clone());
        }
        Ok(supers)
    }

    fn load_supertypes(&mut self, key: TypeKey) -> R<Vec<TypeId>> {
        let Some(decl) = self.decl(key)? else {
            return Ok(vec![self.arena.unknown(Why::Incomplete(key))]);
        };
        let mut out = Vec::new();
        let from_source = matches!(decl.body, super::decl::Body::Entry(_));
        if decl.superclass().is_some() && (from_source || !decl.is_interface()) {
            out.push(self.slot(key, Slot::Superclass)?);
        } else if from_source {
            let implied = match decl.kind {
                TypeKind::Class | TypeKind::Implicit => Some("java/lang/Object"),
                TypeKind::Enum => Some("java/lang/Enum"),
                TypeKind::Record => Some("java/lang/Record"),
                TypeKind::Interface | TypeKind::Annotation => None,
            };
            if let Some(binary) = implied {
                match self.lang_key(binary)? {
                    Some(k) if k == key => {}
                    Some(k) if decl.kind == TypeKind::Enum => {
                        let this = self.this_type(key)?;
                        out.push(self.arena.class(k, None, Some(&[TypeArg::Type(this)])));
                    }
                    Some(k) => out.push(self.arena.class(k, None, None)),
                    None => out.push(self.missing(binary)),
                }
            }
        }
        for i in 0..decl.interfaces().len() as u32 {
            out.push(self.slot(key, Slot::Interface(i))?);
        }
        if from_source && decl.kind == TypeKind::Annotation && decl.interfaces().is_empty() {
            let annotation = "java/lang/annotation/Annotation";
            out.push(match self.lang_key(annotation)? {
                Some(k) => self.arena.class(k, None, None),
                None => self.missing(annotation),
            });
        }
        Ok(out)
    }

    /// `c` as a supertype of the class `k`'s declared type, in terms of `k`'s type variables:
    /// `Collection<E>` for `ArrayList` and `Collection`. `No` if it isn't one; `Unknown` if a
    /// supertype on the way didn't resolve.
    pub(crate) fn super_decl(&mut self, k: TypeKey, c: TypeKey) -> R<Tri<Option<TypeId>>> {
        if k == c {
            return Ok(Tri::Yes(Some(self.this_type(k)?)));
        }
        if let Some(&found) = self.super_decls.get(&(k, c)) {
            return Ok(found);
        }
        let query = Active::SuperDecl(k, c);
        if !self.enter(query) {
            return Ok(Tri::Unknown(Why::Limit));
        }
        let cycles = self.cycles;
        let result = self.find_super_decl(k, c);
        self.leave(query);
        let found = result?;
        if self.cycles == cycles {
            self.super_decls.insert((k, c), found);
        }
        Ok(found)
    }

    fn find_super_decl(&mut self, k: TypeKey, c: TypeKey) -> R<Tri<Option<TypeId>>> {
        let supers = self.declared_supertypes(k)?;
        let mut unknown = None;
        for &s in supers.iter() {
            self.step()?;
            match self.arena.get(s) {
                Type::Class { .. } => match self.as_super_(s, c)? {
                    Tri::Yes(t) => return Ok(Tri::Yes(Some(t))),
                    Tri::No => {}
                    Tri::Unknown(why) => {
                        unknown.get_or_insert(why);
                    }
                },
                Type::Error(e) => {
                    unknown.get_or_insert(Why::Error(e));
                }
                Type::Unknown(why) => {
                    unknown.get_or_insert(why);
                }
                _ => {}
            }
        }
        Ok(match unknown {
            Some(why) => Tri::Unknown(why),
            None => Tri::Yes(None),
        })
    }

    /// Whether a class type is raw: a generic class without arguments, or an inner class of a
    /// generic class without its outer type (JLS 4.8).
    pub(crate) fn is_raw(&mut self, t: TypeId) -> R<bool> {
        let Type::Class { key, outer, args } = self.arena.get(t) else {
            return Ok(false);
        };
        let Some(decl) = self.decl(key)? else {
            return Ok(false);
        };
        if args.is_none() && decl.type_params().len() > 0 {
            return Ok(true);
        }
        if let Some(o) = outer {
            return self.is_raw(o);
        }
        match decl.enclosing {
            Some(enclosing) if decl.inner => self.has_args_chain(enclosing),
            _ => Ok(false),
        }
    }

    /// The substitution a parameterized class type makes: each of its class's type variables,
    /// and its outer types' classes', to its argument.
    pub(crate) fn theta(&mut self, t: TypeId) -> Subst {
        let mut subst = Subst::default();
        let mut current = Some(t);
        while let Some(c) = current {
            let Type::Class { key, outer, args } = self.arena.get(c) else {
                break;
            };
            if let Some(args) = args {
                for (i, arg) in self.arena.args(args).iter().enumerate() {
                    let t = match *arg {
                        TypeArg::Type(t) => t,
                        // Callers capture first; a wildcard left here stands for its bound.
                        TypeArg::Wildcard(Some((WildcardBound::Extends, b))) => b,
                        TypeArg::Wildcard(_) => continue,
                    };
                    subst.push(TypeVarOwner::Type(key), i as u32, t);
                }
            }
            current = outer;
        }
        subst
    }

    /// Substitutes type variables in `t`.
    pub(crate) fn subst(&mut self, t: TypeId, subst: &Subst) -> R<TypeId> {
        if subst.is_empty() {
            return Ok(t);
        }
        ferrion_stack::ensure_stack(|| self.subst_inner(t, subst))
    }

    fn subst_inner(&mut self, t: TypeId, subst: &Subst) -> R<TypeId> {
        self.step()?;
        Ok(match self.arena.get(t) {
            Type::Var(owner, index) => subst.get(owner, index).unwrap_or(t),
            Type::Class { key, outer, args } => {
                let outer = match outer {
                    Some(o) => Some(self.subst(o, subst)?),
                    None => None,
                };
                let args = match args {
                    Some(list) => {
                        let old = self.arena.args(list).to_vec();
                        let mut new = Vec::with_capacity(old.len());
                        for arg in old {
                            new.push(match arg {
                                TypeArg::Type(a) => TypeArg::Type(self.subst(a, subst)?),
                                TypeArg::Wildcard(Some((kind, b))) => {
                                    TypeArg::Wildcard(Some((kind, self.subst(b, subst)?)))
                                }
                                TypeArg::Wildcard(None) => TypeArg::Wildcard(None),
                            });
                        }
                        Some(new)
                    }
                    None => None,
                };
                self.arena.class(key, outer, args.as_deref())
            }
            Type::Array(c) => {
                let c = self.subst(c, subst)?;
                self.arena.array(c)
            }
            Type::Intersection(list) => {
                let parts = self.arena.list(list).to_vec();
                let mut new = Vec::with_capacity(parts.len());
                for p in parts {
                    new.push(self.subst(p, subst)?);
                }
                self.arena.intersection(&new)
            }
            _ => t,
        })
    }

    /// The supertype of `t` whose class is `c`, substituted, if `t` has one (JLS 4.10.2): the
    /// step subtyping, member lookup, and inference take.
    pub(crate) fn as_super_(&mut self, t: TypeId, c: TypeKey) -> R<Tri<TypeId>> {
        self.step()?;
        match self.arena.get(t) {
            Type::Class { key, .. } => {
                // The supertypes of a type with wildcards are its capture's (JLS 4.10.2).
                let t = self.capture_(t)?;
                if key == c {
                    return Ok(Tri::Yes(t));
                }
                if self.lang_key("java/lang/Object")? == Some(c) {
                    return Ok(Tri::Yes(self.arena.class(c, None, None)));
                }
                match self.super_decl(key, c)? {
                    Tri::Yes(Some(g)) => {
                        if self.is_raw(t)? {
                            Ok(Tri::Yes(self.erasure_(g)?))
                        } else {
                            let subst = self.theta(t);
                            Ok(Tri::Yes(self.subst(g, &subst)?))
                        }
                    }
                    Tri::Yes(None) | Tri::No => Ok(Tri::No),
                    Tri::Unknown(why) => Ok(Tri::Unknown(why)),
                }
            }
            Type::Var(..) | Type::Capture(_) => {
                let upper = self.upper(t)?;
                self.as_super_(upper, c)
            }
            Type::Intersection(list) => {
                let parts = self.arena.list(list).to_vec();
                let mut unknown = None;
                for p in parts {
                    match self.as_super_(p, c)? {
                        Tri::Yes(s) => return Ok(Tri::Yes(s)),
                        Tri::No => {}
                        Tri::Unknown(why) => {
                            unknown.get_or_insert(why);
                        }
                    }
                }
                Ok(unknown.map_or(Tri::No, Tri::Unknown))
            }
            Type::Array(_) => {
                for binary in [
                    "java/lang/Object",
                    "java/lang/Cloneable",
                    "java/io/Serializable",
                ] {
                    if self.lang_key(binary)? == Some(c) {
                        return Ok(Tri::Yes(self.arena.class(c, None, None)));
                    }
                }
                Ok(Tri::No)
            }
            Type::Error(e) => Ok(Tri::Unknown(Why::Error(e))),
            Type::Unknown(why) => Ok(Tri::Unknown(why)),
            Type::Primitive(_) | Type::Void | Type::Null => Ok(Tri::No),
            // Only what a type is recorded as: relations ask about its `lub`.
            Type::Union(_) => Ok(Tri::Unknown(Why::Later)),
        }
    }

    /// `t`'s supertype whose class is `c`, substituted, if any.
    pub fn as_super(&mut self, t: TypeId, c: TypeKey) -> Result<Tri<TypeId>, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.as_super_(t, c))
    }

    /// The direct supertypes of `t` (JLS 4.10): substituted from its class's for a
    /// parameterized type, through its capture if it has wildcards; erased for a raw type; with
    /// `Object` for an interface that has no superinterfaces; an array's as JLS 4.10.3 lists them;
    /// a type variable's bounds; a primitive type's by 4.10.1.
    pub fn direct_supertypes(&mut self, t: TypeId) -> Result<Tri<Vec<TypeId>>, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.direct_supertypes_(t))
    }

    pub(crate) fn direct_supertypes_(&mut self, t: TypeId) -> R<Tri<Vec<TypeId>>> {
        self.step()?;
        Ok(Tri::Yes(match self.arena.get(t) {
            Type::Class { key, .. } => {
                let t = self.capture_(t)?;
                let supers = self.declared_supertypes(key)?.to_vec();
                let raw = self.is_raw(t)?;
                let subst = self.theta(t);
                let mut out = Vec::with_capacity(supers.len() + 1);
                for s in supers {
                    out.push(if raw {
                        self.erasure_(s)?
                    } else {
                        self.subst(s, &subst)?
                    });
                }
                let interface = self.decl(key)?.is_some_and(|d| d.is_interface());
                if interface && out.is_empty() {
                    out.push(self.object_or_error()?);
                }
                out
            }
            Type::Var(owner, index) => self.var_bounds(owner, index)?.to_vec(),
            Type::Capture(n) => {
                let upper = self.arena.capture(n).upper;
                match self.arena.get(upper) {
                    Type::Intersection(list) => self.arena.list(list).to_vec(),
                    _ => vec![upper],
                }
            }
            Type::Intersection(list) => self.arena.list(list).to_vec(),
            Type::Array(c) => {
                let object = self.object_or_error()?;
                let cloneable = self.lang_class("java/lang/Cloneable")?;
                let serializable = self.lang_class("java/io/Serializable")?;
                let base = vec![object, cloneable, serializable];
                if !self.arena.is_reference(c) || self.is_object(c)? {
                    base
                } else {
                    match self.direct_supertypes_(c)? {
                        Tri::Yes(supers) => {
                            supers.into_iter().map(|s| self.arena.array(s)).collect()
                        }
                        other => return Ok(other),
                    }
                }
            }
            Type::Primitive(p) => {
                use Primitive::*;
                let supers: &[Primitive] = match p {
                    Byte => &[Short],
                    Short | Char => &[Int],
                    Int => &[Long],
                    Long => &[Float],
                    Float => &[Double],
                    Double | Boolean => &[],
                };
                supers.iter().map(|&s| self.arena.primitive(s)).collect()
            }
            Type::Error(e) => return Ok(Tri::Unknown(Why::Error(e))),
            Type::Unknown(why) => return Ok(Tri::Unknown(why)),
            Type::Void | Type::Null => Vec::new(),
            Type::Union(_) => return Ok(Tri::Unknown(Why::Later)),
        }))
    }

    pub(crate) fn lang_class(&mut self, binary: &str) -> R<TypeId> {
        Ok(match self.lang_key(binary)? {
            Some(key) => self.arena.class(key, None, None),
            None => self.missing(binary),
        })
    }

    // Capture and erasure.

    /// Capture conversion (JLS 5.1.10): a parameterized type's wildcard arguments replaced by
    /// fresh capture variables, each bounded by the glb of the wildcard's bound and its type
    /// parameter's bounds, substituted. Memoized, so capturing a type twice gives the same
    /// type; [`TypeContext::fresh_capture`] makes new variables each time.
    pub fn capture(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        self.top(|cx| cx.arena.unknown(Why::Limit), |cx| cx.capture_(t))
    }

    /// Capture conversion with new capture variables, as each expression gets its own.
    pub fn fresh_capture(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        self.top(
            |cx| cx.arena.unknown(Why::Limit),
            |cx| cx.capture_with(t, false),
        )
    }

    pub(crate) fn capture_(&mut self, t: TypeId) -> R<TypeId> {
        self.capture_with(t, true)
    }

    fn capture_with(&mut self, t: TypeId, memo: bool) -> R<TypeId> {
        let Type::Class { key, outer, args } = self.arena.get(t) else {
            return Ok(t);
        };
        // The type it is a member of is captured too (W2's observation: a variable of type
        // `Mall<? extends Number>.Shop<Integer>` reads as `Mall<capture#1 extends
        // Number>.Shop<Integer>`).
        let captured_outer = match outer {
            Some(o) => Some(ferrion_stack::ensure_stack(|| self.capture_with(o, memo))?),
            None => None,
        };
        let Some(list) = args else {
            return Ok(if captured_outer == outer {
                t
            } else {
                self.arena.class(key, captured_outer, None)
            });
        };
        let args = self.arena.args(list).to_vec();
        if !args.iter().any(|a| matches!(a, TypeArg::Wildcard(_))) {
            return Ok(if captured_outer == outer {
                t
            } else {
                self.arena.class(key, captured_outer, Some(&args))
            });
        }
        if memo && let Some(&c) = self.captured.get(&t) {
            return Ok(c);
        }
        self.step()?;
        let mut new = Vec::with_capacity(args.len());
        let mut captures = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            match *arg {
                // `? super Object` captures to `Object` itself, its upper and lower bounds both
                // (W2's observation in docs/design/semantic-attribution.md: `List<? super
                // Object>` is `java/util/List<java/lang/Object>`).
                TypeArg::Wildcard(Some((WildcardBound::Super, b))) if self.is_object(b)? => {
                    new.push(TypeArg::Type(b));
                }
                TypeArg::Wildcard(w) => {
                    let c = self.arena.new_capture(TypeArg::Wildcard(w), key, i as u32);
                    new.push(TypeArg::Type(c));
                    captures.push((i as u32, c, w));
                }
                arg => new.push(arg),
            }
        }
        let captured = self.arena.class(key, captured_outer, Some(&new));
        // Memoized before its bounds are set, since they can capture `t` again; forgotten if
        // they can't be set.
        if memo {
            self.captured.insert(t, captured);
        }
        let bounded = self.bound_captures(key, captured, captures);
        if bounded.is_err() && memo {
            self.captured.remove(&t);
        }
        bounded.map(|()| captured)
    }

    /// Sets each new capture variable's bounds (JLS 5.1.10).
    fn bound_captures(
        &mut self,
        key: TypeKey,
        captured: TypeId,
        captures: Vec<NewCapture>,
    ) -> R<()> {
        let subst = self.theta(captured);
        for (i, c, wildcard) in captures {
            let declared = self.var_bounds(TypeVarOwner::Type(key), i)?.to_vec();
            let mut bounds = Vec::with_capacity(declared.len() + 1);
            for b in declared {
                bounds.push(self.subst(b, &subst)?);
            }
            let (upper, lower) = match wildcard {
                None => (self.arena.intersection(&bounds), None),
                Some((WildcardBound::Extends, b)) => {
                    let mut all = vec![b];
                    all.extend(bounds);
                    (self.glb_(&all)?, None)
                }
                Some((WildcardBound::Super, b)) => (self.arena.intersection(&bounds), Some(b)),
            };
            let Type::Capture(n) = self.arena.get(c) else {
                unreachable!("a capture variable");
            };
            self.arena.set_bounds(n, upper, lower);
        }
        Ok(())
    }

    /// Type erasure (JLS 4.6).
    pub fn erasure(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        self.top(|cx| cx.arena.unknown(Why::Limit), |cx| cx.erasure_(t))
    }

    pub(crate) fn erasure_(&mut self, t: TypeId) -> R<TypeId> {
        if let Some(&e) = self.erased.get(&t) {
            return Ok(e);
        }
        self.step()?;
        let erased = match self.arena.get(t) {
            Type::Class { key, outer, args } => {
                if outer.is_none() && args.is_none() {
                    t
                } else {
                    self.arena.class(key, None, None)
                }
            }
            Type::Array(c) => {
                let c = ferrion_stack::ensure_stack(|| self.erasure_(c))?;
                self.arena.array(c)
            }
            Type::Var(owner, index) => {
                let first = self.var_bounds(owner, index)?[0];
                ferrion_stack::ensure_stack(|| self.erasure_(first))?
            }
            Type::Capture(n) => {
                let upper = self.arena.capture(n).upper;
                ferrion_stack::ensure_stack(|| self.erasure_(upper))?
            }
            Type::Intersection(list) => {
                let first = self.arena.list(list)[0];
                ferrion_stack::ensure_stack(|| self.erasure_(first))?
            }
            _ => t,
        };
        self.erased.insert(t, erased);
        Ok(erased)
    }

    // Subtyping, containment, sameness.

    /// `s <: t` (JLS 4.10), with `s` captured first, as javac asks it.
    pub fn subtype(&mut self, s: TypeId, t: TypeId) -> Result<Tri, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.subtype_(s, t))
    }

    pub(crate) fn subtype_(&mut self, s: TypeId, t: TypeId) -> R<Tri> {
        if s == t {
            return Ok(match self.arena.non_answer(s) {
                Some(why) => Tri::Unknown(why),
                None => YES,
            });
        }
        if let Some(why) = self.arena.non_answer(s).or(self.arena.non_answer(t)) {
            return Ok(Tri::Unknown(why));
        }
        if let Some(&found) = self.subtypes.get(&(s, t)) {
            return Ok(found);
        }
        let query = Active::Subtype(s, t);
        if !self.enter(query) {
            return Ok(Tri::Unknown(Why::Limit));
        }
        let cycles = self.cycles;
        let result = ferrion_stack::ensure_stack(|| self.find_subtype(s, t));
        self.leave(query);
        let found = result?;
        if self.cycles == cycles {
            self.subtypes.insert((s, t), found);
        }
        Ok(found)
    }

    fn find_subtype(&mut self, s: TypeId, t: TypeId) -> R<Tri> {
        self.step()?;
        let (ts, tt) = (self.arena.get(s), self.arena.get(t));
        match (ts, tt) {
            (Type::Primitive(a), Type::Primitive(b)) => {
                return Ok(Tri::from_bool(primitive_subtype(a, b)));
            }
            (Type::Primitive(_) | Type::Void, _) | (_, Type::Primitive(_) | Type::Void) => {
                return Ok(Tri::No);
            }
            (Type::Null, _) => return Ok(YES),
            (_, Type::Null) => return Ok(Tri::No),
            (_, Type::Intersection(list)) => {
                let parts = self.arena.list(list).to_vec();
                return self.all(parts, |cx, p| cx.subtype_(s, p));
            }
            _ => {}
        }
        if self.is_object(t)? {
            return Ok(YES);
        }
        // `s <: t` when `s` is below a capture variable's lower bound.
        if let Type::Capture(n) = tt
            && let Some(lower) = self.arena.capture(n).lower
            && self.subtype_(s, lower)?.is_yes()
        {
            return Ok(YES);
        }
        match ts {
            Type::Intersection(list) => {
                let parts = self.arena.list(list).to_vec();
                self.any(parts, |cx, p| cx.subtype_(p, t))
            }
            Type::Var(..) | Type::Capture(_) => {
                let upper = self.upper(s)?;
                self.subtype_(upper, t)
            }
            Type::Array(c) => match tt {
                Type::Array(d) => {
                    if self.arena.is_reference(c) && self.arena.is_reference(d) {
                        self.subtype_(c, d)
                    } else if let (Some(why), _) | (_, Some(why)) =
                        (self.arena.non_answer(c), self.arena.non_answer(d))
                    {
                        Ok(Tri::Unknown(why))
                    } else {
                        Ok(Tri::from_bool(c == d))
                    }
                }
                Type::Class { key, .. } => {
                    let mut yes = false;
                    for binary in ["java/lang/Cloneable", "java/io/Serializable"] {
                        yes |= self.lang_key(binary)? == Some(key);
                    }
                    Ok(Tri::from_bool(yes))
                }
                _ => Ok(Tri::No),
            },
            Type::Class { .. } => match tt {
                Type::Class { key, .. } => self.class_subtype(s, t, key),
                _ => Ok(Tri::No),
            },
            _ => Ok(Tri::No),
        }
    }

    /// A class type below another (JLS 4.10.2): `t`'s class among `s`'s supertypes, and each of
    /// `t`'s arguments, and its outer type's, containing the supertype's.
    fn class_subtype(&mut self, s: TypeId, t: TypeId, c: TypeKey) -> R<Tri> {
        let sup = match self.as_super_(s, c)? {
            Tri::Yes(sup) => sup,
            Tri::No => return Ok(Tri::No),
            Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
        };
        self.arguments_contain(t, sup)
    }

    /// Whether a class type's arguments, and its outer types', contain another's of the same
    /// class: `t` is raw or not generic, or each argument of `t` contains `s`'s.
    fn arguments_contain(&mut self, t: TypeId, s: TypeId) -> R<Tri> {
        let (
            Type::Class {
                outer: to,
                args: ta,
                ..
            },
            Type::Class {
                outer: so,
                args: sa,
                ..
            },
        ) = (self.arena.get(t), self.arena.get(s))
        else {
            return Ok(Tri::No);
        };
        let mut answer = YES;
        match (ta, sa) {
            (None, _) => {}
            (Some(_), None) => return Ok(Tri::No),
            (Some(ta), Some(sa)) => {
                let (ta, sa) = (self.arena.args(ta).to_vec(), self.arena.args(sa).to_vec());
                if ta.len() != sa.len() {
                    return Ok(Tri::No);
                }
                for (a, b) in ta.into_iter().zip(sa) {
                    match self.contains_(a, b)? {
                        Tri::No => return Ok(Tri::No),
                        Tri::Unknown(why) => answer = Tri::Unknown(why),
                        Tri::Yes(()) => {}
                    }
                }
            }
        }
        match (to, so) {
            (Some(to), Some(so)) => match self.arguments_contain(to, so)? {
                Tri::No => Ok(Tri::No),
                Tri::Unknown(why) => Ok(Tri::Unknown(why)),
                Tri::Yes(()) => Ok(answer),
            },
            (Some(_), None) => Ok(Tri::No),
            _ => Ok(answer),
        }
    }

    /// All of `items` answer `Yes`: `No` if any says `No`, else `Unknown` if any is unknown.
    pub(crate) fn all(
        &mut self,
        items: Vec<TypeId>,
        mut f: impl FnMut(&mut Self, TypeId) -> R<Tri>,
    ) -> R<Tri> {
        let mut answer = YES;
        for item in items {
            match f(self, item)? {
                Tri::No => return Ok(Tri::No),
                Tri::Unknown(why) => answer = Tri::Unknown(why),
                Tri::Yes(()) => {}
            }
        }
        Ok(answer)
    }

    /// Any of `items` answers `Yes`: `Unknown` if none does and any is unknown, else `No`.
    pub(crate) fn any(
        &mut self,
        items: Vec<TypeId>,
        mut f: impl FnMut(&mut Self, TypeId) -> R<Tri>,
    ) -> R<Tri> {
        let mut answer = Tri::No;
        for item in items {
            match f(self, item)? {
                Tri::Yes(()) => return Ok(YES),
                Tri::Unknown(why) => answer = Tri::Unknown(why),
                Tri::No => {}
            }
        }
        Ok(answer)
    }

    /// Whether the type argument `t` contains `s` (JLS 4.5.1).
    pub fn contains(&mut self, t: TypeArg, s: TypeArg) -> Result<Tri, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.contains_(t, s))
    }

    pub(crate) fn contains_(&mut self, t: TypeArg, s: TypeArg) -> R<Tri> {
        use WildcardBound::{Extends, Super};
        self.step()?;
        match (t, s) {
            (TypeArg::Wildcard(None), _) => Ok(YES),
            (TypeArg::Type(t), TypeArg::Type(s)) => self.same_(t, s),
            (TypeArg::Type(_), TypeArg::Wildcard(_)) => Ok(Tri::No),
            (TypeArg::Wildcard(Some((Extends, t))), TypeArg::Type(s))
            | (TypeArg::Wildcard(Some((Extends, t))), TypeArg::Wildcard(Some((Extends, s)))) => {
                self.subtype_(s, t)
            }
            (TypeArg::Wildcard(Some((Extends, t))), TypeArg::Wildcard(None)) => {
                let object = self.object_or_error()?;
                self.subtype_(object, t)
            }
            (TypeArg::Wildcard(Some((Extends, t))), TypeArg::Wildcard(Some((Super, _)))) => {
                Ok(Tri::from_bool(self.is_object(t)?))
            }
            (TypeArg::Wildcard(Some((Super, t))), TypeArg::Type(s))
            | (TypeArg::Wildcard(Some((Super, t))), TypeArg::Wildcard(Some((Super, s)))) => {
                self.subtype_(t, s)
            }
            (TypeArg::Wildcard(Some((Super, _))), TypeArg::Wildcard(_)) => Ok(Tri::No),
        }
    }

    /// Whether two types are the same (JLS 4.3.4): the same id, or intersections of the same
    /// parts; a capture variable is the same only as itself.
    pub fn same(&mut self, s: TypeId, t: TypeId) -> Result<Tri, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.same_(s, t))
    }

    pub(crate) fn same_(&mut self, s: TypeId, t: TypeId) -> R<Tri> {
        if let Some(why) = self.arena.non_answer(s).or(self.arena.non_answer(t)) {
            return Ok(Tri::Unknown(why));
        }
        if s == t {
            return Ok(YES);
        }
        self.step()?;
        match (self.arena.get(s), self.arena.get(t)) {
            (Type::Intersection(a), Type::Intersection(b)) => {
                let mut a = self.arena.list(a).to_vec();
                let mut b = self.arena.list(b).to_vec();
                a.sort();
                b.sort();
                Ok(Tri::from_bool(a == b))
            }
            (
                Type::Class {
                    key: ka,
                    outer: oa,
                    args: aa,
                },
                Type::Class {
                    key: kb,
                    outer: ob,
                    args: ab,
                },
            ) if ka == kb => {
                // The same class, with arguments the same but for `?` and `? extends Object`,
                // which are one (JLS 4.5.1).
                let (aa, ab) = match (aa, ab) {
                    (Some(a), Some(b)) => {
                        (self.arena.args(a).to_vec(), self.arena.args(b).to_vec())
                    }
                    (None, None) => (Vec::new(), Vec::new()),
                    _ => return Ok(Tri::No),
                };
                if aa.len() != ab.len() {
                    return Ok(Tri::No);
                }
                let mut answer = match (oa, ob) {
                    (Some(a), Some(b)) => ferrion_stack::ensure_stack(|| self.same_(a, b))?,
                    (None, None) => YES,
                    _ => Tri::No,
                };
                for (a, b) in aa.into_iter().zip(ab) {
                    if answer.is_no() {
                        break;
                    }
                    let same = ferrion_stack::ensure_stack(|| self.same_arg(a, b))?;
                    if !same.is_yes() {
                        answer = same;
                    }
                }
                Ok(answer)
            }
            _ => Ok(Tri::No),
        }
    }

    fn same_arg(&mut self, a: TypeArg, b: TypeArg) -> R<Tri> {
        let unbounded = |cx: &mut Self, w: TypeArg| -> R<bool> {
            Ok(match w {
                TypeArg::Wildcard(None) => true,
                TypeArg::Wildcard(Some((WildcardBound::Extends, b))) => cx.is_object(b)?,
                _ => false,
            })
        };
        match (a, b) {
            (TypeArg::Type(x), TypeArg::Type(y)) => self.same_(x, y),
            (TypeArg::Wildcard(Some((ka, x))), TypeArg::Wildcard(Some((kb, y)))) if ka == kb => {
                self.same_(x, y)
            }
            (TypeArg::Wildcard(_), TypeArg::Wildcard(_)) => {
                Ok(Tri::from_bool(unbounded(self, a)? && unbounded(self, b)?))
            }
            _ => Ok(Tri::No),
        }
    }

    // Members.

    /// How a member of `owner` is seen from `of`: as declared for a static member, erased if
    /// `of`'s supertype of class `owner` is raw, else substituted.
    fn seen_from(&mut self, of: TypeId, owner: TypeKey, is_static: bool) -> R<Tri<View>> {
        if is_static {
            return Ok(Tri::Yes(View::Declared));
        }
        let sup = match self.as_super_(of, owner)? {
            Tri::Yes(sup) => sup,
            Tri::No => return Ok(Tri::No),
            Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
        };
        if self.is_raw(sup)? {
            return Ok(Tri::Yes(View::Erased));
        }
        let subst = self.theta(sup);
        Ok(Tri::Yes(if subst.is_empty() {
            View::Declared
        } else {
            View::Substituted(subst)
        }))
    }

    /// A method's declared type.
    pub fn method_type(&mut self, key: TypeKey, method: u32) -> Result<MethodType, Cancelled> {
        self.top(
            |cx| cx.unknown_method(),
            |cx| cx.declared_method(key, method),
        )
    }

    fn unknown_method(&mut self) -> MethodType {
        let unknown = self.arena.unknown(Why::Limit);
        MethodType {
            type_params: Vec::new(),
            params: Vec::new(),
            result: unknown,
            throws: Vec::new(),
        }
    }

    pub(crate) fn declared_method(&mut self, key: TypeKey, method: u32) -> R<MethodType> {
        let Some(decl) = self.decl(key)? else {
            let unknown = self.arena.unknown(Why::Incomplete(key));
            return Ok(MethodType {
                type_params: Vec::new(),
                params: Vec::new(),
                result: unknown,
                throws: Vec::new(),
            });
        };
        let (params, throws, type_params) = match decl.method(method as usize) {
            Some(m) => (m.params.len(), m.throws.len(), m.type_params.len()),
            None => (0, 0, 0),
        };
        let owner = TypeVarOwner::Method(key, method);
        let type_params = (0..type_params as u32)
            .map(|i| self.arena.var(owner, i))
            .collect();
        let mut out = MethodType {
            type_params,
            params: Vec::with_capacity(params),
            result: self.slot(key, Slot::Result(method))?,
            throws: Vec::with_capacity(throws),
        };
        for i in 0..params as u32 {
            out.params.push(self.slot(key, Slot::Parameter(method, i))?);
        }
        for i in 0..throws as u32 {
            out.throws.push(self.slot(key, Slot::Throws(method, i))?);
        }
        Ok(out)
    }

    pub(crate) fn member_flags(&mut self, member: Member) -> R<Flags> {
        Ok(match member {
            Member::Field(key, i) => self
                .decl(key)?
                .and_then(|d| d.field(i as usize).map(|f| f.2))
                .unwrap_or_default(),
            Member::Method(key, i) => self
                .decl(key)?
                .and_then(|d| d.method(i as usize).map(|m| m.flags))
                .unwrap_or_default(),
        })
    }

    /// A field's type as seen from `of` (JLS 4.5.2, 4.8): substituted for a parameterized type,
    /// erased for a raw one, through bounds for a type variable or an intersection.
    pub fn field_type_in(
        &mut self,
        of: TypeId,
        key: TypeKey,
        field: u32,
    ) -> Result<Tri<TypeId>, Cancelled> {
        self.top(
            |_| Tri::Unknown(Why::Limit),
            |cx| cx.field_in(of, key, field),
        )
    }

    pub(crate) fn field_in(&mut self, of: TypeId, key: TypeKey, field: u32) -> R<Tri<TypeId>> {
        let flags = self.member_flags(Member::Field(key, field))?;
        let declared = self.slot(key, Slot::Field(field))?;
        Ok(match self.seen_from(of, key, flags.has(Flags::STATIC))? {
            Tri::Yes(View::Declared) => Tri::Yes(declared),
            Tri::Yes(View::Erased) => Tri::Yes(self.erasure_(declared)?),
            Tri::Yes(View::Substituted(subst)) => Tri::Yes(self.subst(declared, &subst)?),
            Tri::No => Tri::No,
            Tri::Unknown(why) => Tri::Unknown(why),
        })
    }

    /// A method's type as seen from `of`, as [`TypeContext::field_type_in`] sees a field's; a
    /// raw type's methods lose their type parameters too.
    pub fn method_type_in(
        &mut self,
        of: TypeId,
        key: TypeKey,
        method: u32,
    ) -> Result<Tri<MethodType>, Cancelled> {
        self.top(
            |_| Tri::Unknown(Why::Limit),
            |cx| cx.method_in(of, key, method),
        )
    }

    pub(crate) fn method_in(
        &mut self,
        of: TypeId,
        key: TypeKey,
        method: u32,
    ) -> R<Tri<MethodType>> {
        let flags = self.member_flags(Member::Method(key, method))?;
        let declared = self.declared_method(key, method)?;
        Ok(match self.seen_from(of, key, flags.has(Flags::STATIC))? {
            Tri::Yes(View::Declared) => Tri::Yes(declared),
            Tri::Yes(View::Erased) => Tri::Yes(self.erase_method(&declared)?),
            Tri::Yes(View::Substituted(subst)) => Tri::Yes(self.subst_method(&declared, &subst)?),
            Tri::No => Tri::No,
            Tri::Unknown(why) => Tri::Unknown(why),
        })
    }

    /// A generic method's type as seen from `of` (`None` for its declared type) with `args` as its
    /// type arguments: its type parameters replaced and gone. `No` if an argument is outside its
    /// parameter's bound (JLS 15.12.2.2). Yes says whether a bound needed an unchecked conversion,
    /// which javac counts as one for the method's result (observed:
    /// `Comparator.<Comparable>naturalOrder()` is `()java/util/Comparator`).
    pub fn method_with_arguments(
        &mut self,
        of: Option<TypeId>,
        m: &MethodType,
        args: &[TypeId],
    ) -> Result<Tri<(MethodType, bool)>, Cancelled> {
        self.top(
            |_| Tri::Unknown(Why::Limit),
            |cx| {
                let mut subst = Subst::default();
                let mut vars = Vec::with_capacity(args.len());
                for (&var, &arg) in m.type_params.iter().zip(args) {
                    if let Type::Var(owner, index) = cx.arena.get(var) {
                        subst.push(owner, index, arg);
                        vars.push((owner, index, arg));
                    }
                }
                // The bounds, which can name the class's type variables too.
                let view = match (of, vars.first()) {
                    (Some(of), Some(&(TypeVarOwner::Method(owner, _), ..))) => {
                        match cx.seen_from(of, owner, false)? {
                            Tri::Yes(view) => Some(view),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let mut unknown = None;
                let mut unchecked = false;
                for (owner, index, arg) in vars {
                    for bound in cx.var_bounds(owner, index)?.to_vec() {
                        let bound = match &view {
                            Some(View::Substituted(s)) => cx.subst(bound, s)?,
                            Some(View::Erased) => cx.erasure_(bound)?,
                            _ => bound,
                        };
                        let bound = cx.subst(bound, &subst)?;
                        let answer =
                            cx.convert_(super::Context::StrictInvocation, arg, bound, None)?;
                        match answer {
                            Tri::Yes(steps) => {
                                unchecked |=
                                    steps.as_slice().contains(&super::Conversion::Unchecked);
                            }
                            Tri::No => return Ok(Tri::No),
                            Tri::Unknown(why) => {
                                unknown.get_or_insert(why);
                            }
                        }
                    }
                }
                if let Some(why) = unknown {
                    return Ok(Tri::Unknown(why));
                }
                let mut out = cx.subst_method(m, &subst)?;
                out.type_params.clear();
                Ok(Tri::Yes((out, unchecked)))
            },
        )
    }

    pub(crate) fn subst_method(&mut self, m: &MethodType, subst: &Subst) -> R<MethodType> {
        let mut out = MethodType {
            type_params: m.type_params.clone(),
            params: Vec::with_capacity(m.params.len()),
            result: self.subst(m.result, subst)?,
            throws: Vec::with_capacity(m.throws.len()),
        };
        for &p in &m.params {
            out.params.push(self.subst(p, subst)?);
        }
        for &t in &m.throws {
            out.throws.push(self.subst(t, subst)?);
        }
        Ok(out)
    }

    /// The erasure of a method's type: its type parameters dropped, every type erased.
    pub(crate) fn erase_method(&mut self, m: &MethodType) -> R<MethodType> {
        let mut out = MethodType {
            type_params: Vec::new(),
            params: Vec::with_capacity(m.params.len()),
            result: self.erasure_(m.result)?,
            throws: Vec::with_capacity(m.throws.len()),
        };
        for &p in &m.params {
            out.params.push(self.erasure_(p)?);
        }
        for &t in &m.throws {
            out.throws.push(self.erasure_(t)?);
        }
        Ok(out)
    }

    // Signatures.

    /// Whether two methods have the same signature, names aside (JLS 8.4.2): the same type
    /// parameters, after `n`'s are renamed to `m`'s, and the same parameter types.
    fn same_signature(&mut self, m: &MethodType, n: &MethodType) -> R<Tri> {
        if m.params.len() != n.params.len() || m.type_params.len() != n.type_params.len() {
            return Ok(Tri::No);
        }
        let mut subst = Subst::default();
        for (&a, &b) in n.type_params.iter().zip(&m.type_params) {
            if let Type::Var(owner, index) = self.arena.get(a) {
                subst.push(owner, index, b);
            }
        }
        let mut answer = YES;
        // The same type parameters: the same bounds, once renamed (JLS 8.4.4).
        for (&a, &b) in n.type_params.iter().zip(&m.type_params) {
            let (Type::Var(ao, ai), Type::Var(bo, bi)) = (self.arena.get(a), self.arena.get(b))
            else {
                continue;
            };
            let (ab, bb) = (
                self.var_bounds(ao, ai)?.to_vec(),
                self.var_bounds(bo, bi)?.to_vec(),
            );
            if ab.len() != bb.len() {
                return Ok(Tri::No);
            }
            for (x, y) in ab.into_iter().zip(bb) {
                let x = self.subst(x, &subst)?;
                match self.same_(x, y)? {
                    Tri::No => return Ok(Tri::No),
                    Tri::Unknown(why) => answer = Tri::Unknown(why),
                    Tri::Yes(()) => {}
                }
            }
        }
        for (&a, &b) in m.params.iter().zip(&n.params) {
            let b = self.subst(b, &subst)?;
            match self.same_(a, b)? {
                Tri::No => return Ok(Tri::No),
                Tri::Unknown(why) => answer = Tri::Unknown(why),
                Tri::Yes(()) => {}
            }
        }
        Ok(answer)
    }

    /// Whether `m`'s signature is a subsignature of `n`'s (JLS 8.4.2): the same, or the same as
    /// `n`'s erasure.
    pub fn subsignature(&mut self, m: &MethodType, n: &MethodType) -> Result<Tri, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.subsignature_(m, n))
    }

    pub(crate) fn subsignature_(&mut self, m: &MethodType, n: &MethodType) -> R<Tri> {
        let same = self.same_signature(m, n)?;
        if same.is_yes() || !m.type_params.is_empty() {
            return Ok(same);
        }
        let erased = self.erase_method(n)?;
        let mut answer = YES;
        if m.params.len() != erased.params.len() {
            return Ok(same);
        }
        for (&a, &b) in m.params.iter().zip(&erased.params) {
            match self.same_(a, b)? {
                Tri::No => return Ok(same),
                Tri::Unknown(why) => answer = Tri::Unknown(why),
                Tri::Yes(()) => {}
            }
        }
        Ok(match (answer, same) {
            (Tri::Yes(()), _) => YES,
            (Tri::Unknown(why), _) | (_, Tri::Unknown(why)) => Tri::Unknown(why),
            _ => Tri::No,
        })
    }

    /// Whether two methods are override-equivalent (JLS 8.4.2): either is a subsignature of the
    /// other.
    pub fn override_equivalent(
        &mut self,
        m: &MethodType,
        n: &MethodType,
    ) -> Result<Tri, Cancelled> {
        self.top(
            |_| Tri::Unknown(Why::Limit),
            |cx| {
                let a = cx.subsignature_(m, n)?;
                if a.is_yes() {
                    return Ok(a);
                }
                let b = cx.subsignature_(n, m)?;
                Ok(match (a, b) {
                    (_, Tri::Yes(())) => YES,
                    (Tri::Unknown(why), _) | (_, Tri::Unknown(why)) => Tri::Unknown(why),
                    _ => Tri::No,
                })
            },
        )
    }

    /// Whether `m`'s result is return-type-substitutable for `n`'s (JLS 8.4.5).
    pub fn return_substitutable(
        &mut self,
        m: &MethodType,
        n: &MethodType,
    ) -> Result<Tri, Cancelled> {
        self.top(
            |_| Tri::Unknown(Why::Limit),
            |cx| cx.return_substitutable_(m, n),
        )
    }

    fn return_substitutable_(&mut self, m: &MethodType, n: &MethodType) -> R<Tri> {
        let (r1, r2) = (m.result, n.result);
        if let Some(why) = self.arena.non_answer(r1).or(self.arena.non_answer(r2)) {
            return Ok(Tri::Unknown(why));
        }
        match (self.arena.get(r1), self.arena.get(r2)) {
            (Type::Void, other) => return Ok(Tri::from_bool(other == Type::Void)),
            (Type::Primitive(_), _) => return Ok(Tri::from_bool(r1 == r2)),
            (_, Type::Void | Type::Primitive(_)) => return Ok(Tri::No),
            _ => {}
        }
        // `r1`, with `m`'s type parameters renamed to `n`'s, a subtype of `r2`; or `r2` its
        // erasure.
        let mut subst = Subst::default();
        if m.type_params.len() == n.type_params.len() {
            for (&a, &b) in m.type_params.iter().zip(&n.type_params) {
                if let Type::Var(owner, index) = self.arena.get(a) {
                    subst.push(owner, index, b);
                }
            }
        }
        let adapted = self.subst(r1, &subst)?;
        let sub = self.subtype_(adapted, r2)?;
        if sub.is_yes() {
            return Ok(sub);
        }
        // An unchecked conversion: `r1` raw, its class's type a subtype of `r2`'s erasure.
        let erased_r2 = self.erasure_(r2)?;
        let erased_r1 = self.erasure_(adapted)?;
        if self.is_raw(adapted)? && self.subtype_(erased_r1, erased_r2)?.is_yes() {
            return Ok(YES);
        }
        if !self.same_signature(m, n)?.is_yes() && r1 == erased_r2 {
            return Ok(YES);
        }
        Ok(sub)
    }

    /// Whether `m`, as a member of the class `of`, overrides `n` (JLS 8.4.8.1, 9.4.1.1), as
    /// `Elements.overrides` answers it: not the same method, the same name, neither static nor
    /// `n` private, `n` a member of a supertype of `of` accessible to it, and `m`'s signature a
    /// subsignature of `n`'s, both as members of `of`.
    pub fn overrides(
        &mut self,
        m: (TypeKey, u32),
        n: (TypeKey, u32),
        of: TypeKey,
    ) -> Result<Tri, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.overrides_(m, n, of))
    }

    fn overrides_(&mut self, m: (TypeKey, u32), n: (TypeKey, u32), of: TypeKey) -> R<Tri> {
        if m == n {
            return Ok(Tri::No);
        }
        let (Some(md), Some(nd)) = (self.decl(m.0)?, self.decl(n.0)?) else {
            return Ok(Tri::Unknown(Why::Incomplete(m.0)));
        };
        let (Some(mm), Some(nm)) = (md.method(m.1 as usize), nd.method(n.1 as usize)) else {
            return Ok(Tri::No);
        };
        if mm.name != nm.name
            || mm.flags.has(Flags::CONSTRUCTOR)
            || nm.flags.has(Flags::CONSTRUCTOR)
            || mm.flags.has(Flags::STATIC)
            || nm.flags.has(Flags::STATIC)
            || nm.flags.has(Flags::PRIVATE)
            || mm.flags.has(Flags::PRIVATE)
        {
            return Ok(Tri::No);
        }
        let n_interface = nd.is_interface();
        let package_access =
            !nm.flags.has(Flags::PUBLIC) && !nm.flags.has(Flags::PROTECTED) && !n_interface;
        // An interface has `Object`'s public methods only (JLS 9.2): `clone()` in an interface
        // doesn't override `Object.clone()` (W1's observation 6).
        let of_interface = self.decl(of)?.is_some_and(|d| d.is_interface());
        if of_interface && !n_interface && !nm.flags.has(Flags::PUBLIC) {
            return Ok(Tri::No);
        }
        drop((md, nd));
        if package_access {
            let names = self.names.clone();
            if crate::resolve::package_of(n.0, &names) != crate::resolve::package_of(of, &names) {
                return Ok(Tri::No);
            }
        }
        // `n`'s class a proper supertype of `of`.
        if n.0 == of {
            return Ok(Tri::No);
        }
        let this = self.this_type(of)?;
        match self.as_super_(this, n.0)? {
            Tri::Yes(_) => {}
            Tri::No => return Ok(Tri::No),
            Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
        }
        let (Tri::Yes(mt), Tri::Yes(nt)) = (
            self.method_in(this, m.0, m.1)?,
            self.method_in(this, n.0, n.1)?,
        ) else {
            return Ok(Tri::Unknown(Why::Incomplete(of)));
        };
        self.subsignature_(&mt, &nt)
    }
}
