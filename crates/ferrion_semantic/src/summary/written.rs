//! Types as written: the syntax of a type, owned, so a summary outlives its tree.

use std::fmt;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{SyntaxKind, SyntaxNode};

use super::build::{annotations, name_text};
use super::{Annotation, Dim};

/// A type as written, unresolved: its name segments, type arguments, array dimensions, and
/// annotations (docs/design/semantic-names.md#file-summaries).
///
/// The type is held flat, as its parts in pre-order: each part is followed by the parts of its
/// children, and says how many children it has ([`TypePart::arity`]). So `Map<K, V[]>[]` is
///
/// ```text
/// Array [] / Class 1 / Segment Map, 2 arguments / Class 1 / Segment K / Array [] / Class 1 /
/// Segment V
/// ```
///
/// Being flat, a deeply nested type (`List<List<...>>`, 10,000 deep) is built, compared, hashed,
/// printed, and dropped without recursion.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TypeRef {
    parts: Box<[TypePart]>,
}

/// One part of a [`TypeRef`].
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypePart {
    /// `int`, with its annotations (`@A int`). No children.
    Primitive {
        keyword: Primitive,
        annotations: Box<[Annotation]>,
    },
    /// `void`, as a method's result. No children.
    Void,
    /// A class or interface type: its children are `segments` [`TypePart::Segment`]s, outermost
    /// first (`java`, `util`, `Map`, `Entry` for `java.util.Map.Entry`). Which segments are
    /// packages is decided by resolution.
    Class { segments: u32 },
    /// One segment of a class type, with its annotations (`java.util.@A Map`). Its children are
    /// its type arguments: `None` without `<...>`, `Some(0)` for a diamond.
    Segment {
        name: Box<str>,
        annotations: Box<[Annotation]>,
        arguments: Option<u32>,
    },
    /// An array type's dimensions, outermost first, each with its annotations. One child, the
    /// element type.
    Array { dims: Box<[Dim]> },
    /// `?`, `? extends T`, or `? super T`, with its annotations. One child, the bound, unless
    /// `bound` is `None`.
    Wildcard {
        bound: Option<WildcardBound>,
        annotations: Box<[Annotation]>,
    },
    /// `A & B` (in a cast). Its children are the `types` types.
    Intersection { types: u32 },
    /// `A | B` (in a catch clause). Its children are the `types` types.
    Union { types: u32 },
    /// A type the parser couldn't read, or that is missing. No children.
    Missing,
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
    /// How many parts' subtrees follow this one as its children.
    pub fn arity(&self) -> usize {
        match self {
            TypePart::Primitive { .. } | TypePart::Void | TypePart::Missing => 0,
            TypePart::Class { segments } => *segments as usize,
            TypePart::Segment { arguments, .. } => arguments.unwrap_or(0) as usize,
            TypePart::Array { .. } => 1,
            TypePart::Wildcard { bound, .. } => usize::from(bound.is_some()),
            TypePart::Intersection { types } | TypePart::Union { types } => *types as usize,
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
        }
    }

    /// The type `ty` as written, or [`TypeRef::missing`] for `None`.
    pub fn from_syntax(ty: Option<ast::Type<'_>>) -> TypeRef {
        let Some(ty) = ty else {
            return TypeRef::missing();
        };
        // A stack of what is still to be written, last first: a type's node, or a part already
        // made whose children follow it on the stack.
        enum Work<'t> {
            Node(SyntaxNode<'t>),
            Part(TypePart),
        }
        let mut parts = Vec::new();
        let mut stack = vec![Work::Node(ty.syntax())];
        while let Some(work) = stack.pop() {
            let node = match work {
                Work::Part(part) => {
                    parts.push(part);
                    continue;
                }
                Work::Node(node) => node,
            };
            match node.kind() {
                SyntaxKind::PrimitiveType => {
                    let ty = ast::PrimitiveType::cast(node).unwrap();
                    let part = match ty.keyword().and_then(|k| Primitive::of(k.kind())) {
                        Some(keyword) => TypePart::Primitive {
                            keyword,
                            annotations: annotations(ty.annotations()),
                        },
                        None => TypePart::Missing,
                    };
                    parts.push(part);
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
                        let count = ty.type_arguments().map(|_| arguments.len() as u32);
                        for argument in arguments.iter().rev() {
                            stack.push(Work::Node(argument.syntax()));
                        }
                        stack.push(Work::Part(TypePart::Segment {
                            name: ty.name().map_or_else(Box::default, name_text),
                            annotations: annotations(ty.annotations()),
                            arguments: count,
                        }));
                    }
                }
                SyntaxKind::ArrayType => {
                    let ty = ast::ArrayType::cast(node).unwrap();
                    parts.push(TypePart::Array {
                        dims: ty.dims().map(Dim::from_syntax).collect(),
                    });
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
                    parts.push(TypePart::Wildcard {
                        bound,
                        annotations: annotations(wildcard.annotations()),
                    });
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
        };

        /// Pushes a child that the grammar requires, or a missing type in its place.
        fn push_child<'t>(stack: &mut Vec<Work<'t>>, child: Option<SyntaxNode<'t>>) {
            stack.push(match child {
                Some(node) => Work::Node(node),
                None => Work::Part(TypePart::Missing),
            });
        }
    }

    /// The parts in pre-order. Never empty: the first is the type's outermost part.
    pub fn parts(&self) -> &[TypePart] {
        &self.parts
    }

    pub fn is_missing(&self) -> bool {
        matches!(*self.parts, [TypePart::Missing])
    }

    /// The type with `extra` more array dimensions, outermost, and every annotation removed:
    /// what two declarations' types are compared by when only their shape matters (a record's
    /// canonical constructor, JLS 8.10.4). `int[] x[]` and `int x[][]` have the same shape.
    pub(crate) fn shape(&self, extra: usize) -> TypeRef {
        let mut parts = Vec::with_capacity(self.parts.len() + 1);
        let unannotated_dims = |n: usize| -> Box<[Dim]> {
            (0..n)
                .map(|_| Dim {
                    annotations: Box::default(),
                })
                .collect()
        };
        let mut rest = &self.parts[..];
        match rest.first() {
            Some(TypePart::Array { dims }) => {
                parts.push(TypePart::Array {
                    dims: unannotated_dims(dims.len() + extra),
                });
                rest = &rest[1..];
            }
            _ if extra > 0 => parts.push(TypePart::Array {
                dims: unannotated_dims(extra),
            }),
            _ => {}
        }
        for part in rest {
            parts.push(match part {
                TypePart::Primitive { keyword, .. } => TypePart::Primitive {
                    keyword: *keyword,
                    annotations: Box::default(),
                },
                TypePart::Segment {
                    name, arguments, ..
                } => TypePart::Segment {
                    name: name.clone(),
                    annotations: Box::default(),
                    arguments: *arguments,
                },
                TypePart::Array { dims } => TypePart::Array {
                    dims: unannotated_dims(dims.len()),
                },
                TypePart::Wildcard { bound, .. } => TypePart::Wildcard {
                    bound: *bound,
                    annotations: Box::default(),
                },
                other => other.clone(),
            });
        }
        TypeRef {
            parts: parts.into_boxed_slice(),
        }
    }
}

