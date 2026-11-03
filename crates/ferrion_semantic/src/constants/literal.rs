//! Literals' values (JLS 3.10): integer literals in each radix, decimal and hexadecimal
//! floating-point literals rounded to nearest, and character and string literals and text blocks
//! with their Unicode escapes and escape sequences translated, as UTF-16, lone surrogates kept.

use ferrion_lexer::{Span, unicode_escapes};
use ferrion_syntax::{SyntaxKind, SyntaxToken};

use super::Constant;

/// A literal token's value, or `None` for `null` and a literal that is malformed (a lexer error,
/// which javac reports).
pub fn literal_value(token: SyntaxToken<'_>) -> Option<Constant> {
    let tree = token.tree();
    let span = token.span();
    match token.kind() {
        SyntaxKind::TrueKw => Some(Constant::Boolean(true)),
        SyntaxKind::FalseKw => Some(Constant::Boolean(false)),
        SyntaxKind::IntLiteral => {
            let units = translated(tree.text(), span)?;
            integer(&String::from_utf16(&units).ok()?, false).map(|v| Constant::Int(v as i32))
        }
        SyntaxKind::LongLiteral => {
            let units = translated(tree.text(), span)?;
            integer(&String::from_utf16(&units).ok()?, true).map(|v| Constant::Long(v as i64))
        }
        SyntaxKind::FloatLiteral => {
            let units = translated(tree.text(), span)?;
            let text = String::from_utf16(&units).ok()?;
            floating(&text, false).map(|v| Constant::float(v as f32))
        }
        SyntaxKind::DoubleLiteral => {
            let units = translated(tree.text(), span)?;
            let text = String::from_utf16(&units).ok()?;
            floating(&text, true).map(Constant::double)
        }
        SyntaxKind::CharLiteral => {
            let units = translated(tree.text(), span)?;
            let inner = units.get(1..units.len().checked_sub(1)?)?;
            let value = escapes(inner, false)?;
            match value[..] {
                [c] => Some(Constant::Char(c)),
                _ => None,
            }
        }
        SyntaxKind::StringLiteral => {
            let units = translated(tree.text(), span)?;
            let inner = units.get(1..units.len().checked_sub(1)?)?;
            escapes(inner, false).map(Constant::string)
        }
        SyntaxKind::TextBlock => {
            let units = translated(tree.text(), span)?;
            text_block(&units).map(Constant::string)
        }
        _ => None,
    }
}

/// The token's text with its Unicode escapes (JLS 3.3) translated, as UTF-16: an escape of a
/// surrogate stays one code unit. `None` if an escape is invalid.
fn translated(text: &str, span: Span) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(span.len() as usize);
    let mut at = span.start as usize;
    for escape in unicode_escapes(text, span) {
        let (start, end) = (escape.start as usize, escape.end as usize);
        out.extend(text[at..start].encode_utf16());
        let c = escape.ch?;
        if c.len_utf16() == 2 {
            // A pair of escapes, one character.
            let mut buf = [0; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        } else {
            // The digits as written, so that a lone surrogate stays one.
            let digits = text.get(end.checked_sub(4)?..end)?;
            out.push(u16::from_str_radix(digits, 16).ok()?);
        }
        at = end;
    }
    out.extend(text[at..span.end as usize].encode_utf16());
    Some(out)
}

/// An integer literal's value as 64 bits: its digits in its radix, underscores dropped, the
/// suffix ignored. `None` if it doesn't fit its type (`2147483648` is `int`'s only decimal
/// literal beyond `int`'s range, valid only after `-`, and is `Integer.MIN_VALUE`).
fn integer(text: &str, long: bool) -> Option<u64> {
    let text: String = text
        .chars()
        .filter(|&c| c != '_' && c != 'l' && c != 'L')
        .collect();
    let (digits, radix) = if let Some(hex) = text.strip_prefix("0x").or(text.strip_prefix("0X")) {
        (hex, 16)
    } else if let Some(bin) = text.strip_prefix("0b").or(text.strip_prefix("0B")) {
        (bin, 2)
    } else if text.len() > 1 && text.starts_with('0') {
        (&text[1..], 8)
    } else {
        (&text[..], 10)
    };
    let value = u64::from_str_radix(digits, radix).ok()?;
    let limit: u64 = match (long, radix) {
        (false, 10) => 1 << 31,
        (true, 10) => 1 << 63,
        (false, _) => u64::from(u32::MAX),
        (true, _) => u64::MAX,
    };
    (value <= limit).then_some(value)
}

