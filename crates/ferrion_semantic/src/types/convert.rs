//! Conversions and contexts (JLS 5): which conversions take a value of one type to another in
//! each context, returned as the steps taken, so that attribution and rules can see an unboxing
//! or an unchecked conversion happen.

use super::relations::primitive_subtype;
use super::{R, Tri, Type, TypeArg, TypeContext, TypeId, Why};
use crate::Cancelled;
use crate::constants::Constant;
use crate::summary::{Primitive, WildcardBound};
use crate::symbols::TypeKey;

/// One conversion (JLS 5.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Conversion {
    Identity,
    WideningPrimitive,
    NarrowingPrimitive,
    /// `byte` to `char` (JLS 5.1.4).
    WideningNarrowingPrimitive,
    WideningReference,
    /// Allowed only in casts.
    NarrowingReference,
    Boxing,
    Unboxing,
    Unchecked,
    String,
}

/// The conversions a context applied, in order: at most three.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Steps {
    steps: [Conversion; 3],
    len: u8,
}

impl Steps {
    fn of(steps: &[Conversion]) -> Steps {
        let mut out = Steps {
            steps: [Conversion::Identity; 3],
            len: 0,
        };
        for &s in steps.iter().take(3) {
            out.steps[out.len as usize] = s;
            out.len += 1;
        }
        out
    }

    pub fn as_slice(&self) -> &[Conversion] {
        &self.steps[..self.len as usize]
    }
}

/// A conversion context (JLS 5.2 to 5.6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Context {
    Assignment,
    StrictInvocation,
    LooseInvocation,
    /// String conversion, for `+` (JLS 5.4).
    String,
    Casting,
    /// Numeric promotion to the given type (JLS 5.6).
    Numeric,
}

const BOXES: [(Primitive, &str); 8] = [
    (Primitive::Boolean, "java/lang/Boolean"),
    (Primitive::Byte, "java/lang/Byte"),
    (Primitive::Char, "java/lang/Character"),
    (Primitive::Short, "java/lang/Short"),
    (Primitive::Int, "java/lang/Integer"),
    (Primitive::Long, "java/lang/Long"),
    (Primitive::Float, "java/lang/Float"),
    (Primitive::Double, "java/lang/Double"),
];

fn yes(steps: &[Conversion]) -> Tri<Steps> {
    Tri::Yes(Steps::of(steps))
}

/// Whether an `int` constant's value fits `p` (JLS 5.2).
fn fits(value: i32, p: Primitive) -> bool {
    match p {
        Primitive::Byte => i8::try_from(value).is_ok(),
        Primitive::Short => i16::try_from(value).is_ok(),
        Primitive::Char => u16::try_from(value).is_ok(),
        _ => false,
    }
}

