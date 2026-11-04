//! Method invocation and class instance creation (JLS 15.9, 15.12,
//! docs/design/semantic-attribution.md#method-invocation): the methods to choose among, the
//! inference line, the three phases, the most specific method, and the type as invoked.

use std::sync::Arc;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeId, SyntaxKind, SyntaxNode};

use super::{Value, Walker, What, child, id_of};
use crate::Cancelled;
use crate::attr::{Call, Phase, Special};
use crate::decl_names::Symbol;
use crate::members::Members;
use crate::resolve::package_of;
use crate::summary::{TypeKind, WildcardBound, name_text};
use crate::symbols::{Cause, ErrorSymbol, MemberType, MethodRef, TypeKey};
use crate::types::{Context, Conversion, MethodType, Tri, Type, TypeArg, TypeId, Why};
use crate::{Interner, Name};

/// How a call names what it searches, as JLS 6.6.2's rule on protected members asks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Qualifier {
    /// A simple name, `this`, `super`, a type's name, or a `super(...)` call: nothing to check
    /// beyond being in a subclass.
    Own,
    /// `e.m()`: the qualifier's type must be the class the call is in, or a subclass of it.
    Expression(TypeId),
    /// `new C(...)` without a body: a protected constructor is for its package alone.
    Creation,
    /// `new C(...) { ... }`: the anonymous class calls the constructor, as a subclass (JLS 15.9.3).
    Anonymous,
}

/// The methods a call chooses among.
struct Search {
    methods: Vec<MethodRef>,
    /// The type they are seen from; `None` for statically imported methods, as declared.
    of: Option<TypeId>,
    qualifier: Qualifier,
    /// The type searched, whose erasure `getClass()`'s result names.
    searched: TypeId,
    /// The search is an array's: `clone()` is its own.
    array: bool,
}

/// What a call asks of overload resolution.
struct Request<'a> {
    methods: &'a [MethodRef],
    of: Option<TypeId>,
    args: &'a [Value],
    explicit: Option<&'a [TypeId]>,
    qualifier: Qualifier,
    name: Name,
}

/// A candidate that fits the call's arity.
struct Candidate {
    method: MethodRef,
    /// As seen from the type searched, with explicit type arguments substituted.
    ty: MethodType,
    varargs: bool,
    is_abstract: bool,
    /// Generic, with no explicit type arguments to substitute: the line.
    generic: bool,
    /// An explicit type argument needed an unchecked conversion to its bound.
    unchecked: bool,
}

/// The method chosen.
struct Chosen {
    method: MethodRef,
    invoked: MethodType,
    phase: Phase,
    unchecked: bool,
}

/// The `i`th variable arity parameter type (JLS 15.12.2.4): the `i`th parameter's, and from the
/// last on the last's component.
fn variable_arity_type(
    cx: &crate::types::TypeContext<'_>,
    m: &MethodType,
    i: usize,
) -> Option<TypeId> {
    if i + 1 < m.params.len() {
        return Some(m.params[i]);
    }
    match cx.get(*m.params.last()?) {
        Type::Array(component) => Some(component),
        _ => None,
    }
}

/// How an expression is an argument of a call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArgumentPosition {
    Direct,
    /// An operand of a conditional that is one.
    Operand,
}

/// Whether an expression is an argument of a call, through parentheses and conditionals.
fn argument_position(node: SyntaxNode<'_>) -> Option<ArgumentPosition> {
    let mut position = ArgumentPosition::Direct;
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            SyntaxKind::ParenExpr => {}
            SyntaxKind::ConditionalExpr => {
                let condition = ast::ConditionalExpr::cast(parent).unwrap().condition();
                if id_of(condition) == Some(current.id()) {
                    return None;
                }
                position = ArgumentPosition::Operand;
            }
            SyntaxKind::Arguments => return Some(position),
            _ => return None,
        }
        current = parent;
    }
    None
}

fn phase_context(phase: Phase) -> Context {
    match phase {
        Phase::Strict => Context::StrictInvocation,
        Phase::Loose | Phase::VariableArity => Context::LooseInvocation,
    }
}