/// A floating-point literal's value, rounded to `double`, or to `float` through the value's own
/// rounding (`double` is false): a decimal literal is parsed directly to its type, a hexadecimal
/// one by its exact binary value.
fn floating(text: &str, double: bool) -> Option<f64> {
    let text: String = text.chars().filter(|&c| c != '_').collect();
    // The suffix is last: after a hexadecimal literal's decimal exponent, so never a digit.
    let text = text.trim_end_matches(['f', 'F', 'd', 'D']);
    if let Some(hex) = text.strip_prefix("0x").or(text.strip_prefix("0X")) {
        return hex_float(hex, double);
    }
    if double {
        text.parse::<f64>().ok()
    } else {
        text.parse::<f32>().ok().map(f64::from)
    }
}

/// A hexadecimal floating-point literal's value (`1.8p3`, after its `0x`), correctly rounded:
/// its significant bits are kept to 62, any further nonzero bit as a sticky bit below them, so
/// the one rounding of the integer to the type is the literal's.
fn hex_float(text: &str, double: bool) -> Option<f64> {
    let (mantissa, exponent) = text.split_once(['p', 'P'])?;
    let exponent: i64 = exponent.parse().ok()?;
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut m: u64 = 0;
    let mut scale: i64 = 0;
    let mut sticky = false;
    for (i, c) in int_part.chars().chain(frac_part.chars()).enumerate() {
        let d = u64::from(c.to_digit(16)?);
        let fractional = i >= int_part.len();
        if m >> 58 == 0 {
            m = m << 4 | d;
            if fractional {
                scale -= 4;
            }
        } else {
            sticky |= d != 0;
            if !fractional {
                scale += 4;
            }
        }
    }
    if m == 0 {
        return Some(0.0);
    }
    if sticky {
        m |= 1;
    }
    let e = exponent + scale;
    if double {
        Some(scale_f64(m as f64, e))
    } else {
        Some(f64::from(scale_f32(m as f32, e)))
    }
}

fn scale_f64(mut x: f64, mut e: i64) -> f64 {
    while e > 0 {
        let step = e.min(1000);
        x *= f64::from_bits(((1023 + step) as u64) << 52);
        e -= step;
    }
    while e < 0 {
        let step = (-e).min(1000);
        x *= f64::from_bits(((1023 - step) as u64) << 52);
        e += step;
    }
    x
}

fn scale_f32(mut x: f32, mut e: i64) -> f32 {
    while e > 0 {
        let step = e.min(100);
        x *= f32::from_bits(((127 + step) as u32) << 23);
        e -= step;
    }
    while e < 0 {
        let step = (-e).min(100);
        x *= f32::from_bits(((127 - step) as u32) << 23);
        e += step;
    }
    x
}

const BACKSLASH: u16 = b'\\' as u16;

/// Escape sequences (JLS 3.10.7) translated; in a text block, `\` and a line terminator is
/// nothing. `None` for an invalid one.
fn escapes(units: &[u16], text_block: bool) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        if u != BACKSLASH {
            out.push(u);
            i += 1;
            continue;
        }
        let next = *units.get(i + 1)?;
        i += 2;
        let simple = match u8::try_from(next).ok() {
            Some(b'b') => Some(0x08),
            Some(b't') => Some(0x09),
            Some(b'n') => Some(0x0a),
            Some(b'f') => Some(0x0c),
            Some(b'r') => Some(0x0d),
            Some(b's') => Some(0x20),
            Some(b'"') => Some(0x22),
            Some(b'\'') => Some(0x27),
            Some(b'\\') => Some(0x5c),
            _ => None,
        };
        if let Some(c) = simple {
            out.push(c);
            continue;
        }
        if text_block && next == 0x0a {
            continue;
        }
        // An octal escape: up to three digits, the first 0 to 3 if there are three.
        let digit = |u: u16| (0x30..=0x37).contains(&u).then(|| u - 0x30);
        let first = digit(next)?;
        let mut value = first;
        let max = if first <= 3 { 2 } else { 1 };
        for _ in 0..max {
            match units.get(i).copied().and_then(digit) {
                Some(d) => {
                    value = value * 8 + d;
                    i += 1;
                }
                None => break,
            }
        }
        out.push(value);
    }
    Some(out)
}

