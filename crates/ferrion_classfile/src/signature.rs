//! Generic signatures (JVMS 4.7.9.1), parsed when a member is used.
//!
//! The parser keeps its own stack for nested type arguments instead of recursing, so deeply
//! nested input can't overflow the call stack, and stops at [`MAX_NESTING`] levels. Names borrow
//! from the signature text.

use crate::descriptor::{BaseType, MAX_DIMENSIONS, malformed};
use crate::error::Result;

/// How deeply type arguments may nest.
pub const MAX_NESTING: usize = 1000;

/// A Java type in a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeSignature<'a> {
    Base(BaseType),
    Class(ClassTypeSignature<'a>),
    /// A type variable, by name.
    TypeVariable(&'a str),
    /// An array; `element` is never itself an array.
    Array {
        dimensions: u8,
        element: Box<TypeSignature<'a>>,
    },
}

/// A class type: the first segment's name has the package (`java/util/Map`), and each later
/// segment is an inner class (`Entry`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassTypeSignature<'a> {
    pub segments: Vec<SimpleClassTypeSignature<'a>>,
}

impl ClassTypeSignature<'_> {
    /// The erasure's internal name, joining inner segments with `$`.
    pub fn erasure(&self) -> String {
        let mut name = String::new();
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                name.push('$');
            }
            name.push_str(segment.name);
        }
        name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleClassTypeSignature<'a> {
    pub name: &'a str,
    pub arguments: Vec<TypeArgument<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeArgument<'a> {
    /// `*`, an unbounded wildcard.
    Any,
    /// `+`, `? extends`.
    Extends(TypeSignature<'a>),
    /// `-`, `? super`.
    Super(TypeSignature<'a>),
    Exact(TypeSignature<'a>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeParameter<'a> {
    pub name: &'a str,
    pub class_bound: Option<TypeSignature<'a>>,
    pub interface_bounds: Vec<TypeSignature<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassSignature<'a> {
    pub type_parameters: Vec<TypeParameter<'a>>,
    pub superclass: ClassTypeSignature<'a>,
    pub interfaces: Vec<ClassTypeSignature<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodSignature<'a> {
    pub type_parameters: Vec<TypeParameter<'a>>,
    pub parameters: Vec<TypeSignature<'a>>,
    /// `None` for `void`.
    pub result: Option<TypeSignature<'a>>,
    /// Class types or type variables.
    pub throws: Vec<TypeSignature<'a>>,
}

// The signature text (JVMS 4.7.9.1), so that `parse(text).to_string() == text`.

impl std::fmt::Display for TypeSignature<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeSignature::Base(base) => write!(f, "{}", base.descriptor()),
            TypeSignature::Class(class) => write!(f, "{class}"),
            TypeSignature::TypeVariable(name) => write!(f, "T{name};"),
            TypeSignature::Array {
                dimensions,
                element,
            } => {
                for _ in 0..*dimensions {
                    f.write_str("[")?;
                }
                write!(f, "{element}")
            }
        }
    }
}

impl std::fmt::Display for ClassTypeSignature<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("L")?;
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            f.write_str(segment.name)?;
            if !segment.arguments.is_empty() {
                f.write_str("<")?;
                for argument in &segment.arguments {
                    match argument {
                        TypeArgument::Any => f.write_str("*")?,
                        TypeArgument::Extends(t) => write!(f, "+{t}")?,
                        TypeArgument::Super(t) => write!(f, "-{t}")?,
                        TypeArgument::Exact(t) => write!(f, "{t}")?,
                    }
                }
                f.write_str(">")?;
            }
        }
        f.write_str(";")
    }
}

