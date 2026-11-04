//! Type attribution (B3b, docs/design/semantic-attribution.md): a second walk over each file's
//! bodies, after its names ([`crate::body`]), giving every expression that needs no inference
//! its type, the conversions its context applies, and its constant value.
//!
//! The walk is the tree's flat pre-order walk, as B2's is, with an explicit stack of values: a
//! node's type is decided as it is left, from its children's values, which are on the stack
//! above where it started. Names come from the file's names by node; what B2 left to B3 (a field
//! of an expression's type, a member of a type variable, an array's `length`, a `case` label
//! whose selector needed a type) is looked up in the member tables of the qualifier's type
//! ([`crate::members`]). Fuel is drawn per body: a body that runs out has the rest of its
//! relation queries `Unknown(Limit)`, and the walk carries on.
//!
//! Method invocations and class instance creations choose their methods by overload resolution
//! (JLS 15.9, 15.12), inside the inference line: a call that needs inference is
//! `Unknown(LeftToB3c)`, and so is everything whose type depends on one. So are lambdas, method
//! references, poly conditionals, and implicitly typed lambda parameters; `switch` expressions
//! and `var` in patterns are B3d's (`Unknown(LeftToB3d)`).
//!
//! A file's result is a derived entry, read through [`Snapshot::body_types`], with its own arena.

#[cfg(test)]
mod call_tests;
#[cfg(test)]
mod tests;
mod walk;

use std::sync::Arc;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeId, SyntaxKind};

use crate::constants::Constant;
use crate::decl_names::Symbol;
use crate::engine::derived::{Memo, Reads};
use crate::members::Members;
use crate::symbols::{MethodRef, TypeKey};
use crate::types::{Arena, MethodType, Printed, Steps, TypeContext, TypeId};
use crate::{Cancelled, FileId, Snapshot};

pub(crate) use self::walk::Walker;

/// The fuel each body gets: a top-level member's body, a field's initializer, an enum
/// constant's arguments and body, an annotation's arguments. Provisional, as the relations'
/// [`DEFAULT_FUEL`](crate::types::DEFAULT_FUEL) is (docs/design/semantic-attribution.md#results).
pub const BODY_FUEL: u64 = crate::types::DEFAULT_FUEL;

/// A file's attributed bodies, by `NodeId` of the tree they were attributed on.
#[derive(Clone, Debug, Default, Hash)]
pub struct BodyTypes {
    /// Every type below lives here.
    pub arena: Arena,
    /// Each expression's type, by node, in node order: a non-answer type (`Error`, `Unknown`)
    /// where it has none.
    pub types: Box<[(NodeId, TypeId)]>,
    /// Each call's and creation's chosen method and its type as invoked, in node order, by node:
    /// a method invocation's, an explicit constructor invocation's, or a creation's. Calls javac
    /// makes up are by the node they belong to: a constructor's implicit `super()` by its body's
    /// `Block`, a default constructor's by the type's declaration, an enum constant's creation
    /// by the `EnumConstant`. A creation of an anonymous class has the superclass constructor it
    /// chooses ([`Snapshot::call_text`] gives javac's key for each).
    pub calls: Box<[(NodeId, Call)]>,
    /// The creations, explicit and implicit constructor calls, and enum constants without a chosen
    /// method, by the same nodes as `calls`, with why: a non-answer. A creation's own type doesn't
    /// say, since it is the class named whichever constructor is chosen, and a constructor call's
    /// is `void`.
    pub unchosen: Box<[(NodeId, TypeId)]>,
    /// Each constant expression's value (JLS 15.29), in node order.
    pub constants: Box<[(NodeId, Constant)]>,
    /// The conversions an expression's context applied to it, where any but identity was, in
    /// node order.
    pub conversions: Box<[(NodeId, Steps)]>,
    /// For each compound assignment whose result needs one, the implied cast's conversions
    /// (JLS 15.26.2), by the assignment's node.
    pub compound: Box<[(NodeId, Steps)]>,
    /// Each local variable's and parameter's type, by its declaring node (the node B2's
    /// `Symbol::Local` names), in node order: the declared type, or for `var` the upward
    /// projection of its initializer's type or its enhanced `for`'s element type.
    pub variables: Box<[(NodeId, TypeId)]>,
    /// The names B2 left to B3 that the walk resolved, by node: fields of an expression's type
    /// and of type variables, an array's `length`, `case` constants of an enum selector, and the
    /// type declaring a statically imported method, which B2 couldn't tell among the imports'.
    pub resolved: Box<[(NodeId, Symbol)]>,
    /// The member tables of the file's local and anonymous classes that the walk looked up.
    pub local_members: Box<[(TypeKey, Arc<Members>)]>,
    /// Each body's root node and the fuel it used, in node order.
    pub fuel: Box<[(NodeId, u64)]>,
}