/// Java syntax: `java.util.Map<K, @A V>[]`, with `?` for a missing type.
impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The parts open, each with how many of its children are still to be written.
        let mut open: Vec<(&TypePart, usize)> = Vec::new();
        for part in self.parts.iter() {
            write_before(f, part)?;
            if part.arity() > 0 {
                open.push((part, part.arity()));
                continue;
            }
            write_after(f, part)?;
            // The part is finished; so is each open part whose last child it was.
            while let Some((parent, remaining)) = open.last_mut() {
                *remaining -= 1;
                if *remaining > 0 {
                    f.write_str(separator(parent))?;
                    break;
                }
                write_after(f, parent)?;
                open.pop();
            }
        }
        return Ok(());

        fn write_before(f: &mut fmt::Formatter<'_>, part: &TypePart) -> fmt::Result {
            match part {
                TypePart::Primitive {
                    keyword,
                    annotations,
                } => {
                    write_annotations(f, annotations)?;
                    f.write_str(keyword.keyword())
                }
                TypePart::Void => f.write_str("void"),
                TypePart::Segment {
                    name,
                    annotations,
                    arguments,
                } => {
                    write_annotations(f, annotations)?;
                    f.write_str(name)?;
                    match arguments {
                        Some(0) => f.write_str("<>"),
                        Some(_) => f.write_str("<"),
                        None => Ok(()),
                    }
                }
                TypePart::Wildcard { bound, annotations } => {
                    write_annotations(f, annotations)?;
                    f.write_str(match bound {
                        None => "?",
                        Some(WildcardBound::Extends) => "? extends ",
                        Some(WildcardBound::Super) => "? super ",
                    })
                }
                TypePart::Missing => f.write_str("?"),
                TypePart::Class { .. }
                | TypePart::Array { .. }
                | TypePart::Intersection { .. }
                | TypePart::Union { .. } => Ok(()),
            }
        }

        fn write_after(f: &mut fmt::Formatter<'_>, part: &TypePart) -> fmt::Result {
            match part {
                TypePart::Segment {
                    arguments: Some(n), ..
                } if *n > 0 => f.write_str(">"),
                TypePart::Array { dims } => {
                    for dim in dims {
                        if !dim.annotations.is_empty() {
                            f.write_str(" ")?;
                            write_annotations(f, &dim.annotations)?;
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

        fn write_annotations(
            f: &mut fmt::Formatter<'_>,
            annotations: &[Annotation],
        ) -> fmt::Result {
            for annotation in annotations {
                write!(f, "{annotation} ")?;
            }
            Ok(())
        }
    }
}

impl fmt::Debug for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypeRef({self})")
    }
}