fn write_type_parameters(
    f: &mut std::fmt::Formatter<'_>,
    parameters: &[TypeParameter<'_>],
) -> std::fmt::Result {
    if parameters.is_empty() {
        return Ok(());
    }
    f.write_str("<")?;
    for parameter in parameters {
        write!(f, "{}:", parameter.name)?;
        if let Some(bound) = &parameter.class_bound {
            write!(f, "{bound}")?;
        }
        for bound in &parameter.interface_bounds {
            write!(f, ":{bound}")?;
        }
    }
    f.write_str(">")
}

impl std::fmt::Display for ClassSignature<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_type_parameters(f, &self.type_parameters)?;
        write!(f, "{}", self.superclass)?;
        for interface in &self.interfaces {
            write!(f, "{interface}")?;
        }
        Ok(())
    }
}

impl std::fmt::Display for MethodSignature<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_type_parameters(f, &self.type_parameters)?;
        f.write_str("(")?;
        for parameter in &self.parameters {
            write!(f, "{parameter}")?;
        }
        f.write_str(")")?;
        match &self.result {
            Some(result) => write!(f, "{result}")?,
            None => f.write_str("V")?,
        }
        for thrown in &self.throws {
            write!(f, "^{thrown}")?;
        }
        Ok(())
    }
}

/// Parses a class signature.
pub fn parse_class(text: &str) -> Result<ClassSignature<'_>> {
    let mut p = Parser { text, pos: 0 };
    let type_parameters = p.type_parameters()?;
    let superclass = p.class_type()?;
    let mut interfaces = Vec::new();
    while !p.at_end() {
        interfaces.push(p.class_type()?);
    }
    Ok(ClassSignature {
        type_parameters,
        superclass,
        interfaces,
    })
}

/// Parses a method signature.
pub fn parse_method(text: &str) -> Result<MethodSignature<'_>> {
    let mut p = Parser { text, pos: 0 };
    let type_parameters = p.type_parameters()?;
    p.expect(b'(')?;
    let mut parameters = Vec::new();
    while p.peek() != Some(b')') {
        parameters.push(p.reference(true)?);
    }
    p.pos += 1;
    let result = if p.peek() == Some(b'V') {
        p.pos += 1;
        None
    } else {
        Some(p.reference(true)?)
    };
    let mut throws = Vec::new();
    while p.peek() == Some(b'^') {
        p.pos += 1;
        let start = p.pos;
        let thrown = p.reference(false)?;
        if !matches!(
            thrown,
            TypeSignature::Class(_) | TypeSignature::TypeVariable(_)
        ) {
            return Err(p.error_at(start, "a thrown type that isn't a class or type variable"));
        }
        throws.push(thrown);
    }
    if !p.at_end() {
        return Err(p.error("text after the signature"));
    }
    Ok(MethodSignature {
        type_parameters,
        parameters,
        result,
        throws,
    })
}

/// Parses a field signature (also a record component's).
pub fn parse_field(text: &str) -> Result<TypeSignature<'_>> {
    let mut p = Parser { text, pos: 0 };
    let field = p.reference(false)?;
    if !p.at_end() {
        return Err(p.error("text after the signature"));
    }
    Ok(field)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

/// A class type whose type arguments are being parsed.
struct Frame<'a> {
    class: ClassTypeSignature<'a>,
    dimensions: usize,
    /// The wildcard indicator of the argument being parsed.
    wildcard: u8,
}

impl<'a> Frame<'a> {
    fn arguments(&mut self) -> &mut Vec<TypeArgument<'a>> {
        // A class type always has its first segment.
        &mut self
            .class
            .segments
            .last_mut()
            .expect("a class type has a segment")
            .arguments
    }
}

enum Next {
    /// A type argument follows, after its wildcard indicator, if any.
    Type,
    /// The arguments ended with `>`.
    Close,
}

enum After {
    /// `<` opened type arguments.
    Arguments,
    /// `;` ended the class type.
    End,
}

