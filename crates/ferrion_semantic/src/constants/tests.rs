//! Tests of constant folding's arithmetic and text. `to_string.txt` is Java's own output: doubles
//! and floats (random bits, random decimals, values at the plain notation's bounds, and the
//! extremes) with `Double.toString` and `Float.toString` on Temurin 27.0.1, one per line as
//! `d <bits> <text>` or `f <bits> <text>`.

use super::value::{BinaryOp, Constant, UnaryOp, binary, unary};
use super::{double_hex, double_to_string, float_hex, float_to_string};
use crate::summary::Primitive;

fn check_samples(text: &str) -> usize {
    let mut checked = 0;
    for line in text.lines() {
        let mut words = line.splitn(3, ' ');
        let (kind, bits, expected) = (
            words.next().unwrap(),
            words.next().unwrap(),
            words.next().unwrap(),
        );
        let actual = match kind {
            "d" => double_to_string(f64::from_bits(u64::from_str_radix(bits, 16).unwrap())),
            _ => float_to_string(f32::from_bits(u32::from_str_radix(bits, 16).unwrap())),
        };
        assert_eq!(actual, expected, "{kind} {bits}");
        checked += 1;
    }
    checked
}

/// `Double.toString` and `Float.toString` as JDK 19 and later have them, against Java's own.
#[test]
fn floating_point_text() {
    let checked = check_samples(include_str!("to_string.txt"));
    assert!(checked > 1000);
    // Two digits where one would round to the value but two are closer.
    assert_eq!(double_to_string(f64::from_bits(1)), "4.9E-324");
    assert_eq!(double_to_string(1e23), "1.0E23");
    assert_eq!(double_to_string(2e-3), "0.002");
    assert_eq!(double_to_string(-0.0), "-0.0");
    assert_eq!(double_to_string(100.0), "100.0");
    assert_eq!(double_to_string(1e7), "1.0E7");
    assert_eq!(float_to_string(1.0e10), "1.0E10");
    assert_eq!(float_to_string(f32::from_bits(1)), "1.4E-45");
}

/// A larger sample, made the same way (`java Gen.java 400000 7` in the scratch directory of
/// W2's observations), from the file `FERRION_FLOAT_SAMPLES` names.
#[test]
#[ignore = "needs a sample file"]
fn floating_point_text_at_scale() {
    let path = std::env::var("FERRION_FLOAT_SAMPLES").expect("FERRION_FLOAT_SAMPLES");
    let text = std::fs::read_to_string(path).unwrap();
    let checked = check_samples(&text);
    eprintln!("{checked} values");
}

/// The oracle's hexadecimal forms (observation 11 in docs/design/types-oracle-normalization.md).
#[test]
fn hexadecimal_text() {
    assert_eq!(float_hex(1.0 / 3.0), "0x1.555556p-2");
    assert_eq!(double_hex(-0.0), "-0x0.0p0");
    assert_eq!(double_hex(1.0), "0x1.0p0");
    assert_eq!(double_hex(12.0), "0x1.8p3");
    assert_eq!(double_hex(f64::from_bits(1)), "0x0.0000000000001p-1022");
    assert_eq!(float_hex(f32::from_bits(1)), "0x0.000002p-126");
    assert_eq!(double_hex(1e23), "0x1.52d02c7e14af6p76");
    assert_eq!(double_hex(f64::INFINITY), "Infinity");
    assert_eq!(double_hex(f64::NAN), "NaN");
}

/// Java's arithmetic: wrapping, division by zero not a constant, shifts masked, narrowing
/// through `int`, string conversion.
#[test]
fn arithmetic() {
    use Constant::*;
    assert_eq!(
        binary(BinaryOp::Add, &Long(i64::MAX), &Int(1)),
        Some(Long(i64::MIN))
    );
    assert_eq!(binary(BinaryOp::Div, &Int(1), &Int(0)), None);
    assert_eq!(binary(BinaryOp::Rem, &Long(1), &Long(0)), None);
    assert_eq!(
        binary(BinaryOp::Div, &Int(i32::MIN), &Int(-1)),
        Some(Int(i32::MIN))
    );
    assert_eq!(binary(BinaryOp::Shl, &Int(1), &Int(33)), Some(Int(2)));
    assert_eq!(binary(BinaryOp::Shl, &Int(1), &Long(33)), Some(Int(2)));
    assert_eq!(binary(BinaryOp::UShr, &Int(-1), &Int(28)), Some(Int(15)));
    assert_eq!(binary(BinaryOp::Shr, &Long(-16), &Int(2)), Some(Long(-4)));
    assert_eq!(
        binary(BinaryOp::Div, &Constant::double(1.0), &Int(0)),
        Some(Constant::double(f64::INFINITY))
    );
    assert_eq!(Int(300).convert(Primitive::Byte), Some(Byte(44)));
    assert_eq!(
        Constant::double(300.5).convert(Primitive::Byte),
        Some(Byte(44))
    );
    assert_eq!(
        Constant::double(1e10).convert(Primitive::Char),
        Some(Char(0xffff))
    );
    assert_eq!(
        Constant::double(f64::NAN).convert(Primitive::Int),
        Some(Int(0))
    );
    assert_eq!(unary(UnaryOp::Minus, &Char(97)), Some(Int(-97)));
    assert_eq!(unary(UnaryOp::Complement, &Long(0)), Some(Long(-1)));
    assert_eq!(unary(UnaryOp::Not, &Int(0)), None);
    assert_eq!(
        binary(BinaryOp::Add, &Constant::str("a"), &Char(98)),
        Some(Constant::str("ab"))
    );
    assert_eq!(
        binary(BinaryOp::Add, &Constant::str(""), &Constant::float(1.5)),
        Some(Constant::str("1.5"))
    );
    assert_eq!(
        binary(BinaryOp::Eq, &Constant::str("a"), &Constant::str("a")),
        Some(Boolean(true))
    );
    assert_eq!(
        binary(BinaryOp::Lt, &Constant::double(f64::NAN), &Int(1)),
        Some(Boolean(false))
    );
    assert_eq!(
        binary(BinaryOp::Xor, &Boolean(true), &Boolean(true)),
        Some(Boolean(false))
    );
    assert_eq!(binary(BinaryOp::Add, &Boolean(true), &Int(1)), None);
}

/// The oracle's constant text: a `char` and a string quoted and escaped, a lone surrogate one
/// escape.
#[test]
fn oracle_text() {
    assert_eq!(Constant::Char(0x41).oracle_text(), "char \"A\"");
    assert_eq!(Constant::Char(0x0a).oracle_text(), "char \"\\u000a\"");
    assert_eq!(
        Constant::string(vec![0x74, 0x09, 0xd83d]).oracle_text(),
        "String \"t\\u0009\\ud83d\""
    );
    assert_eq!(Constant::Byte(44).oracle_text(), "byte 44");
    assert_eq!(
        Constant::float(1.0 / 3.0).oracle_text(),
        "float 0x1.555556p-2"
    );
}
