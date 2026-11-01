//! Numeric literals out of their type's range, which javac reports while parsing: an `int` or
//! `long` too large for its type, and a floating-point literal that rounds to infinity, or to zero
//! though it isn't zero.
//!
//! As javac does, a decimal `int` or `long` literal right after a prefix `-` may be one more than
//! the type's largest value (`-2147483648`, `-9223372036854775808L`); a binary, octal, or
//! hexadecimal one may use all the type's bits (`0xffffffff`). Floating-point literals are
//! rounded to their type to nearest, ties to even, as Java's conversions do.

use crate::SyntaxKind::{self, *};
use crate::diagnostic::ParseDiagnosticKind;

/// The error for a literal of `kind` whose text, with Unicode escapes translated, is `text`, if
/// it is out of range. `negated` if a prefix `-` comes right before it. A malformed literal, which
/// the lexer reports, is never out of range.
pub(crate) fn out_of_range(
    kind: SyntaxKind,
    text: &str,
    negated: bool,
) -> Option<ParseDiagnosticKind> {
    let text: String = text.chars().filter(|&c| c != '_').collect();
    match kind {
        IntLiteral => {
            integer_too_large(&text, 32, negated).then_some(ParseDiagnosticKind::IntTooLarge)
        }
        LongLiteral => {
            let digits = text.strip_suffix(['l', 'L'])?;
            integer_too_large(digits, 64, negated).then_some(ParseDiagnosticKind::IntTooLarge)
        }
        FloatLiteral => float_range(text.strip_suffix(['f', 'F'])?, Format::F32),
        DoubleLiteral => float_range(text.strip_suffix(['d', 'D']).unwrap_or(&text), Format::F64),
        _ => None,
    }
}

/// Whether an integer literal's digits (with its prefix, without underscores and suffix) are
/// too large for a type of `bits` bits.
fn integer_too_large(text: &str, bits: u32, negated: bool) -> bool {
    let lower = text.to_ascii_lowercase();
    let (radix, digits) = if let Some(digits) = lower.strip_prefix("0x") {
        (16, digits)
    } else if let Some(digits) = lower.strip_prefix("0b") {
        (2, digits)
    } else if lower.len() > 1 && lower.starts_with('0') {
        (8, &lower[1..])
    } else {
        (10, lower.as_str())
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return false;
    }
    // Decimal literals are signed, the others use every bit.
    let max: u128 = match radix {
        10 if negated => 1 << (bits - 1),
        10 => (1 << (bits - 1)) - 1,
        _ => (1 << bits) - 1,
    };
    let mut value: u128 = 0;
    for c in digits.chars() {
        let digit = c.to_digit(radix).map_or(0, u128::from);
        value = value * u128::from(radix) + digit;
        if value > max {
            return true;
        }
    }
    false
}

/// A floating-point type: its precision in bits, and the exponents of its largest finite value
/// and smallest subnormal one.
#[derive(Clone, Copy)]
struct Format {
    precision: i64,
    max_exponent: i64,
    min_exponent: i64,
}

impl Format {
    const F32: Format = Format {
        precision: 24,
        max_exponent: 127,
        min_exponent: -149,
    };
    const F64: Format = Format {
        precision: 53,
        max_exponent: 1023,
        min_exponent: -1074,
    };
}

/// For a floating-point literal (without underscores and suffix): too large if it rounds to
/// infinity, too small if it rounds to zero but isn't zero.
fn float_range(text: &str, format: Format) -> Option<ParseDiagnosticKind> {
    let lower = text.to_ascii_lowercase();
    let (infinite, zero) = match lower.strip_prefix("0x") {
        Some(hex) => hex_float(hex, format)?,
        None => {
            let mantissa = lower.split('e').next().unwrap_or("");
            let nonzero = mantissa.chars().any(|c| matches!(c, '1'..='9'));
            if format.precision == Format::F32.precision {
                let value: f32 = lower.parse().ok()?;
                (value.is_infinite(), nonzero && value == 0.0)
            } else {
                let value: f64 = lower.parse().ok()?;
                (value.is_infinite(), nonzero && value == 0.0)
            }
        }
    };
    if infinite {
        Some(ParseDiagnosticKind::FloatTooLarge)
    } else if zero {
        Some(ParseDiagnosticKind::FloatTooSmall)
    } else {
        None
    }
}

