use std::sync::Arc;

use super::*;
use crate::error::ErrorKind;
use crate::testing::ClassBuilder;

fn parse(bytes: Vec<u8>) -> ClassFile {
    ClassFile::parse(Arc::from(bytes)).unwrap()
}

fn u2(value: u16) -> [u8; 2] {
    value.to_be_bytes()
}

#[test]
fn header_and_members() {
    let mut b = ClassBuilder::new("p/A");
    b.interface("java/lang/Runnable");
    b.interface("java/io/Serializable");
    let value = b.integer(42);
    let constant = b.attribute("ConstantValue", &u2(value));
    b.field(0x0019, "X", "I", &[constant]);
    let sig = b.utf8("()TT;");
    let signature = b.attribute("Signature", &u2(sig));
    let code = b.attribute("Code", &[0; 12]);
    b.method(0x0001, "get", "()Ljava/lang/Object;", &[code, signature]);
    let source = b.utf8("A.java");
    let source = b.attribute("SourceFile", &u2(source));
    b.class_attribute(source);
    let class = parse(b.finish());

    assert_eq!(
        class.version(),
        Version {
            major: 71,
            minor: 0
        }
    );
    assert!(!class.version().is_preview());
    assert!(
        class
            .access()
            .contains(AccessFlags::PUBLIC | AccessFlags::SUPER)
    );
    assert!(class.this_class().unwrap().eq_str("p/A"));
    assert!(
        class
            .super_class()
            .unwrap()
            .unwrap()
            .eq_str("java/lang/Object")
    );
    let interfaces: Vec<_> = class
        .interfaces()
        .map(|i| i.unwrap().to_str().unwrap().into_owned())
        .collect();
    assert_eq!(interfaces, ["java/lang/Runnable", "java/io/Serializable"]);

    let field = class.fields().next().unwrap();
    assert!(field.name().unwrap().eq_str("X"));
    let Attribute::ConstantValue(ConstantValue::Int(42)) =
        field.attributes().next().unwrap().decode().unwrap()
    else {
        panic!("expected a constant");
    };
    let method = class.methods().next().unwrap();
    assert_eq!(method.attributes().len(), 2);
    assert!(method.signature().unwrap().unwrap().eq_str("()TT;"));
    // Code is kept, not decoded.
    let code = method.attributes().named("Code").unwrap();
    assert!(matches!(code.decode().unwrap(), Attribute::Other { bytes, .. } if bytes.len() == 12));
    assert!(
        matches!(class.attributes().next().unwrap().decode().unwrap(), Attribute::SourceFile(n) if n.eq_str("A.java"))
    );
    class.trailing_bytes().unwrap();
}

#[test]
fn object_and_module_info_have_no_superclass() {
    let mut b = ClassBuilder::new("java/lang/Object");
    b.set_super(None);
    let class = parse(b.finish());
    assert_eq!(class.super_class().unwrap(), None);
}