fn wrap(element: TypeSignature<'_>, dimensions: usize) -> TypeSignature<'_> {
    if dimensions == 0 {
        element
    } else {
        TypeSignature::Array {
            dimensions: dimensions as u8,
            element: Box::new(element),
        }
    }
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn at_end(&self) -> bool {
        self.pos == self.text.len()
    }

    fn error(&self, what: &str) -> crate::error::Error {
        malformed(self.text, self.pos, what)
    }

    fn error_at(&self, pos: usize, what: &str) -> crate::error::Error {
        malformed(self.text, pos, what)
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        if self.peek() != Some(c) {
            return Err(self.error(&format!("`{}` expected", char::from(c))));
        }
        self.pos += 1;
        Ok(())
    }

    /// An identifier, which may contain `/` when `slashes` is true (a class name with its
    /// package). Identifiers are non-empty and contain none of `. ; [ / < > :`.
    fn identifier(&mut self, slashes: bool) -> Result<&'a str> {
        let start = self.pos;
        let rest = &self.text[start..];
        let len = rest
            .find(|c: char| {
                matches!(c, '.' | ';' | '[' | '<' | '>' | ':') || (c == '/' && !slashes)
            })
            .unwrap_or(rest.len());
        let name = &rest[..len];
        if name.is_empty() || name.split('/').any(str::is_empty) {
            return Err(self.error("an empty identifier"));
        }
        self.pos += len;
        Ok(name)
    }

    fn dimensions(&mut self) -> Result<usize> {
        let start = self.pos;
        while self.peek() == Some(b'[') {
            self.pos += 1;
        }
        if self.pos - start > MAX_DIMENSIONS {
            return Err(self.error_at(start, "more than 255 array dimensions"));
        }
        Ok(self.pos - start)
    }

    /// After a segment's name: `<`, `.` and another segment, or `;`.
    fn after_name(&mut self, class: &mut ClassTypeSignature<'a>) -> Result<After> {
        loop {
            match self.peek() {
                Some(b'<') => {
                    self.pos += 1;
                    return Ok(After::Arguments);
                }
                Some(b'.') => {
                    self.pos += 1;
                    let name = self.identifier(false)?;
                    class.segments.push(SimpleClassTypeSignature {
                        name,
                        arguments: Vec::new(),
                    });
                }
                Some(b';') => {
                    self.pos += 1;
                    return Ok(After::End);
                }
                _ => return Err(self.error("`<`, `.`, or `;` expected")),
            }
        }
    }

    /// After a segment's type arguments: `.` and another segment, or `;`.
    fn after_arguments(&mut self, class: &mut ClassTypeSignature<'a>) -> Result<After> {
        match self.peek() {
            Some(b'.') => self.after_name(class),
            Some(b';') => {
                self.pos += 1;
                Ok(After::End)
            }
            _ => Err(self.error("`.` or `;` expected")),
        }
    }

    /// Moves to the next type argument of `frame`, adding unbounded wildcards on the way.
    fn next_argument(&mut self, frame: &mut Frame<'a>) -> Result<Next> {
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.pos += 1;
                    frame.arguments().push(TypeArgument::Any);
                }
                Some(b'>') => {
                    if frame.arguments().is_empty() {
                        return Err(self.error("empty type arguments"));
                    }
                    self.pos += 1;
                    return Ok(Next::Close);
                }
                Some(c @ (b'+' | b'-')) => {
                    self.pos += 1;
                    frame.wildcard = c;
                    return Ok(Next::Type);
                }
                Some(_) => return Ok(Next::Type),
                None => return Err(self.error("unclosed type arguments")),
            }
        }
    }

    /// A reference type, or any Java type when `base` is true.
    fn reference(&mut self, base: bool) -> Result<TypeSignature<'a>> {
        let mut stack: Vec<Frame<'a>> = Vec::new();
        'types: loop {
            let dimensions = self.dimensions()?;
            let mut done = match self.peek() {
                Some(b'T') => {
                    self.pos += 1;
                    let name = self.identifier(false)?;
                    self.expect(b';')?;
                    wrap(TypeSignature::TypeVariable(name), dimensions)
                }
                Some(b'L') => {
                    self.pos += 1;
                    let name = self.identifier(true)?;
                    let mut class = ClassTypeSignature {
                        segments: vec![SimpleClassTypeSignature {
                            name,
                            arguments: Vec::new(),
                        }],
                    };
                    match self.after_name(&mut class)? {
                        After::End => wrap(TypeSignature::Class(class), dimensions),
                        After::Arguments => {
                            if stack.len() >= MAX_NESTING {
                                return Err(self.error("type arguments nest more than 1000 levels"));
                            }
                            let mut frame = Frame {
                                class,
                                dimensions,
                                wildcard: 0,
                            };
                            if self.advance(&mut frame)? {
                                stack.push(frame);
                                continue 'types;
                            }
                            wrap(TypeSignature::Class(frame.class), frame.dimensions)
                        }
                    }
                }
                Some(c) => match BaseType::from_char(c) {
                    Some(primitive) if dimensions > 0 || (base && stack.is_empty()) => {
                        self.pos += 1;
                        wrap(TypeSignature::Base(primitive), dimensions)
                    }
                    _ => return Err(self.error("a type expected")),
                },
                None => return Err(self.error("a type expected")),
            };
            // Give the finished type to the class whose argument it is, closing classes whose
            // arguments end.
            loop {
                let Some(frame) = stack.last_mut() else {
                    return Ok(done);
                };
                let argument = match std::mem::take(&mut frame.wildcard) {
                    b'+' => TypeArgument::Extends(done),
                    b'-' => TypeArgument::Super(done),
                    _ => TypeArgument::Exact(done),
                };
                frame.arguments().push(argument);
                if self.advance(frame)? {
                    continue 'types;
                }
                let Some(frame) = stack.pop() else {
                    unreachable!("the frame is on the stack");
                };
                done = wrap(TypeSignature::Class(frame.class), frame.dimensions);
            }
        }
    }

    /// Moves `frame` to its next type argument: true when a type follows, false when the class
    /// type ended (`;`), through any number of unbounded wildcards, `>`, and inner classes.
    fn advance(&mut self, frame: &mut Frame<'a>) -> Result<bool> {
        loop {
            match self.next_argument(frame)? {
                Next::Type => return Ok(true),
                Next::Close => match self.after_arguments(&mut frame.class)? {
                    After::Arguments => continue,
                    After::End => return Ok(false),
                },
            }
        }
    }

    fn class_type(&mut self) -> Result<ClassTypeSignature<'a>> {
        let start = self.pos;
        match self.reference(false)? {
            TypeSignature::Class(class) => Ok(class),
            _ => Err(self.error_at(start, "a class type expected")),
        }
    }

    fn type_parameters(&mut self) -> Result<Vec<TypeParameter<'a>>> {
        let mut parameters = Vec::new();
        if self.peek() != Some(b'<') {
            return Ok(parameters);
        }
        self.pos += 1;
        loop {
            let name = self.identifier(false)?;
            self.expect(b':')?;
            let class_bound = match self.peek() {
                Some(b':' | b'>') => None,
                _ => Some(self.reference(false)?),
            };
            let mut interface_bounds = Vec::new();
            while self.peek() == Some(b':') {
                self.pos += 1;
                interface_bounds.push(self.reference(false)?);
            }
            parameters.push(TypeParameter {
                name,
                class_bound,
                interface_bounds,
            });
            if self.peek() == Some(b'>') {
                self.pos += 1;
                return Ok(parameters);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str) -> TypeSignature<'_> {
        TypeSignature::Class(ClassTypeSignature {
            segments: vec![SimpleClassTypeSignature {
                name,
                arguments: vec![],
            }],
        })
    }

    #[test]
    fn class_signatures() {
        let text = "<T::Ljava/lang/Comparable<-TT;>;:Ljava/io/Serializable;>Ljava/util/AbstractList<Ljava/lang/String;>;Ljava/util/RandomAccess;";
        let sig = parse_class(text).unwrap();
        assert_eq!(sig.to_string(), text);
        assert_eq!(sig.type_parameters.len(), 1);
        let t = &sig.type_parameters[0];
        assert_eq!(t.name, "T");
        assert_eq!(t.class_bound, None);
        assert_eq!(t.interface_bounds.len(), 2);
        let TypeSignature::Class(comparable) = &t.interface_bounds[0] else {
            panic!()
        };
        assert_eq!(
            comparable.segments[0].arguments,
            [TypeArgument::Super(TypeSignature::TypeVariable("T"))]
        );
        assert_eq!(sig.superclass.erasure(), "java/util/AbstractList");
        assert_eq!(sig.interfaces.len(), 1);
    }

    #[test]
    fn inner_classes_and_wildcards() {
        let field =
            parse_field("[[Ljava/util/Map<TK;*>.Entry<Ljava/lang/String;+[I>.Deeper;").unwrap();
        let TypeSignature::Array {
            dimensions: 2,
            element,
        } = field
        else {
            panic!()
        };
        let TypeSignature::Class(class) = *element else {
            panic!()
        };
        assert_eq!(class.erasure(), "java/util/Map$Entry$Deeper");
        assert_eq!(
            class.segments[0].arguments,
            [
                TypeArgument::Exact(TypeSignature::TypeVariable("K")),
                TypeArgument::Any
            ]
        );
        assert_eq!(
            class.segments[1].arguments[0],
            TypeArgument::Exact(class_of("java/lang/String"))
        );
        assert!(matches!(
            &class.segments[1].arguments[1],
            TypeArgument::Extends(TypeSignature::Array { dimensions: 1, .. })
        ));
        assert!(class.segments[2].arguments.is_empty());
    }

    fn class_of(name: &str) -> TypeSignature<'_> {
        class(name)
    }

    #[test]
    fn method_signatures() {
        let sig =
            parse_method("<U:Ljava/lang/Number;>(ITU;[Z)TU;^Ljava/io/IOException;^TX;").unwrap();
        assert_eq!(
            sig.type_parameters[0].class_bound,
            Some(class("java/lang/Number"))
        );
        assert_eq!(sig.parameters.len(), 3);
        assert_eq!(sig.result, Some(TypeSignature::TypeVariable("U")));
        assert_eq!(sig.throws.len(), 2);
        let sig = parse_method("()V").unwrap();
        assert_eq!(sig.result, None);
        // Arguments that are all unbounded wildcards, also on an inner class (seen in the JDK).
        for text in [
            "(Ljava/lang/Class<*>;)Ljava/lang/Class<*>;",
            "()Ljava/util/Set<Lp/Option<*>;>;",
            "()Lp/A<*>.B<**>.C;",
        ] {
            assert_eq!(parse_method(text).unwrap().to_string(), text);
        }
        // The class bound is optional (`T:` alone).
        let sig = parse_class("<T:>Ljava/lang/Object;").unwrap();
        assert_eq!(sig.type_parameters[0].class_bound, None);
    }

    #[test]
    fn malformed() {
        for bad in [
            "",
            "I",
            "Ljava/lang/Object",
            "L;",
            "Ljava//Object;",
            "Lp/A<>;",
            "Lp/A<I>;",
            "TT",
            "T;",
            "Lp/A<Lp/B;",
            "Lp/A;;",
            "Lp/A.;",
            "[",
        ] {
            assert!(parse_field(bad).is_err(), "{bad}");
        }
        for bad in [
            "",
            "(",
            "()",
            "()VV",
            "(V)V",
            "()V^I",
            "()V^[Ljava/lang/Error;",
            "<>()V",
            "<T>()V",
        ] {
            assert!(parse_method(bad).is_err(), "{bad}");
        }
        for bad in [
            "",
            "I",
            "TT;",
            "<T>Ljava/lang/Object;",
            "Ljava/lang/Object;I",
        ] {
            assert!(parse_class(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let depth = MAX_NESTING;
        let ok = "Lp/A<".repeat(depth - 1) + "Lp/A;" + &">;".repeat(depth - 1);
        assert!(parse_field(&ok).is_ok());
        let deep = "Lp/A<".repeat(depth + 1) + "Lp/A;" + &">;".repeat(depth + 1);
        assert!(parse_field(&deep).is_err());
        let huge = "Lp/A<".repeat(100_000);
        assert!(parse_field(&huge).is_err());
    }
}
