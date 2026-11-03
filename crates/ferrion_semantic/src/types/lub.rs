//! Least upper bounds (JLS 4.10.4), greatest lower bounds (5.1.10), the order of an
//! intersection's parts, and upward projection (4.10.5).
//!
//! javac's choices, observed (docs/design/semantic-type-relations.md#observations, and
//! observations 1 to 3 in docs/design/types-oracle-normalization.md):
//!
//! - `lub`'s recursion through type arguments, which JLS 4.10.4 lets produce an infinite type
//!   (`lub(Integer, String)` is `Comparable<? extends lub(Integer, String)> & ...`), is computed
//!   while the same operands are on the stack at most twice; a third time, the argument is `?`.
//! - `lub` captures its operands, and each parameterization it combines, so its type arguments
//!   are `? extends` the lub of two types, or a type both have.
//! - `glb` compares two classes' types erased: `glb(Integer, Comparable<CAP>)` is `Integer`.
//! - An intersection's parts are ordered, `lub`'s and `glb`'s alike: the class first, then
//!   interfaces deepest first (the longest chain of superinterfaces above them), then by
//!   qualified name.

use super::{Active, R, Tri, Type, TypeArg, TypeContext, TypeId, Why};
use crate::Cancelled;
use crate::body::Map;
use crate::summary::WildcardBound;
use crate::symbols::TypeKey;

/// A candidate of a least upper bound: an erased class, or a type variable.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Candidate {
    Class(TypeKey),
    Var(TypeId),
}

