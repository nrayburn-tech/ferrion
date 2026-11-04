//! Constant values and Java's arithmetic on them (JLS 15.29, 4.2, 5.1, 5.1.11): `int` and `long`
//! wrap, integer division by zero isn't a constant, `float` and `double` are IEEE 754 binary32
//! and binary64 with round to nearest, narrowing follows 5.1.3, and string conversion of a
//! floating-point value is `Double.toString`'s (JDK 19 and later, docs/design/semantic-attribution.md#constants).

use std::fmt::Write as _;

use super::string::JavaString;
use crate::summary::Primitive;

/// A constant expression's value, typed as the expression is.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Constant {
    Boolean(bool),
    Char(u16),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    /// The bits, so that equal values are equal and NaN is one value.
    Float(u32),
    Double(u64),
    /// UTF-16 code units, as a Java string holds them: lone surrogates are kept.
    String(JavaString),
}

/// A binary operator a constant expression can have (JLS 15.29).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BinaryOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    UShr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Xor,
    Or,
    AndAnd,
    OrOr,
}

/// A unary operator a constant expression can have.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    Complement,
}

impl Constant {
    pub fn float(value: f32) -> Constant {
        Constant::Float(canonical_f32(value).to_bits())
    }

    pub fn double(value: f64) -> Constant {
        Constant::Double(canonical_f64(value).to_bits())
    }

    pub fn string(units: Vec<u16>) -> Constant {
        Constant::String(JavaString::from_units(units))
    }

    pub fn str(text: &str) -> Constant {
        Constant::string(text.encode_utf16().collect())
    }

    /// The string conversion of the value (JLS 5.1.11).
    fn java_string(&self) -> JavaString {
        match self {
            Constant::String(s) => s.clone(),
            other => JavaString::from_units(other.to_java_string()),
        }
    }

    /// The primitive type of the value, `None` for a string.
    pub fn primitive(&self) -> Option<Primitive> {
        Some(match self {
            Constant::Boolean(_) => Primitive::Boolean,
            Constant::Char(_) => Primitive::Char,
            Constant::Byte(_) => Primitive::Byte,
            Constant::Short(_) => Primitive::Short,
            Constant::Int(_) => Primitive::Int,
            Constant::Long(_) => Primitive::Long,
            Constant::Float(_) => Primitive::Float,
            Constant::Double(_) => Primitive::Double,
            Constant::String(_) => return None,
        })
    }

    /// The value of an `int`, `short`, `char`, or `byte` constant, which assignment narrows when
    /// it fits (JLS 5.2).
    pub fn int_value(&self) -> Option<i32> {
        Some(match *self {
            Constant::Char(c) => i32::from(c),
            Constant::Byte(b) => i32::from(b),
            Constant::Short(s) => i32::from(s),
            Constant::Int(i) => i,
            _ => return None,
        })
    }

    fn as_long(&self) -> Option<i64> {
        Some(match *self {
            Constant::Long(l) => l,
            Constant::Float(_) | Constant::Double(_) => return None,
            _ => i64::from(self.int_value()?),
        })
    }

    fn as_f32(&self) -> Option<f32> {
        Some(match *self {
            Constant::Float(bits) => f32::from_bits(bits),
            Constant::Double(bits) => f64::from_bits(bits) as f32,
            Constant::Long(l) => l as f32,
            _ => self.int_value()? as f32,
        })
    }

    fn as_f64(&self) -> Option<f64> {
        Some(match *self {
            Constant::Float(bits) => f64::from(f32::from_bits(bits)),
            Constant::Double(bits) => f64::from_bits(bits),
            Constant::Long(l) => l as f64,
            _ => f64::from(self.int_value()?),
        })
    }

