//! Field and method descriptors (JVMS 4.3.2, 4.3.3).

use crate::error::{Error, Result};

/// The most array dimensions a type may have (JVMS 4.4.1).
pub const MAX_DIMENSIONS: usize = 255;

/// A primitive type, by its descriptor character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BaseType {
    Byte,
    Char,
    Double,
    Float,
    Int,
    Long,
    Short,
    Boolean,
}

impl BaseType {
    pub fn from_char(c: u8) -> Option<BaseType> {
        Some(match c {
            b'B' => BaseType::Byte,
            b'C' => BaseType::Char,
            b'D' => BaseType::Double,
            b'F' => BaseType::Float,
            b'I' => BaseType::Int,
            b'J' => BaseType::Long,
            b'S' => BaseType::Short,
            b'Z' => BaseType::Boolean,
            _ => return None,
        })
    }

    /// The Java keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            BaseType::Byte => "byte",
            BaseType::Char => "char",
            BaseType::Double => "double",
            BaseType::Float => "float",
            BaseType::Int => "int",
            BaseType::Long => "long",
            BaseType::Short => "short",
            BaseType::Boolean => "boolean",
        }
    }
}

/// The element of a field type: a primitive or a class in internal form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementType<'a> {
    Base(BaseType),
    Object(&'a str),
}

/// A field type (4.3.2): an element type with zero or more array dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FieldType<'a> {
    pub dimensions: u8,
    pub element: ElementType<'a>,
}

/// A method descriptor (4.3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodDescriptor<'a> {
    pub parameters: Vec<FieldType<'a>>,
    /// `None` for `void`.
    pub result: Option<FieldType<'a>>,
}

impl BaseType {
    /// The descriptor character.
    pub fn descriptor(self) -> char {
        match self {
            BaseType::Byte => 'B',
            BaseType::Char => 'C',
            BaseType::Double => 'D',
            BaseType::Float => 'F',
            BaseType::Int => 'I',
            BaseType::Long => 'J',
            BaseType::Short => 'S',
            BaseType::Boolean => 'Z',
        }
    }
}

/// The descriptor text.
impl std::fmt::Display for FieldType<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for _ in 0..self.dimensions {
            f.write_str("[")?;
        }
        match self.element {
            ElementType::Base(base) => write!(f, "{}", base.descriptor()),
            ElementType::Object(name) => write!(f, "L{name};"),
        }
    }
}

/// The descriptor text.
impl std::fmt::Display for MethodDescriptor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?;
        for parameter in &self.parameters {
            write!(f, "{parameter}")?;
        }
        f.write_str(")")?;
        match &self.result {
            Some(result) => write!(f, "{result}"),
            None => f.write_str("V"),
        }
    }
}

pub(crate) fn malformed(text: &str, pos: usize, what: &str) -> Error {
    Error::malformed(pos as u64, format!("{what} at character {pos} of `{text}`"))
}

/// Reads a field type at `pos`, returning it and the position after it.
fn field_type_at(text: &str, mut pos: usize) -> Result<(FieldType<'_>, usize)> {
    let bytes = text.as_bytes();
    let start = pos;
    while bytes.get(pos) == Some(&b'[') {
        pos += 1;
    }
    let dimensions = pos - start;
    if dimensions > MAX_DIMENSIONS {
        return Err(malformed(text, start, "more than 255 array dimensions"));
    }
    let element = match bytes.get(pos) {
        Some(&b'L') => {
            let end = text[pos..]
                .find(';')
                .map(|i| pos + i)
                .ok_or_else(|| malformed(text, pos, "a class type without `;`"))?;
            let name = &text[pos + 1..end];
            if name.is_empty() || name.contains(['.', '[']) {
                return Err(malformed(text, pos, "a bad class name"));
            }
            pos = end + 1;
            ElementType::Object(name)
        }
        Some(&c) => match BaseType::from_char(c) {
            Some(base) => {
                pos += 1;
                ElementType::Base(base)
            }
            None => return Err(malformed(text, pos, "an unexpected character")),
        },
        None => return Err(malformed(text, pos, "a missing type")),
    };
    Ok((
        FieldType {
            dimensions: dimensions as u8,
            element,
        },
        pos,
    ))
}

/// Parses a field descriptor (4.3.2).
pub fn parse_field(text: &str) -> Result<FieldType<'_>> {
    let (field, end) = field_type_at(text, 0)?;
    if end != text.len() {
        return Err(malformed(text, end, "text after the type"));
    }
    Ok(field)
}

/// Parses a method descriptor (4.3.3).
pub fn parse_method(text: &str) -> Result<MethodDescriptor<'_>> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err(malformed(
            text,
            0,
            "a method descriptor that doesn't start with `(`",
        ));
    }
    let mut pos = 1;
    let mut parameters = Vec::new();
    while bytes.get(pos) != Some(&b')') {
        let (parameter, end) = field_type_at(text, pos)?;
        parameters.push(parameter);
        pos = end;
    }
    pos += 1;
    let result = if bytes.get(pos) == Some(&b'V') {
        pos += 1;
        None
    } else {
        let (result, end) = field_type_at(text, pos)?;
        pos = end;
        Some(result)
    };
    if pos != text.len() {
        return Err(malformed(text, pos, "text after the result"));
    }
    Ok(MethodDescriptor { parameters, result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields() {
        assert_eq!(
            parse_field("[[Ljava/util/Map$Entry;").unwrap(),
            FieldType {
                dimensions: 2,
                element: ElementType::Object("java/util/Map$Entry")
            }
        );
        assert_eq!(
            parse_field("J").unwrap(),
            FieldType {
                dimensions: 0,
                element: ElementType::Base(BaseType::Long)
            }
        );
        for bad in [
            "",
            "V",
            "L;",
            "Ljava/lang/Object",
            "II",
            "Q",
            "[",
            "Ljava.lang.Object;",
        ] {
            assert!(parse_field(bad).is_err(), "{bad}");
        }
        let deep = "[".repeat(255) + "I";
        assert_eq!(parse_field(&deep).unwrap().dimensions, 255);
        assert!(parse_field(&("[".repeat(256) + "I")).is_err());
    }

    #[test]
    fn methods() {
        let method = parse_method("(I[JLjava/lang/String;)V").unwrap();
        assert_eq!(method.parameters.len(), 3);
        assert_eq!(method.result, None);
        let method = parse_method("()Ljava/lang/Object;").unwrap();
        assert!(method.parameters.is_empty());
        assert!(method.result.is_some());
        for bad in ["", "()", "(V)V", "(I", "()VV", "I)V", "(I)V;"] {
            assert!(parse_method(bad).is_err(), "{bad}");
        }
    }
}