impl TypeContext<'_> {
    /// The least upper bound of reference types (JLS 4.10.4), cut off as javac cuts it.
    pub fn lub(&mut self, types: &[TypeId]) -> Result<TypeId, Cancelled> {
        self.top(
            |cx| cx.arena.unknown(Why::Limit),
            |cx| {
                Ok(match cx.lub_(types)? {
                    Some(t) => t,
                    None => cx.object_or_error()?,
                })
            },
        )
    }

    /// The least upper bound, or `None` where javac cuts its recursion off.
    pub(crate) fn lub_(&mut self, types: &[TypeId]) -> R<Option<TypeId>> {
        self.step()?;
        let mut operands: Vec<TypeId> = Vec::with_capacity(types.len());
        for &t in types {
            if self.arena.non_answer(t).is_some() {
                return Ok(Some(t));
            }
            // The operands are captured, as an expression's type is (W1's observation 4): JLS
            // 4.10.4's `lcta(? extends U, ? super U) = U` isn't above `G<? super U>`.
            let t = self.capture_(t)?;
            if self.arena.get(t) == Type::Null || operands.contains(&t) {
                continue;
            }
            if !self.arena.is_reference(t) {
                return Ok(Some(self.arena.unknown(Why::Later)));
            }
            operands.push(t);
        }
        match operands[..] {
            [] => return Ok(Some(self.arena.intern(Type::Null))),
            [one] => return Ok(Some(one)),
            _ => {}
        }
        // One operand a supertype of every other's is the answer.
        for &candidate in &operands {
            let mut all = true;
            for &other in &operands {
                if other != candidate && !self.subtype_(other, candidate)?.is_yes() {
                    all = false;
                    break;
                }
            }
            if all {
                return Ok(Some(candidate));
            }
        }
        let mut key = operands.clone();
        key.sort();
        let list = self.arena.type_list(&key);
        let depth = self.lubs.get(&list).copied().unwrap_or(0);
        if depth >= 2 {
            return Ok(None);
        }
        self.lubs.insert(list, depth + 1);
        let query = Active::Lub(list);
        let fresh = self.active.insert(query);
        let result = self.find_lub(&operands);
        if fresh {
            self.leave(query);
        }
        match self.lubs.get_mut(&list) {
            Some(n) if *n > 1 => *n -= 1,
            _ => {
                self.lubs.remove(&list);
            }
        }
        result.map(Some)
    }

    fn find_lub(&mut self, operands: &[TypeId]) -> R<TypeId> {
        // Arrays: of the same primitive, that type; of references, an array of their
        // components' lub; otherwise, as `Object`, `Cloneable`, `Serializable` would give.
        if operands
            .iter()
            .all(|&t| matches!(self.arena.get(t), Type::Array(_)))
        {
            let components: Vec<TypeId> = operands
                .iter()
                .map(|&t| match self.arena.get(t) {
                    Type::Array(c) => c,
                    _ => unreachable!("an array"),
                })
                .collect();
            if components.iter().all(|&c| self.arena.is_reference(c)) {
                return Ok(match self.lub_(&components)? {
                    Some(c) => self.arena.array(c),
                    None => {
                        let object = self.object_or_error()?;
                        self.arena.array(object)
                    }
                });
            }
        }
        // Each operand's supertypes, by candidate: every parameterization of each class.
        let mut closures: Vec<Map<Candidate, Vec<TypeId>>> = Vec::with_capacity(operands.len());
        for &t in operands {
            closures.push(self.closure(t)?);
        }
        let mut common: Vec<Candidate> = closures[0]
            .keys()
            .copied()
            .filter(|c| closures[1..].iter().all(|m| m.contains_key(c)))
            .collect();
        common.sort_by_key(|c| match c {
            Candidate::Class(k) => (0, Some(*k), None),
            Candidate::Var(t) => (1, None, Some(*t)),
        });
        // The minimal candidates.
        let mut erased = Vec::with_capacity(common.len());
        for &c in &common {
            erased.push(match c {
                Candidate::Class(key) => self.arena.class(key, None, None),
                Candidate::Var(t) => t,
            });
        }
        let mut minimal = Vec::new();
        for (i, &c) in common.iter().enumerate() {
            let mut keep = true;
            for (j, &other) in erased.iter().enumerate() {
                if i != j && self.subtype_(other, erased[i])?.is_yes() {
                    keep = false;
                    break;
                }
            }
            if keep {
                minimal.push(c);
            }
        }
        let mut parts = Vec::with_capacity(minimal.len());
        for c in minimal {
            let best = match c {
                Candidate::Var(t) => t,
                Candidate::Class(key) => {
                    let mut relevant: Vec<TypeId> = Vec::new();
                    for closure in &closures {
                        for &t in &closure[&c] {
                            if !relevant.contains(&t) {
                                relevant.push(t);
                            }
                        }
                    }
                    let mut raw = false;
                    for &t in &relevant {
                        raw |= self.is_raw(t)?;
                    }
                    if raw || relevant.is_empty() {
                        self.arena.class(key, None, None)
                    } else {
                        let mut best = relevant[0];
                        for &t in &relevant[1..] {
                            best = self.lci(best, t)?;
                        }
                        best
                    }
                }
            };
            parts.push(best);
        }
        if parts.is_empty() {
            return self.object_or_error();
        }
        let parts = self.order(parts)?;
        Ok(self.arena.intersection(&parts))
    }

    /// A type's supertypes, itself included, by candidate, each with the parameterizations
    /// found; `Object` is among every reference type's.
    fn closure(&mut self, t: TypeId) -> R<Map<Candidate, Vec<TypeId>>> {
        let mut out: Map<Candidate, Vec<TypeId>> = Map::default();
        let mut pending = vec![t];
        let mut seen = Vec::new();
        while let Some(t) = pending.pop() {
            self.step()?;
            if seen.contains(&t) {
                continue;
            }
            seen.push(t);
            match self.arena.get(t) {
                Type::Class { key, .. } => {
                    out.entry(Candidate::Class(key)).or_default().push(t);
                }
                Type::Var(..) | Type::Capture(_) => {
                    out.entry(Candidate::Var(t)).or_default().push(t);
                }
                Type::Intersection(_) | Type::Array(_) => {}
                _ => continue,
            }
            if let Tri::Yes(supers) = self.direct_supertypes_(t)? {
                pending.extend(supers);
            }
        }
        if let Some(object) = self.object()?
            && let Type::Class { key, .. } = self.arena.get(object)
        {
            let entry = out.entry(Candidate::Class(key)).or_default();
            if entry.is_empty() {
                entry.push(object);
            }
        }
        Ok(out)
    }

    /// The least containing invocation of two parameterizations of one class (JLS 4.10.4).
    fn lci(&mut self, a: TypeId, b: TypeId) -> R<TypeId> {
        if a == b {
            return Ok(a);
        }
        // Each parameterization is captured, as the operands are (W1's observation 4): javac's
        // `lub(List<? super Comparable<? super C>>, List<String>)` is `List<? extends Object>`,
        // not 4.10.4's `List<? super glb(...)>`.
        let (a, b) = (self.capture_(a)?, self.capture_(b)?);
        if a == b {
            return Ok(a);
        }
        let (
            Type::Class {
                key,
                outer: ao,
                args: aa,
            },
            Type::Class {
                outer: bo,
                args: ba,
                ..
            },
        ) = (self.arena.get(a), self.arena.get(b))
        else {
            return Ok(a);
        };
        let args = match (aa, ba) {
            (Some(aa), Some(ba)) => {
                let (aa, ba) = (self.arena.args(aa).to_vec(), self.arena.args(ba).to_vec());
                let mut args = Vec::with_capacity(aa.len());
                for (x, y) in aa.into_iter().zip(ba) {
                    args.push(self.lcta(x, y)?);
                }
                Some(args)
            }
            _ => None,
        };
        // A member of two parameterizations of its outer class is a member of their lci.
        let outer = match (ao, bo) {
            (Some(x), Some(y)) => Some(ferrion_stack::ensure_stack(|| self.lci(x, y))?),
            _ => None,
        };
        Ok(self.arena.class(key, outer, args.as_deref()))
    }

    /// The least containing type argument (JLS 4.10.4).
    fn lcta(&mut self, a: TypeArg, b: TypeArg) -> R<TypeArg> {
        use WildcardBound::{Extends, Super};
        let extends = |cx: &mut Self, u: TypeId, v: TypeId| -> R<TypeArg> {
            Ok(match cx.lub_(&[u, v])? {
                Some(l) => TypeArg::Wildcard(Some((Extends, l))),
                None => TypeArg::Wildcard(None),
            })
        };
        Ok(match (a, b) {
            (TypeArg::Type(u), TypeArg::Type(v)) if u == v => TypeArg::Type(u),
            (TypeArg::Type(u), TypeArg::Type(v)) => extends(self, u, v)?,
            (TypeArg::Type(u), TypeArg::Wildcard(Some((Extends, v))))
            | (TypeArg::Wildcard(Some((Extends, v))), TypeArg::Type(u))
            | (TypeArg::Wildcard(Some((Extends, u))), TypeArg::Wildcard(Some((Extends, v)))) => {
                extends(self, u, v)?
            }
            (TypeArg::Type(u), TypeArg::Wildcard(Some((Super, v))))
            | (TypeArg::Wildcard(Some((Super, v))), TypeArg::Type(u))
            | (TypeArg::Wildcard(Some((Super, u))), TypeArg::Wildcard(Some((Super, v)))) => {
                TypeArg::Wildcard(Some((Super, self.glb_(&[u, v])?)))
            }
            (TypeArg::Wildcard(Some((Extends, u))), TypeArg::Wildcard(Some((Super, v))))
            | (TypeArg::Wildcard(Some((Super, v))), TypeArg::Wildcard(Some((Extends, u)))) => {
                if u == v {
                    TypeArg::Type(u)
                } else {
                    TypeArg::Wildcard(None)
                }
            }
            (TypeArg::Wildcard(None), _) | (_, TypeArg::Wildcard(None)) => TypeArg::Wildcard(None),
        })
    }

    /// The greatest lower bound (JLS 5.1.10): the parts that no other part is below, ordered as
    /// an intersection's are.
    pub fn glb(&mut self, types: &[TypeId]) -> Result<TypeId, Cancelled> {
        self.top(|cx| cx.arena.unknown(Why::Limit), |cx| cx.glb_(types))
    }

    pub(crate) fn glb_(&mut self, types: &[TypeId]) -> R<TypeId> {
        let mut flat: Vec<TypeId> = Vec::with_capacity(types.len());
        for &t in types {
            if self.arena.non_answer(t).is_some() {
                return Ok(t);
            }
            match self.arena.get(t) {
                Type::Intersection(list) => {
                    for &p in self.arena.list(list) {
                        if !flat.contains(&p) {
                            flat.push(p);
                        }
                    }
                }
                _ if !flat.contains(&t) => flat.push(t),
                _ => {}
            }
        }
        let mut kept: Vec<TypeId> = Vec::with_capacity(flat.len());
        for (i, &t) in flat.iter().enumerate() {
            let mut redundant = false;
            for (j, &other) in flat.iter().enumerate() {
                if i == j {
                    continue;
                }
                // A part above another is redundant; of two equal ones, the later goes. Two
                // class types are compared erased, as javac does (W1's observation 5):
                // `glb(Integer, Comparable<CAP>)` is `Integer`.
                // Two parameterizations of one class are compared as they are.
                let classes = match (self.arena.get(t), self.arena.get(other)) {
                    (Type::Class { key: a, .. }, Type::Class { key: b, .. }) => a != b,
                    _ => false,
                };
                let (a, b) = if classes {
                    (self.erasure_(other)?, self.erasure_(t)?)
                } else {
                    (other, t)
                };
                if self.subtype_(a, b)?.is_yes() && (j < i || !self.subtype_(b, a)?.is_yes()) {
                    redundant = true;
                    break;
                }
            }
            if !redundant {
                kept.push(t);
            }
        }
        match kept[..] {
            [] => self.object_or_error(),
            [one] => Ok(one),
            _ => {
                let kept = self.order(kept)?;
                Ok(self.arena.intersection(&kept))
            }
        }
    }

    /// Orders an intersection's parts as javac does (observation 2, and W1's observations 2 and
    /// 5): type variables and the class first, then interfaces deepest first, then by qualified
    /// name.
    pub(crate) fn order(&mut self, parts: Vec<TypeId>) -> R<Vec<TypeId>> {
        let mut keyed = Vec::with_capacity(parts.len());
        for t in parts {
            let (rank, depth, name) = match self.arena.get(t) {
                Type::Class { key, .. } => {
                    let interface = self.decl(key)?.is_some_and(|d| d.is_interface());
                    let name = self.qualified_name(key);
                    if interface {
                        (2, self.interface_depth(key, 0)?, name)
                    } else {
                        (1, 0, name)
                    }
                }
                _ => (0, 0, String::new()),
            };
            keyed.push((rank, std::cmp::Reverse(depth), name, t));
        }
        keyed.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        Ok(keyed.into_iter().map(|k| k.3).collect())
    }

    /// The longest chain of superinterfaces above an interface: 0 for one with none.
    fn interface_depth(&mut self, key: TypeKey, level: u32) -> R<u32> {
        if level > 256 {
            return Ok(0);
        }
        self.step()?;
        let supers = self.declared_supertypes(key)?.to_vec();
        let mut depth = 0;
        for s in supers {
            if let Type::Class { key: k, .. } = self.arena.get(s)
                && self.decl(k)?.is_some_and(|d| d.is_interface())
            {
                depth = depth
                    .max(1 + ferrion_stack::ensure_stack(|| self.interface_depth(k, level + 1))?);
            }
        }
        Ok(depth)
    }

    /// A class's qualified name, for ordering: its binary name with `.` for `/` and `$`.
    fn qualified_name(&mut self, key: TypeKey) -> String {
        self.binary_text(key).replace(['/', '$'], ".")
    }

    // Upward projection.

    /// The upward projection of a type (JLS 4.10.5): its capture variables replaced by types
    /// that mention none, as `var` takes an initializer's type.
    pub fn upward_projection(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        self.top(
            |cx| cx.arena.unknown(Why::Limit),
            |cx| {
                let mut active = Vec::new();
                Ok(match cx.upward(t, &mut active)? {
                    Some(t) => t,
                    None => cx.object_or_error()?,
                })
            },
        )
    }

    /// Whether a type mentions a capture variable.
    pub(crate) fn mentions_capture(&mut self, t: TypeId) -> R<bool> {
        self.step()?;
        Ok(match self.arena.get(t) {
            Type::Capture(_) => true,
            Type::Class { outer, args, .. } => {
                if let Some(o) = outer
                    && ferrion_stack::ensure_stack(|| self.mentions_capture(o))?
                {
                    return Ok(true);
                }
                let args = args
                    .map(|a| self.arena.args(a).to_vec())
                    .unwrap_or_default();
                for a in args {
                    let t = match a {
                        TypeArg::Type(t) | TypeArg::Wildcard(Some((_, t))) => t,
                        TypeArg::Wildcard(None) => continue,
                    };
                    if ferrion_stack::ensure_stack(|| self.mentions_capture(t))? {
                        return Ok(true);
                    }
                }
                false
            }
            Type::Array(c) => ferrion_stack::ensure_stack(|| self.mentions_capture(c))?,
            Type::Intersection(list) => {
                for p in self.arena.list(list).to_vec() {
                    if ferrion_stack::ensure_stack(|| self.mentions_capture(p))? {
                        return Ok(true);
                    }
                }
                false
            }
            _ => false,
        })
    }

    /// The upward projection, or `None` where a capture variable's bound mentions itself, which
    /// leaves `?` (observation 9: `Enum<?>`'s capture projects to `Enum<?>`).
    fn upward(&mut self, t: TypeId, active: &mut Vec<TypeId>) -> R<Option<TypeId>> {
        if !self.mentions_capture(t)? {
            return Ok(Some(t));
        }
        self.step()?;
        Ok(Some(match self.arena.get(t) {
            Type::Capture(n) => {
                if active.contains(&t) {
                    return Ok(None);
                }
                active.push(t);
                let upper = self.arena.capture(n).upper;
                let projected = ferrion_stack::ensure_stack(|| self.upward(upper, active));
                active.pop();
                return projected;
            }
            Type::Class { key, outer, args } => {
                let outer = match outer {
                    Some(o) => ferrion_stack::ensure_stack(|| self.upward(o, active))?,
                    None => None,
                };
                let old = args
                    .map(|a| self.arena.args(a).to_vec())
                    .unwrap_or_default();
                let mut new = Vec::with_capacity(old.len());
                for arg in old {
                    new.push(match arg {
                        TypeArg::Type(a) if !self.mentions_capture(a)? => arg,
                        TypeArg::Type(a) => {
                            match ferrion_stack::ensure_stack(|| self.upward(a, active))? {
                                Some(u) if !self.is_object(u)? => {
                                    TypeArg::Wildcard(Some((WildcardBound::Extends, u)))
                                }
                                Some(_) | None => match self.downward(a)? {
                                    Some(d) => TypeArg::Wildcard(Some((WildcardBound::Super, d))),
                                    None => TypeArg::Wildcard(None),
                                },
                            }
                        }
                        TypeArg::Wildcard(Some((WildcardBound::Extends, b))) => {
                            match ferrion_stack::ensure_stack(|| self.upward(b, active))? {
                                Some(u) => TypeArg::Wildcard(Some((WildcardBound::Extends, u))),
                                None => TypeArg::Wildcard(None),
                            }
                        }
                        TypeArg::Wildcard(Some((WildcardBound::Super, b))) => {
                            match self.downward(b)? {
                                Some(d) => TypeArg::Wildcard(Some((WildcardBound::Super, d))),
                                None => TypeArg::Wildcard(None),
                            }
                        }
                        TypeArg::Wildcard(None) => arg,
                    });
                }
                let args = args.map(|_| &new[..]);
                self.arena.class(key, outer, args)
            }
            Type::Array(c) => match ferrion_stack::ensure_stack(|| self.upward(c, active))? {
                Some(c) => self.arena.array(c),
                None => {
                    let object = self.object_or_error()?;
                    self.arena.array(object)
                }
            },
            Type::Intersection(list) => {
                let parts = self.arena.list(list).to_vec();
                let mut new = Vec::with_capacity(parts.len());
                for p in parts {
                    if let Some(u) = ferrion_stack::ensure_stack(|| self.upward(p, active))? {
                        new.push(u);
                    }
                }
                self.glb_(&new)?
            }
            _ => t,
        }))
    }

    /// The downward projection (JLS 4.10.5), where it is defined: a capture variable's lower
    /// bound, or a type that mentions none.
    fn downward(&mut self, t: TypeId) -> R<Option<TypeId>> {
        if !self.mentions_capture(t)? {
            return Ok(Some(t));
        }
        Ok(match self.arena.get(t) {
            Type::Capture(n) => match self.arena.capture(n).lower {
                Some(lower) => ferrion_stack::ensure_stack(|| self.downward(lower))?,
                None => None,
            },
            _ => None,
        })
    }
}