    /// The value converted to `to` by a primitive conversion (JLS 5.1.2, 5.1.3, 5.1.4), as a
    /// cast or a numeric promotion converts it; `None` for a string or a boolean to a number.
    pub fn convert(&self, to: Primitive) -> Option<Constant> {
        if let Constant::Boolean(b) = *self {
            return (to == Primitive::Boolean).then_some(Constant::Boolean(b));
        }
        if matches!(self, Constant::String(_)) || to == Primitive::Boolean {
            return None;
        }
        // Narrowing a floating-point value to `byte`, `short`, or `char` goes through `int`
        // (JLS 5.1.3), and Rust's `as` saturates as Java's conversion to `int` and `long` does,
        // NaN to zero.
        let int = || -> Option<i32> {
            Some(match *self {
                Constant::Float(bits) => f32::from_bits(bits) as i32,
                Constant::Double(bits) => f64::from_bits(bits) as i32,
                Constant::Long(l) => l as i32,
                _ => self.int_value()?,
            })
        };
        Some(match to {
            Primitive::Byte => Constant::Byte(int()? as i8),
            Primitive::Short => Constant::Short(int()? as i16),
            Primitive::Char => Constant::Char(int()? as u16),
            Primitive::Int => Constant::Int(int()?),
            Primitive::Long => Constant::Long(match *self {
                Constant::Float(bits) => f32::from_bits(bits) as i64,
                Constant::Double(bits) => f64::from_bits(bits) as i64,
                _ => self.as_long()?,
            }),
            Primitive::Float => Constant::float(self.as_f32()?),
            Primitive::Double => Constant::double(self.as_f64()?),
            Primitive::Boolean => unreachable!("handled above"),
        })
    }

    /// The string conversion of the value (JLS 5.1.11), as UTF-16.
    pub fn to_java_string(&self) -> Vec<u16> {
        let text = match *self {
            Constant::String(ref s) => return s.to_vec(),
            Constant::Char(c) => return vec![c],
            Constant::Boolean(b) => b.to_string(),
            Constant::Byte(v) => v.to_string(),
            Constant::Short(v) => v.to_string(),
            Constant::Int(v) => v.to_string(),
            Constant::Long(v) => v.to_string(),
            Constant::Float(bits) => float_to_string(f32::from_bits(bits)),
            Constant::Double(bits) => double_to_string(f64::from_bits(bits)),
        };
        text.encode_utf16().collect()
    }

    /// The value as the types oracle writes it (docs/design/types-oracle-normalization.md#constant-values):
    /// `int 32`, `char "A"`, `float 0x1.555556p-2`, `String "a\u0009"`.
    pub fn oracle_text(&self) -> String {
        match *self {
            Constant::Boolean(b) => format!("boolean {b}"),
            Constant::Char(c) => format!("char {}", quoted(&[c])),
            Constant::Byte(v) => format!("byte {v}"),
            Constant::Short(v) => format!("short {v}"),
            Constant::Int(v) => format!("int {v}"),
            Constant::Long(v) => format!("long {v}"),
            Constant::Float(bits) => format!("float {}", float_hex(f32::from_bits(bits))),
            Constant::Double(bits) => format!("double {}", double_hex(f64::from_bits(bits))),
            Constant::String(ref s) => format!("String {}", quoted(&s.to_vec())),
        }
    }
}

/// NaN as Java's canonical NaN, so that every NaN constant is one value.
fn canonical_f32(value: f32) -> f32 {
    if value.is_nan() { f32::NAN } else { value }
}

fn canonical_f64(value: f64) -> f64 {
    if value.is_nan() { f64::NAN } else { value }
}

/// UTF-16 text quoted and escaped as the oracle writes it: printable ASCII as it is, but `"` and
/// `\` escaped, and every other code unit as `\u` and four lowercase hexadecimal digits.
pub(crate) fn quoted(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len() + 2);
    out.push('"');
    for &u in units {
        match u {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e => out.push(u as u8 as char),
            _ => {
                let _ = write!(out, "\\u{u:04x}");
            }
        }
    }
    out.push('"');
    out
}