impl TypeContext<'_> {
    /// The box class of a primitive type (JLS 5.1.7).
    pub fn boxed(&mut self, p: Primitive) -> Result<TypeId, Cancelled> {
        self.top(|cx| cx.arena.unknown(Why::Limit), |cx| cx.box_(p))
    }

    fn box_(&mut self, p: Primitive) -> R<TypeId> {
        let binary = BOXES
            .iter()
            .find(|(q, _)| *q == p)
            .map(|b| b.1)
            .unwrap_or("");
        self.lang_class(binary)
    }

    /// The primitive type a box class unboxes to (JLS 5.1.8), for one of the eight classes.
    fn box_class(&mut self, key: TypeKey) -> R<Option<Primitive>> {
        for (p, binary) in BOXES {
            if self.lang_key(binary)? == Some(key) {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// What a type unboxes to: a box class directly, or a type variable, capture variable, or
    /// intersection through a box class among its supertypes (a widening reference conversion
    /// first). `No` if it doesn't unbox.
    pub(crate) fn unboxed(&mut self, t: TypeId) -> R<Tri<(Primitive, bool)>> {
        match self.arena.get(t) {
            Type::Class { key, .. } => Ok(self
                .box_class(key)?
                .map_or(Tri::No, |p| Tri::Yes((p, false)))),
            Type::Var(..) | Type::Capture(_) | Type::Intersection(_) => {
                for (p, binary) in BOXES {
                    let Some(key) = self.lang_key(binary)? else {
                        continue;
                    };
                    match self.as_super_(t, key)? {
                        Tri::Yes(_) => return Ok(Tri::Yes((p, true))),
                        Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
                        Tri::No => {}
                    }
                }
                Ok(Tri::No)
            }
            Type::Error(e) => Ok(Tri::Unknown(Why::Error(e))),
            Type::Unknown(why) => Ok(Tri::Unknown(why)),
            _ => Ok(Tri::No),
        }
    }

    /// The conversions that take a value of type `from` to `to` in `context`, in the order
    /// applied; `constant` is the value of `from`'s expression if it is a constant expression,
    /// whose value assignment narrows to `byte`, `short`, or `char` when it fits (JLS 5.2).
    pub fn convert(
        &mut self,
        context: Context,
        from: TypeId,
        to: TypeId,
        constant: Option<&Constant>,
    ) -> Result<Tri<Steps>, Cancelled> {
        let constant = constant.and_then(Constant::int_value);
        self.top(
            |_| Tri::Unknown(Why::Limit),
            |cx| cx.convert_(context, from, to, constant),
        )
    }

    pub(crate) fn convert_(
        &mut self,
        context: Context,
        from: TypeId,
        to: TypeId,
        constant: Option<i32>,
    ) -> R<Tri<Steps>> {
        use Conversion::*;
        if let Some(why) = self.arena.non_answer(from).or(self.arena.non_answer(to)) {
            return Ok(Tri::Unknown(why));
        }
        if context == Context::String {
            let string = self.lang_key("java/lang/String")?;
            return Ok(match self.arena.get(to) {
                Type::Class { key, .. } if Some(key) == string => {
                    if self.arena.get(from) == Type::Void {
                        Tri::No
                    } else if from == to {
                        yes(&[Identity])
                    } else {
                        yes(&[String])
                    }
                }
                _ => Tri::No,
            });
        }
        if from == to {
            return Ok(yes(&[Identity]));
        }
        let (tf, tt) = (self.arena.get(from), self.arena.get(to));
        if matches!(tf, Type::Void) || matches!(tt, Type::Void) {
            return Ok(Tri::No);
        }
        let loose = matches!(
            context,
            Context::Assignment | Context::LooseInvocation | Context::Casting | Context::Numeric
        );
        // Primitive to primitive.
        if let (Type::Primitive(a), Type::Primitive(b)) = (tf, tt) {
            if primitive_subtype(a, b) {
                return Ok(yes(&[WideningPrimitive]));
            }
            if context == Context::Assignment
                && let Some(value) = constant
                && matches!(
                    a,
                    Primitive::Int | Primitive::Short | Primitive::Char | Primitive::Byte
                )
                && fits(value, b)
            {
                return Ok(yes(&[NarrowingPrimitive]));
            }
            if context == Context::Casting && a != Primitive::Boolean && b != Primitive::Boolean {
                if a == Primitive::Byte && b == Primitive::Char {
                    return Ok(yes(&[WideningNarrowingPrimitive]));
                }
                return Ok(yes(&[NarrowingPrimitive]));
            }
            return Ok(Tri::No);
        }
        // Primitive to reference: boxing, then a widening reference conversion; in assignment, a
        // constant narrowed to `Byte`, `Short`, or `Character` first.
        if let Type::Primitive(a) = tf {
            if !loose || context == Context::Numeric {
                return Ok(Tri::No);
            }
            let boxed = self.box_(a)?;
            if boxed == to {
                return Ok(yes(&[Boxing]));
            }
            if context == Context::Assignment
                && let Some(value) = constant
                && let Type::Class { key, .. } = tt
                && let Some(b) = self.box_class(key)?
                && matches!(
                    a,
                    Primitive::Int | Primitive::Short | Primitive::Char | Primitive::Byte
                )
                && matches!(b, Primitive::Byte | Primitive::Short | Primitive::Char)
                && fits(value, b)
            {
                return Ok(yes(&[NarrowingPrimitive, Boxing]));
            }
            return Ok(match self.subtype_(boxed, to)? {
                Tri::Yes(()) => yes(&[Boxing, WideningReference]),
                Tri::No => Tri::No,
                Tri::Unknown(why) => Tri::Unknown(why),
            });
        }
        // Reference to primitive: unboxing (after a widening reference conversion for a type
        // variable), then a widening primitive conversion; in a cast, a narrowing reference
        // conversion to the box first.
        if let Type::Primitive(b) = tt {
            if !loose {
                return Ok(Tri::No);
            }
            match self.unboxed(from)? {
                Tri::Yes((a, widened)) => {
                    let mut steps = Vec::with_capacity(3);
                    if widened {
                        steps.push(WideningReference);
                    }
                    steps.push(Unboxing);
                    if a == b {
                        return Ok(yes(&steps));
                    }
                    if primitive_subtype(a, b) {
                        steps.push(WideningPrimitive);
                        return Ok(yes(&steps));
                    }
                    return Ok(Tri::No);
                }
                Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
                Tri::No => {}
            }
            if context == Context::Casting {
                // `(int) object`: to `Integer`, checked, then unboxed (JLS 5.5).
                let boxed = self.box_(b)?;
                return Ok(match self.subtype_(boxed, from)? {
                    Tri::Yes(()) => yes(&[NarrowingReference, Unboxing]),
                    Tri::No => Tri::No,
                    Tri::Unknown(why) => Tri::Unknown(why),
                });
            }
            return Ok(Tri::No);
        }
        if context == Context::Numeric {
            return Ok(Tri::No);
        }
        // Reference to reference.
        match self.subtype_(from, to)? {
            Tri::Yes(()) => return Ok(yes(&[WideningReference])),
            Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
            Tri::No => {}
        }
        // Unchecked conversion (JLS 5.1.9): a raw type's class a subclass of a parameterized
        // type's, after a widening reference conversion.
        if let Type::Class { key, .. } = tt
            && self.has_args(to)
        {
            match self.as_super_(from, key)? {
                Tri::Yes(sup) if self.is_raw(sup)? => {
                    return Ok(if sup == self.erasure_(to)? && from == sup {
                        yes(&[Unchecked])
                    } else {
                        yes(&[WideningReference, Unchecked])
                    });
                }
                Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
                _ => {}
            }
        }
        if context == Context::Casting {
            return self.reference_cast(from, to);
        }
        Ok(Tri::No)
    }

    /// A cast between reference types that isn't a widening one (JLS 5.5): a narrowing
    /// reference conversion, unchecked where the target's arguments aren't known from the
    /// source's; `No` for provably distinct types, and for two classes neither of which is
    /// below the other; `Unknown` where legality needs more than typing does.
    fn reference_cast(&mut self, from: TypeId, to: TypeId) -> R<Tri<Steps>> {
        use Conversion::*;
        let (tf, tt) = (self.arena.get(from), self.arena.get(to));
        match (tf, tt) {
            (Type::Class { key: a, .. }, Type::Class { key: b, .. }) => {
                let up = self.as_super_(to, a)?;
                let down = self.as_super_(from, b)?;
                let interface_a = self.decl(a)?.is_some_and(|d| d.is_interface());
                let interface_b = self.decl(b)?.is_some_and(|d| d.is_interface());
                match (up, down) {
                    (Tri::Yes(sup), _) => {
                        // A downcast: checked if `to`'s arguments follow from `from`'s.
                        if self.provably_distinct(sup, from)? {
                            return Ok(Tri::No);
                        }
                        let checked = !self.has_args(to)
                            || self.all_unbounded(to)
                            || self.same_(sup, from)?.is_yes();
                        Ok(if checked {
                            yes(&[NarrowingReference])
                        } else {
                            yes(&[NarrowingReference, Unchecked])
                        })
                    }
                    (Tri::Unknown(why), _) | (_, Tri::Unknown(why)) => Ok(Tri::Unknown(why)),
                    (Tri::No, _) if !interface_a && !interface_b => Ok(Tri::No),
                    // Between an interface and a type not below it: legal unless a final class
                    // stands in the way, which typing doesn't need to decide.
                    (Tri::No, _) => Ok(Tri::Unknown(Why::Later)),
                }
            }
            (Type::Array(a), Type::Array(b)) => {
                if self.arena.is_reference(a) && self.arena.is_reference(b) {
                    self.reference_cast(a, b)
                } else {
                    Ok(Tri::No)
                }
            }
            (Type::Class { .. }, Type::Array(_)) => {
                // `Object`, `Cloneable`, and `Serializable` cast to arrays.
                Ok(match self.subtype_(to, from)? {
                    Tri::Yes(()) => yes(&[NarrowingReference]),
                    Tri::No => Tri::No,
                    Tri::Unknown(why) => Tri::Unknown(why),
                })
            }
            (Type::Var(..) | Type::Capture(_) | Type::Intersection(_), _)
            | (_, Type::Var(..) | Type::Capture(_) | Type::Intersection(_)) => {
                Ok(yes(&[NarrowingReference, Unchecked]))
            }
            _ => Ok(Tri::No),
        }
    }

    fn all_unbounded(&self, t: TypeId) -> bool {
        self.arena
            .class_args(t)
            .iter()
            .all(|a| matches!(a, TypeArg::Wildcard(None)))
    }

    /// Whether two parameterizations of one class are provably distinct (JLS 4.5): some pair of
    /// arguments is, where neither is a type variable or wildcard and they aren't the same, or
    /// one is a wildcard or type variable whose bound's erasure and the other's aren't related.
    pub(crate) fn provably_distinct(&mut self, a: TypeId, b: TypeId) -> R<bool> {
        let (
            Type::Class {
                key: ka,
                args: Some(la),
                ..
            },
            Type::Class {
                key: kb,
                args: Some(lb),
                ..
            },
        ) = (self.arena.get(a), self.arena.get(b))
        else {
            return Ok(false);
        };
        if ka != kb {
            return Ok(false);
        }
        let (la, lb) = (self.arena.args(la).to_vec(), self.arena.args(lb).to_vec());
        for (x, y) in la.into_iter().zip(lb) {
            if self.arguments_distinct(x, y)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn arguments_distinct(&mut self, x: TypeArg, y: TypeArg) -> R<bool> {
        let bound = |cx: &mut Self, a: TypeArg| -> R<Option<TypeId>> {
            Ok(match a {
                TypeArg::Type(t) => match cx.arena.get(t) {
                    Type::Var(..) | Type::Capture(_) => Some(cx.upper(t)?),
                    _ => None,
                },
                TypeArg::Wildcard(Some((WildcardBound::Extends, b))) => Some(b),
                TypeArg::Wildcard(_) => Some(cx.object_or_error()?),
            })
        };
        let plain = |cx: &Self, a: TypeArg| match a {
            TypeArg::Type(t) => !matches!(cx.arena.get(t), Type::Var(..) | Type::Capture(_)),
            TypeArg::Wildcard(_) => false,
        };
        if plain(self, x) && plain(self, y) {
            let (TypeArg::Type(a), TypeArg::Type(b)) = (x, y) else {
                return Ok(false);
            };
            return Ok(self.same_(a, b)?.is_no());
        }
        let ex = match bound(self, x)? {
            Some(b) => b,
            None => match x {
                TypeArg::Type(t) => t,
                TypeArg::Wildcard(_) => return Ok(false),
            },
        };
        let ey = match bound(self, y)? {
            Some(b) => b,
            None => match y {
                TypeArg::Type(t) => t,
                TypeArg::Wildcard(_) => return Ok(false),
            },
        };
        let (ex, ey) = (self.erasure_(ex)?, self.erasure_(ey)?);
        Ok(self.subtype_(ex, ey)?.is_no() && self.subtype_(ey, ex)?.is_no())
    }

    /// Unary numeric promotion (JLS 5.6): unboxed, then `byte`, `short`, and `char` widened to
    /// `int`.
    pub fn unary_promotion(&mut self, t: TypeId) -> Result<Tri<TypeId>, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.promote(&[t]))
    }

    /// Binary numeric promotion (JLS 5.6): `double`, `float`, `long`, or else `int`.
    pub fn binary_promotion(&mut self, a: TypeId, b: TypeId) -> Result<Tri<TypeId>, Cancelled> {
        self.top(|_| Tri::Unknown(Why::Limit), |cx| cx.promote(&[a, b]))
    }

    fn promote(&mut self, operands: &[TypeId]) -> R<Tri<TypeId>> {
        let mut widest = Primitive::Int;
        for &t in operands {
            let p = match self.arena.get(t) {
                Type::Primitive(p) => p,
                _ => match self.unboxed(t)? {
                    Tri::Yes((p, _)) => p,
                    Tri::No => return Ok(Tri::No),
                    Tri::Unknown(why) => return Ok(Tri::Unknown(why)),
                },
            };
            let rank = |p| match p {
                Primitive::Double => 3,
                Primitive::Float => 2,
                Primitive::Long => 1,
                _ => 0,
            };
            if p == Primitive::Boolean {
                return Ok(Tri::No);
            }
            if rank(p) > rank(widest) {
                widest = p;
            }
        }
        Ok(Tri::Yes(self.arena.primitive(widest)))
    }
}
