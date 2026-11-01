//! Annotations (4.7.16), parameter annotations (4.7.18), type annotations (4.7.20), and element
//! values (4.7.16.1).
//!
//! Decoding an attribute walks its whole structure once, without resolving constants, so that a
//! length that doesn't match is reported when the attribute is decoded. The iterators then
//! decode one annotation or value at a time, resolving constants as they go. Nested annotations
//! and arrays are walked with an explicit stack and a limit of [`MAX_NESTING`] levels, so a deep
//! input can't overflow the call stack.

use super::{ClassFile, Constant};
use crate::cursor::Cursor;
use crate::error::{Error, Result};
use crate::text::{JavaStr, Name};

/// How deeply annotations and arrays may nest in an element value.
pub const MAX_NESTING: usize = 64;

/// What remains to walk at one level of nesting.
enum Pending {
    /// Element values, as in an array.
    Values(u16),
    /// Element-value pairs, as in an annotation.
    Pairs(u16),
}

/// Skips `start` (values or pairs) at nesting `depth`, checking the structure.
fn skip(c: &mut Cursor<'_>, start: Pending, depth: usize) -> Result<()> {
    let mut stack = vec![start];
    while let Some(top) = stack.last_mut() {
        match top {
            Pending::Values(0) | Pending::Pairs(0) => {
                stack.pop();
                continue;
            }
            Pending::Values(n) => *n -= 1,
            Pending::Pairs(n) => {
                *n -= 1;
                c.skip(2)?;
            }
        }
        let tag = c.u8()?;
        match tag {
            b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b's' | b'c' => c.skip(2)?,
            b'e' => c.skip(4)?,
            b'@' | b'[' => {
                if depth + stack.len() > MAX_NESTING {
                    return Err(c.error(format!(
                        "annotations or arrays nest more than {MAX_NESTING} levels"
                    )));
                }
                if tag == b'@' {
                    c.skip(2)?;
                    stack.push(Pending::Pairs(c.u16()?));
                } else {
                    stack.push(Pending::Values(c.u16()?));
                }
            }
            _ => {
                return Err(Error::malformed(
                    c.offset() - 1,
                    format!("unknown element value tag {:?}", char::from(tag)),
                ));
            }
        }
    }
    Ok(())
}

/// Skips one annotation (type, pairs) at `depth`.
fn skip_annotation(c: &mut Cursor<'_>, depth: usize) -> Result<()> {
    c.skip(2)?;
    let pairs = c.u16()?;
    skip(c, Pending::Pairs(pairs), depth)
}

/// Reads `RuntimeVisibleAnnotations` or `RuntimeInvisibleAnnotations` contents.
pub(super) fn annotations<'a>(class: &'a ClassFile, c: &mut Cursor<'a>) -> Result<Annotations<'a>> {
    let count = c.u16()?;
    let start = c.clone();
    for _ in 0..count {
        skip_annotation(c, 0)?;
    }
    Ok(Annotations {
        class,
        cursor: start,
        remaining: count,
        depth: 0,
    })
}

pub(super) fn parameter_annotations<'a>(
    class: &'a ClassFile,
    c: &mut Cursor<'a>,
) -> Result<ParameterAnnotations<'a>> {
    let count = c.u8()?;
    let start = c.clone();
    for _ in 0..count {
        annotations(class, c)?;
    }
    Ok(ParameterAnnotations {
        class,
        cursor: start,
        remaining: count,
    })
}

pub(super) fn type_annotations<'a>(
    class: &'a ClassFile,
    c: &mut Cursor<'a>,
) -> Result<TypeAnnotations<'a>> {
    let count = c.u16()?;
    let start = c.clone();
    for _ in 0..count {
        target(c)?;
        path(c)?;
        skip_annotation(c, 0)?;
    }
    Ok(TypeAnnotations {
        class,
        cursor: start,
        remaining: count,
    })
}

/// Reads one element value (for `AnnotationDefault`), checking its structure first.
pub(super) fn element_value<'a>(
    class: &'a ClassFile,
    c: &mut Cursor<'a>,
) -> Result<ElementValue<'a>> {
    let start = c.clone();
    skip(c, Pending::Values(1), 0)?;
    let mut start = start;
    value(class, &mut start, 0)
}

