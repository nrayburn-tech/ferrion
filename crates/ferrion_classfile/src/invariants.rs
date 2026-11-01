//! Checks that the class file reader keeps the promises its API documents, for the fuzz targets
//! and the tests. Not part of the public API.
//!
//! Each check reads its input with every accessor, so a panic anywhere is found, and fails with a
//! message when a documented promise is broken: an entry the index lists that then can't be found,
//! a read whose size differs from the declared one without an error, or a signature that doesn't
//! print back as its text.

use std::sync::Arc;

use crate::class::{Annotation, Attribute, Attributes, ClassFile, ElementValue};
use crate::text::{Name, decode_utf16, encode_utf16};
use crate::{descriptor, signature};

type Check = Result<(), String>;

/// Reads `bytes` as a class file and calls every accessor.
pub fn check_class(bytes: &[u8]) -> Check {
    let Ok(class) = ClassFile::parse(Arc::from(bytes)) else {
        return Ok(());
    };
    let _ = class.version().is_preview();
    let _ = class.access();
    let _ = class.trailing_bytes();
    for index in 0..class.constant_pool_count() {
        if let Ok(crate::class::Constant::Utf8(name)) = class.constant(index) {
            check_name(name)?;
        }
    }
    if let Ok(name) = class.this_class() {
        check_name(name)?;
    }
    let _ = class.super_class();
    for interface in class.interfaces() {
        let _ = interface;
    }
    let signature = class.signature();
    if let Ok(Some(text)) = signature
        && let Ok(text) = text.to_str()
    {
        check_signature_kind(&text, Kind::Class)?;
    }
    check_attributes(class.attributes())?;
    for (members, kind) in [
        (class.fields(), Kind::Field),
        (class.methods(), Kind::Method),
    ] {
        let expected = members.len();
        let mut count = 0;
        for member in members {
            count += 1;
            let _ = member.access();
            let _ = member.name().map(check_name);
            if let Ok(text) = member
                .descriptor()
                .and_then(|d| d.to_str().map(|s| s.into_owned()))
            {
                match kind {
                    Kind::Field => {
                        round_trip(&text, descriptor::parse_field(&text).map(|d| d.to_string()))?
                    }
                    _ => round_trip(
                        &text,
                        descriptor::parse_method(&text).map(|d| d.to_string()),
                    )?,
                }
            }
            if let Ok(Some(text)) = member.signature()
                && let Ok(text) = text.to_str()
            {
                check_signature_kind(&text, kind)?;
            }
            check_attributes(member.attributes())?;
        }
        if count != expected {
            return Err(format!(
                "members iterated {count} times, but the length said {expected}"
            ));
        }
    }
    if class.access().contains(crate::class::AccessFlags::MODULE) {
        let _ = class.module_descriptor();
    }
    Ok(())
}

fn round_trip(text: &str, printed: crate::Result<String>) -> Check {
    match printed {
        Ok(printed) if printed != text => {
            Err(format!("`{text}` parsed and printed as `{printed}`"))
        }
        _ => Ok(()),
    }
}

