//! Type relations (B3a, docs/design/semantic-type-relations.md): types in an arena per
//! computation, translated from B2's [`Ty`], and the relations every later part of B3 asks of
//! them: subtyping, containment, capture, erasure, supertypes, member types as seen from a type,
//! signatures, least upper and greatest lower bounds, and conversions in each context.
//!
//! A [`TypeContext`] is one computation: a view (a source set), an [`Arena`] its types live in,
//! and memo tables for what it has answered. Nothing in it is shared between threads or outlives
//! it, but the arena, which a result may keep ([`TypeContext::into_arena`]).
//!
//! Every relation answers [`Tri`]: `Yes`, `No`, or `Unknown`. A type that depends on a name that
//! didn't resolve is an [`Type::Error`] with its root cause, and anything that touches one, or a
//! part of a hierarchy that didn't resolve, is `Unknown` unless the parts that are known already
//! settle it. Every step of a relation's recursion draws on the context's fuel; a query that runs
//! out answers `Unknown(Limit)`, and so does one that meets itself further down (a cycle), which
//! costs no fuel (docs/design/semantic-type-relations.md#termination).
//!
//! Where javac departs from the JLS, the relations follow javac, and each departure is an
//! observation in the design, with its input, and a test here.

mod arena;
mod convert;
mod decl;
mod lub;
mod print;
mod relations;
#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;

pub use self::arena::{Arena, ArgList, CaptureVar, ErrorId, Type, TypeArg, TypeId, TypeList, Why};
pub use self::convert::{Context, Conversion, Steps};
pub use self::print::Printed;
pub use self::relations::{Member, MethodType};

use self::decl::{Body, Decl};
use crate::body::{Map, Set};
use crate::summary::{Primitive, TypeKind};
use crate::symbols::{Cause, ErrorSymbol, Ty, TyPart, TypeKey, TypeVarOwner};
use crate::{Cancelled, Interner, Name, Snapshot, SourceSetId};

/// A three-valued answer: `Yes` with what was found, `No`, or `Unknown` with why.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tri<T = ()> {
    Yes(T),
    No,
    Unknown(Why),
}

impl<T> Tri<T> {
    pub fn is_yes(&self) -> bool {
        matches!(self, Tri::Yes(_))
    }

    pub fn is_no(&self) -> bool {
        matches!(self, Tri::No)
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Tri<U> {
        match self {
            Tri::Yes(t) => Tri::Yes(f(t)),
            Tri::No => Tri::No,
            Tri::Unknown(why) => Tri::Unknown(why),
        }
    }

    /// The answer without what was found.
    pub fn plain(&self) -> Tri {
        match self {
            Tri::Yes(_) => Tri::Yes(()),
            Tri::No => Tri::No,
            Tri::Unknown(why) => Tri::Unknown(*why),
        }
    }
}

/// `Yes` for a plain answer.
pub const YES: Tri = Tri::Yes(());

impl Tri {
    fn from_bool(b: bool) -> Tri {
        if b { YES } else { Tri::No }
    }
}

/// The fuel a context starts with, and [`TypeContext::refuel`] gives it: per query in the
/// relations comparison, per body in attribution. Provisional, ten times above the most any
/// query of the corpus's relations comparison used (docs/design/semantic-type-relations.md,
/// "Results"), until B3c's review sets the limits.
pub const DEFAULT_FUEL: u64 = 100_000;

/// Why a computation stopped before its answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stop {
    Limit,
    Cancelled,
}

impl From<Cancelled> for Stop {
    fn from(_: Cancelled) -> Stop {
        Stop::Cancelled
    }
}

pub(crate) type R<T> = Result<T, Stop>;

/// Which of a declaration's types: the memo key of a translated [`Ty`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Slot {
    Superclass,
    Interface(u32),
    Field(u32),
    Parameter(u32, u32),
    Result(u32),
    Throws(u32, u32),
    /// A type parameter's bound: of the class (`None`) or a method, the parameter, the bound.
    Bound(Option<u32>, u32, u32),
}

/// A query in progress, so that one that meets itself is a cycle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Active {
    Subtype(TypeId, TypeId),
    SuperDecl(TypeKey, TypeKey),
    Outer(TypeKey, TypeKey),
    Supertypes(TypeKey),
    Bounds(TypeVarOwner, u32),
    Lub(TypeList),
}