/// Unary numeric promotion's type for a constant (JLS 5.6).
fn promoted(p: Primitive) -> Primitive {
    match p {
        Primitive::Byte | Primitive::Short | Primitive::Char => Primitive::Int,
        p => p,
    }
}

/// Binary numeric promotion's type (JLS 5.6).
fn widest(a: Primitive, b: Primitive) -> Option<Primitive> {
    use Primitive::*;
    if a == Boolean || b == Boolean {
        return None;
    }
    Some(if a == Double || b == Double {
        Double
    } else if a == Float || b == Float {
        Float
    } else if a == Long || b == Long {
        Long
    } else {
        Int
    })
}

/// Folds a unary operator, or `None` if the result isn't a constant.
pub fn unary(op: UnaryOp, value: &Constant) -> Option<Constant> {
    let p = value.primitive()?;
    if op == UnaryOp::Not {
        return match *value {
            Constant::Boolean(b) => Some(Constant::Boolean(!b)),
            _ => None,
        };
    }
    if p == Primitive::Boolean {
        return None;
    }
    let v = value.convert(promoted(p))?;
    Some(match (op, v) {
        (UnaryOp::Plus, v) => v,
        (UnaryOp::Minus, Constant::Int(i)) => Constant::Int(i.wrapping_neg()),
        (UnaryOp::Minus, Constant::Long(l)) => Constant::Long(l.wrapping_neg()),
        (UnaryOp::Minus, Constant::Float(bits)) => Constant::float(-f32::from_bits(bits)),
        (UnaryOp::Minus, Constant::Double(bits)) => Constant::double(-f64::from_bits(bits)),
        (UnaryOp::Complement, Constant::Int(i)) => Constant::Int(!i),
        (UnaryOp::Complement, Constant::Long(l)) => Constant::Long(!l),
        _ => return None,
    })
}