/// A name that decodes compares equal to its text, and its UTF-16 encodes back to the same
/// units.
fn check_name(name: Name<'_>) -> Check {
    if let Ok(text) = name.to_str()
        && !name.eq_str(&text)
    {
        return Err(format!("{name:?} isn't eq_str to its own text"));
    }
    if let Ok(units) = decode_utf16(name.as_bytes()) {
        let again = decode_utf16(&encode_utf16(&units)).map_err(|e| e.to_string())?;
        if again != units {
            return Err(format!("{name:?} doesn't survive encoding its UTF-16"));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Kind {
    Class,
    Field,
    Method,
}

fn check_signature_kind(text: &str, kind: Kind) -> Check {
    match kind {
        Kind::Class => round_trip(text, signature::parse_class(text).map(|s| s.to_string())),
        Kind::Method => round_trip(text, signature::parse_method(text).map(|s| s.to_string())),
        Kind::Field => round_trip(text, signature::parse_field(text).map(|s| s.to_string())),
    }
}

/// Parses `text` as each kind of signature and descriptor; whatever parses must print back as
/// `text`.
pub fn check_signature(text: &str) -> Check {
    for kind in [Kind::Class, Kind::Field, Kind::Method] {
        check_signature_kind(text, kind)?;
    }
    round_trip(text, descriptor::parse_field(text).map(|d| d.to_string()))?;
    round_trip(text, descriptor::parse_method(text).map(|d| d.to_string()))
}

fn check_attributes(attributes: Attributes<'_>) -> Check {
    let expected = attributes.len();
    let mut count = 0;
    for info in attributes {
        count += 1;
        let _ = info.name();
        let Ok(attribute) = info.decode() else {
            continue;
        };
        check_attribute(attribute)?;
    }
    if count != expected {
        return Err(format!(
            "attributes iterated {count} times, but the length said {expected}"
        ));
    }
    Ok(())
}

fn check_attribute(attribute: Attribute<'_>) -> Check {
    match attribute {
        Attribute::ConstantValue(crate::class::ConstantValue::String(text)) => {
            let _ = text.to_utf16();
            let _ = text.to_string_lossy();
        }
        Attribute::Exceptions(refs)
        | Attribute::NestMembers(refs)
        | Attribute::PermittedSubclasses(refs)
        | Attribute::ModulePackages(refs) => {
            let expected = refs.len();
            if refs.count() != expected {
                return Err("a reference list's length is wrong".to_owned());
            }
        }
        Attribute::InnerClasses(entries) => entries.for_each(drop),
        Attribute::Annotations { annotations, .. } => {
            for annotation in annotations.flatten() {
                check_annotation(&annotation)?;
            }
        }
        Attribute::ParameterAnnotations { parameters, .. } => {
            for annotations in parameters.flatten() {
                for annotation in annotations.flatten() {
                    check_annotation(&annotation)?;
                }
            }
        }
        Attribute::TypeAnnotations { annotations, .. } => {
            for annotation in annotations.flatten() {
                let _ = annotation.target.target_type();
                check_annotation(&annotation.annotation)?;
            }
        }
        Attribute::AnnotationDefault(value) => check_value(value)?,
        Attribute::Record(components) => {
            for component in components {
                let _ = component.name();
                if let Ok(text) = component
                    .descriptor()
                    .and_then(|d| d.to_str().map(|s| s.into_owned()))
                {
                    round_trip(&text, descriptor::parse_field(&text).map(|d| d.to_string()))?;
                }
                if let Ok(Some(text)) = component.signature()
                    && let Ok(text) = text.to_str()
                {
                    check_signature_kind(&text, Kind::Field)?;
                }
                check_attributes(component.attributes())?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Walks an annotation's values with an explicit stack, as deep as they go.
fn check_annotation(annotation: &Annotation<'_>) -> Check {
    let _ = annotation.type_descriptor();
    for (name, value) in annotation.elements().flatten() {
        let _ = name.to_str();
        check_value(value)?;
    }
    Ok(())
}

fn check_value(value: ElementValue<'_>) -> Check {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            ElementValue::String(text) => {
                let _ = text.to_string_lossy();
            }
            ElementValue::Annotation(annotation) => {
                let _ = annotation.type_descriptor();
                stack.extend(annotation.elements().flatten().map(|(_, v)| v));
            }
            ElementValue::Array(values) => {
                let expected = values.len();
                let items: Vec<_> = values.collect();
                if items.len() > expected {
                    return Err("an array yielded more values than its length".to_owned());
                }
                stack.extend(items.into_iter().flatten());
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ClassBuilder;

    #[test]
    fn checks_pass_on_valid_input() {
        let mut b = ClassBuilder::new("p/A");
        let sig = b.utf8("<T:Ljava/lang/Object;>Ljava/lang/Object;");
        let attribute = b.attribute("Signature", &sig.to_be_bytes());
        b.class_attribute(attribute);
        b.field(0, "f", "[[Ljava/lang/String;", &[]);
        b.method(0, "m", "(IJ)V", &[]);
        check_class(&b.finish()).unwrap();
        check_signature("Ljava/util/Map<TK;+[I>.Entry;").unwrap();
    }
}