/// One computation's types and relations in a view (docs/design/semantic-types.md#the-type-model).
pub struct TypeContext<'s> {
    snapshot: &'s Snapshot,
    view: SourceSetId,
    names: Arc<Interner>,
    arena: Arena,
    decls: Map<TypeKey, Option<Rc<Decl>>>,
    binaries: Map<Name, Option<TypeKey>>,
    translated: Map<(TypeKey, Slot), TypeId>,
    bounds: Map<(TypeVarOwner, u32), Rc<[TypeId]>>,
    supertypes: Map<TypeKey, Rc<[TypeId]>>,
    this_types: Map<TypeKey, TypeId>,
    outers: Map<(TypeKey, TypeKey), Option<TypeId>>,
    super_decls: Map<(TypeKey, TypeKey), Tri<Option<TypeId>>>,
    subtypes: Map<(TypeId, TypeId), Tri>,
    captured: Map<TypeId, TypeId>,
    erased: Map<TypeId, TypeId>,
    active: Set<Active>,
    /// Lub operand lists being computed, each with how many times it is on the stack.
    lubs: Map<TypeList, u32>,
    /// Cycles met so far: an answer computed while this grew isn't memoized.
    cycles: u64,
    fuel: u64,
    used: u64,
    /// The steps at the next check for cancellation.
    next_check: u64,
    init: Name,
    texts: print::Texts,
}

impl Snapshot {
    /// A relations context for `view`, with its own arena: the entry point for tests, the
    /// relations comparison, and attribution.
    pub fn types(&self, view: SourceSetId) -> TypeContext<'_> {
        let names = self.names().clone();
        TypeContext {
            snapshot: self,
            view,
            init: names.intern("<init>"),
            names,
            arena: Arena::new(),
            decls: Map::default(),
            binaries: Map::default(),
            translated: Map::default(),
            bounds: Map::default(),
            supertypes: Map::default(),
            this_types: Map::default(),
            outers: Map::default(),
            super_decls: Map::default(),
            subtypes: Map::default(),
            captured: Map::default(),
            erased: Map::default(),
            active: Set::default(),
            lubs: Map::default(),
            cycles: 0,
            fuel: DEFAULT_FUEL,
            used: 0,
            next_check: 4096,
            texts: print::Texts::default(),
        }
    }
}