/// Decodes the element value at the cursor, which `skip` has already checked, and moves past
/// it.
fn value<'a>(class: &'a ClassFile, c: &mut Cursor<'a>, depth: usize) -> Result<ElementValue<'a>> {
    let offset = c.offset();
    let tag = c.u8()?;
    let at = |error: Error| error.at(offset);
    Ok(match tag {
        b'B' | b'C' | b'I' | b'S' | b'Z' => {
            let value = class.integer(c.u16()?).map_err(at)?;
            match tag {
                b'B' => ElementValue::Byte(value),
                b'C' => ElementValue::Char(value),
                b'I' => ElementValue::Int(value),
                b'S' => ElementValue::Short(value),
                _ => ElementValue::Boolean(value),
            }
        }
        b'J' | b'F' | b'D' => {
            let index = c.u16()?;
            match (tag, class.constant(index).map_err(at)?) {
                (b'J', Constant::Long(value)) => ElementValue::Long(value),
                (b'F', Constant::Float(bits)) => ElementValue::Float(bits),
                (b'D', Constant::Double(bits)) => ElementValue::Double(bits),
                _ => {
                    return Err(at(class.wrong(
                        index,
                        &format!("the constant for tag {}", char::from(tag)),
                    )));
                }
            }
        }
        b's' => ElementValue::String(JavaStr(class.utf8(c.u16()?).map_err(at)?.0)),
        b'e' => ElementValue::Enum {
            type_descriptor: class.utf8(c.u16()?).map_err(at)?,
            name: class.utf8(c.u16()?).map_err(at)?,
        },
        b'c' => ElementValue::Class(class.utf8(c.u16()?).map_err(at)?),
        b'@' => ElementValue::Annotation(annotation(class, c, depth + 1)?),
        b'[' => {
            let count = c.u16()?;
            let values = ElementValues {
                class,
                cursor: c.clone(),
                remaining: count,
                depth: depth + 1,
            };
            skip(c, Pending::Values(count), depth + 1)?;
            ElementValue::Array(values)
        }
        _ => {
            return Err(Error::malformed(
                offset,
                format!("unknown element value tag {:?}", char::from(tag)),
            ));
        }
    })
}

/// Decodes the annotation at the cursor and moves past it.
fn annotation<'a>(
    class: &'a ClassFile,
    c: &mut Cursor<'a>,
    depth: usize,
) -> Result<Annotation<'a>> {
    let offset = c.offset();
    let type_index = c.u16()?;
    let count = c.u16()?;
    let pairs = ElementValuePairs {
        class,
        cursor: c.clone(),
        remaining: count,
        depth,
    };
    skip(c, Pending::Pairs(count), depth)?;
    Ok(Annotation {
        class,
        type_index,
        offset,
        pairs,
    })
}

/// An annotation (4.7.16).
#[derive(Clone)]
pub struct Annotation<'a> {
    class: &'a ClassFile,
    type_index: u16,
    offset: u64,
    pairs: ElementValuePairs<'a>,
}

impl<'a> Annotation<'a> {
    /// The annotation's type, as a field descriptor (`Ljava/lang/Deprecated;`).
    pub fn type_descriptor(&self) -> Result<Name<'a>> {
        self.class
            .utf8(self.type_index)
            .map_err(|e| e.at(self.offset))
    }

    /// The element-value pairs, decoded one at a time.
    pub fn elements(&self) -> ElementValuePairs<'a> {
        self.pairs.clone()
    }
}

impl std::fmt::Debug for Annotation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Annotation")
            .field("type", &self.type_descriptor().ok())
            .field("elements", &self.elements())
            .finish()
    }
}

/// The annotations of one attribute or parameter.
#[derive(Clone)]
pub struct Annotations<'a> {
    class: &'a ClassFile,
    cursor: Cursor<'a>,
    remaining: u16,
    depth: usize,
}