/// For a hexadecimal floating-point literal's digits after `0x` (`1.8p3`), whether it rounds to
/// infinity, and whether it rounds to zero without being zero. `None` if it is malformed.
fn hex_float(text: &str, format: Format) -> Option<(bool, bool)> {
    let (significand, exponent) = text.split_once('p')?;
    let (whole, fraction) = significand.split_once('.').unwrap_or((significand, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let mut bits = Vec::new();
    for c in whole.chars().chain(fraction.chars()) {
        let digit = c.to_digit(16)?;
        bits.extend((0..4).rev().map(|i| digit >> i & 1 == 1));
    }
    let exponent = parse_exponent(exponent)?;
    let Some(first) = bits.iter().position(|&bit| bit) else {
        return Some((false, false));
    };
    let bits = &bits[first..];
    // The value is 1.xxx (the bits from the first one) times 2 to the power `top`.
    let fraction_bits = 4 * fraction.len() as i64;
    let top = exponent + (bits.len() as i64 - 1) - fraction_bits;
    let precision = format.precision as usize;
    if top > format.max_exponent {
        return Some((true, false));
    }
    if top == format.max_exponent && bits.len() > precision {
        // Rounding up past the largest value, whose significand is all ones.
        let kept = &bits[..precision];
        let rest = &bits[precision..];
        let half_or_more = rest[0];
        let more_than_half = half_or_more && rest[1..].iter().any(|&bit| bit);
        let round_up = more_than_half || half_or_more && kept[precision - 1];
        return Some((round_up && kept.iter().all(|&bit| bit), false));
    }
    // Below half the smallest subnormal value, or exactly half, which rounds to zero (even).
    let half_smallest = format.min_exponent - 1;
    let zero = top < half_smallest || top == half_smallest && bits[1..].iter().all(|&bit| !bit);
    Some((false, zero))
}

/// A binary exponent's digits, with an optional sign, saturated far beyond any type's range.
fn parse_exponent(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix(['+', '-']) {
        Some(digits) => (text.starts_with('-'), digits),
        None => (false, text),
    };
    if digits.is_empty() {
        return None;
    }
    let mut value: i64 = 0;
    for c in digits.chars() {
        value = (value * 10 + i64::from(c.to_digit(10)?)).min(1 << 40);
    }
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ParseDiagnosticKind::{FloatTooLarge, FloatTooSmall, IntTooLarge};

    fn check(kind: SyntaxKind, text: &str) -> Option<ParseDiagnosticKind> {
        out_of_range(kind, text, false)
    }

    #[test]
    fn ints() {
        assert_eq!(check(IntLiteral, "2147483647"), None);
        assert_eq!(check(IntLiteral, "2147483648"), Some(IntTooLarge));
        assert_eq!(out_of_range(IntLiteral, "2147483648", true), None);
        assert_eq!(
            out_of_range(IntLiteral, "2147483649", true),
            Some(IntTooLarge)
        );
        assert_eq!(check(IntLiteral, "2_147_483_648"), Some(IntTooLarge));
        assert_eq!(check(IntLiteral, "0xffffffff"), None);
        assert_eq!(check(IntLiteral, "0x1_0000_0000"), Some(IntTooLarge));
        assert_eq!(check(IntLiteral, "037777777777"), None);
        assert_eq!(check(IntLiteral, "040000000000"), Some(IntTooLarge));
        assert_eq!(check(IntLiteral, &format!("0b{}", "1".repeat(32))), None);
        assert_eq!(
            check(IntLiteral, &format!("0b{}", "1".repeat(33))),
            Some(IntTooLarge)
        );
        assert_eq!(check(IntLiteral, &"9".repeat(100)), Some(IntTooLarge));
        assert_eq!(check(IntLiteral, "0"), None);
        assert_eq!(check(IntLiteral, "00"), None);
        // Malformed literals are the lexer's to report.
        assert_eq!(check(IntLiteral, "099999999999"), None);
        assert_eq!(check(IntLiteral, "0x"), None);
    }

    #[test]
    fn longs() {
        assert_eq!(check(LongLiteral, "9223372036854775807L"), None);
        assert_eq!(
            check(LongLiteral, "9223372036854775808l"),
            Some(IntTooLarge)
        );
        assert_eq!(
            out_of_range(LongLiteral, "9223372036854775808L", true),
            None
        );
        assert_eq!(check(LongLiteral, "0xffffffffffffffffL"), None);
        assert_eq!(
            check(LongLiteral, "0x1ffffffffffffffffL"),
            Some(IntTooLarge)
        );
    }

    #[test]
    fn floats() {
        assert_eq!(check(DoubleLiteral, "1e308"), None);
        assert_eq!(check(DoubleLiteral, "1e9999"), Some(FloatTooLarge));
        assert_eq!(check(DoubleLiteral, "1e310d"), Some(FloatTooLarge));
        assert_eq!(check(DoubleLiteral, "1e-9999"), Some(FloatTooSmall));
        assert_eq!(check(DoubleLiteral, "4.9e-324"), None);
        assert_eq!(check(DoubleLiteral, "3e-324"), None);
        assert_eq!(check(DoubleLiteral, "2e-324"), Some(FloatTooSmall));
        assert_eq!(check(DoubleLiteral, "0e9999"), None);
        assert_eq!(check(DoubleLiteral, "0.0"), None);
        assert_eq!(
            check(DoubleLiteral, "1e-999999999999999999999"),
            Some(FloatTooSmall)
        );
        assert_eq!(check(FloatLiteral, "1e39f"), Some(FloatTooLarge));
        assert_eq!(check(FloatLiteral, "3.4028235e38f"), None);
        assert_eq!(check(FloatLiteral, "1e-46f"), Some(FloatTooSmall));
        assert_eq!(check(FloatLiteral, "1.4e-45f"), None);
        assert_eq!(check(FloatLiteral, "0.7e-45f"), Some(FloatTooSmall));
        assert_eq!(check(FloatLiteral, "0.8e-45f"), None);
    }

    #[test]
    fn hex_floats() {
        assert_eq!(check(DoubleLiteral, "0x1p1023"), None);
        assert_eq!(check(DoubleLiteral, "0x1p1024"), Some(FloatTooLarge));
        assert_eq!(check(DoubleLiteral, "0x1p99999"), Some(FloatTooLarge));
        assert_eq!(check(DoubleLiteral, "0x1.fffffffffffffp1023"), None);
        // Half an ulp more rounds to even, which is up, past the largest value.
        assert_eq!(
            check(DoubleLiteral, "0x1.fffffffffffff8p1023"),
            Some(FloatTooLarge)
        );
        assert_eq!(check(DoubleLiteral, "0x1.fffffffffffff7p1023"), None);
        assert_eq!(check(DoubleLiteral, "0x0.0000000000001p-1022"), None);
        assert_eq!(
            check(DoubleLiteral, "0x0.00000000000001p-1022"),
            Some(FloatTooSmall)
        );
        assert_eq!(check(DoubleLiteral, "0x1p-1075"), Some(FloatTooSmall));
        assert_eq!(check(DoubleLiteral, "0x1.1p-1075"), None);
        assert_eq!(check(DoubleLiteral, "0x0p0"), None);
        assert_eq!(check(DoubleLiteral, "0x.8p1"), None);
        assert_eq!(check(FloatLiteral, "0x1p-200f"), Some(FloatTooSmall));
        assert_eq!(check(FloatLiteral, "0x1p-149f"), None);
        assert_eq!(check(FloatLiteral, "0x1p128f"), Some(FloatTooLarge));
        assert_eq!(check(FloatLiteral, "0x1.fffffep127f"), None);
        assert_eq!(check(FloatLiteral, "0x1.ffffffp127f"), Some(FloatTooLarge));
    }
}