/// A call's chosen method.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Call {
    pub method: MethodRef,
    /// Its type as invoked: parameters and result after substitution, as javac reports it. Not
    /// always the call's type: that is the type recorded for the call's node, captured, and
    /// `getClass()`'s and an array's `clone()` have a result of their own (JLS 4.3.2, 10.7).
    pub invoked: MethodType,
    pub phase: Phase,
    pub special: Special,
}

/// A call the language treats specially.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Special {
    None,
    /// `clone()` on an array (JLS 10.7): `method` is `Object`'s, which javac keys `Array.clone`.
    ArrayClone,
    /// A signature polymorphic method of `MethodHandle` or `VarHandle` (JLS 15.12.3): the
    /// parameters are the arguments' types, and the result comes from the context.
    SignaturePolymorphic,
}

/// A call as javac reports it.
#[doc(hidden)]
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallText {
    /// The method's key: for an anonymous class's creation, the superclass constructor's, which
    /// javac has for the implicit `super(...)` in it.
    pub method: String,
    /// Its type as invoked.
    pub invoked: String,
    /// For an anonymous class's creation, the key of the anonymous class's own constructor,
    /// which javac has for the creation.
    pub anonymous: Option<String>,
}

/// Which phase of JLS 15.12.2 chose a method.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase {
    Strict,
    Loose,
    VariableArity,
}

fn find<T>(list: &[(NodeId, T)], node: NodeId) -> Option<&T> {
    let i = list.binary_search_by_key(&node, |(n, _)| *n).ok()?;
    Some(&list[i].1)
}

impl BodyTypes {
    pub fn type_of(&self, node: NodeId) -> Option<TypeId> {
        find(&self.types, node).copied()
    }

    /// What the walk settled of a name B2 left, or couldn't tell.
    pub fn resolved_name(&self, node: NodeId) -> Option<&Symbol> {
        find(&self.resolved, node)
    }

    /// A call's chosen method, by the node [`BodyTypes::calls`] has it under.
    pub fn call(&self, node: NodeId) -> Option<&Call> {
        find(&self.calls, node)
    }

    pub fn constant(&self, node: NodeId) -> Option<&Constant> {
        find(&self.constants, node)
    }

    pub fn conversion(&self, node: NodeId) -> Option<&Steps> {
        find(&self.conversions, node)
    }

    /// A local variable's or parameter's type, by its declaring node.
    pub fn variable(&self, node: NodeId) -> Option<TypeId> {
        find(&self.variables, node).copied()
    }

    /// The most fuel a body of the file used.
    pub fn most_fuel(&self) -> u64 {
        self.fuel.iter().map(|(_, f)| *f).max().unwrap_or(0)
    }
}

impl Snapshot {
    /// Every expression's type, every constant's value, and the conversions applied, in
    /// `file`'s bodies, by node of its current tree ([`Snapshot::tree`]), with the arena the
    /// types live in.
    pub fn body_types(&self, file: FileId) -> Result<Arc<BodyTypes>, Cancelled> {
        self.check()?;
        Ok(self.body_types_memo(file)?.value.clone())
    }

    pub(crate) fn body_types_memo(&self, file: FileId) -> Result<Arc<Memo<BodyTypes>>, Cancelled> {
        self.shared().body_types.get(self, &file, |reads| {
            Walker::attribute(reads, file, BODY_FUEL)
        })
    }

    /// `file`'s attribution with `fuel` per body, computed now and not kept: for measuring what
    /// the bodies use (the corpus run) without the budget cutting them off.
    #[doc(hidden)]
    pub fn attribute_with_fuel(&self, file: FileId, fuel: u64) -> Result<BodyTypes, Cancelled> {
        self.check()?;
        let mut reads = Reads::untracked(self);
        Walker::attribute(&mut reads, file, fuel)
    }

