//! Types as written: the syntax of a type, owned, so a summary outlives its tree.

use std::fmt;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{SyntaxKind, SyntaxNode};

use super::Annotation;
use super::build::{annotations, name_text};
use crate::{Interner, Name};

/// A type as written, unresolved: its name segments, type arguments, array dimensions, and
/// annotations (docs/design/semantic-names.md#file-summaries).
///
/// The type is held flat, as its parts in pre-order: each part is followed by the parts of its
/// children, and says how many children it has ([`TypePart::arity`]). So `Map<K, V[]>[]` is
///
/// ```text
/// Array 1 / Class 1 / Segment Map, 2 arguments / Class 1 / Segment K / Array 1 / Class 1 /
/// Segment V
/// ```
///
/// Being flat, a deeply nested type (`List<List<...>>`, 10,000 deep) is built, compared, hashed,
/// printed, and dropped without recursion. Parts are 12 bytes; their annotations, which few types
/// have, are kept beside them ([`TypeRef::annotations`]).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TypeRef {
    parts: Box<[TypePart]>,
    annotations: Option<Box<Annotations>>,
}

/// A type's annotations, boxed so that a type without any (nearly every type) spends one pointer
/// on them.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Annotations(Box<[TypeAnnotation]>);

/// One part of a [`TypeRef`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TypePart {
    /// `int`. No children.
    Primitive(Primitive),
    /// `void`, as a method's result. No children.
    Void,
    /// A class or interface type: its children are `segments` [`TypePart::Segment`]s, outermost
    /// first (`java`, `util`, `Map`, `Entry` for `java.util.Map.Entry`). Which segments are
    /// packages is decided by resolution.
    Class { segments: u32 },
    /// One segment of a class type. Its children are its type arguments: `arguments` is
    /// [`TypePart::NO_ARGUMENTS`] without `<...>`, and 0 for a diamond.
    Segment { name: Name, arguments: u32 },
    /// An array type with `dims` dimensions. One child, the element type.
    Array { dims: u32 },
    /// `?`, `? extends T`, or `? super T`. One child, the bound, unless `bound` is `None`.
    Wildcard { bound: Option<WildcardBound> },
    /// `A & B` (in a cast). Its children are the `types` types.
    Intersection { types: u32 },
    /// `A | B` (in a catch clause). Its children are the `types` types.
    Union { types: u32 },
    /// A type the parser couldn't read, or that is missing. No children.
    Missing,
}

/// An annotation on one part of a type: on a primitive type, a segment, or a wildcard, or with
/// `dim` set, on one dimension of an array part, outermost first.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TypeAnnotation {
    pub part: u32,
    pub dim: Option<u32>,
    pub annotation: Annotation,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Primitive {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WildcardBound {
    Extends,
    Super,
}

impl TypePart {
    /// [`TypePart::Segment::arguments`] for a segment without `<...>`.
    pub const NO_ARGUMENTS: u32 = u32::MAX;

    /// How many parts' subtrees follow this one as its children.
    pub fn arity(&self) -> usize {
        match *self {
            TypePart::Primitive(_) | TypePart::Void | TypePart::Missing => 0,
            TypePart::Class { segments } => segments as usize,
            TypePart::Segment { arguments, .. } if arguments == TypePart::NO_ARGUMENTS => 0,
            TypePart::Segment { arguments, .. } => arguments as usize,
            TypePart::Array { .. } => 1,
            TypePart::Wildcard { bound } => usize::from(bound.is_some()),
            TypePart::Intersection { types } | TypePart::Union { types } => types as usize,
        }
    }

    /// A segment's type arguments: `None` without `<...>`, `Some(0)` for a diamond.
    pub fn arguments(&self) -> Option<u32> {
        match *self {
            TypePart::Segment { arguments, .. } if arguments != TypePart::NO_ARGUMENTS => {
                Some(arguments)
            }
            _ => None,
        }
    }
}

impl Primitive {
    pub fn keyword(self) -> &'static str {
        match self {
            Primitive::Boolean => "boolean",
            Primitive::Byte => "byte",
            Primitive::Char => "char",
            Primitive::Short => "short",
            Primitive::Int => "int",
            Primitive::Long => "long",
            Primitive::Float => "float",
            Primitive::Double => "double",
        }
    }

    /// The descriptor character (JVMS 4.3.2).
    pub fn descriptor(self) -> char {
        match self {
            Primitive::Boolean => 'Z',
            Primitive::Byte => 'B',
            Primitive::Char => 'C',
            Primitive::Short => 'S',
            Primitive::Int => 'I',
            Primitive::Long => 'J',
            Primitive::Float => 'F',
            Primitive::Double => 'D',
        }
    }

    fn of(kind: SyntaxKind) -> Option<Primitive> {
        Some(match kind {
            SyntaxKind::BooleanKw => Primitive::Boolean,
            SyntaxKind::ByteKw => Primitive::Byte,
            SyntaxKind::CharKw => Primitive::Char,
            SyntaxKind::ShortKw => Primitive::Short,
            SyntaxKind::IntKw => Primitive::Int,
            SyntaxKind::LongKw => Primitive::Long,
            SyntaxKind::FloatKw => Primitive::Float,
            SyntaxKind::DoubleKw => Primitive::Double,
            _ => return None,
        })
    }
}

