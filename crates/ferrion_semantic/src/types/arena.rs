//! The arena: one computation's types, hash-consed, so that equal types have equal ids
//! (docs/design/semantic-type-relations.md#the-arena).

use crate::body::Map;
use crate::summary::{Primitive, WildcardBound};
use crate::symbols::{ErrorSymbol, TypeKey, TypeVarOwner};

/// A type in one computation's arena. Two equal types have equal ids, except capture variables,
/// which are fresh each time one is made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct TypeId(u32);

impl TypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A list of type arguments in an arena.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ArgList(u32);

/// A list of types in an arena: an intersection's parts.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct TypeList(u32);

/// An error symbol in an arena: the root cause of an [`Type::Error`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ErrorId(u32);

/// Why a type or an answer is unknown.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Why {
    /// A query ran out of fuel, or met itself (docs/design/semantic-type-relations.md#termination).
    Limit,
    /// A later milestone types it.
    Later,
    /// The answer needs a part of this type's hierarchy that didn't resolve.
    Incomplete(TypeKey),
    /// It depends on a name that didn't resolve.
    Error(ErrorId),
}

/// A type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    Primitive(Primitive),
    /// A method's result when it has none.
    Void,
    Null,
    /// A class or interface type. `outer` is the type it is a member of, when that type, or one
    /// enclosing it, has type arguments (written, or implied by where the type is used inside a
    /// generic class); `args` is `None` for a non-generic class and for a raw type, which the
    /// class's type parameters tell apart.
    Class {
        key: TypeKey,
        outer: Option<TypeId>,
        args: Option<ArgList>,
    },
    Array(TypeId),
    /// A declared type variable: B2's owner and index.
    Var(TypeVarOwner, u32),
    /// A capture variable (JLS 5.1.10), by its index in the arena's captures, where its bounds
    /// are: they are set after it is made, since they can name it.
    Capture(u32),
    /// `A & B & ...`, in the order the oracle compares.
    Intersection(TypeList),
    /// Depends on a name that didn't resolve, or an incomplete hierarchy: the root cause.
    Error(ErrorId),
    /// Ferrion can't type it: left to a later milestone, or a limit hit.
    Unknown(Why),
}

/// A type argument.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TypeArg {
    Type(TypeId),
    /// `?`, `? extends T`, `? super T`.
    Wildcard(Option<(WildcardBound, TypeId)>),
}

/// A capture variable's bounds, and the wildcard it captured.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CaptureVar {
    pub upper: TypeId,
    pub lower: Option<TypeId>,
    pub wildcard: TypeArg,
    /// The class whose argument it captured, and the argument's index: the type parameter whose
    /// bound is part of its upper bound.
    pub of: (TypeKey, u32),
}

/// One computation's types.
#[derive(Default, Debug)]
pub struct Arena {
    types: Vec<Type>,
    ids: Map<Type, TypeId>,
    arg_lists: Vec<Box<[TypeArg]>>,
    arg_ids: Map<Box<[TypeArg]>, ArgList>,
    type_lists: Vec<Box<[TypeId]>>,
    list_ids: Map<Box<[TypeId]>, TypeList>,
    captures: Vec<CaptureVar>,
    errors: Vec<ErrorSymbol>,
    error_ids: Map<ErrorSymbol, ErrorId>,
}

impl Arena {
    pub fn new() -> Arena {
        Arena::default()
    }

    pub fn get(&self, id: TypeId) -> Type {
        self.types[id.index()]
    }

