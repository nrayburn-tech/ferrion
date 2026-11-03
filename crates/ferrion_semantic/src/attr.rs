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
//! Method invocations and class instance creations wait for overload resolution (W3): each is
//! `Unknown(Calls)`, and so is everything whose type depends on one. Lambdas, method references,
//! poly conditionals, and implicitly typed lambda parameters are B3c's (`Unknown(LeftToB3c)`),
//! and `switch` expressions and `var` in patterns B3d's (`Unknown(LeftToB3d)`).
//!
//! A file's result is a derived entry, read through [`Snapshot::body_types`], with its own arena.

#[cfg(test)]
mod tests;
mod walk;

use std::sync::Arc;

use ferrion_syntax::NodeId;

use crate::constants::Constant;
use crate::decl_names::Symbol;
use crate::engine::derived::{Memo, Reads};
use crate::members::Members;
use crate::symbols::{MethodRef, TypeKey};
use crate::types::{Arena, MethodType, Printed, Steps, TypeId};
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
    /// Each call's and creation's chosen method and its type as invoked (W3's; empty until
    /// overload resolution).
    pub calls: Box<[(NodeId, Call)]>,
    /// Each constant expression's value (JLS 15.29), in node order.
    pub constants: Box<[(NodeId, Constant)]>,
    /// The conversions an expression's context applied to it, where any but identity was, in
    /// node order.
    pub conversions: Box<[(NodeId, Steps)]>,
    /// For each compound assignment whose result needs one, the implied cast's conversions
    /// (JLS 15.26.2), by the assignment's node.
    pub compound: Box<[(NodeId, Steps)]>,
    /// The names B2 left to B3 that the walk resolved, by node: fields of an expression's type
    /// and of type variables, an array's `length`, `case` constants of an enum selector.
    pub resolved: Box<[(NodeId, Symbol)]>,
    /// The member tables of the file's local and anonymous classes that the walk looked up.
    pub local_members: Box<[(TypeKey, Arc<Members>)]>,
    /// Each body's root node and the fuel it used, in node order.
    pub fuel: Box<[(NodeId, u64)]>,
}

/// A call's chosen method (W3).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Call {
    pub method: MethodRef,
    /// Its type as invoked: parameters and result after substitution.
    pub invoked: MethodType,
    pub phase: Phase,
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

    pub fn constant(&self, node: NodeId) -> Option<&Constant> {
        find(&self.constants, node)
    }

    pub fn conversion(&self, node: NodeId) -> Option<&Steps> {
        find(&self.conversions, node)
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