    /// What javac reports for a call in [`BodyTypes::calls`], in the oracle's forms
    /// (docs/design/types-oracle-normalization.md#calls).
    #[doc(hidden)]
    pub fn call_text(
        &self,
        file: FileId,
        cx: &mut TypeContext<'_>,
        types: &BodyTypes,
        node: NodeId,
        call: &Call,
    ) -> Result<CallText, Cancelled> {
        let method = match call.special {
            Special::ArrayClone => "method Array.clone()Ljava/lang/Object;".to_owned(),
            _ => self.method_text(call.method.owner, call.method.index)?,
        };
        let invoked = cx.method_string(&call.invoked, Printed::Captures)?;
        // An anonymous class's creation chooses its own constructor, with the parameters of the
        // superclass's as invoked, after an outer instance; the superclass's is the implicit
        // `super(...)` in it (JLS 15.9.5.1).
        let tree = self.tree(file)?;
        let syntax = tree.node(node);
        let (body, outer) = match syntax.kind() {
            SyntaxKind::NewExpr => {
                let new = ast::NewExpr::cast(syntax).unwrap();
                (
                    new.body().map(|b| b.syntax().id()),
                    new.outer().map(|o| o.syntax().id()),
                )
            }
            SyntaxKind::EnumConstant => (
                ast::EnumConstant::cast(syntax)
                    .unwrap()
                    .body()
                    .map(|b| b.syntax().id()),
                None,
            ),
            _ => (None, None),
        };
        let local = match body {
            Some(body) => self
                .body_names(file)?
                .local_types
                .iter()
                .position(|t| t.node == body)
                .map(|index| TypeKey::Local {
                    file,
                    index: index as u32,
                }),
            None => None,
        };
        let Some(local) = local else {
            return Ok(CallText {
                method,
                invoked,
                anonymous: None,
            });
        };
        let mut descriptor = String::from("(");
        if let Some(t) = outer.and_then(|o| types.type_of(o)) {
            descriptor.push_str(&cx.erased_descriptor(t)?);
        }
        for &p in &call.invoked.params {
            descriptor.push_str(&cx.erased_descriptor(p)?);
        }
        descriptor.push_str(")V");
        let anonymous = format!("method {}.<init>{descriptor}", self.binary_text(local)?);
        Ok(CallText {
            method,
            invoked,
            anonymous: Some(anonymous),
        })
    }

    /// A file's attribution as text, for tests and the differential check: each expression's
    /// range, its type in the types oracle's form, and its constant and conversions, in node
    /// order.
    #[doc(hidden)]
    pub fn body_types_dump(&self, file: FileId) -> Result<String, Cancelled> {
        let types = self.body_types(file)?;
        self.dump_types(file, &types)
    }

    #[doc(hidden)]
    pub fn dump_types(&self, file: FileId, types: &BodyTypes) -> Result<String, Cancelled> {
        use std::fmt::Write as _;
        let tree = self.tree(file)?;
        let view = self.file(file)?.set;
        let mut cx = self.types_in(view, types.arena.clone());
        let mut out = String::new();
        for (node, ty) in types.types.iter() {
            let syntax = tree.node(*node);
            let span = syntax.span();
            let text: String = syntax
                .text()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let text: String = text.chars().take(40).collect();
            let _ = write!(
                out,
                "{}-{} {:?} `{}`: {}",
                span.start,
                span.end,
                syntax.kind(),
                text,
                cx.type_string(*ty, Printed::Captures)?
            );
            if let Some(c) = types.constant(*node) {
                let _ = write!(out, " = {}", c.oracle_text());
            }
            if let Some(steps) = types.conversion(*node) {
                let _ = write!(out, " {:?}", steps.as_slice());
            }
            out.push('\n');
        }
        for (node, steps) in types.compound.iter() {
            let span = tree.node(*node).span();
            let _ = writeln!(
                out,
                "{}-{} compound {:?}",
                span.start,
                span.end,
                steps.as_slice()
            );
        }
        for (node, call) in types.calls.iter() {
            let span = tree.node(*node).span();
            let _ = writeln!(
                out,
                "{}-{} call {} {} {:?}",
                span.start,
                span.end,
                self.method_text(call.method.owner, call.method.index)?,
                cx.method_string(&call.invoked, Printed::Captures)?,
                call.phase
            );
        }
        for (node, why) in types.unchosen.iter() {
            let span = tree.node(*node).span();
            let _ = writeln!(
                out,
                "{}-{} no method: {}",
                span.start,
                span.end,
                cx.type_string(*why, Printed::Captures)?
            );
        }
        for (node, symbol) in types.resolved.iter() {
            let span = tree.node(*node).span();
            let _ = writeln!(
                out,
                "{}-{} resolved {}",
                span.start,
                span.end,
                self.symbol_key(symbol)?
            );
        }
        Ok(out)
    }
}