impl<'s> TypeContext<'s> {
    pub fn snapshot(&self) -> &'s Snapshot {
        self.snapshot
    }

    pub fn view(&self) -> SourceSetId {
        self.view
    }

    pub fn arena(&self) -> &Arena {
        &self.arena
    }

    pub fn arena_mut(&mut self) -> &mut Arena {
        &mut self.arena
    }

    /// The arena, for a result that keeps its types.
    pub fn into_arena(self) -> Arena {
        self.arena
    }

    pub fn get(&self, id: TypeId) -> Type {
        self.arena.get(id)
    }

    /// Sets the fuel for what follows to `budget` steps, and starts counting again.
    pub fn refuel(&mut self, budget: u64) {
        self.fuel = budget;
        self.used = 0;
        self.next_check = 4096;
    }

    /// The steps taken since the last [`TypeContext::refuel`].
    pub fn fuel_used(&self) -> u64 {
        self.used
    }

    /// One step of a relation's recursion.
    pub(crate) fn step(&mut self) -> R<()> {
        self.used += 1;
        if self.used > self.fuel {
            return Err(Stop::Limit);
        }
        if self.used >= self.next_check {
            self.next_check = self.used + 4096;
            self.snapshot.check()?;
        }
        Ok(())
    }

    /// Runs a query from outside: a limit hit is `limit`'s answer, cancellation is returned.
    pub(crate) fn top<T>(
        &mut self,
        limit: impl FnOnce(&mut Self) -> T,
        f: impl FnOnce(&mut Self) -> R<T>,
    ) -> Result<T, Cancelled> {
        let active = self.active.len();
        match f(self) {
            Ok(t) => Ok(t),
            Err(Stop::Limit) => {
                // Whatever was in progress is abandoned, and isn't memoized.
                debug_assert!(self.active.len() >= active);
                self.active.clear();
                self.lubs.clear();
                Ok(limit(self))
            }
            Err(Stop::Cancelled) => {
                self.active.clear();
                self.lubs.clear();
                Err(Cancelled)
            }
        }
    }

    /// Enters a query; `false` if it is already in progress: a cycle.
    pub(crate) fn enter(&mut self, query: Active) -> bool {
        if self.active.insert(query) {
            true
        } else {
            self.cycles += 1;
            false
        }
    }

    pub(crate) fn leave(&mut self, query: Active) {
        self.active.remove(&query);
    }

    // Declarations.

    /// A type's declaration, from its entry or class file.
    pub(crate) fn decl(&mut self, key: TypeKey) -> R<Option<Rc<Decl>>> {
        if let Some(decl) = self.decls.get(&key) {
            return Ok(decl.clone());
        }
        let decl = self.load_decl(key)?.map(Rc::new);
        self.decls.insert(key, decl.clone());
        Ok(decl)
    }

    fn load_decl(&mut self, key: TypeKey) -> R<Option<Decl>> {
        if let TypeKey::Class { .. } = key {
            let Some(entry) = self.snapshot.class_entry(key) else {
                return Ok(None);
            };
            let enclosing = match entry.outer {
                Some(outer) if !entry.local => self.binary(outer)?,
                _ => None,
            };
            let kind = entry.kind();
            let access = ferrion_classfile::class::AccessFlags(entry.access);
            let inner = kind == TypeKind::Class
                && enclosing.is_some()
                && !access.contains(ferrion_classfile::class::AccessFlags::STATIC);
            return Ok(Some(Decl {
                key,
                kind,
                body: Body::Class(entry),
                enclosing,
                inner,
                init: self.init,
            }));
        }
        let Some(entry) = self.snapshot.type_entry(key)? else {
            return Ok(None);
        };
        let enclosing = match key {
            TypeKey::Source { set, package, path } => self
                .names
                .path_outer(path)
                .map(|path| TypeKey::Source { set, package, path }),
            TypeKey::Compact { set, file, path } => path.map(|path| TypeKey::Compact {
                set,
                file,
                path: self.names.path_outer(path),
            }),
            TypeKey::Class { .. } | TypeKey::Local { .. } => None,
        };
        let mut inner = entry.kind == TypeKind::Class
            && enclosing.is_some()
            && !entry.modifiers.contains(crate::summary::Modifiers::STATIC);
        if inner && let Some(outer) = enclosing {
            // A member of an interface is static (JLS 9.5).
            let outer = self.snapshot.type_entry(outer)?;
            inner = outer
                .is_some_and(|o| !matches!(o.kind, TypeKind::Interface | TypeKind::Annotation));
        }
        Ok(Some(Decl {
            key,
            kind: entry.kind,
            body: Body::Entry(entry),
            enclosing,
            inner,
            init: self.init,
        }))
    }

    /// The type a class file's binary name (`java/util/Map$Entry`) names in the view.
    pub(crate) fn binary(&mut self, binary: Name) -> R<Option<TypeKey>> {
        if let Some(&key) = self.binaries.get(&binary) {
            return Ok(key);
        }
        let key = self.snapshot.resolve_binary(self.view, binary)?;
        self.binaries.insert(binary, key);
        Ok(key)
    }

    /// A class by binary name in the view: `java/lang/Object`.
    pub fn lang(&mut self, binary: &str) -> Result<Option<TypeKey>, Cancelled> {
        let name = self.names.intern(binary);
        match self.binary(name) {
            Ok(key) => Ok(key),
            Err(_) => Err(Cancelled),
        }
    }

    pub(crate) fn lang_key(&mut self, binary: &str) -> R<Option<TypeKey>> {
        let name = self.names.intern(binary);
        self.binary(name)
    }

    /// `java.lang.Object`'s type, if the view has it.
    pub(crate) fn object(&mut self) -> R<Option<TypeId>> {
        Ok(self
            .lang_key("java/lang/Object")?
            .map(|key| self.arena.class(key, None, None)))
    }

    pub(crate) fn object_or_error(&mut self) -> R<TypeId> {
        Ok(match self.object()? {
            Some(object) => object,
            None => self.missing("java/lang/Object"),
        })
    }

    pub(crate) fn is_object(&mut self, t: TypeId) -> R<bool> {
        Ok(match self.arena.get(t) {
            Type::Class { key, .. } => self.lang_key("java/lang/Object")? == Some(key),
            _ => false,
        })
    }

    /// An error type for a class the view lacks.
    pub(crate) fn missing(&mut self, binary: &str) -> TypeId {
        let error = self.arena.error(ErrorSymbol {
            name: self.names.intern(binary),
            cause: Cause::NotFound,
        });
        self.arena.intern(Type::Error(error))
    }

    // Translation.

    /// The declared type of a class: its own type variables as arguments, and its enclosing
    /// class's declared type as its outer type if it is an inner class of a generic class.
    pub fn declared_type(&mut self, key: TypeKey) -> Result<TypeId, Cancelled> {
        self.top(|cx| cx.arena.unknown(Why::Limit), |cx| cx.this_type(key))
    }

    pub(crate) fn this_type(&mut self, key: TypeKey) -> R<TypeId> {
        if let Some(&t) = self.this_types.get(&key) {
            return Ok(t);
        }
        // The chain of enclosing classes whose types are outer types, innermost first.
        let mut chain = vec![key];
        let mut current = key;
        loop {
            self.step()?;
            let Some(decl) = self.decl(current)? else {
                break;
            };
            match decl.enclosing {
                Some(outer) if decl.inner && self.has_args_chain(outer)? => {
                    chain.push(outer);
                    current = outer;
                }
                _ => break,
            }
        }
        let mut outer = None;
        for &k in chain.iter().rev() {
            let params = match self.decl(k)? {
                Some(decl) => decl.type_params().len(),
                None => 0,
            };
            let args: Vec<TypeArg> = (0..params as u32)
                .map(|i| TypeArg::Type(self.arena.var(TypeVarOwner::Type(k), i)))
                .collect();
            let args = (!args.is_empty()).then_some(&args[..]);
            let t = self.arena.class(k, outer, args);
            self.this_types.insert(k, t);
            outer = Some(t);
        }
        Ok(outer.expect("the chain has the type itself"))
    }

    /// Whether a class, or a class whose inner class it is, has type parameters, so that its
    /// inner classes' types have it as their outer type.
    pub(crate) fn has_args_chain(&mut self, key: TypeKey) -> R<bool> {
        let mut current = key;
        for _ in 0..1024 {
            let Some(decl) = self.decl(current)? else {
                return Ok(false);
            };
            if decl.type_params().len() > 0 {
                return Ok(true);
            }
            match decl.enclosing {
                Some(outer) if decl.inner => current = outer,
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// The outer type a class type written without one has where it is used, inside `site`: an
    /// inner class of a generic class, used inside that class or a subclass of it, or a class
    /// nested in one, is a member of the generic class's type as `site` sees it (JLS 6.5.5.1).
    fn implicit_outer(&mut self, key: TypeKey, site: TypeKey) -> R<Option<TypeId>> {
        let Some(decl) = self.decl(key)? else {
            return Ok(None);
        };
        let Some(outer) = decl.enclosing.filter(|_| decl.inner) else {
            return Ok(None);
        };
        if !self.has_args_chain(outer)? {
            return Ok(None);
        }
        if let Some(&found) = self.outers.get(&(key, site)) {
            return Ok(found);
        }
        let query = Active::Outer(key, site);
        if !self.enter(query) {
            return Ok(None);
        }
        let cycles = self.cycles;
        let found = self.find_outer(outer, site);
        self.leave(query);
        let found = found?;
        if self.cycles == cycles {
            self.outers.insert((key, site), found);
        }
        Ok(found)
    }

    fn find_outer(&mut self, outer: TypeKey, site: TypeKey) -> R<Option<TypeId>> {
        let mut current = Some(site);
        while let Some(at) = current {
            self.step()?;
            if at == outer {
                return Ok(Some(self.this_type(outer)?));
            }
            if let Tri::Yes(Some(t)) = self.super_decl(at, outer)? {
                return Ok(Some(t));
            }
            current = self.decl(at)?.and_then(|d| d.enclosing);
        }
        Ok(None)
    }

    /// One of a declaration's types, translated once.
    pub(crate) fn slot(&mut self, key: TypeKey, slot: Slot) -> R<TypeId> {
        if let Some(&t) = self.translated.get(&(key, slot)) {
            return Ok(t);
        }
        let Some(decl) = self.decl(key)? else {
            return Ok(self.arena.unknown(Why::Incomplete(key)));
        };
        let ty = match slot {
            Slot::Superclass => decl.superclass(),
            Slot::Interface(i) => decl.interfaces().get(i as usize),
            Slot::Field(i) => decl.field(i as usize).map(|f| f.1),
            Slot::Parameter(m, i) => decl
                .method(m as usize)
                .and_then(|m| m.params.get(i as usize)),
            Slot::Result(m) => match decl.method(m as usize) {
                Some(method) => match method.result {
                    Some(result) => Some(result),
                    None => {
                        let void = self.arena.intern(Type::Void);
                        self.translated.insert((key, slot), void);
                        return Ok(void);
                    }
                },
                None => None,
            },
            Slot::Throws(m, i) => decl
                .method(m as usize)
                .and_then(|m| m.throws.get(i as usize)),
            Slot::Bound(method, p, b) => decl.params_of(method).bounds(p as usize).get(b as usize),
        };
        let t = match ty {
            Some(ty) => self.translate(ty, &decl)?,
            None => self.arena.unknown(Why::Incomplete(key)),
        };
        self.translated.insert((key, slot), t);
        Ok(t)
    }

    /// Translates a type of `decl`'s entry or class file.
    pub(crate) fn translate(&mut self, ty: &Ty, decl: &Decl) -> R<TypeId> {
        enum Node {
            Type(TypeId),
            Arg(TypeArg),
        }
        let node_type = |node: Node| match node {
            Node::Type(t) | Node::Arg(TypeArg::Type(t)) => Some(t),
            Node::Arg(TypeArg::Wildcard(_)) => None,
        };
        let node_args = |nodes: Vec<Node>| -> Vec<TypeArg> {
            nodes
                .into_iter()
                .map(|n| match n {
                    Node::Type(t) => TypeArg::Type(t),
                    Node::Arg(a) => a,
                })
                .collect()
        };
        let parts = ty.parts();
        let mut stack: Vec<Node> = Vec::new();
        for part in parts.iter().rev() {
            self.step()?;
            let arity = part.arity().min(stack.len());
            let mut children: Vec<Node> = stack.split_off(stack.len() - arity);
            children.reverse();
            let node = match *part {
                TyPart::Primitive(p) => Node::Type(self.arena.primitive(p)),
                TyPart::Void => Node::Type(self.arena.intern(Type::Void)),
                TyPart::Class {
                    ty: key,
                    outer,
                    arguments,
                } => {
                    // Another source set's entry names a class file as its own view found it:
                    // this view may find the name elsewhere (another platform, another jar), as
                    // javac, compiling this source set, does.
                    let key = match key {
                        TypeKey::Class { binary, .. } if decl.key.home(self.view) != self.view => {
                            self.binary(binary)?.unwrap_or(key)
                        }
                        key => key,
                    };
                    let explicit = if outer && !children.is_empty() {
                        node_type(children.remove(0))
                    } else {
                        None
                    };
                    let implicit = if outer {
                        None
                    } else {
                        self.implicit_outer(key, decl.key)?
                    };
                    let outer = explicit.filter(|&o| self.has_args(o)).or(implicit);
                    Node::Type(self.class_of(key, outer, arguments, node_args(children)))
                }
                TyPart::Binary {
                    binary,
                    outer,
                    arguments,
                } => match self.binary(binary)? {
                    Some(key) => {
                        let explicit = if outer && !children.is_empty() {
                            node_type(children.remove(0))
                        } else {
                            None
                        };
                        let outer = explicit.filter(|&o| self.has_args(o));
                        Node::Type(self.class_of(key, outer, arguments, node_args(children)))
                    }
                    None => {
                        let error = self.arena.error(ErrorSymbol {
                            name: binary,
                            cause: Cause::NotFound,
                        });
                        Node::Type(self.arena.intern(Type::Error(error)))
                    }
                },
                TyPart::TypeVar { owner, index } => Node::Type(self.arena.var(owner, index)),
                TyPart::UnknownTypeVar { name } => {
                    let error = self.arena.error(ErrorSymbol {
                        name,
                        cause: Cause::NotFound,
                    });
                    Node::Type(self.arena.intern(Type::Error(error)))
                }
                TyPart::Array { dims } => {
                    let mut t = children
                        .pop()
                        .and_then(node_type)
                        .unwrap_or_else(|| self.arena.unknown(Why::Later));
                    for _ in 0..dims {
                        t = self.arena.array(t);
                    }
                    Node::Type(t)
                }
                TyPart::Wildcard { bound } => {
                    let bound = match (bound, children.pop().and_then(node_type)) {
                        (Some(kind), Some(t)) => Some((kind, t)),
                        _ => None,
                    };
                    Node::Arg(TypeArg::Wildcard(bound))
                }
                TyPart::Intersection { .. } => {
                    let parts: Vec<TypeId> = children.into_iter().filter_map(node_type).collect();
                    Node::Type(self.arena.intersection(&parts))
                }
                TyPart::Union { .. } => Node::Type(self.arena.unknown(Why::Later)),
                TyPart::Error(i) => {
                    let error = decl
                        .errors()
                        .get(i as usize)
                        .cloned()
                        .unwrap_or(ErrorSymbol {
                            name: Name::EMPTY,
                            cause: Cause::NotFound,
                        });
                    let error = self.arena.error(error);
                    Node::Type(self.arena.intern(Type::Error(error)))
                }
            };
            stack.push(node);
        }
        Ok(match stack.pop() {
            Some(node) => node_type(node).unwrap_or_else(|| self.arena.unknown(Why::Later)),
            None => self.arena.unknown(Why::Later),
        })
    }

    fn class_of(
        &mut self,
        key: TypeKey,
        outer: Option<TypeId>,
        arguments: u32,
        args: Vec<TypeArg>,
    ) -> TypeId {
        let args = (arguments != TyPart::NO_ARGUMENTS && !args.is_empty()).then_some(&args[..]);
        self.arena.class(key, outer, args)
    }

    /// Whether a class type, or one it is a member of, has type arguments.
    pub(crate) fn has_args(&self, t: TypeId) -> bool {
        matches!(
            self.arena.get(t),
            Type::Class { args: Some(_), .. } | Type::Class { outer: Some(_), .. }
        )
    }

    /// A type variable's bounds, translated when first needed; `Object` if it has none.
    pub(crate) fn var_bounds(&mut self, owner: TypeVarOwner, index: u32) -> R<Rc<[TypeId]>> {
        if let Some(bounds) = self.bounds.get(&(owner, index)) {
            return Ok(bounds.clone());
        }
        let (key, method) = match owner {
            TypeVarOwner::Type(key) => (key, None),
            TypeVarOwner::Method(key, m) => (key, Some(m)),
        };
        let query = Active::Bounds(owner, index);
        if !self.enter(query) {
            return Ok(Rc::from([self.arena.unknown(Why::Limit)]));
        }
        let result = self.load_bounds(key, method, index);
        self.leave(query);
        let bounds: Rc<[TypeId]> = Rc::from(result?);
        self.bounds.insert((owner, index), bounds.clone());
        Ok(bounds)
    }

    fn load_bounds(&mut self, key: TypeKey, method: Option<u32>, index: u32) -> R<Vec<TypeId>> {
        let count = match self.decl(key)? {
            Some(decl) => decl.params_of(method).bounds(index as usize).len(),
            None => return Ok(vec![self.arena.unknown(Why::Incomplete(key))]),
        };
        let mut bounds = Vec::with_capacity(count.max(1));
        for b in 0..count as u32 {
            bounds.push(self.slot(key, Slot::Bound(method, index, b))?);
        }
        if bounds.is_empty() {
            bounds.push(self.object_or_error()?);
        }
        Ok(bounds)
    }

    /// A type variable's or capture variable's upper bound: its bound, or the intersection of
    /// its bounds.
    pub(crate) fn upper(&mut self, t: TypeId) -> R<TypeId> {
        Ok(match self.arena.get(t) {
            Type::Var(owner, index) => {
                let bounds = self.var_bounds(owner, index)?;
                self.arena.intersection(&bounds)
            }
            Type::Capture(n) => self.arena.capture(n).upper,
            _ => t,
        })
    }

    /// A field's declared type.
    pub fn field_type(&mut self, key: TypeKey, field: u32) -> Result<TypeId, Cancelled> {
        self.top(
            |cx| cx.arena.unknown(Why::Limit),
            |cx| cx.slot(key, Slot::Field(field)),
        )
    }

    /// Translates `ty`, which is one of `owner`'s entry's or class file's types.
    pub fn translate_in(&mut self, ty: &Ty, owner: TypeKey) -> Result<TypeId, Cancelled> {
        self.top(
            |cx| cx.arena.unknown(Why::Limit),
            |cx| match cx.decl(owner)? {
                Some(decl) => cx.translate(ty, &decl),
                None => Ok(cx.arena.unknown(Why::Incomplete(owner))),
            },
        )
    }

    /// The kind of a class.
    pub fn kind(&mut self, key: TypeKey) -> Result<Option<TypeKind>, Cancelled> {
        self.top(|_| None, |cx| Ok(cx.decl(key)?.map(|d| d.kind)))
    }

    /// The number of type parameters a class, or its `method`th method, declares.
    pub fn type_param_count(
        &mut self,
        key: TypeKey,
        method: Option<u32>,
    ) -> Result<usize, Cancelled> {
        self.top(
            |_| 0,
            |cx| Ok(cx.decl(key)?.map_or(0, |d| d.params_of(method).len())),
        )
    }

    pub fn primitive(&mut self, p: Primitive) -> TypeId {
        self.arena.primitive(p)
    }

    // A class's declaration, for tests and the javac comparison.

    /// A class's superclass as declared, written or implied (`Object`, `Enum<E>`, `Record`);
    /// `None` for an interface and for `Object`.
    pub fn superclass(&mut self, key: TypeKey) -> Result<Option<TypeId>, Cancelled> {
        self.top(
            |_| None,
            |cx| {
                let Some(decl) = cx.decl(key)? else {
                    return Ok(None);
                };
                let has_superclass = !decl.is_interface()
                    && match decl.body {
                        Body::Class(_) => decl.superclass().is_some(),
                        Body::Entry(_) => cx.lang_key("java/lang/Object")? != Some(key),
                    };
                let supers = cx.declared_supertypes(key)?;
                Ok(supers.first().copied().filter(|_| has_superclass))
            },
        )
    }

    /// A class's interfaces as declared, with an annotation interface's implied
    /// `java.lang.annotation.Annotation`.
    pub fn interfaces(&mut self, key: TypeKey) -> Result<Vec<TypeId>, Cancelled> {
        let superclass = self.superclass(key)?;
        self.top(
            |_| Vec::new(),
            |cx| {
                let supers = cx.declared_supertypes(key)?;
                Ok(supers[usize::from(superclass.is_some())..].to_vec())
            },
        )
    }

    pub fn is_interface(&mut self, key: TypeKey) -> Result<bool, Cancelled> {
        self.top(
            |_| false,
            |cx| Ok(cx.decl(key)?.is_some_and(|d| d.is_interface())),
        )
    }

    /// The index of a class's field named `name`.
    pub fn field_index(&mut self, key: TypeKey, name: &str) -> Result<Option<u32>, Cancelled> {
        let name = self.names.intern(name);
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

    /// How many methods and constructors a class's entry or class file lists.
    pub fn method_count(&mut self, key: TypeKey) -> Result<usize, Cancelled> {
        self.top(
            |_| 0,
            |cx| Ok(cx.decl(key)?.map_or(0, |d| d.method_count())),
        )
    }

    /// A method's key, as the javac comparison names it: `method p/A.m(I)V`.
    pub fn method_key(&mut self, key: TypeKey, method: u32) -> Result<String, Cancelled> {
        self.snapshot.method_text(key, method)
    }

    /// The index of the type parameter named `name` of a class or method.
    pub fn type_param_index(
        &mut self,
        owner: TypeVarOwner,
        name: &str,
    ) -> Result<Option<u32>, Cancelled> {
        let name = self.names.intern(name);
        let (key, method) = match owner {
            TypeVarOwner::Type(key) => (key, None),
            TypeVarOwner::Method(key, m) => (key, Some(m)),
        };
        self.top(
            |_| None,
            |cx| {
                let Some(decl) = cx.decl(key)? else {
                    return Ok(None);
                };
                let params = decl.params_of(method);
                Ok((0..params.len())
                    .find(|&i| params.name(i) == Some(name))
                    .map(|i| i as u32))
            },
        )
    }

    /// A type variable's bounds, `Object` if it has none.
    pub fn bounds(&mut self, owner: TypeVarOwner, index: u32) -> Result<Vec<TypeId>, Cancelled> {
        self.top(
            |_| Vec::new(),
            |cx| Ok(cx.var_bounds(owner, index)?.to_vec()),
        )
    }

    /// A class type: `args` `None` for a non-generic class or a raw type.
    pub fn class_type(
        &mut self,
        key: TypeKey,
        outer: Option<TypeId>,
        args: Option<&[TypeArg]>,
    ) -> TypeId {
        self.arena.class(key, outer, args)
    }
}