/// Whether a code unit is white space as `String.stripIndent` counts it
/// (`Character.isWhitespace`).
fn is_white(u: u16) -> bool {
    matches!(u, 0x09..=0x0d | 0x1c..=0x20)
        || matches!(
            u,
            0x1680 | 0x2000..=0x2006 | 0x2008..=0x200a | 0x2028 | 0x2029 | 0x205f | 0x3000
        )
}

/// A text block's value (JLS 3.10.6): its content after the opening delimiter's line, line
/// terminators as `\n`, incidental white space stripped as `String.stripIndent` strips it,
/// trailing white space removed from each line, then escape sequences translated.
fn text_block(units: &[u16]) -> Option<Vec<u16>> {
    let quote = 0x22;
    if units.len() < 6 || units[..3] != [quote; 3] || units[units.len() - 3..] != [quote; 3] {
        return None;
    }
    let body = &units[3..units.len() - 3];
    // The opening delimiter's line ends at its line terminator.
    let start = body.iter().position(|&u| u == 0x0a || u == 0x0d)?;
    let mut rest = &body[start..];
    if rest.starts_with(&[0x0d, 0x0a]) {
        rest = &rest[2..];
    } else {
        rest = &rest[1..];
    }
    // Normalize line terminators.
    let mut content = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            0x0d => {
                content.push(0x0a);
                if rest.get(i + 1) == Some(&0x0a) {
                    i += 1;
                }
            }
            u => content.push(u),
        }
        i += 1;
    }
    let lines: Vec<&[u16]> = content.split(|&u| u == 0x0a).collect();
    let last = lines.len() - 1;
    let blank = |line: &[u16]| line.iter().all(|&u| is_white(u));
    let indent = |line: &[u16]| line.iter().take_while(|&&u| is_white(u)).count();
    let min = lines
        .iter()
        .enumerate()
        .filter(|&(i, line)| i == last || !blank(line))
        .map(|(_, line)| indent(line))
        .min()
        .unwrap_or(0);
    let mut stripped: Vec<u16> = Vec::with_capacity(content.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            stripped.push(0x0a);
        }
        if blank(line) {
            continue;
        }
        let line = &line[min.min(line.len())..];
        let end = line.len() - line.iter().rev().take_while(|&&u| is_white(u)).count();
        stripped.extend_from_slice(&line[..end]);
    }
    escapes(&stripped, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn integers() {
        assert_eq!(integer("0xFFFF_FFFF", false), Some(0xffff_ffff));
        assert_eq!(integer("2147483648", false), Some(1 << 31));
        assert_eq!(integer("2147483649", false), None);
        assert_eq!(integer("017", false), Some(15));
        assert_eq!(integer("0b101", false), Some(5));
        assert_eq!(integer("0", false), Some(0));
        assert_eq!(integer("9223372036854775808L", true), Some(1 << 63));
        assert_eq!(integer("0xFFFFFFFFFFFFFFFFL", true), Some(u64::MAX));
    }

    #[test]
    fn floats() {
        assert_eq!(floating("0x1.8p3", true), Some(12.0));
        assert_eq!(floating("1e10f", false), Some(f64::from(1e10f32)));
        assert_eq!(floating("2.5", true), Some(2.5));
        assert_eq!(floating("1.", true), Some(1.0));
        assert_eq!(floating(".5", true), Some(0.5));
        assert_eq!(floating("0x1p-1074", true), Some(f64::from_bits(1)));
        assert_eq!(floating("0x1.fffffffffffff8p0", true), Some(2.0));
        assert_eq!(
            floating("0x1.fffffep0f", false),
            Some(f64::from(1.9999999f32))
        );
    }

    #[test]
    fn text_blocks() {
        // Observed: the oracle's `TB` in docs/design/semantic-attribution.md's observations.
        let block = units("\"\"\"\n      hello\n        world\\t!\n      \"\"\"");
        assert_eq!(text_block(&block), Some(units("hello\n  world\t!\n")));
        let joined = units("\"\"\"\n  a \\\n  b\"\"\"");
        assert_eq!(text_block(&joined), Some(units("a b")));
        let crlf = units("\"\"\"\r\n  x  \r\n   y\"\"\"");
        assert_eq!(text_block(&crlf), Some(units("x\n y")));
    }

    #[test]
    fn escape_sequences() {
        assert_eq!(
            escapes(&units("a\\tb\\101\\0\\377\\s"), false),
            Some(units("a\tbA\0\u{ff} "))
        );
        assert_eq!(escapes(&units("\\q"), false), None);
    }
}