impl Annotations<'_> {
    pub fn len(&self) -> usize {
        usize::from(self.remaining)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl<'a> Iterator for Annotations<'a> {
    type Item = Result<Annotation<'a>>;

    fn next(&mut self) -> Option<Result<Annotation<'a>>> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let result = annotation(self.class, &mut self.cursor, self.depth);
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

impl std::fmt::Debug for Annotations<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// An annotation's element-value pairs.
#[derive(Clone)]
pub struct ElementValuePairs<'a> {
    class: &'a ClassFile,
    cursor: Cursor<'a>,
    remaining: u16,
    depth: usize,
}

impl ElementValuePairs<'_> {
    pub fn len(&self) -> usize {
        usize::from(self.remaining)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl<'a> Iterator for ElementValuePairs<'a> {
    type Item = Result<(Name<'a>, ElementValue<'a>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let c = &mut self.cursor;
        let result = (|| {
            let offset = c.offset();
            let name = self.class.utf8(c.u16()?).map_err(|e| e.at(offset))?;
            Ok((name, value(self.class, c, self.depth)?))
        })();
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

impl std::fmt::Debug for ElementValuePairs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// An element value (4.7.16.1). The integral kinds keep the `Integer` constant as stored.
#[derive(Clone, Debug)]
pub enum ElementValue<'a> {
    Byte(i32),
    Char(i32),
    Int(i32),
    Short(i32),
    Boolean(i32),
    Long(i64),
    /// The bits, so NaN payloads survive.
    Float(u32),
    Double(u64),
    String(JavaStr<'a>),
    /// An enum constant: the enum's type as a field descriptor, and the constant's name.
    Enum {
        type_descriptor: Name<'a>,
        name: Name<'a>,
    },
    /// A class literal, as a return descriptor (`Ljava/util/List;`, `I`, or `V`).
    Class(Name<'a>),
    Annotation(Annotation<'a>),
    Array(ElementValues<'a>),
}

/// The values of an array element value.
#[derive(Clone)]
pub struct ElementValues<'a> {
    class: &'a ClassFile,
    cursor: Cursor<'a>,
    remaining: u16,
    depth: usize,
}

impl ElementValues<'_> {
    pub fn len(&self) -> usize {
        usize::from(self.remaining)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl<'a> Iterator for ElementValues<'a> {
    type Item = Result<ElementValue<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let result = value(self.class, &mut self.cursor, self.depth);
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

impl std::fmt::Debug for ElementValues<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// The annotations of each parameter, in the attribute's own numbering (4.7.18), which may
/// differ from the descriptor's.
#[derive(Clone)]
pub struct ParameterAnnotations<'a> {
    class: &'a ClassFile,
    cursor: Cursor<'a>,
    remaining: u8,
}

impl ParameterAnnotations<'_> {
    pub fn len(&self) -> usize {
        usize::from(self.remaining)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl<'a> Iterator for ParameterAnnotations<'a> {
    type Item = Result<Annotations<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let result = annotations(self.class, &mut self.cursor);
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

impl std::fmt::Debug for ParameterAnnotations<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// Where a type annotation applies (4.7.20.1), for the targets that appear outside `Code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeAnnotationTarget {
    /// 0x00: a type parameter of a generic class or interface.
    ClassTypeParameter { index: u8 },
    /// 0x01: a type parameter of a generic method or constructor.
    MethodTypeParameter { index: u8 },
    /// 0x10: a supertype. Index 65,535 is the superclass; otherwise an index into the
    /// interfaces.
    Supertype { index: u16 },
    /// 0x11: a bound of a class type parameter.
    ClassTypeParameterBound { type_parameter: u8, bound: u8 },
    /// 0x12: a bound of a method type parameter.
    MethodTypeParameterBound { type_parameter: u8, bound: u8 },
    /// 0x13: a field's or record component's type.
    Field,
    /// 0x14: a method's return type, or the type of a newly constructed object.
    Return,
    /// 0x15: a method's receiver type.
    Receiver,
    /// 0x16: a formal parameter's type.
    FormalParameter { index: u8 },
    /// 0x17: a type in a `throws` clause, by index into `Exceptions`.
    Throws { index: u16 },
}

impl TypeAnnotationTarget {
    /// The `target_type` value.
    pub fn target_type(&self) -> u8 {
        match self {
            TypeAnnotationTarget::ClassTypeParameter { .. } => 0x00,
            TypeAnnotationTarget::MethodTypeParameter { .. } => 0x01,
            TypeAnnotationTarget::Supertype { .. } => 0x10,
            TypeAnnotationTarget::ClassTypeParameterBound { .. } => 0x11,
            TypeAnnotationTarget::MethodTypeParameterBound { .. } => 0x12,
            TypeAnnotationTarget::Field => 0x13,
            TypeAnnotationTarget::Return => 0x14,
            TypeAnnotationTarget::Receiver => 0x15,
            TypeAnnotationTarget::FormalParameter { .. } => 0x16,
            TypeAnnotationTarget::Throws { .. } => 0x17,
        }
    }
}

fn target(c: &mut Cursor<'_>) -> Result<TypeAnnotationTarget> {
    let offset = c.offset();
    Ok(match c.u8()? {
        0x00 => TypeAnnotationTarget::ClassTypeParameter { index: c.u8()? },
        0x01 => TypeAnnotationTarget::MethodTypeParameter { index: c.u8()? },
        0x10 => TypeAnnotationTarget::Supertype { index: c.u16()? },
        0x11 => TypeAnnotationTarget::ClassTypeParameterBound {
            type_parameter: c.u8()?,
            bound: c.u8()?,
        },
        0x12 => TypeAnnotationTarget::MethodTypeParameterBound {
            type_parameter: c.u8()?,
            bound: c.u8()?,
        },
        0x13 => TypeAnnotationTarget::Field,
        0x14 => TypeAnnotationTarget::Return,
        0x15 => TypeAnnotationTarget::Receiver,
        0x16 => TypeAnnotationTarget::FormalParameter { index: c.u8()? },
        0x17 => TypeAnnotationTarget::Throws { index: c.u16()? },
        target @ 0x40..=0x4b => {
            return Err(Error::malformed(
                offset,
                format!("type annotation target {target:#04x} appears only inside Code"),
            ));
        }
        target => {
            return Err(Error::malformed(
                offset,
                format!("unknown type annotation target {target:#04x}"),
            ));
        }
    })
}

/// One step of a type annotation's path (4.7.20.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypePathStep {
    /// Deeper in an array type.
    Array,
    /// Deeper in a nested type.
    Nested,
    /// On the bound of a wildcard type argument.
    Wildcard,
    /// On a type argument of a parameterized type.
    TypeArgument(u8),
}

fn path(c: &mut Cursor<'_>) -> Result<Vec<TypePathStep>> {
    let len = c.u8()?;
    let mut steps = Vec::with_capacity(usize::from(len));
    for _ in 0..len {
        let offset = c.offset();
        let kind = c.u8()?;
        let index = c.u8()?;
        steps.push(match (kind, index) {
            (0, 0) => TypePathStep::Array,
            (1, 0) => TypePathStep::Nested,
            (2, 0) => TypePathStep::Wildcard,
            (3, index) => TypePathStep::TypeArgument(index),
            (0..=2, index) => {
                return Err(Error::malformed(
                    offset,
                    format!("type path kind {kind} has type argument index {index}, not 0"),
                ));
            }
            _ => {
                return Err(Error::malformed(
                    offset,
                    format!("unknown type path kind {kind}"),
                ));
            }
        });
    }
    Ok(steps)
}

/// A type annotation (4.7.20).
#[derive(Clone, Debug)]
pub struct TypeAnnotation<'a> {
    pub target: TypeAnnotationTarget,
    /// Applied by B3 to the type it walks, once that's parsed.
    pub path: Vec<TypePathStep>,
    pub annotation: Annotation<'a>,
}

/// The type annotations of one attribute.
#[derive(Clone)]
pub struct TypeAnnotations<'a> {
    class: &'a ClassFile,
    cursor: Cursor<'a>,
    remaining: u16,
}

impl TypeAnnotations<'_> {
    pub fn len(&self) -> usize {
        usize::from(self.remaining)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl<'a> Iterator for TypeAnnotations<'a> {
    type Item = Result<TypeAnnotation<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let c = &mut self.cursor;
        let result = (|| {
            let target = target(c)?;
            let path = path(c)?;
            let annotation = annotation(self.class, c, 0)?;
            Ok(TypeAnnotation {
                target,
                path,
                annotation,
            })
        })();
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

impl std::fmt::Debug for TypeAnnotations<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}