impl TypeRef {
    /// A type that is missing from the tree.
    pub fn missing() -> TypeRef {
        TypeRef {
            parts: Box::new([TypePart::Missing]),
            annotations: None,
        }
    }

    /// The type `ty` as written, or [`TypeRef::missing`] for `None`.
    pub fn from_syntax(ty: Option<ast::Type<'_>>, names: &Interner) -> TypeRef {
        let Some(ty) = ty else {
            return TypeRef::missing();
        };
        // A stack of what is still to be written, last first: a type's node, or a part already
        // made (with its annotations) whose children follow it on the stack.
        enum Work<'t> {
            Node(SyntaxNode<'t>),
            Part(TypePart, Box<[Annotation]>),
        }
        let mut parts = Vec::new();
        let mut type_annotations = Vec::new();
        let mut annotate = |parts: &Vec<TypePart>, dim: Option<u32>, list: Box<[Annotation]>| {
            for annotation in list {
                type_annotations.push(TypeAnnotation {
                    part: parts.len() as u32,
                    dim,
                    annotation,
                });
            }
        };
        let mut stack = vec![Work::Node(ty.syntax())];
        while let Some(work) = stack.pop() {
            let node = match work {
                Work::Part(part, list) => {
                    annotate(&parts, None, list);
                    parts.push(part);
                    continue;
                }
                Work::Node(node) => node,
            };
            match node.kind() {
                SyntaxKind::PrimitiveType => {
                    let ty = ast::PrimitiveType::cast(node).unwrap();
                    match ty.keyword().and_then(|k| Primitive::of(k.kind())) {
                        Some(keyword) => {
                            annotate(&parts, None, annotations(ty.annotations(), names));
                            parts.push(TypePart::Primitive(keyword));
                        }
                        None => parts.push(TypePart::Missing),
                    }
                }
                SyntaxKind::VoidType => parts.push(TypePart::Void),
                SyntaxKind::ClassType => {
                    // Segments nest to the left: the node is the last segment, and its qualifier
                    // the one before.
                    let mut segments = Vec::new();
                    let mut segment = ast::ClassType::cast(node);
                    while let Some(ty) = segment {
                        segments.push(ty);
                        segment = ty.qualifier();
                    }
                    parts.push(TypePart::Class {
                        segments: segments.len() as u32,
                    });
                    // `segments` is last first, so pushing in its order leaves the first segment
                    // on top. Each segment's arguments go under it, last first.
                    for ty in segments {
                        let arguments: Vec<ast::TypeArgument<'_>> = match ty.type_arguments() {
                            Some(args) => args.arguments().collect(),
                            None => Vec::new(),
                        };
                        let count = ty
                            .type_arguments()
                            .map_or(TypePart::NO_ARGUMENTS, |_| arguments.len() as u32);
                        for argument in arguments.iter().rev() {
                            stack.push(Work::Node(argument.syntax()));
                        }
                        let name = ty.name().map_or(Name::EMPTY, |n| name_text(n, names));
                        stack.push(Work::Part(
                            TypePart::Segment {
                                name,
                                arguments: count,
                            },
                            annotations(ty.annotations(), names),
                        ));
                    }
                }
                SyntaxKind::ArrayType => {
                    let ty = ast::ArrayType::cast(node).unwrap();
                    let mut dims = 0;
                    for dim in ty.dims() {
                        annotate(&parts, Some(dims), annotations(dim.annotations(), names));
                        dims += 1;
                    }
                    parts.push(TypePart::Array { dims });
                    push_child(&mut stack, ty.element_type().map(|t| t.syntax()));
                }
                SyntaxKind::Wildcard => {
                    let wildcard = ast::Wildcard::cast(node).unwrap();
                    let bound = if wildcard.extends_kw().is_some() {
                        Some(WildcardBound::Extends)
                    } else if wildcard.super_kw().is_some() {
                        Some(WildcardBound::Super)
                    } else {
                        None
                    };
                    annotate(&parts, None, annotations(wildcard.annotations(), names));
                    parts.push(TypePart::Wildcard { bound });
                    if bound.is_some() {
                        push_child(&mut stack, wildcard.bound().map(|t| t.syntax()));
                    }
                }
                SyntaxKind::IntersectionType | SyntaxKind::UnionType => {
                    let types: Vec<SyntaxNode<'_>> = match ast::IntersectionType::cast(node) {
                        Some(ty) => ty.types().map(|t| t.syntax()).collect(),
                        None => ast::UnionType::cast(node)
                            .unwrap()
                            .types()
                            .map(|t| t.syntax())
                            .collect(),
                    };
                    let count = types.len() as u32;
                    parts.push(if node.kind() == SyntaxKind::IntersectionType {
                        TypePart::Intersection { types: count }
                    } else {
                        TypePart::Union { types: count }
                    });
                    for ty in types.into_iter().rev() {
                        stack.push(Work::Node(ty));
                    }
                }
                _ => parts.push(TypePart::Missing),
            }
        }
        return TypeRef {
            parts: parts.into_boxed_slice(),
            annotations: (!type_annotations.is_empty())
                .then(|| Box::new(Annotations(type_annotations.into_boxed_slice()))),
        };

        /// Pushes a child that the grammar requires, or a missing type in its place.
        fn push_child<'t>(stack: &mut Vec<Work<'t>>, child: Option<SyntaxNode<'t>>) {
            stack.push(match child {
                Some(node) => Work::Node(node),
                None => Work::Part(TypePart::Missing, Box::default()),
            });
        }
    }

    /// The parts in pre-order. Never empty: the first is the type's outermost part.
    pub fn parts(&self) -> &[TypePart] {
        &self.parts
    }

    /// The annotations on the parts, in the order of their parts.
    pub fn annotations(&self) -> &[TypeAnnotation] {
        self.annotations.as_deref().map_or(&[], |a| &a.0)
    }

    /// The annotations on the part at `index` itself, not on its dimensions.
    pub fn annotations_of(&self, index: usize) -> impl Iterator<Item = &Annotation> {
        self.annotations()
            .iter()
            .filter(move |a| a.part as usize == index && a.dim.is_none())
            .map(|a| &a.annotation)
    }

    /// The annotations on dimension `dim` of the array part at `index`.
    pub fn dim_annotations(&self, index: usize, dim: u32) -> impl Iterator<Item = &Annotation> {
        self.annotations()
            .iter()
            .filter(move |a| a.part as usize == index && a.dim == Some(dim))
            .map(|a| &a.annotation)
    }

    pub fn is_missing(&self) -> bool {
        matches!(*self.parts, [TypePart::Missing])
    }

    /// The type with `extra` more array dimensions, outermost, and every annotation removed:
    /// what two declarations' types are compared by when only their shape matters (a record's
    /// canonical constructor, JLS 8.10.4). `int[] x[]` and `int x[][]` have the same shape.
    pub(crate) fn shape(&self, extra: usize) -> TypeRef {
        let mut parts = Vec::with_capacity(self.parts.len() + 1);
        let mut rest = &self.parts[..];
        match rest.first() {
            Some(TypePart::Array { dims }) => {
                parts.push(TypePart::Array {
                    dims: dims + extra as u32,
                });
                rest = &rest[1..];
            }
            _ if extra > 0 => parts.push(TypePart::Array { dims: extra as u32 }),
            _ => {}
        }
        parts.extend_from_slice(rest);
        TypeRef {
            parts: parts.into_boxed_slice(),
            annotations: None,
        }
    }

    /// Java syntax: `java.util.Map<K, @A V>[]`, with `?` for a missing type.
    pub fn display<'a>(&'a self, names: &'a Interner) -> impl fmt::Display + 'a {
        Display { ty: self, names }
    }
}