    /// The id of `ty`, adding it the first time.
    pub fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(&id) = self.ids.get(&ty) {
            return id;
        }
        let id = TypeId(u32::try_from(self.types.len()).expect("fewer than 2^32 types"));
        self.types.push(ty);
        self.ids.insert(ty, id);
        id
    }

    pub fn args(&self, list: ArgList) -> &[TypeArg] {
        &self.arg_lists[list.0 as usize]
    }

    pub fn arg_list(&mut self, args: &[TypeArg]) -> ArgList {
        if let Some(&id) = self.arg_ids.get(args) {
            return id;
        }
        let id = ArgList(self.arg_lists.len() as u32);
        self.arg_lists.push(args.into());
        self.arg_ids.insert(args.into(), id);
        id
    }

    pub fn list(&self, list: TypeList) -> &[TypeId] {
        &self.type_lists[list.0 as usize]
    }

    pub fn type_list(&mut self, types: &[TypeId]) -> TypeList {
        if let Some(&id) = self.list_ids.get(types) {
            return id;
        }
        let id = TypeList(self.type_lists.len() as u32);
        self.type_lists.push(types.into());
        self.list_ids.insert(types.into(), id);
        id
    }

    pub fn error(&mut self, error: ErrorSymbol) -> ErrorId {
        if let Some(&id) = self.error_ids.get(&error) {
            return id;
        }
        let id = ErrorId(self.errors.len() as u32);
        self.errors.push(error.clone());
        self.error_ids.insert(error, id);
        id
    }

    pub fn error_symbol(&self, id: ErrorId) -> &ErrorSymbol {
        &self.errors[id.0 as usize]
    }

    /// A fresh capture variable of `wildcard`, the `index`th argument of `of`, with no bounds
    /// yet: [`Arena::set_bounds`] sets them.
    pub fn new_capture(&mut self, wildcard: TypeArg, of: TypeKey, index: u32) -> TypeId {
        let n = self.captures.len() as u32;
        let placeholder = self.intern(Type::Unknown(Why::Later));
        self.captures.push(CaptureVar {
            upper: placeholder,
            lower: None,
            wildcard,
            of: (of, index),
        });
        self.intern(Type::Capture(n))
    }

    pub fn set_bounds(&mut self, capture: u32, upper: TypeId, lower: Option<TypeId>) {
        let var = &mut self.captures[capture as usize];
        var.upper = upper;
        var.lower = lower;
    }

    pub fn capture(&self, n: u32) -> &CaptureVar {
        &self.captures[n as usize]
    }

    /// How many types the arena holds.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// How many capture variables it has made.
    pub fn captures(&self) -> usize {
        self.captures.len()
    }

    pub fn primitive(&mut self, p: Primitive) -> TypeId {
        self.intern(Type::Primitive(p))
    }

    pub fn array(&mut self, component: TypeId) -> TypeId {
        self.intern(Type::Array(component))
    }

    /// A class type with `args`, `None` for a non-generic class or a raw type.
    pub fn class(
        &mut self,
        key: TypeKey,
        outer: Option<TypeId>,
        args: Option<&[TypeArg]>,
    ) -> TypeId {
        let args = args.map(|a| self.arg_list(a));
        self.intern(Type::Class { key, outer, args })
    }

    pub fn var(&mut self, owner: TypeVarOwner, index: u32) -> TypeId {
        self.intern(Type::Var(owner, index))
    }

    pub fn intersection(&mut self, parts: &[TypeId]) -> TypeId {
        if let [one] = parts {
            return *one;
        }
        let list = self.type_list(parts);
        self.intern(Type::Intersection(list))
    }

    pub fn unknown(&mut self, why: Why) -> TypeId {
        match why {
            Why::Error(e) => self.intern(Type::Error(e)),
            why => self.intern(Type::Unknown(why)),
        }
    }

    /// The class type's arguments, empty for a non-generic or raw class or another type.
    pub fn class_args(&self, id: TypeId) -> &[TypeArg] {
        match self.get(id) {
            Type::Class { args: Some(a), .. } => self.args(a),
            _ => &[],
        }
    }

    /// Whether the type is a reference type: not primitive, `void`, or a non-answer.
    pub fn is_reference(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            Type::Null
                | Type::Class { .. }
                | Type::Array(_)
                | Type::Var(..)
                | Type::Capture(_)
                | Type::Intersection(_)
        )
    }

    /// The root cause, if the type is a non-answer at its top.
    pub fn non_answer(&self, id: TypeId) -> Option<Why> {
        match self.get(id) {
            Type::Error(e) => Some(Why::Error(e)),
            Type::Unknown(why) => Some(why),
            _ => None,
        }
    }
}