#[test]
fn versions() {
    let mut b = ClassBuilder::new("p/A");
    b.major = MAX_MAJOR_VERSION + 1;
    let error = ClassFile::parse(Arc::from(b.finish())).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    b.major = 44;
    assert_eq!(
        ClassFile::parse(Arc::from(b.finish())).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    b.major = 45;
    b.minor = 3;
    assert_eq!(
        parse(b.finish()).version(),
        Version {
            major: 45,
            minor: 3
        }
    );
    b.major = 69;
    b.minor = 0xffff;
    assert!(parse(b.finish()).version().is_preview());
}

#[test]
fn constants() {
    let mut b = ClassBuilder::new("p/A");
    let long = b.long(-5);
    let double = b.double(f64::NAN.to_bits() | 1);
    let float = b.float((-0.0f32).to_bits());
    let string = b.string("a\0b");
    let nat = b.name_and_type("m", "()V");
    let module = b.module("java.base");
    let package = b.package("java/lang");
    let class = parse(b.finish());
    assert_eq!(class.constant(long).unwrap(), Constant::Long(-5));
    // The slot after a long can't be used.
    assert_eq!(
        class.constant(long + 1).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    assert_eq!(
        class.constant(double).unwrap(),
        Constant::Double(f64::NAN.to_bits() | 1)
    );
    assert_eq!(class.constant(float).unwrap(), Constant::Float(0x8000_0000));
    assert_eq!(
        class.string(string).unwrap().to_utf16().unwrap(),
        [0x61, 0, 0x62]
    );
    assert!(matches!(
        class.constant(nat).unwrap(),
        Constant::NameAndType { .. }
    ));
    assert!(class.module_name(module).unwrap().eq_str("java.base"));
    assert!(class.package_name(package).unwrap().eq_str("java/lang"));
    // Wrong kinds and bad indexes are errors, not panics.
    assert_eq!(
        class.class_name(string).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    assert_eq!(class.utf8(0).unwrap_err().kind(), ErrorKind::Malformed);
    assert_eq!(class.utf8(60000).unwrap_err().kind(), ErrorKind::Malformed);
}

#[test]
fn malformed_structures() {
    let good = ClassBuilder::new("p/A").finish();
    // Every truncation is an error.
    for len in 0..good.len() {
        let error = ClassFile::parse(Arc::from(&good[..len])).unwrap_err();
        assert_eq!(
            error.kind(),
            if len < 8 {
                ErrorKind::Malformed
            } else {
                error.kind()
            },
            "{len}"
        );
    }
    // Bad magic.
    let mut bad = good.clone();
    bad[0] = 0;
    assert_eq!(
        ClassFile::parse(Arc::from(bad)).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    // An unknown constant pool tag.
    let mut bad = good.clone();
    bad[10] = 2;
    assert_eq!(
        ClassFile::parse(Arc::from(bad)).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    // Trailing bytes leave the class usable.
    let mut extra = good.clone();
    extra.push(0);
    let class = parse(extra);
    assert!(class.this_class().unwrap().eq_str("p/A"));
    assert_eq!(
        class.trailing_bytes().unwrap_err().kind(),
        ErrorKind::Malformed
    );
    // A long in the last slot.
    let mut b = ClassBuilder::new("p/A");
    b.long(1);
    let mut bytes = b.finish();
    let count = u16::from_be_bytes([bytes[8], bytes[9]]) - 1;
    bytes[8..10].copy_from_slice(&count.to_be_bytes());
    assert!(ClassFile::parse(Arc::from(bytes)).is_err());
}

#[test]
fn attribute_lengths_are_exact() {
    let mut b = ClassBuilder::new("p/A");
    let source = b.utf8("A.java");
    let mut contents = u2(source).to_vec();
    contents.push(0);
    let attribute = b.attribute("SourceFile", &contents);
    b.class_attribute(attribute);
    let deprecated = b.attribute("Deprecated", &[1]);
    b.class_attribute(deprecated);
    let class = parse(b.finish());
    for info in class.attributes() {
        assert_eq!(info.decode().unwrap_err().kind(), ErrorKind::Malformed);
    }
}

#[test]
fn inner_classes_and_nests() {
    let mut b = ClassBuilder::new("p/A");
    let inner = b.class("p/A$B");
    let outer = b.class("p/A");
    let name = b.utf8("B");
    let anonymous = b.class("p/A$1");
    let mut contents = u2(2).to_vec();
    for value in [inner, outer, name, 0x0018, anonymous, 0, 0, 0] {
        contents.extend_from_slice(&u2(value));
    }
    let attribute = b.attribute("InnerClasses", &contents);
    b.class_attribute(attribute);
    let mut members = u2(2).to_vec();
    members.extend_from_slice(&u2(inner));
    members.extend_from_slice(&u2(anonymous));
    let attribute = b.attribute("NestMembers", &members);
    b.class_attribute(attribute);
    let attribute = b.attribute("PermittedSubclasses", &members);
    b.class_attribute(attribute);
    let class = parse(b.finish());
    let mut attributes = class.attributes();
    let Attribute::InnerClasses(entries) = attributes.next().unwrap().decode().unwrap() else {
        panic!()
    };
    let entries: Vec<_> = entries.map(Result::unwrap).collect();
    assert!(entries[0].inner.eq_str("p/A$B"));
    assert!(entries[0].outer.unwrap().eq_str("p/A"));
    assert!(entries[0].name.unwrap().eq_str("B"));
    assert_eq!(entries[0].access, AccessFlags(0x0018));
    assert!(entries[1].outer.is_none() && entries[1].name.is_none());
    let Attribute::NestMembers(nest) = attributes.next().unwrap().decode().unwrap() else {
        panic!()
    };
    assert_eq!(nest.len(), 2);
    let Attribute::PermittedSubclasses(permitted) = attributes.next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    assert_eq!(permitted.map(Result::unwrap).count(), 2);
}

/// Writes an annotation with element-value pairs given as (name index, value bytes).
fn annotation(type_index: u16, pairs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = u2(type_index).to_vec();
    bytes.extend_from_slice(&u2(pairs.len() as u16));
    for (name, value) in pairs {
        bytes.extend_from_slice(&u2(*name));
        bytes.extend_from_slice(value);
    }
    bytes
}

fn tagged(tag: u8, index: u16) -> Vec<u8> {
    let mut bytes = vec![tag];
    bytes.extend_from_slice(&u2(index));
    bytes
}

#[test]
fn annotations_and_element_values() {
    let mut b = ClassBuilder::new("p/A");
    let ty = b.utf8("Lp/Ann;");
    let nested_ty = b.utf8("Lp/Nested;");
    let names: Vec<u16> = ["b", "c", "s", "e", "k", "n", "a", "j"]
        .iter()
        .map(|n| b.utf8(n))
        .collect();
    let one = b.integer(-1);
    let text = b.utf8("hi");
    let enum_ty = b.utf8("Lp/E;");
    let enum_name = b.utf8("X");
    let class_lit = b.utf8("Ljava/util/List;");
    let long = b.long(i64::MIN);
    let mut enum_value = vec![b'e'];
    enum_value.extend_from_slice(&u2(enum_ty));
    enum_value.extend_from_slice(&u2(enum_name));
    let mut nested = vec![b'@'];
    nested.extend_from_slice(&annotation(nested_ty, &[]));
    let mut array = vec![b'['];
    array.extend_from_slice(&u2(2));
    array.extend_from_slice(&tagged(b'I', one));
    array.extend_from_slice(&tagged(b'Z', one));
    let pairs = [
        (names[0], tagged(b'B', one)),
        (names[1], tagged(b'C', one)),
        (names[2], tagged(b's', text)),
        (names[3], enum_value),
        (names[4], tagged(b'c', class_lit)),
        (names[5], nested),
        (names[6], array),
        (names[7], tagged(b'J', long)),
    ];
    let mut contents = u2(1).to_vec();
    contents.extend_from_slice(&annotation(ty, &pairs));
    let attribute = b.attribute("RuntimeInvisibleAnnotations", &contents);
    b.class_attribute(attribute);
    let class = parse(b.finish());
    let Attribute::Annotations {
        visible: false,
        annotations,
    } = class.attributes().next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    let annotations: Vec<_> = annotations.map(Result::unwrap).collect();
    assert_eq!(annotations.len(), 1);
    assert!(annotations[0].type_descriptor().unwrap().eq_str("Lp/Ann;"));
    let values: Vec<_> = annotations[0].elements().map(Result::unwrap).collect();
    assert_eq!(values.len(), 8);
    assert!(matches!(values[0].1, ElementValue::Byte(-1)));
    assert!(matches!(values[1].1, ElementValue::Char(-1)));
    assert!(matches!(&values[2].1, ElementValue::String(s) if s.to_string_lossy() == "hi"));
    assert!(matches!(&values[3].1, ElementValue::Enum { name, .. } if name.eq_str("X")));
    assert!(matches!(&values[4].1, ElementValue::Class(c) if c.eq_str("Ljava/util/List;")));
    let ElementValue::Annotation(inner) = &values[5].1 else {
        panic!()
    };
    assert!(inner.type_descriptor().unwrap().eq_str("Lp/Nested;"));
    let ElementValue::Array(items) = values[6].1.clone() else {
        panic!()
    };
    let items: Vec<_> = items.map(Result::unwrap).collect();
    assert!(matches!(
        items[..],
        [ElementValue::Int(-1), ElementValue::Boolean(-1)]
    ));
    assert!(matches!(values[7].1, ElementValue::Long(i64::MIN)));
}

#[test]
fn element_values_check_constant_kinds() {
    let mut b = ClassBuilder::new("p/A");
    let ty = b.utf8("Lp/Ann;");
    let name = b.utf8("v");
    let long = b.long(1);
    let mut contents = u2(1).to_vec();
    contents.extend_from_slice(&annotation(ty, &[(name, tagged(b'I', long))]));
    let attribute = b.attribute("RuntimeVisibleAnnotations", &contents);
    b.class_attribute(attribute);
    let class = parse(b.finish());
    let Attribute::Annotations { annotations, .. } =
        class.attributes().next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    let annotation = annotations.into_iter().next().unwrap().unwrap();
    assert_eq!(
        annotation.elements().next().unwrap().unwrap_err().kind(),
        ErrorKind::Malformed
    );
}

#[test]
fn deep_annotations_are_an_error() {
    for (depth, ok) in [
        (annotation::MAX_NESTING, true),
        (annotation::MAX_NESTING + 1, false),
    ] {
        let mut b = ClassBuilder::new("p/A");
        let ty = b.utf8("Lp/Ann;");
        let name = b.utf8("v");
        // `depth` arrays, each holding the next.
        let mut value = tagged(b's', name);
        for _ in 0..depth {
            let mut array = vec![b'['];
            array.extend_from_slice(&u2(1));
            array.extend_from_slice(&value);
            value = array;
        }
        let mut contents = u2(1).to_vec();
        contents.extend_from_slice(&annotation(ty, &[(name, value)]));
        let attribute = b.attribute("RuntimeVisibleAnnotations", &contents);
        b.class_attribute(attribute);
        let class = parse(b.finish());
        let decoded = class.attributes().next().unwrap().decode();
        assert_eq!(decoded.is_ok(), ok, "{depth}");
    }
}

#[test]
fn type_annotations() {
    let mut b = ClassBuilder::new("p/A");
    let ty = b.utf8("Lp/TA;");
    let mut contents = u2(3).to_vec();
    // Supertype 65535 (the superclass), path [ARRAY, TYPE_ARGUMENT(1)].
    contents.extend_from_slice(&[0x10, 0xff, 0xff, 2, 0, 0, 3, 1]);
    contents.extend_from_slice(&annotation(ty, &[]));
    // A method type parameter bound.
    contents.extend_from_slice(&[0x12, 1, 2, 1, 2, 0]);
    contents.extend_from_slice(&annotation(ty, &[]));
    // A receiver, with a nested step.
    contents.extend_from_slice(&[0x15, 1, 1, 0]);
    contents.extend_from_slice(&annotation(ty, &[]));
    let attribute = b.attribute("RuntimeVisibleTypeAnnotations", &contents);
    b.class_attribute(attribute);
    let class = parse(b.finish());
    let Attribute::TypeAnnotations {
        visible: true,
        annotations,
    } = class.attributes().next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    let annotations: Vec<_> = annotations.map(Result::unwrap).collect();
    assert_eq!(
        annotations[0].target,
        TypeAnnotationTarget::Supertype { index: 65535 }
    );
    assert_eq!(
        annotations[0].path,
        [TypePathStep::Array, TypePathStep::TypeArgument(1)]
    );
    assert_eq!(
        annotations[1].target,
        TypeAnnotationTarget::MethodTypeParameterBound {
            type_parameter: 1,
            bound: 2
        }
    );
    assert_eq!(annotations[1].path, [TypePathStep::Wildcard]);
    assert_eq!(annotations[2].target, TypeAnnotationTarget::Receiver);
    assert_eq!(annotations[2].path, [TypePathStep::Nested]);

    // Targets that belong in Code, and bad paths, are malformed.
    for bad in [&[0x40u8, 0, 0, 0][..], &[0x13, 1, 4, 0], &[0x13, 1, 0, 1]] {
        let mut b = ClassBuilder::new("p/A");
        let ty = b.utf8("Lp/TA;");
        let mut contents = u2(1).to_vec();
        contents.extend_from_slice(bad);
        contents.extend_from_slice(&annotation(ty, &[]));
        let attribute = b.attribute("RuntimeInvisibleTypeAnnotations", &contents);
        b.class_attribute(attribute);
        let class = parse(b.finish());
        assert_eq!(
            class
                .attributes()
                .next()
                .unwrap()
                .decode()
                .unwrap_err()
                .kind(),
            ErrorKind::Malformed
        );
    }
}

#[test]
fn parameter_annotations_and_method_parameters() {
    let mut b = ClassBuilder::new("p/A");
    let ty = b.utf8("Lp/Ann;");
    let mut contents = vec![2];
    contents.extend_from_slice(&u2(1));
    contents.extend_from_slice(&annotation(ty, &[]));
    contents.extend_from_slice(&u2(0));
    let annotations = b.attribute("RuntimeVisibleParameterAnnotations", &contents);
    let x = b.utf8("x");
    let mut parameters = vec![2];
    parameters.extend_from_slice(&u2(x));
    parameters.extend_from_slice(&u2(0x0010));
    parameters.extend_from_slice(&u2(0));
    parameters.extend_from_slice(&u2(0x9000));
    let parameters = b.attribute("MethodParameters", &parameters);
    b.method(0, "<init>", "(II)V", &[annotations, parameters]);
    let class = parse(b.finish());
    let method = class.methods().next().unwrap();
    let mut attributes = method.attributes();
    let Attribute::ParameterAnnotations { parameters, .. } =
        attributes.next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    let counts: Vec<_> = parameters.map(|p| p.unwrap().count()).collect();
    assert_eq!(counts, [1, 0]);
    let Attribute::MethodParameters(parameters) = attributes.next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    assert!(parameters[0].name.unwrap().eq_str("x"));
    assert_eq!(parameters[0].access, AccessFlags(0x0010));
    assert!(parameters[1].name.is_none());
    assert!(
        parameters[1]
            .access
            .contains(AccessFlags::MANDATED | AccessFlags::SYNTHETIC)
    );
}

#[test]
fn records_and_defaults() {
    let mut b = ClassBuilder::new("p/R");
    let sig = b.utf8("Ljava/util/List<Ljava/lang/String;>;");
    let signature = b.attribute("Signature", &u2(sig));
    let x = b.utf8("x");
    let desc = b.utf8("Ljava/util/List;");
    let mut contents = u2(1).to_vec();
    contents.extend_from_slice(&u2(x));
    contents.extend_from_slice(&u2(desc));
    contents.extend_from_slice(&u2(1));
    contents.extend_from_slice(&signature);
    let record = b.attribute("Record", &contents);
    b.class_attribute(record);
    let value = b.integer(3);
    let default = b.attribute("AnnotationDefault", &tagged(b'S', value));
    b.method(0x0401, "v", "()S", &[default]);
    let class = parse(b.finish());
    let Attribute::Record(mut components) = class.attributes().next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    let component = components.next().unwrap();
    assert!(component.name().unwrap().eq_str("x"));
    assert!(component.descriptor().unwrap().eq_str("Ljava/util/List;"));
    assert!(
        component
            .signature()
            .unwrap()
            .unwrap()
            .eq_str("Ljava/util/List<Ljava/lang/String;>;")
    );
    assert!(components.next().is_none());
    let method = class.methods().next().unwrap();
    let Attribute::AnnotationDefault(ElementValue::Short(3)) =
        method.attributes().next().unwrap().decode().unwrap()
    else {
        panic!()
    };
}

#[test]
fn module_descriptors() {
    let mut b = ClassBuilder::new("module-info");
    b.set_super(None);
    b.access = AccessFlags::MODULE;
    let name = b.module("m.one");
    let base = b.module("java.base");
    let other = b.module("m.two");
    let version = b.utf8("1.0");
    let api = b.package("m/one/api");
    let internal = b.package("m/one/internal");
    let service = b.class("m/one/api/Service");
    let implementation = b.class("m/one/internal/Impl");
    let mut c = Vec::new();
    for value in [name, 0x0020, version] {
        c.extend_from_slice(&u2(value));
    }
    c.extend_from_slice(&u2(1));
    for value in [base, 0x8000, 0] {
        c.extend_from_slice(&u2(value));
    }
    c.extend_from_slice(&u2(2)); // exports
    for value in [api, 0, 0, internal, 0, 1, other] {
        c.extend_from_slice(&u2(value));
    }
    c.extend_from_slice(&u2(0)); // opens
    c.extend_from_slice(&u2(1));
    c.extend_from_slice(&u2(service)); // uses
    c.extend_from_slice(&u2(1));
    for value in [service, 1, implementation] {
        c.extend_from_slice(&u2(value));
    }
    let module = b.attribute("Module", &c);
    b.class_attribute(module);
    let mut packages = u2(2).to_vec();
    packages.extend_from_slice(&u2(api));
    packages.extend_from_slice(&u2(internal));
    let packages = b.attribute("ModulePackages", &packages);
    b.class_attribute(packages);
    let resolution = b.attribute("ModuleResolution", &u2(9));
    b.class_attribute(resolution);
    let class = parse(b.finish());
    let descriptor = class.module_descriptor().unwrap();
    assert_eq!(descriptor.name, "m.one");
    assert_eq!(descriptor.version.as_deref(), Some("1.0"));
    assert_eq!(descriptor.requires[0].module, "java.base");
    assert_eq!(
        descriptor.unqualified_exports().collect::<Vec<_>>(),
        ["m/one/api"]
    );
    assert_eq!(descriptor.exports[1].to, ["m.two"]);
    assert_eq!(descriptor.uses, ["m/one/api/Service"]);
    assert_eq!(descriptor.provides[0].with, ["m/one/internal/Impl"]);
    assert_eq!(descriptor.packages.as_ref().unwrap().len(), 2);
    assert_eq!(descriptor.resolution, Some(9));
    assert!(!descriptor.is_resolved_by_default());
}

#[test]
fn unknown_attributes_are_kept() {
    let mut b = ClassBuilder::new("p/A");
    let attribute = b.attribute("ScalaSig", &[1, 2, 3]);
    b.class_attribute(attribute);
    let class = parse(b.finish());
    let Attribute::Other { name, bytes } = class.attributes().next().unwrap().decode().unwrap()
    else {
        panic!()
    };
    assert!(name.eq_str("ScalaSig"));
    assert_eq!(bytes, [1, 2, 3]);
}

#[test]
fn send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<ClassFile>();
}

#[test]
fn a_bad_main_class_leaves_the_module_readable() {
    // As in JDK 21's ct.sym, for jdk.httpserver at releases 18 to 20: ModuleMainClass points at
    // a CONSTANT_Utf8, not a CONSTANT_Class.
    let mut b = ClassBuilder::new("module-info");
    b.set_super(None);
    b.access = AccessFlags::MODULE;
    let name = b.module("m");
    let api = b.package("m/api");
    let mut c = Vec::new();
    for value in [name, 0, 0, 0, 1, api, 0, 0, 0, 0, 0] {
        c.extend_from_slice(&u2(value));
    }
    let module = b.attribute("Module", &c);
    b.class_attribute(module);
    let main = b.utf8("m/api/Main");
    let attribute = b.attribute("ModuleMainClass", &u2(main));
    b.class_attribute(attribute);
    let descriptor = parse(b.finish()).module_descriptor().unwrap();
    assert_eq!(
        descriptor.unqualified_exports().collect::<Vec<_>>(),
        ["m/api"]
    );
    assert_eq!(descriptor.main_class, None);
    assert_eq!(descriptor.problems.len(), 1);
}