/// Folds a binary operator, or `None` if the result isn't a constant (integer division by zero,
/// an operand of a type the operator doesn't take). `+` with a string operand concatenates.
pub fn binary(op: BinaryOp, a: &Constant, b: &Constant) -> Option<Constant> {
    use BinaryOp::*;
    let string = matches!(a, Constant::String(_)) || matches!(b, Constant::String(_));
    if string {
        return match op {
            Add => Some(Constant::String(a.java_string().concat(&b.java_string()))),
            // Constant strings are interned, so `==` compares their values (observation 11).
            Eq | Ne => match (a, b) {
                (Constant::String(x), Constant::String(y)) => {
                    Some(Constant::Boolean((x == y) == (op == Eq)))
                }
                _ => None,
            },
            _ => None,
        };
    }
    let (pa, pb) = (a.primitive()?, b.primitive()?);
    if let (Constant::Boolean(x), Constant::Boolean(y)) = (a, b) {
        let (x, y) = (*x, *y);
        return Some(Constant::Boolean(match op {
            Eq => x == y,
            Ne => x != y,
            And | AndAnd => x & y,
            Or | OrOr => x | y,
            Xor => x ^ y,
            _ => return None,
        }));
    }
    if matches!(op, Shl | Shr | UShr) {
        let left = a.convert(promoted(pa))?;
        let distance = b.convert(promoted(pb))?.as_long()?;
        return Some(match left {
            Constant::Int(i) => {
                let s = (distance & 31) as u32;
                Constant::Int(match op {
                    Shl => i.wrapping_shl(s),
                    Shr => i.wrapping_shr(s),
                    _ => ((i as u32) >> s) as i32,
                })
            }
            Constant::Long(l) => {
                let s = (distance & 63) as u32;
                Constant::Long(match op {
                    Shl => l.wrapping_shl(s),
                    Shr => l.wrapping_shr(s),
                    _ => ((l as u64) >> s) as i64,
                })
            }
            _ => return None,
        });
    }
    if matches!(op, AndAnd | OrOr) {
        return None;
    }
    let t = widest(pa, pb)?;
    let (x, y) = (a.convert(t)?, b.convert(t)?);
    Some(match (x, y) {
        (Constant::Int(x), Constant::Int(y)) => match op {
            Mul => Constant::Int(x.wrapping_mul(y)),
            Div if y == 0 => return None,
            Div => Constant::Int(x.wrapping_div(y)),
            Rem if y == 0 => return None,
            Rem => Constant::Int(x.wrapping_rem(y)),
            Add => Constant::Int(x.wrapping_add(y)),
            Sub => Constant::Int(x.wrapping_sub(y)),
            And => Constant::Int(x & y),
            Or => Constant::Int(x | y),
            Xor => Constant::Int(x ^ y),
            _ => Constant::Boolean(compare(op, x.cmp(&y))?),
        },
        (Constant::Long(x), Constant::Long(y)) => match op {
            Mul => Constant::Long(x.wrapping_mul(y)),
            Div if y == 0 => return None,
            Div => Constant::Long(x.wrapping_div(y)),
            Rem if y == 0 => return None,
            Rem => Constant::Long(x.wrapping_rem(y)),
            Add => Constant::Long(x.wrapping_add(y)),
            Sub => Constant::Long(x.wrapping_sub(y)),
            And => Constant::Long(x & y),
            Or => Constant::Long(x | y),
            Xor => Constant::Long(x ^ y),
            _ => Constant::Boolean(compare(op, x.cmp(&y))?),
        },
        (Constant::Float(x), Constant::Float(y)) => {
            let (x, y) = (f32::from_bits(x), f32::from_bits(y));
            match op {
                Mul => Constant::float(x * y),
                Div => Constant::float(x / y),
                Rem => Constant::float(x % y),
                Add => Constant::float(x + y),
                Sub => Constant::float(x - y),
                _ => Constant::Boolean(compare_float(op, f64::from(x), f64::from(y))?),
            }
        }
        (Constant::Double(x), Constant::Double(y)) => {
            let (x, y) = (f64::from_bits(x), f64::from_bits(y));
            match op {
                Mul => Constant::double(x * y),
                Div => Constant::double(x / y),
                Rem => Constant::double(x % y),
                Add => Constant::double(x + y),
                Sub => Constant::double(x - y),
                _ => Constant::Boolean(compare_float(op, x, y)?),
            }
        }
        _ => return None,
    })
}

fn compare(op: BinaryOp, ordering: std::cmp::Ordering) -> Option<bool> {
    use std::cmp::Ordering::*;
    Some(match op {
        BinaryOp::Lt => ordering == Less,
        BinaryOp::Gt => ordering == Greater,
        BinaryOp::Le => ordering != Greater,
        BinaryOp::Ge => ordering != Less,
        BinaryOp::Eq => ordering == Equal,
        BinaryOp::Ne => ordering != Equal,
        _ => return None,
    })
}

/// IEEE 754 comparison: NaN is unordered, `-0.0 == 0.0`.
fn compare_float(op: BinaryOp, x: f64, y: f64) -> Option<bool> {
    Some(match op {
        BinaryOp::Lt => x < y,
        BinaryOp::Gt => x > y,
        BinaryOp::Le => x <= y,
        BinaryOp::Ge => x >= y,
        BinaryOp::Eq => x == y,
        BinaryOp::Ne => x != y,
        _ => return None,
    })
}

// Floating-point text.

/// `Double.toString` as JDK 19 and later have it: the shortest decimal that rounds to the value,
/// of at least two digits when one would do and a two-digit one is closer (`4.9E-324`, not
/// `5.0E-324`), in plain notation from 10^-3 up to 10^7 and in computerized scientific notation
/// outside it.
pub fn double_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    let m = x.abs();
    let digits = closest_shortest(
        format!("{m:e}"),
        |n| format!("{m:.n$e}"),
        |t| t.parse::<f64>() == Ok(m),
    );
    java_notation(&digits, x < 0.0, (1e-3..1e7).contains(&m))
}