impl<'s> Walker<'s, '_> {
    /// A method invocation, an explicit constructor invocation, or a class instance creation.
    /// `children` are its receiver or qualifier, if any, and its arguments.
    pub(super) fn call(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        match node.kind() {
            SyntaxKind::MethodCallExpr => self.method_call(node, children),
            SyntaxKind::NewExpr => self.creation(node, children),
            _ => self.constructor_call(node, children),
        }
    }

    /// A call's arguments' values, in order.
    fn argument_values(
        &self,
        arguments: Option<ast::Arguments<'_>>,
        children: &[Value],
    ) -> Vec<Value> {
        let mut values = Vec::new();
        for argument in arguments.into_iter().flat_map(|a| a.arguments()) {
            if let Some(value) = child(children, Some(argument.syntax().id())) {
                values.push(value.clone());
            }
        }
        values
    }

    /// Explicit type arguments as written; `None` if the call has none.
    fn explicit_arguments(
        &mut self,
        arguments: Option<ast::TypeArguments<'_>>,
    ) -> Result<Option<Vec<TypeId>>, Cancelled> {
        let Some(arguments) = arguments else {
            return Ok(None);
        };
        let mut types = Vec::new();
        for a in arguments.arguments() {
            match a {
                ast::TypeArgument::Type(t) => types.push(self.syntax_type(t)?),
                ast::TypeArgument::Wildcard(_) => {
                    types.push(self.unknown(Why::Later));
                }
            }
        }
        Ok(Some(types))
    }

    fn method_call(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let call = ast::MethodCallExpr::cast(node).unwrap();
        let id = node.id();
        let receiver = child(children, id_of(call.receiver())).cloned();
        let args = self.argument_values(call.arguments(), children);
        let name = call
            .name()
            .map_or(Name::EMPTY, |t| name_text(t, &self.names));
        if let Some(r) = &receiver
            && r.what == What::Expr
            && let Some(why) = self.non_answer(r.ty)
        {
            let t = self.unknown(why);
            return Ok(self.expr(id, t));
        }
        let search = match self.symbol(id).cloned() {
            Some(Symbol::Error(e)) => {
                let t = self.error_type(e);
                return Ok(self.expr(id, t));
            }
            Some(Symbol::Methods {
                search, candidates, ..
            }) => self.named_search(receiver.as_ref(), search, &candidates, name)?,
            Some(Symbol::LeftToB3(_)) => match &receiver {
                Some(r) => self.expression_search(r.ty, name)?,
                None => Err(self.unknown(Why::Later)),
            },
            _ => Err(self.unknown(Why::Later)),
        };
        let search = match search {
            Ok(search) => search,
            Err(t) => return Ok(self.expr(id, t)),
        };
        let explicit = self.explicit_arguments(call.type_arguments())?;
        if search.array && args.is_empty() && name == self.names.intern("clone") {
            return self.array_clone(id, &search);
        }
        let request = Request {
            methods: &search.methods,
            of: search.of,
            args: &args,
            explicit: explicit.as_deref(),
            qualifier: search.qualifier,
            name,
        };
        let chosen = match self.resolve(&request)? {
            Ok(chosen) => chosen,
            Err(t) => return Ok(self.expr(id, t)),
        };
        self.method_result(node, chosen, &search, &args)
    }

    /// The methods a name B2 resolved searches: of a class around the call, or of a type or
    /// `this` or `super` qualifying it, from the type's member table; of the static imports of
    /// the name, as B2 gathered them from all of them (JLS 15.12.1).
    fn named_search(
        &mut self,
        receiver: Option<&Value>,
        search: TypeKey,
        candidates: &Arc<[MethodRef]>,
        name: Name,
    ) -> Result<Result<Search, TypeId>, Cancelled> {
        let enclosing = receiver.is_some() || self.classes.iter().any(|c| c.key == search);
        if !enclosing {
            return self.imported_search(candidates);
        }
        // Inside an anonymous class made with `<>`, what it inherits is seen through arguments
        // inference gives, and its table doesn't have it.
        if self.inferred_supertypes.contains(&search)
            && candidates.iter().any(|m| m.owner != search)
        {
            return Ok(Err(self.unknown(Why::LeftToB3c)));
        }
        let Some(table) = self.members(search)? else {
            return Ok(Err(self.unknown(Why::Incomplete(search))));
        };
        if table.incomplete {
            return Ok(Err(self.unknown(Why::Incomplete(search))));
        }
        let methods: Vec<MethodRef> = table.methods(name).iter().map(|m| m.method).collect();
        let (of, qualifier) = match receiver {
            Some(Value {
                what: What::Expr,
                ty,
                ..
            }) => (*ty, Qualifier::Own),
            _ => (self.cx.declared_type(search)?, Qualifier::Own),
        };
        Ok(Ok(Search {
            methods,
            of: Some(of),
            qualifier,
            searched: of,
            array: false,
        }))
    }

    /// Statically imported methods: the static ones among B2's candidates, without those another
    /// of them hides.
    fn imported_search(
        &mut self,
        candidates: &Arc<[MethodRef]>,
    ) -> Result<Result<Search, TypeId>, Cancelled> {
        let mut methods = Vec::with_capacity(candidates.len());
        for &m in candidates.iter() {
            if self.cx.method_info(m)?.is_some_and(|i| i.is_static) {
                methods.push(m);
            }
            if let Some(table) = self.members(m.owner)?
                && table.incomplete
            {
                return Ok(Err(self.unknown(Why::Incomplete(m.owner))));
            }
        }
        let methods = self.without_overridden(None, methods)?;
        let searched = self.unknown(Why::Later);
        Ok(Ok(Search {
            methods,
            of: None,
            qualifier: Qualifier::Own,
            searched,
            array: false,
        }))
    }

    /// The methods of a call on an expression: the member methods of its type, an array's being
    /// `Object`'s, a type variable's or intersection's those of its bounds (JLS 4.4, 4.9, 10.7).
    fn expression_search(
        &mut self,
        t: TypeId,
        name: Name,
    ) -> Result<Result<Search, TypeId>, Cancelled> {
        let of = self.cx.fresh_capture(t)?;
        let mut parts = Vec::new();
        let mut pending = vec![of];
        let mut array = false;
        let mut steps = 0;
        while let Some(part) = pending.pop() {
            steps += 1;
            if steps > 256 {
                return Ok(Err(self.unknown(Why::Limit)));
            }
            match self.cx.get(part) {
                Type::Class { key, .. } => parts.push(key),
                Type::Array(_) => {
                    array = true;
                    let object = self.lang_key("java/lang/Object")?;
                    parts.push(object);
                }
                Type::Var(..) | Type::Capture(_) => {
                    let upper = self.cx.upper_bound(part)?;
                    pending.push(upper);
                }
                Type::Intersection(list) => {
                    pending.extend(self.cx.arena().list(list).iter().rev().copied());
                }
                _ => {
                    return Ok(Err(match self.non_answer(part) {
                        Some(why) => self.unknown(why),
                        None => self.error_type(ErrorSymbol {
                            name,
                            cause: Cause::NotFound,
                        }),
                    }));
                }
            }
        }
        let many = parts.len() > 1;
        let mut methods: Vec<MethodRef> = Vec::new();
        for key in parts {
            let Some(table) = self.members(key)? else {
                return Ok(Err(self.unknown(Why::Incomplete(key))));
            };
            if table.incomplete {
                return Ok(Err(self.unknown(Why::Incomplete(key))));
            }
            self.gather(&table, name, &mut methods);
        }
        // An array's members are seen from `Object`.
        let site = if array {
            self.lang("java/lang/Object")?
        } else {
            of
        };
        if many {
            methods = self.without_overridden(Some(site), methods)?;
        }
        Ok(Ok(Search {
            methods,
            of: Some(site),
            qualifier: Qualifier::Expression(of),
            searched: of,
            array,
        }))
    }

    fn gather(&self, table: &Members, name: Name, methods: &mut Vec<MethodRef>) {
        for m in table.methods(name) {
            if !methods.contains(&m.method) {
                methods.push(m.method);
            }
        }
    }

    fn lang_key(&mut self, binary: &str) -> Result<TypeKey, Cancelled> {
        let key = self.cx.lang(binary)?;
        Ok(key.unwrap_or(TypeKey::Local {
            file: crate::FileId::from_index(0),
            index: u32::MAX,
        }))
    }

    /// `methods` without those another of them overrides or hides: methods gathered from several
    /// types, which no one member table has worked out.
    fn without_overridden(
        &mut self,
        of: Option<TypeId>,
        methods: Vec<MethodRef>,
    ) -> Result<Vec<MethodRef>, Cancelled> {
        if methods.len() < 2 {
            return Ok(methods);
        }
        let mut types = Vec::with_capacity(methods.len());
        for &m in &methods {
            types.push(match self.seen_from(of, m)? {
                Tri::Yes(t) => Some(t),
                _ => None,
            });
        }
        let mut kept = Vec::with_capacity(methods.len());
        'next: for (i, &m) in methods.iter().enumerate() {
            for (j, &other) in methods.iter().enumerate() {
                if i != j
                    && other.owner != m.owner
                    && self.cx.is_subclass(other.owner, m.owner)?
                    && let (Some(a), Some(b)) = (&types[j], &types[i])
                    && self.cx.subsignature(a, b)?.is_yes()
                {
                    continue 'next;
                }
            }
            kept.push(m);
        }
        Ok(kept)
    }

    fn seen_from(
        &mut self,
        of: Option<TypeId>,
        m: MethodRef,
    ) -> Result<Tri<MethodType>, Cancelled> {
        match of {
            Some(of) => self.cx.method_type_in(of, m.owner, m.index),
            None => Ok(Tri::Yes(self.cx.method_type(m.owner, m.index)?)),
        }
    }

    /// Whether code in the class the walk is in can reach a member (JLS 6.6).
    fn accessible(&mut self, m: MethodRef, qualifier: Qualifier) -> Result<bool, Cancelled> {
        let Some(info) = self.cx.method_info(m)? else {
            return Ok(false);
        };
        if info.is_private {
            let Some(current) = self.classes.last().map(|c| c.key) else {
                return Ok(false);
            };
            return Ok(self.cx.outermost(m.owner)? == self.cx.outermost(current)?);
        }
        if info.is_public {
            return Ok(true);
        }
        if self.same_package(m.owner)? {
            return Ok(true);
        }
        if info.package_access || !info.is_protected || qualifier == Qualifier::Creation {
            return Ok(false);
        }
        if qualifier == Qualifier::Anonymous {
            return Ok(true);
        }
        // A protected member in another package: from a subclass of its class, through a
        // qualifier of that subclass or a subclass of it (JLS 6.6.2).
        let classes: Vec<TypeKey> = self.classes.iter().rev().map(|c| c.key).collect();
        for class in classes {
            if !self.cx.is_subclass(class, m.owner)? {
                continue;
            }
            let Qualifier::Expression(q) = qualifier else {
                return Ok(true);
            };
            if info.is_static {
                return Ok(true);
            }
            let erased = self.cx.erasure(q)?;
            match self.cx.get(erased) {
                Type::Class { key, .. } => {
                    if self.cx.is_subclass(key, class)? {
                        return Ok(true);
                    }
                }
                _ => return Ok(true),
            }
        }
        Ok(false)
    }

    fn same_package(&mut self, owner: TypeKey) -> Result<bool, Cancelled> {
        let Some(current) = self.classes.last().map(|c| c.key) else {
            return Ok(false);
        };
        let names: &Interner = &self.names.clone();
        let a = self.cx.outermost(owner)?;
        let b = self.cx.outermost(current)?;
        Ok(package_of(a, names) == package_of(b, names))
    }

    /// The first non-answer among a call's arguments, a poly conditional's operands standing for
    /// it.
    fn arguments_non_answer(&self, args: &[Value]) -> Option<Why> {
        for arg in args {
            if arg.what != What::Expr {
                return Some(Why::Later);
            }
            if let Some(operands) = &arg.operands {
                if let Some(why) = self.arguments_non_answer(operands) {
                    return Some(why);
                }
            } else if let Some(why) = self.non_answer(arg.ty) {
                return Some(why);
            }
        }
        None
    }

    /// Chooses a method (JLS 15.12.2), or says why there is no answer: a type to give the call.
    fn resolve(&mut self, request: &Request<'_>) -> Result<Result<Chosen, TypeId>, Cancelled> {
        let n = request.args.len();
        // Potentially applicable (15.12.2.1).
        let mut candidates = Vec::with_capacity(request.methods.len());
        let mut left_to_b3c = false;
        let mut unknown = None;
        for &method in request.methods {
            let Some(method) = self.cx.method_in_view(method)? else {
                continue;
            };
            let Some(info) = self.cx.method_info(method)? else {
                continue;
            };
            let fits = info.params == n || (info.varargs && n + 1 >= info.params);
            if !fits || !self.accessible(method, request.qualifier)? {
                continue;
            }
            let ty = match self.seen_from(request.of, method)? {
                Tri::Yes(ty) => ty,
                Tri::No => continue,
                Tri::Unknown(why) => {
                    unknown.get_or_insert(why);
                    continue;
                }
            };
            let mut generic = !ty.type_params.is_empty();
            let mut ty = ty;
            let mut unchecked = false;
            if let (true, Some(explicit)) = (generic, request.explicit) {
                if explicit.len() != ty.type_params.len() {
                    continue;
                }
                (ty, unchecked) = match self.cx.method_with_arguments(request.of, &ty, explicit)? {
                    Tri::Yes(found) => found,
                    Tri::No => continue,
                    Tri::Unknown(why) => {
                        unknown.get_or_insert(why);
                        continue;
                    }
                };
                generic = false;
            }
            left_to_b3c |= generic;
            candidates.push(Candidate {
                method,
                ty,
                varargs: info.varargs,
                is_abstract: info.is_abstract,
                generic,
                unchecked,
            });
        }
        // A non-answer's root cause before the line: a call with an error in it is that error.
        if let Some(why) = self.arguments_non_answer(request.args) {
            return Ok(Err(self.non_answer_type(why)));
        }
        if left_to_b3c {
            return Ok(Err(self.unknown(Why::LeftToB3c)));
        }
        if let Some(why) = unknown {
            return Ok(Err(self.unknown(why)));
        }
        // The phases (15.12.2.2 to 15.12.2.4).
        for phase in [Phase::Strict, Phase::Loose, Phase::VariableArity] {
            let mut applicable = Vec::new();
            for (i, c) in candidates.iter().enumerate() {
                match self.applicable(c, request.args, phase)? {
                    Tri::Yes(unchecked) => applicable.push((i, unchecked)),
                    Tri::No => {}
                    Tri::Unknown(why) => {
                        unknown.get_or_insert(why);
                    }
                }
            }
            if let Some(why) = unknown {
                return Ok(Err(self.unknown(why)));
            }
            if applicable.is_empty() {
                continue;
            }
            let index = match self.most_specific(&mut candidates, &applicable, n, phase, request)? {
                Ok(index) => index,
                Err(t) => return Ok(Err(t)),
            };
            let unchecked = applicable.iter().any(|a| a.0 == index && a.1);
            let c = &candidates[index];
            return Ok(Ok(Chosen {
                method: c.method,
                invoked: c.ty.clone(),
                phase,
                unchecked,
            }));
        }
        Ok(Err(self.error_type(ErrorSymbol {
            name: request.name,
            cause: Cause::NotApplicable,
        })))
    }

    /// A non-answer as a type: `Why::Error` keeps its error.
    fn non_answer_type(&mut self, why: Why) -> TypeId {
        match why {
            Why::Error(e) => self.cx.arena_mut().intern(Type::Error(e)),
            why => self.unknown(why),
        }
    }

    /// Whether a candidate is applicable in a phase; yes says whether an unchecked conversion was
    /// needed.
    fn applicable(
        &mut self,
        c: &Candidate,
        args: &[Value],
        phase: Phase,
    ) -> Result<Tri<bool>, Cancelled> {
        match phase {
            Phase::Strict | Phase::Loose if c.ty.params.len() != args.len() => return Ok(Tri::No),
            Phase::VariableArity if !c.varargs => return Ok(Tri::No),
            _ => {}
        }
        let context = phase_context(phase);
        let mut unchecked = c.unchecked;
        let mut unknown = None;
        for (i, arg) in args.iter().enumerate() {
            let formal = if phase == Phase::VariableArity {
                match variable_arity_type(&self.cx, &c.ty, i) {
                    Some(t) => t,
                    None => return Ok(Tri::No),
                }
            } else {
                c.ty.params[i]
            };
            match self.argument_converts(arg, context, formal)? {
                Tri::Yes(u) => unchecked |= u,
                Tri::No => return Ok(Tri::No),
                Tri::Unknown(why) => {
                    unknown.get_or_insert(why);
                }
            }
        }
        Ok(match unknown {
            Some(why) => Tri::Unknown(why),
            None => Tri::Yes(unchecked),
        })
    }

    /// Whether an argument converts to a formal parameter: a poly conditional if both operands
    /// do (JLS 15.25). Yes says whether the conversion was unchecked. A constant isn't narrowed
    /// here, as it is in an assignment (JLS 5.3).
    fn argument_converts(
        &mut self,
        arg: &Value,
        context: Context,
        formal: TypeId,
    ) -> Result<Tri<bool>, Cancelled> {
        if let Some(operands) = arg.operands.clone() {
            let mut unchecked = false;
            let mut unknown = None;
            for operand in operands.iter() {
                let answer = ferrion_stack::ensure_stack(|| {
                    self.argument_converts(operand, context, formal)
                })?;
                match answer {
                    Tri::Yes(u) => unchecked |= u,
                    Tri::No => return Ok(Tri::No),
                    Tri::Unknown(why) => {
                        unknown.get_or_insert(why);
                    }
                }
            }
            return Ok(match unknown {
                Some(why) => Tri::Unknown(why),
                None => Tri::Yes(unchecked),
            });
        }
        Ok(match self.cx.convert(context, arg.ty, formal, None)? {
            Tri::Yes(steps) => Tri::Yes(
                steps.as_slice().contains(&Conversion::Unchecked)
                    && !self.cx.takes_raw_freely(formal)?,
            ),
            Tri::No => Tri::No,
            Tri::Unknown(why) => Tri::Unknown(why),
        })
    }

    /// The most specific of the applicable candidates (JLS 15.12.2.5).
    fn most_specific(
        &mut self,
        candidates: &mut Vec<Candidate>,
        applicable: &[(usize, bool)],
        k: usize,
        phase: Phase,
        request: &Request<'_>,
    ) -> Result<Result<usize, TypeId>, Cancelled> {
        if let [(only, _)] = applicable {
            return Ok(Ok(*only));
        }
        // A generic method among several, explicit type arguments or not, is compared by
        // inference (18.5.4).
        for &(i, _) in applicable {
            let m = candidates[i].method;
            if candidates[i].generic || self.cx.method_info(m)?.is_some_and(|m| m.type_params > 0) {
                return Ok(Err(self.unknown(Why::LeftToB3c)));
            }
        }
        let mut maximal = Vec::new();
        let mut unknown = None;
        for &(i, _) in applicable {
            let mut beaten = false;
            for &(j, _) in applicable {
                if i == j {
                    continue;
                }
                let over = self.more_specific(&candidates[j].ty, &candidates[i].ty, k, phase)?;
                match over {
                    Tri::No => continue,
                    Tri::Unknown(why) => {
                        unknown.get_or_insert(why);
                        continue;
                    }
                    Tri::Yes(()) => {}
                }
                match self.more_specific(&candidates[i].ty, &candidates[j].ty, k, phase)? {
                    Tri::No => {
                        beaten = true;
                        break;
                    }
                    Tri::Unknown(why) => {
                        unknown.get_or_insert(why);
                    }
                    Tri::Yes(()) => {}
                }
            }
            if !beaten {
                maximal.push(i);
            }
        }
        if let Some(why) = unknown {
            return Ok(Err(self.unknown(why)));
        }
        if let [only] = maximal[..] {
            return Ok(Ok(only));
        }
        self.merge_equivalent(candidates, &maximal, request)
    }

    /// Whether `m1` is more specific than `m2` for `k` arguments: by the subtyping of their
    /// parameter types, for variable arity with its rule on one more parameter.
    fn more_specific(
        &mut self,
        m1: &MethodType,
        m2: &MethodType,
        k: usize,
        phase: Phase,
    ) -> Result<Tri, Cancelled> {
        let mut unknown = None;
        let count = if phase == Phase::VariableArity && m2.params.len() == k + 1 {
            k + 1
        } else {
            k
        };
        for i in 0..count {
            let (s, t) = if phase == Phase::VariableArity {
                match (
                    variable_arity_type(&self.cx, m1, i),
                    variable_arity_type(&self.cx, m2, i),
                ) {
                    (Some(s), Some(t)) => (s, t),
                    _ => return Ok(Tri::No),
                }
            } else {
                (m1.params[i], m2.params[i])
            };
            match self.cx.subtype(s, t)? {
                Tri::Yes(()) => {}
                Tri::No => return Ok(Tri::No),
                Tri::Unknown(why) => {
                    unknown.get_or_insert(why);
                }
            }
        }
        Ok(match unknown {
            Some(why) => Tri::Unknown(why),
            None => Tri::Yes(()),
        })
    }

    /// Several maximally specific methods: override-equivalent ones have one concrete or default
    /// method, or are abstract and javac takes one with the most specific return type; anything
    /// else is ambiguous (JLS 15.12.2.5). Among abstract ones, which is javac's, by observation:
    /// in the order it searches supertypes, with the first two swapped, the first whose return
    /// type is a subtype of the others' (observation 22).
    fn merge_equivalent(
        &mut self,
        candidates: &mut Vec<Candidate>,
        maximal: &[usize],
        request: &Request<'_>,
    ) -> Result<Result<usize, TypeId>, Cancelled> {
        for &i in &maximal[1..] {
            let first = &candidates[maximal[0]];
            match self.cx.override_equivalent(&first.ty, &candidates[i].ty)? {
                Tri::Yes(()) => {}
                Tri::No => return Ok(Err(self.ambiguous(candidates, maximal, request))),
                Tri::Unknown(why) => return Ok(Err(self.unknown(why))),
            }
        }
        let concrete: Vec<usize> = maximal
            .iter()
            .copied()
            .filter(|&i| !candidates[i].is_abstract)
            .collect();
        match concrete[..] {
            [only] => return Ok(Ok(only)),
            [] => {}
            _ => return Ok(Err(self.ambiguous(candidates, maximal, request))),
        }
        // A class's abstract method over a concrete one above it wins (observed: `B1.has` of `C
        // extends B1 implements Lo`, with `B1`'s `has` overriding `A1`'s, where without the
        // concrete one `Lo.has` is chosen, by the order below).
        for &i in maximal {
            if self.overrides_concrete_above(&candidates[i], request.of)? {
                return Ok(Ok(i));
            }
        }
        // Several interfaces' methods of a signature `Object` has: `Object`'s (observed:
        // `hashCode()` of `interface C extends A, B`, with both declaring it).
        if let Some(found) = self.object_counterpart(&candidates[maximal[0]])? {
            candidates.push(found);
            return Ok(Ok(candidates.len() - 1));
        }
        // Abstract methods whose declarations erase differently clash, as javac reports.
        let mut erased: Vec<Vec<TypeId>> = Vec::with_capacity(maximal.len());
        for &i in maximal {
            let m = candidates[i].method;
            let declared = self.cx.method_type(m.owner, m.index)?;
            let mut params = Vec::with_capacity(declared.params.len());
            for p in declared.params {
                params.push(self.cx.erasure(p)?);
            }
            erased.push(params);
        }
        if erased.iter().any(|e| *e != erased[0]) {
            return Ok(Err(self.ambiguous(candidates, maximal, request)));
        }
        let owners: Vec<TypeKey> = maximal
            .iter()
            .map(|&i| candidates[i].method.owner)
            .collect();
        let ordered = self.cx.search_order(&owners)?;
        let mut order: Vec<usize> = ordered
            .iter()
            .filter_map(|&o| {
                maximal
                    .iter()
                    .copied()
                    .find(|&i| candidates[i].method.owner == o)
            })
            .collect();
        if order.len() >= 2 {
            order.swap(0, 1);
        }
        for &i in &order {
            let mut most = true;
            for &j in &order {
                if i != j
                    && !self
                        .cx
                        .subtype(candidates[i].ty.result, candidates[j].ty.result)?
                        .is_yes()
                    && candidates[i].ty.result != candidates[j].ty.result
                {
                    most = false;
                    break;
                }
            }
            if most {
                return Ok(Ok(i));
            }
        }
        Ok(Ok(order[0]))
    }

    /// `Object`'s method of an interface method's signature.
    fn object_counterpart(&mut self, c: &Candidate) -> Result<Option<Candidate>, Cancelled> {
        let Some(info) = self.cx.method_info(c.method)? else {
            return Ok(None);
        };
        let (Some(name), Some(object), true) = (
            self.cx.method_name(c.method)?,
            self.cx.lang("java/lang/Object")?,
            info.interface,
        ) else {
            return Ok(None);
        };
        for index in 0..self.cx.method_count(object)? as u32 {
            let method = MethodRef {
                owner: object,
                index,
            };
            if self.cx.method_name(method)? != Some(name)
                || self.cx.method_info(method)?.is_none_or(|i| !i.is_public)
            {
                continue;
            }
            let ty = self.cx.method_type(object, index)?;
            if self.cx.subsignature(&c.ty, &ty)?.is_yes() {
                return Ok(Some(Candidate {
                    method,
                    ty,
                    varargs: false,
                    is_abstract: false,
                    generic: false,
                    unchecked: false,
                }));
            }
        }
        Ok(None)
    }

    /// Whether a method of a class overrides a concrete method of a superclass of that class.
    fn overrides_concrete_above(
        &mut self,
        c: &Candidate,
        of: Option<TypeId>,
    ) -> Result<bool, Cancelled> {
        let (owner, Some(of)) = (c.method.owner, of) else {
            return Ok(false);
        };
        if self.cx.is_interface(owner)? {
            return Ok(false);
        }
        let Some(name) = self.cx.method_name(c.method)? else {
            return Ok(false);
        };
        let mut current = owner;
        for _ in 0..4096 {
            let Some(sup) = self.cx.superclass(current)? else {
                return Ok(false);
            };
            let Type::Class { key, .. } = self.cx.get(sup) else {
                return Ok(false);
            };
            for index in 0..self.cx.method_count(key)? as u32 {
                let above = MethodRef { owner: key, index };
                if self.cx.method_name(above)? != Some(name)
                    || self.cx.method_info(above)?.is_none_or(|i| i.is_abstract)
                {
                    continue;
                }
                if let Tri::Yes(t) = self.cx.method_type_in(of, key, index)?
                    && self.cx.subsignature(&c.ty, &t)?.is_yes()
                {
                    return Ok(true);
                }
            }
            current = key;
        }
        Ok(false)
    }

    fn ambiguous(
        &mut self,
        candidates: &[Candidate],
        maximal: &[usize],
        request: &Request<'_>,
    ) -> TypeId {
        let owners = maximal
            .iter()
            .map(|&i| candidates[i].method.owner)
            .collect();
        self.error_type(ErrorSymbol {
            name: request.name,
            cause: Cause::Ambiguous(owners),
        })
    }

    /// Records the conversions the chosen method's parameters apply to its arguments.
    fn record_arguments(&mut self, args: &[Value], chosen: &Chosen) -> Result<(), Cancelled> {
        let context = phase_context(chosen.phase);
        for (i, arg) in args.iter().enumerate() {
            let formal = if chosen.phase == Phase::VariableArity {
                variable_arity_type(&self.cx, &chosen.invoked, i)
            } else {
                chosen.invoked.params.get(i).copied()
            };
            let Some(formal) = formal else { continue };
            match &arg.operands {
                Some(operands) => {
                    for operand in operands.clone().iter() {
                        self.record_conversion(operand, context, formal)?;
                    }
                }
                None => {
                    self.record_conversion(arg, context, formal)?;
                }
            }
        }
        Ok(())
    }

    /// A method call's type, from the method chosen (JLS 15.12.3): the result as invoked, erased
    /// if an unchecked conversion was needed (15.12.2.6), captured; `getClass()`'s (JLS 4.3.2),
    /// or a signature polymorphic method's.
    fn method_result(
        &mut self,
        node: SyntaxNode<'_>,
        mut chosen: Chosen,
        search: &Search,
        args: &[Value],
    ) -> Result<Value, Cancelled> {
        let id = node.id();
        if self.is_signature_polymorphic(chosen.method)? {
            return self.signature_polymorphic(node, chosen, args);
        }
        self.record_arguments(args, &chosen)?;
        if chosen.unchecked {
            chosen.invoked.result = self.cx.erasure(chosen.invoked.result)?;
            for t in &mut chosen.invoked.throws {
                *t = self.cx.erasure(*t)?;
            }
        }
        let result = if args.is_empty() && self.is_get_class(chosen.method)? {
            // A raw interface's `getClass()` is erased as a raw type's members are, a raw class's
            // isn't (observed: `()java/lang/Class` and `()java/lang/Class<?>`).
            if self.cx.is_raw_interface(search.searched)? {
                chosen.invoked.result = self.cx.erasure(chosen.invoked.result)?;
            }
            self.get_class_type(search.searched)?
        } else {
            chosen.invoked.result
        };
        let t = self.cx.fresh_capture(result)?;
        let t = self.call_as_argument(node, t)?;
        self.calls.push((
            id,
            Call {
                method: chosen.method,
                invoked: chosen.invoked,
                phase: chosen.phase,
                special: Special::None,
            },
        ));
        Ok(self.expr(id, t))
    }

    /// A call that is an argument of another, directly or as a conditional's operand, whose type
    /// is a capture variable, is recorded with its upper bound, itself captured (observed:
    /// `g(xs.get(0))` is `java/lang/Number` for `List<? extends Number> xs`, where `xs.get(0);` is
    /// `capture#1 extends java/lang/Number`, and `Enum<capture#1 ...>` for a bound `Enum<?>`; a
    /// capture inside a type stays).
    fn call_as_argument(&mut self, node: SyntaxNode<'_>, t: TypeId) -> Result<TypeId, Cancelled> {
        if matches!(self.cx.get(t), Type::Capture(_)) && argument_position(node).is_some() {
            let upper = self.cx.upper_bound(t)?;
            return self.cx.fresh_capture(upper);
        }
        Ok(t)
    }

    /// A field or array element as a conditional's operand in an argument has its capture variable
    /// replaced the same way, though as the argument itself it keeps it (observed: `g(b ? bq.f :
    /// null)`'s `bq.f` is `java/lang/Object`, `g(bq.f)`'s `capture#1 extends java/lang/Object`).
    pub(super) fn variable_as_argument(
        &mut self,
        node: SyntaxNode<'_>,
        t: TypeId,
    ) -> Result<TypeId, Cancelled> {
        if matches!(self.cx.get(t), Type::Capture(_))
            && argument_position(node) == Some(ArgumentPosition::Operand)
        {
            let upper = self.cx.upper_bound(t)?;
            return self.cx.fresh_capture(upper);
        }
        Ok(t)
    }

    fn is_get_class(&mut self, m: MethodRef) -> Result<bool, Cancelled> {
        if self.cx.lang("java/lang/Object")? != Some(m.owner) {
            return Ok(false);
        }
        Ok(self.cx.method_name(m)? == Some(self.names.intern("getClass")))
    }

    /// `getClass()`'s type before capture: `Class<? extends |T|>`, `T` the type searched
    /// (JLS 4.3.2; observed: `Class<capture#1 extends java/util/List>` for `xs` of a `List<?
    /// extends Number>`, `Class<capture#1 extends java/lang/Object[]>` for `T[]`). Its type as
    /// invoked stays `()java/lang/Class<?>`.
    fn get_class_type(&mut self, searched: TypeId) -> Result<TypeId, Cancelled> {
        let erased = self.cx.erasure(searched)?;
        if let Some(why) = self.non_answer(erased) {
            return Ok(self.unknown(why));
        }
        Ok(match self.cx.lang("java/lang/Class")? {
            Some(class) => self.cx.class_type(
                class,
                None,
                Some(&[TypeArg::Wildcard(Some((WildcardBound::Extends, erased)))]),
            ),
            None => self.lang("java/lang/Class")?,
        })
    }

    /// `clone()` on an array (JLS 10.7): the array's type, with no exceptions. javac keys it
    /// `Array.clone()Ljava/lang/Object;`, typed as invoked `()java/lang/Object`.
    fn array_clone(&mut self, id: NodeId, search: &Search) -> Result<Value, Cancelled> {
        let object = self.lang("java/lang/Object")?;
        let clone = self.names.intern("clone");
        let method = search
            .methods
            .iter()
            .copied()
            .find(|m| self.cx.method_name(*m).ok().flatten() == Some(clone))
            .or_else(|| self.object_method(clone));
        let t = self.array_of(search.searched)?;
        if let Some(method) = method {
            self.calls.push((
                id,
                Call {
                    method,
                    invoked: MethodType {
                        type_params: Vec::new(),
                        params: Vec::new(),
                        result: object,
                        throws: Vec::new(),
                    },
                    phase: Phase::Strict,
                    special: Special::ArrayClone,
                },
            ));
        }
        Ok(self.expr(id, t))
    }

    /// `Object`'s method of a name with no parameters.
    fn object_method(&mut self, name: Name) -> Option<MethodRef> {
        let object = self.cx.lang("java/lang/Object").ok()??;
        let count = self.cx.method_count(object).ok()?;
        (0..count as u32)
            .map(|index| MethodRef {
                owner: object,
                index,
            })
            .find(|m| self.cx.method_name(*m).ok().flatten() == Some(name))
    }

    /// The array type a searched type is: itself, or a capture's bound.
    fn array_of(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        Ok(match self.cx.get(t) {
            Type::Array(_) => t,
            _ => self.cx.upper_bound(t)?,
        })
    }

    /// Whether a method is signature polymorphic (JLS 15.12.3): native, of `MethodHandle` or
    /// `VarHandle`, with a single variable arity parameter of `Object[]`.
    fn is_signature_polymorphic(&mut self, m: MethodRef) -> Result<bool, Cancelled> {
        let TypeKey::Class { .. } = m.owner else {
            return Ok(false);
        };
        let Some(info) = self.cx.method_info(m)? else {
            return Ok(false);
        };
        if !info.is_native || !info.varargs || info.params != 1 {
            return Ok(false);
        }
        Ok(
            self.cx.lang("java/lang/invoke/MethodHandle")? == Some(m.owner)
                || self.cx.lang("java/lang/invoke/VarHandle")? == Some(m.owner),
        )
    }

    /// A signature polymorphic call's type (JLS 15.12.3): the parameters are the arguments' types,
    /// class types erased, `null`'s being `Void`, and a type variable staying (observed:
    /// `java/util/List` for a `List<E>`, `E:K` for an `E`, `E:K[]` for an `E[]`); a result declared
    /// `Object` is the type of a cast around the call, `void` as a statement, and `Object` else;
    /// any other is as declared.
    fn signature_polymorphic(
        &mut self,
        node: SyntaxNode<'_>,
        chosen: Chosen,
        args: &[Value],
    ) -> Result<Value, Cancelled> {
        let id = node.id();
        let mut params = Vec::with_capacity(args.len());
        for arg in args {
            if arg.operands.is_some() {
                let t = self.unknown(Why::LeftToB3c);
                return Ok(self.expr(id, t));
            }
            params.push(self.polymorphic_parameter(arg.ty)?);
        }
        let object = self.lang("java/lang/Object")?;
        let result = if chosen.invoked.result == object {
            self.polymorphic_result(node, object)?
        } else {
            chosen.invoked.result
        };
        self.calls.push((
            id,
            Call {
                method: chosen.method,
                invoked: MethodType {
                    type_params: Vec::new(),
                    params,
                    result,
                    throws: chosen.invoked.throws,
                },
                phase: chosen.phase,
                special: Special::SignaturePolymorphic,
            },
        ));
        let t = self.cx.fresh_capture(result)?;
        Ok(self.expr(id, t))
    }

    fn polymorphic_parameter(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        let mut dimensions = 0;
        let mut element = t;
        while let Type::Array(component) = self.cx.get(element) {
            dimensions += 1;
            element = component;
        }
        let mut parameter = match self.cx.get(element) {
            Type::Null => self.lang("java/lang/Void")?,
            Type::Var(..) | Type::Primitive(_) => element,
            _ => self.cx.erasure(element)?,
        };
        for _ in 0..dimensions {
            parameter = self.cx.arena_mut().array(parameter);
        }
        Ok(parameter)
    }

    /// What context makes of a signature polymorphic method's `Object` result (observed through
    /// parentheses for a cast, and in a `for`'s init and update). A cast's type is erased from
    /// release 24 on, and as written before (observed: `(E) vh.get(this)` has the result
    /// `java/lang/Object` at `--release 24`, 25, 26, and 27, on JDK 27, and `E:L` at 21, 22, and 23).
    fn polymorphic_result(
        &mut self,
        node: SyntaxNode<'_>,
        object: TypeId,
    ) -> Result<TypeId, Cancelled> {
        let mut current = node;
        while let Some(parent) = current
            .parent()
            .filter(|p| p.kind() == SyntaxKind::ParenExpr)
        {
            current = parent;
        }
        let Some(parent) = current.parent() else {
            return Ok(object);
        };
        Ok(match parent.kind() {
            SyntaxKind::ExprStmt | SyntaxKind::ForInit | SyntaxKind::ForUpdate => {
                self.cx.arena_mut().intern(Type::Void)
            }
            SyntaxKind::CastExpr => match ast::CastExpr::cast(parent).unwrap().ty() {
                Some(ty) => {
                    let t = self.syntax_type(ty)?;
                    if self.release >= 24 {
                        self.cx.erasure(t)?
                    } else {
                        t
                    }
                }
                None => object,
            },
            _ => object,
        })
    }

    // -------------------------------------------------------------------------------------------
    // Constructors.

    /// Chooses a constructor of the class of `of`.
    fn constructor(
        &mut self,
        of: TypeId,
        key: TypeKey,
        args: &[Value],
        explicit: Option<&[TypeId]>,
        qualifier: Qualifier,
    ) -> Result<Result<Chosen, TypeId>, Cancelled> {
        // Lombok may add constructors (until B4).
        if self.cx.is_lombok(key)? {
            return Ok(Err(self.unknown(Why::Incomplete(key))));
        }
        let methods: Vec<MethodRef> = self
            .cx
            .constructors(key)?
            .into_iter()
            .map(|index| MethodRef { owner: key, index })
            .collect();
        let name = self.names.intern("<init>");
        let request = Request {
            methods: &methods,
            of: Some(of),
            args,
            explicit,
            qualifier,
            name,
        };
        self.resolve(&request)
    }

    fn record_constructor(&mut self, id: NodeId, chosen: Chosen) {
        self.calls.push((
            id,
            Call {
                method: chosen.method,
                invoked: chosen.invoked,
                phase: chosen.phase,
                special: Special::None,
            },
        ));
    }

    /// A class instance creation (JLS 15.9): its type is the class named, or the anonymous class
    /// made, whichever constructor is chosen, so only an error in it makes it a non-answer;
    /// `<>` is inference's (B3c).
    fn creation(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let new = ast::NewExpr::cast(node).unwrap();
        let id = node.id();
        let outer = child(children, id_of(new.outer())).cloned();
        let args = self.argument_values(new.arguments(), children);
        if let Some(o) = &outer
            && let Some(why) = self.non_answer(o.ty)
        {
            let t = self.non_answer_type(why);
            return Ok(self.expr(id, t));
        }
        let Some(ty) = new.ty() else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(id, t));
        };
        let diamond = matches!(ty, ast::Type::ClassType(c)
            if c.type_arguments().is_some_and(|a| a.arguments().next().is_none()));
        if diamond {
            let t = self.unknown(Why::LeftToB3c);
            return Ok(self.expr(id, t));
        }
        let written = self.created_type(ty, outer.as_ref())?;
        if let Some(why) = self.non_answer(written) {
            let t = self.non_answer_type(why);
            return Ok(self.expr(id, t));
        }
        let Type::Class { key, .. } = self.cx.get(written) else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(id, t));
        };
        let anonymous = id_of(new.body()).and_then(|b| self.local_of_node.get(&b).copied());
        let created = match anonymous {
            Some(local) => self.cx.class_type(local, None, None),
            None => written,
        };
        let explicit = self.explicit_arguments(new.type_arguments())?;
        let qualifier = if anonymous.is_some() {
            Qualifier::Anonymous
        } else {
            Qualifier::Creation
        };
        // An anonymous class of an interface extends `Object`, with no arguments (JLS 15.9.5).
        let chosen = if self.cx.is_interface(key)? {
            let object = self.lang("java/lang/Object")?;
            let object_key = self.lang_key("java/lang/Object")?;
            self.constructor(object, object_key, &args, None, Qualifier::Own)?
        } else {
            self.constructor(written, key, &args, explicit.as_deref(), qualifier)?
        };
        match chosen {
            Ok(chosen) => {
                self.record_arguments(&args, &chosen)?;
                self.record_constructor(id, chosen);
            }
            Err(t) => {
                if let Some(Why::Error(_)) = self.non_answer(t) {
                    return Ok(self.expr(id, t));
                }
            }
        }
        Ok(self.expr(id, created))
    }

    /// An anonymous class made through an outer instance, as its body is entered: its entry has no
    /// generic superclass, whose outer type is the instance's, which only the walk knows.
    pub(super) fn enter_anonymous_of_outer(
        &mut self,
        key: TypeKey,
        new: ast::NewExpr<'_>,
    ) -> Result<(), Cancelled> {
        let (Some(outer), Some(ty)) = (id_of(new.outer()), new.ty()) else {
            return Ok(());
        };
        let Some(outer) = self.values.iter().rev().find(|v| v.node == outer).cloned() else {
            return Ok(());
        };
        if self.non_answer(outer.ty).is_some() {
            return Ok(());
        }
        let superclass = self.created_type(ty, Some(&outer))?;
        if let Type::Class { key: class, .. } = self.cx.get(superclass) {
            self.cx.set_supertypes(key, &[superclass]);
            self.outer_superclasses.insert(key, class);
        }
        Ok(())
    }

    /// The class a creation names: as written, or for `outer.new Inner()` a member of `outer`'s
    /// type, which B2 finds when it can, and is otherwise found here.
    fn created_type(
        &mut self,
        ty: ast::Type<'_>,
        outer: Option<&Value>,
    ) -> Result<TypeId, Cancelled> {
        let (Some(outer), ast::Type::ClassType(c)) = (outer, ty) else {
            return self.syntax_type(ty);
        };
        let symbol = self.symbol(c.syntax().id()).cloned();
        let key = match symbol {
            Some(Symbol::Type(key)) => key,
            Some(Symbol::Error(e)) => return Ok(self.error_type(e)),
            _ => {
                let name = c.name().map_or(Name::EMPTY, |t| name_text(t, &self.names));
                match self.member_type_of(outer.ty, name)? {
                    Ok(key) => key,
                    Err(t) => return Ok(t),
                }
            }
        };
        let args = self.written_arguments(c)?;
        let of = self.cx.outer_of_created(key, outer.ty)?;
        Ok(self.cx.class_type(key, of, args.as_deref()))
    }

    /// A member type by name of a qualifier's type.
    fn member_type_of(
        &mut self,
        t: TypeId,
        name: Name,
    ) -> Result<Result<TypeKey, TypeId>, Cancelled> {
        let upper = self.cx.upper_bound(t)?;
        let Type::Class { key, .. } = self.cx.get(upper) else {
            let t = self.unknown(Why::Later);
            return Ok(Err(t));
        };
        let members = match key {
            TypeKey::Local { .. } => Some(self.snapshot.member_types(self.view, key)?),
            _ => self.reads.member_types(self.view, key)?,
        };
        Ok(match members.as_ref().and_then(|m| m.get(name)) {
            Some((MemberType::One(found), _)) => Ok(*found),
            Some((MemberType::Ambiguous(types), _)) => Err(self.error_type(ErrorSymbol {
                name,
                cause: Cause::Ambiguous(types.clone()),
            })),
            None => Err(self.error_type(ErrorSymbol {
                name,
                cause: Cause::NotFound,
            })),
        })
    }

    /// `this(...)` or `super(...)` (JLS 8.8.7.1), `void`.
    fn constructor_call(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let call = ast::ConstructorCallExpr::cast(node).unwrap();
        let id = node.id();
        let args = self.argument_values(call.arguments(), children);
        let void = self.cx.arena_mut().intern(Type::Void);
        let Some(class) = self.classes.last().map(|c| c.key) else {
            return Ok(self.expr(id, void));
        };
        let site = if call.this_kw().is_some() {
            Some(self.cx.declared_type(class)?)
        } else {
            self.cx.superclass(class)?
        };
        let explicit = self.explicit_arguments(call.type_arguments())?;
        if let Some(site) = site
            && let Type::Class { key, .. } = self.cx.get(site)
            && let Ok(chosen) =
                self.constructor(site, key, &args, explicit.as_deref(), Qualifier::Own)?
        {
            self.record_arguments(&args, &chosen)?;
            self.record_constructor(id, chosen);
        }
        Ok(self.expr(id, void))
    }

    /// An enum constant's creation (JLS 8.9.1): a constructor of its enum, for its arguments.
    /// Its fuel is its own, since its arguments and body were bodies.
    pub(super) fn enum_constant(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<(), Cancelled> {
        let constant = ast::EnumConstant::cast(node).unwrap();
        let args = self.argument_values(constant.arguments(), children);
        let Some(key) = self.classes.last().map(|c| c.key) else {
            return Ok(());
        };
        let of = self.cx.declared_type(key)?;
        if let Ok(chosen) = self.constructor(of, key, &args, None, Qualifier::Own)? {
            self.record_arguments(&args, &chosen)?;
            self.record_constructor(node.id(), chosen);
        }
        self.body_fuel.push((node.id(), self.cx.fuel_used()));
        self.cx.refuel(self.fuel);
        Ok(())
    }

    /// A constructor body without `this(...)` or `super(...)` calls `super()` (JLS 8.8.7).
    pub(super) fn implicit_super_in_body(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let is_constructor = node.parent().is_some_and(|p| {
            matches!(
                p.kind(),
                SyntaxKind::ConstructorDecl | SyntaxKind::CompactConstructorDecl
            )
        });
        if !is_constructor {
            return Ok(());
        }
        let explicit = ast::Block::cast(node).unwrap().statements().any(|s| {
            s.syntax().kind() == SyntaxKind::ExprStmt
                && s.syntax()
                    .children()
                    .any(|e| e.kind() == SyntaxKind::ConstructorCallExpr)
        });
        if !explicit {
            self.implicit_super(node.id())?;
        }
        Ok(())
    }

    /// A class with a default constructor calls `super()` from it (JLS 8.8.9), by the class's
    /// node.
    pub(super) fn default_constructor(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let Some(class) = self.classes.last().map(|c| c.key) else {
            return Ok(());
        };
        if !self.cx.has_default_constructor(class)? {
            return Ok(());
        }
        self.implicit_super(node.id())
    }

    /// The `super()` a constructor of the class the walk is in calls without saying so: an enum's
    /// is `Enum`'s, with its two parameters (observed:
    /// `java/lang/Enum.<init>(Ljava/lang/String;I)V`).
    fn implicit_super(&mut self, id: NodeId) -> Result<(), Cancelled> {
        let Some(class) = self.classes.last().map(|c| c.key) else {
            return Ok(());
        };
        let Some(site) = self.cx.superclass(class)? else {
            return Ok(());
        };
        let Type::Class { key, .. } = self.cx.get(site) else {
            return Ok(());
        };
        if self.cx.kind(class)? == Some(TypeKind::Enum) {
            let constructors = self.cx.constructors(key)?;
            if let [index] = constructors[..] {
                let method = MethodRef { owner: key, index };
                if let Tri::Yes(invoked) = self.cx.method_type_in(site, key, index)? {
                    self.record_constructor(
                        id,
                        Chosen {
                            method,
                            invoked,
                            phase: Phase::Strict,
                            unchecked: false,
                        },
                    );
                }
            }
            return Ok(());
        }
        if let Ok(chosen) = self.constructor(site, key, &[], None, Qualifier::Own)? {
            self.record_constructor(id, chosen);
        }
        Ok(())
    }
}