struct Display<'a> {
    ty: &'a TypeRef,
    names: &'a Interner,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ty, names) = (self.ty, self.names);
        // The parts open, each with its index and how many of its children are still to be
        // written.
        let mut open: Vec<(usize, usize)> = Vec::new();
        for (index, part) in ty.parts.iter().enumerate() {
            write_before(f, ty, index, names)?;
            if part.arity() > 0 {
                open.push((index, part.arity()));
                continue;
            }
            write_after(f, ty, index, names)?;
            // The part is finished; so is each open part whose last child it was.
            while let Some((parent, remaining)) = open.last_mut() {
                *remaining -= 1;
                if *remaining > 0 {
                    f.write_str(separator(&ty.parts[*parent]))?;
                    break;
                }
                write_after(f, ty, *parent, names)?;
                open.pop();
            }
        }
        return Ok(());

        fn write_before(
            f: &mut fmt::Formatter<'_>,
            ty: &TypeRef,
            index: usize,
            names: &Interner,
        ) -> fmt::Result {
            let part = ty.parts[index];
            if !matches!(part, TypePart::Array { .. }) {
                for annotation in ty.annotations_of(index) {
                    write!(f, "{} ", annotation.display(names))?;
                }
            }
            match part {
                TypePart::Primitive(keyword) => f.write_str(keyword.keyword()),
                TypePart::Void => f.write_str("void"),
                TypePart::Segment { name, .. } => {
                    f.write_str(&names.text(name))?;
                    match part.arguments() {
                        Some(0) => f.write_str("<>"),
                        Some(_) => f.write_str("<"),
                        None => Ok(()),
                    }
                }
                TypePart::Wildcard { bound } => f.write_str(match bound {
                    None => "?",
                    Some(WildcardBound::Extends) => "? extends ",
                    Some(WildcardBound::Super) => "? super ",
                }),
                TypePart::Missing => f.write_str("?"),
                TypePart::Class { .. }
                | TypePart::Array { .. }
                | TypePart::Intersection { .. }
                | TypePart::Union { .. } => Ok(()),
            }
        }

        fn write_after(
            f: &mut fmt::Formatter<'_>,
            ty: &TypeRef,
            index: usize,
            names: &Interner,
        ) -> fmt::Result {
            match ty.parts[index] {
                part @ TypePart::Segment { .. } if part.arguments().is_some_and(|n| n > 0) => {
                    f.write_str(">")
                }
                TypePart::Array { dims } => {
                    for dim in 0..dims {
                        let mut first = true;
                        for annotation in ty.dim_annotations(index, dim) {
                            f.write_str(if first { " " } else { "" })?;
                            write!(f, "{} ", annotation.display(names))?;
                            first = false;
                        }
                        f.write_str("[]")?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        }

        fn separator(part: &TypePart) -> &'static str {
            match part {
                TypePart::Class { .. } => ".",
                TypePart::Intersection { .. } => " & ",
                TypePart::Union { .. } => " | ",
                _ => ", ",
            }
        }
    }
}

impl fmt::Debug for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeRef")
            .field("parts", &self.parts)
            .field("annotations", &self.annotations())
            .finish()
    }
}