/// `Float.toString`, by the same rules for `float`.
pub fn float_to_string(x: f32) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    let a = x.abs();
    let digits = closest_shortest(
        format!("{a:e}"),
        |n| format!("{a:.n$e}"),
        |t| t.parse::<f32>() == Ok(a),
    );
    java_notation(&digits, x < 0.0, (1e-3..1e7).contains(&f64::from(a)))
}

/// Of the decimals of the shortest length that round to the value, the closest to it, ties to
/// an even last digit; and of one or two digits when one would do (JDK 19's `Double.toString`).
/// Rust's shortest form has the length but not always the closest digits (`419025.63` for the
/// `float` `419025.625`, where Java's is `419025.62`), so the value rounded exactly to that many
/// digits is taken when it too rounds to the value. `exact(n)` is the value with `n` digits
/// after the point in scientific form, rounded half to even; `rounds` whether a decimal rounds
/// to the value.
fn closest_shortest(
    shortest: String,
    exact: impl Fn(usize) -> String,
    rounds: impl Fn(&str) -> bool,
) -> String {
    let digits = mantissa_digits(&shortest);
    let mut best = shortest;
    let closest = exact(digits - 1);
    if rounds(&closest) {
        best = closest;
    }
    if digits == 1 {
        let two = exact(1);
        if rounds(&two) {
            best = two;
        }
    }
    best
}

/// The number of significant digits of Rust's `{:e}` form, `4.9e-324`.
fn mantissa_digits(text: &str) -> usize {
    let mantissa = text.split('e').next().unwrap_or("");
    mantissa.chars().filter(char::is_ascii_digit).count()
}

/// Writes Rust's `{:e}` form (`1.2345e-5`) as Java does: plain (`0.000012345`, `100.0`) or
/// `1.2345E-5`, with at least one digit after the point.
fn java_notation(scientific: &str, negative: bool, plain: bool) -> String {
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let mut digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if plain {
        if exponent >= 0 {
            let int_len = exponent as usize + 1;
            if digits.len() <= int_len {
                out.push_str(&digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exponent - 1) as usize));
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        let _ = write!(out, "E{exponent}");
    }
    out
}

/// `Double.toHexString`: `0x1.8p3`, `-0x0.0p0`, a subnormal `0x0.0000000000001p-1022`.
pub fn double_hex(x: f64) -> String {
    hex(x.to_bits(), 52, 11, x.is_nan(), x.is_infinite())
}

/// `Float.toHexString`.
pub fn float_hex(x: f32) -> String {
    hex(u64::from(x.to_bits()), 23, 8, x.is_nan(), x.is_infinite())
}

fn hex(bits: u64, fraction_bits: u32, exponent_bits: u32, nan: bool, infinite: bool) -> String {
    let negative = bits >> (fraction_bits + exponent_bits) & 1 == 1;
    if nan {
        return "NaN".to_owned();
    }
    let sign = if negative { "-" } else { "" };
    if infinite {
        return format!("{sign}Infinity");
    }
    let fraction = bits & ((1u64 << fraction_bits) - 1);
    let biased = (bits >> fraction_bits) & ((1u64 << exponent_bits) - 1);
    let bias = (1i64 << (exponent_bits - 1)) - 1;
    if biased == 0 && fraction == 0 {
        return format!("{sign}0x0.0p0");
    }
    // The fraction as hexadecimal digits: a float's 23 bits are shifted to 24, six digits.
    let digits = fraction_bits.div_ceil(4);
    let shifted = fraction << (digits * 4 - fraction_bits);
    let mut text = format!("{shifted:0width$x}", width = digits as usize);
    while text.len() > 1 && text.ends_with('0') {
        text.pop();
    }
    if biased == 0 {
        format!("{sign}0x0.{text}p{}", 1 - bias)
    } else {
        format!("{sign}0x1.{text}p{}", biased as i64 - bias)
    }
}
