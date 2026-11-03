//! Prints Ferrion's class file model the way `javap -v -p` prints a class, after the javap
//! oracle's normalization (tools/javap-oracle: no constant pool, no `Code`, references replaced by
//! what they resolve to, no blank lines). The comparison is then line for line, and a mismatch
//! shows what javap printed.
//!
//! Everything printed comes from the model's accessors, so a part Ferrion reads wrongly, or
//! doesn't read, shows up as a difference. The formats here were learned by running javap on
//! class files written for the purpose and on the JDK's classes.

use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow};
use ferrion_classfile::class::{
    AccessFlags, Annotation, Annotations, Attribute, AttributeInfo, Attributes, ClassFile,
    ConstantValue, ElementValue, Member, TypeAnnotationTarget, TypePathStep, Version,
};
use ferrion_classfile::descriptor::{self, ElementType, FieldType};
use ferrion_classfile::signature::{
    self, ClassTypeSignature, TypeArgument, TypeParameter, TypeSignature,
};
use ferrion_classfile::{JavaStr, Name};

/// Lines being written, with javap's indentation: text containing a line break continues on a
/// new line at the current line's indentation, as javap's own output does.
struct Out {
    lines: Vec<String>,
    indent: usize,
}

impl Out {
    fn line(&mut self, indent: usize, text: &str) {
        self.lines.push(" ".repeat(indent));
        self.indent = indent;
        self.text(text);
    }

    fn text(&mut self, text: &str) {
        let mut parts = text.split('\n');
        if let Some(first) = parts.next() {
            self.lines
                .last_mut()
                .expect("a line was started")
                .push_str(first);
        }
        for part in parts {
            self.lines.push(" ".repeat(self.indent) + part);
        }
    }

    /// A line with a `//` comment at javap's column: the text padded to 40 characters.
    fn commented(&mut self, indent: usize, text: &str, comment: &str) {
        self.line(indent, &format!("{text:<40}// {comment}"));
    }

    fn finish(self) -> Vec<String> {
        self.lines
            .into_iter()
            .map(|line| line.trim_end_matches(' ').to_owned())
            .collect()
    }
}

fn text(name: Name<'_>) -> Result<String> {
    Ok(name.to_str()?.into_owned())
}

fn dotted(name: &str) -> String {
    name.replace('/', ".")
}

/// Renders `class` as javap's normalized lines.
pub fn render(class: &ClassFile) -> Result<Vec<String>> {
    let mut out = Out {
        lines: Vec::new(),
        indent: 0,
    };
    let this = text(class.this_class()?)?;
    let is_module = class.access().contains(AccessFlags::MODULE);
    let is_interface = class.access().contains(AccessFlags::INTERFACE);
    out.line(0, &declaration(class, &this, is_module, is_interface)?);
    let version = class.version();
    out.line(2, &format!("minor version: {}", version.minor));
    out.line(2, &format!("major version: {}", version.major));
    let defined = Defined::at(version);
    out.line(
        2,
        &flags_line(class.access().0, defined.class, &CLASS_FLAGS, true),
    );
    out.line(2, &format!("this_class: {this}"));
    match class.super_class()? {
        Some(name) => out.line(2, &format!("super_class: {}", text(name)?)),
        None => out.line(2, "super_class: #0"),
    }
    out.line(
        2,
        &format!(
            "interfaces: {}, fields: {}, methods: {}, attributes: {}",
            class.interfaces().len(),
            class.fields().len(),
            class.methods().len(),
            class.attributes().len()
        ),
    );
    out.line(0, "{");
    for field in class.fields() {
        member(&mut out, &defined, &this, &field, false, is_interface)?;
    }
    for method in class.methods() {
        member(&mut out, &defined, &this, &method, true, is_interface)?;
    }
    out.line(0, "}");
    attributes(&mut out, class.attributes(), 0)?;
    Ok(out.finish())
}

const CLASS_FLAGS: [(u16, &str); 9] = [
    (0x0001, "ACC_PUBLIC"),
    (0x0010, "ACC_FINAL"),
    (0x0020, "ACC_SUPER"),
    (0x0200, "ACC_INTERFACE"),
    (0x0400, "ACC_ABSTRACT"),
    (0x1000, "ACC_SYNTHETIC"),
    (0x2000, "ACC_ANNOTATION"),
    (0x4000, "ACC_ENUM"),
    (0x8000, "ACC_MODULE"),
];

const FIELD_FLAGS: [(u16, &str); 9] = [
    (0x0001, "ACC_PUBLIC"),
    (0x0002, "ACC_PRIVATE"),
    (0x0004, "ACC_PROTECTED"),
    (0x0008, "ACC_STATIC"),
    (0x0010, "ACC_FINAL"),
    (0x0040, "ACC_VOLATILE"),
    (0x0080, "ACC_TRANSIENT"),
    (0x1000, "ACC_SYNTHETIC"),
    (0x4000, "ACC_ENUM"),
];

const METHOD_FLAGS: [(u16, &str); 12] = [
    (0x0001, "ACC_PUBLIC"),
    (0x0002, "ACC_PRIVATE"),
    (0x0004, "ACC_PROTECTED"),
    (0x0008, "ACC_STATIC"),
    (0x0010, "ACC_FINAL"),
    (0x0020, "ACC_SYNCHRONIZED"),
    (0x0040, "ACC_BRIDGE"),
    (0x0080, "ACC_VARARGS"),
    (0x0100, "ACC_NATIVE"),
    (0x0400, "ACC_ABSTRACT"),
    (0x0800, "ACC_STRICT"),
    (0x1000, "ACC_SYNTHETIC"),
];

/// The flags a class file's version defines at each location, as javap 27 reads them: the ones
/// `java.lang.reflect.AccessFlag` gives the location for the version's `ClassFileFormatVersion`.
/// javap reports any other bit as an error the comparison leaves out (`FLAG_ERROR`).
struct Defined {
    class: u16,
    field: u16,
    method: u16,
}

impl Defined {
    fn at(version: Version) -> Defined {
        // javap reads a preview version (a minor version other than 0, from version 56) as the
        // latest release.
        let major = if version.major >= 56 && version.minor != 0 {
            u16::MAX
        } else {
            version.major
        };
        let since = |first: u16, bits: u16| if major >= first { bits } else { 0 };
        let java5 = AccessFlags::SYNTHETIC | AccessFlags::ENUM;
        // ACC_STRICT is defined for methods from version 46 to 60 (JVMS 4.6).
        let strict = if (46..=60).contains(&major) {
            AccessFlags::STRICT
        } else {
            0
        };
        Defined {
            class: AccessFlags::PUBLIC
                | AccessFlags::FINAL
                | AccessFlags::SUPER
                | AccessFlags::INTERFACE
                | AccessFlags::ABSTRACT
                | since(49, java5 | AccessFlags::ANNOTATION)
                | since(53, AccessFlags::MODULE),
            field: AccessFlags::PUBLIC
                | AccessFlags::PRIVATE
                | AccessFlags::PROTECTED
                | AccessFlags::STATIC
                | AccessFlags::FINAL
                | AccessFlags::VOLATILE
                | AccessFlags::TRANSIENT
                | since(49, java5),
            method: AccessFlags::PUBLIC
                | AccessFlags::PRIVATE
                | AccessFlags::PROTECTED
                | AccessFlags::STATIC
                | AccessFlags::FINAL
                | AccessFlags::SYNCHRONIZED
                | AccessFlags::NATIVE
                | AccessFlags::ABSTRACT
                | strict
                | since(
                    49,
                    AccessFlags::BRIDGE | AccessFlags::VARARGS | AccessFlags::SYNTHETIC,
                ),
        }
    }
}

/// `flags: (0x0021) ACC_PUBLIC, ACC_SUPER`: `flags`, and the names of its bits that `defined`
/// holds. javap prints any other bit in hex, highest first, on a class's line (`undefined_in_hex`)
/// and leaves it out on a field's or a method's.
fn flags_line(flags: u16, defined: u16, names: &[(u16, &str)], undefined_in_hex: bool) -> String {
    let mut line = format!("flags: ({flags:#06x})");
    let mut parts: Vec<String> = names
        .iter()
        .filter(|&&(bit, _)| flags & defined & bit != 0)
        .map(|&(_, name)| name.to_owned())
        .collect();
    if undefined_in_hex {
        let undefined = flags & !defined;
        parts.extend(
            (0..16)
                .rev()
                .map(|shift| 1u16 << shift)
                .filter(|bit| undefined & bit != 0)
                .map(|bit| format!("{bit:#x}")),
        );
    }
    if !parts.is_empty() {
        line.push(' ');
        line.push_str(&parts.join(", "));
    }
    line
}

/// The class's first line: `public class java.util.ArrayList<E> extends ...`.
fn declaration(
    class: &ClassFile,
    this: &str,
    is_module: bool,
    is_interface: bool,
) -> Result<String> {
    if is_module {
        for info in class.attributes() {
            if let Attribute::Module(module) = info.decode()? {
                let mut line = String::new();
                if module.flags.contains(AccessFlags::OPEN) {
                    line.push_str("open ");
                }
                line.push_str("module ");
                line.push_str(&text(module.name)?);
                if let Some(version) = module.version {
                    let _ = write!(line, "@{}", text(version)?);
                }
                return Ok(line);
            }
        }
    }
    let flags = class.access();
    let mut line = String::new();
    if flags.contains(AccessFlags::PUBLIC) {
        line.push_str("public ");
    }
    if flags.contains(AccessFlags::ABSTRACT) && !is_interface {
        line.push_str("abstract ");
    }
    if flags.contains(AccessFlags::FINAL) {
        line.push_str("final ");
    }
    line.push_str(if is_interface { "interface " } else { "class " });
    line.push_str(&dotted(this));
    let signature = class.signature()?.map(text).transpose()?;
    match signature.as_deref().map(signature::parse_class) {
        Some(Ok(sig)) => {
            line.push_str(&type_parameters(&sig.type_parameters));
            if is_interface {
                if !sig.interfaces.is_empty() {
                    line.push_str(" extends ");
                    line.push_str(&join(sig.interfaces.iter().map(class_type)));
                }
            } else {
                line.push_str(" extends ");
                line.push_str(&class_type(&sig.superclass));
                if !sig.interfaces.is_empty() {
                    line.push_str(" implements ");
                    line.push_str(&join(sig.interfaces.iter().map(class_type)));
                }
            }
        }
        Some(Err(error)) => return Err(anyhow!(error).context("the class's Signature")),
        None => {
            let interfaces: Vec<String> = class
                .interfaces()
                .map(|i| Ok(dotted(&text(i?)?)))
                .collect::<Result<_>>()?;
            if is_interface {
                if !interfaces.is_empty() {
                    line.push_str(" extends ");
                    line.push_str(&interfaces.join(","));
                }
            } else {
                if let Some(superclass) = class.super_class()? {
                    let superclass = text(superclass)?;
                    if superclass != "java/lang/Object" {
                        line.push_str(" extends ");
                        line.push_str(&dotted(&superclass));
                    }
                }
                if !interfaces.is_empty() {
                    line.push_str(" implements ");
                    line.push_str(&interfaces.join(","));
                }
            }
        }
    }
    Ok(line)
}

fn join(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}

fn type_parameters(parameters: &[TypeParameter<'_>]) -> String {
    if parameters.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = parameters
        .iter()
        .map(|p| {
            let bounds: Vec<String> = p
                .class_bound
                .iter()
                .chain(&p.interface_bounds)
                .map(java_type)
                .collect();
            if bounds.is_empty() {
                p.name.to_owned()
            } else {
                format!("{} extends {}", p.name, bounds.join(" & "))
            }
        })
        .collect();
    format!("<{}>", rendered.join(", "))
}

fn class_type(class: &ClassTypeSignature<'_>) -> String {
    let mut out = String::new();
    for (i, segment) in class.segments.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&dotted(segment.name));
        if !segment.arguments.is_empty() {
            let arguments: Vec<String> = segment
                .arguments
                .iter()
                .map(|a| match a {
                    TypeArgument::Any => "?".to_owned(),
                    TypeArgument::Extends(t) => format!("? extends {}", java_type(t)),
                    TypeArgument::Super(t) => format!("? super {}", java_type(t)),
                    TypeArgument::Exact(t) => java_type(t),
                })
                .collect();
            let _ = write!(out, "<{}>", arguments.join(", "));
        }
    }
    out
}

fn java_type(t: &TypeSignature<'_>) -> String {
    match t {
        TypeSignature::Base(base) => base.keyword().to_owned(),
        TypeSignature::Class(class) => class_type(class),
        TypeSignature::TypeVariable(name) => (*name).to_owned(),
        TypeSignature::Array {
            dimensions,
            element,
        } => java_type(element) + &"[]".repeat(usize::from(*dimensions)),
    }
}

fn field_type(t: &FieldType<'_>) -> String {
    let element = match t.element {
        ElementType::Base(base) => base.keyword().to_owned(),
        ElementType::Object(name) => dotted(name),
    };
    element + &"[]".repeat(usize::from(t.dimensions))
}

/// One field or method: its declaration, descriptor, flags, and attributes.
fn member(
    out: &mut Out,
    defined: &Defined,
    this: &str,
    member: &Member<'_>,
    is_method: bool,
    in_interface: bool,
) -> Result<()> {
    let name = text(member.name()?)?;
    let descriptor = text(member.descriptor()?)?;
    let declaration = if is_method {
        method_declaration(
            // javap prints `strictfp` only where the version defines ACC_STRICT.
            defined.method & AccessFlags::STRICT == 0,
            this,
            member,
            &name,
            &descriptor,
            in_interface,
        )?
    } else {
        field_declaration(member, &name, &descriptor)?
    };
    out.line(2, &declaration);
    out.line(4, &format!("descriptor: {descriptor}"));
    let flags = member.access().0;
    let line = if is_method {
        flags_line(flags, defined.method, &METHOD_FLAGS, false)
    } else {
        flags_line(flags, defined.field, &FIELD_FLAGS, false)
    };
    out.line(4, &line);
    attributes(out, member.attributes(), 4)
}

fn field_declaration(member: &Member<'_>, name: &str, descriptor: &str) -> Result<String> {
    let flags = member.access();
    let mut line = String::new();
    for (bit, word) in [
        (AccessFlags::PUBLIC, "public "),
        (AccessFlags::PRIVATE, "private "),
        (AccessFlags::PROTECTED, "protected "),
        (AccessFlags::STATIC, "static "),
        (AccessFlags::FINAL, "final "),
        (AccessFlags::VOLATILE, "volatile "),
        (AccessFlags::TRANSIENT, "transient "),
    ] {
        if flags.contains(bit) {
            line.push_str(word);
        }
    }
    let signature = member.signature()?.map(text).transpose()?;
    let ty = match signature.as_deref().map(signature::parse_field) {
        Some(Ok(sig)) => java_type(&sig),
        Some(Err(error)) => return Err(anyhow!(error).context("the field's Signature")),
        None => field_type(&descriptor::parse_field(descriptor)?),
    };
    let _ = write!(line, "{ty} {name};");
    Ok(line)
}

fn method_declaration(
    strict_undefined: bool,
    this: &str,
    member: &Member<'_>,
    name: &str,
    descriptor: &str,
    in_interface: bool,
) -> Result<String> {
    let flags = member.access();
    let mut line = String::new();
    for (bit, word) in [
        (AccessFlags::PUBLIC, "public "),
        (AccessFlags::PRIVATE, "private "),
        (AccessFlags::PROTECTED, "protected "),
    ] {
        if flags.contains(bit) {
            line.push_str(word);
        }
    }
    let is_default = in_interface
        && !flags.contains(AccessFlags::ABSTRACT)
        && !flags.contains(AccessFlags::STATIC)
        && !flags.contains(AccessFlags::PRIVATE)
        && name != "<init>";
    if is_default {
        line.push_str("default ");
    }
    for (bit, word) in [
        (AccessFlags::STATIC, "static "),
        (AccessFlags::FINAL, "final "),
        (AccessFlags::SYNCHRONIZED, "synchronized "),
        (AccessFlags::NATIVE, "native "),
        (AccessFlags::ABSTRACT, "abstract "),
        (
            if strict_undefined {
                0
            } else {
                AccessFlags::STRICT
            },
            "strictfp ",
        ),
    ] {
        if bit != 0 && flags.contains(bit) {
            line.push_str(word);
        }
    }
    let parsed = descriptor::parse_method(descriptor)?;
    let signature = member.signature()?.map(text).transpose()?;
    let sig = signature
        .as_deref()
        .map(signature::parse_method)
        .transpose()
        .map_err(|e| anyhow!(e))?;
    let (type_params, mut params, result, mut throws) = match &sig {
        Some(sig) => (
            type_parameters(&sig.type_parameters),
            sig.parameters.iter().map(java_type).collect::<Vec<_>>(),
            sig.result.as_ref().map_or("void".to_owned(), java_type),
            sig.throws.iter().map(java_type).collect::<Vec<_>>(),
        ),
        None => (
            String::new(),
            parsed.parameters.iter().map(field_type).collect(),
            parsed.result.as_ref().map_or("void".to_owned(), field_type),
            Vec::new(),
        ),
    };
    if throws.is_empty()
        && let Some(info) = member.attributes().named("Exceptions")
        && let Attribute::Exceptions(list) = info.decode()?
    {
        throws = list
            .map(|n| Ok(dotted(&text(n?)?)))
            .collect::<Result<_>>()?;
    }
    // javap puts `...` on the last array parameter, which isn't always the last parameter (in
    // synthetic constructors of anonymous classes, for example).
    if flags.contains(AccessFlags::VARARGS)
        && let Some(last) = params.iter_mut().rev().find(|p| p.ends_with("[]"))
    {
        last.truncate(last.len() - 2);
        last.push_str("...");
    }
    if !type_params.is_empty() {
        line.push_str(&type_params);
        line.push(' ');
    }
    if name == "<clinit>" {
        line.push_str("{};");
        return Ok(line);
    }
    if name == "<init>" {
        line.push_str(&dotted(this));
    } else {
        let _ = write!(line, "{result} {name}");
    }
    let _ = write!(line, "({})", params.join(", "));
    if !throws.is_empty() {
        let _ = write!(line, " throws {}", throws.join(", "));
    }
    line.push(';');
    Ok(line)
}

fn attributes(out: &mut Out, attributes: Attributes<'_>, indent: usize) -> Result<()> {
    // javap prints unknown attributes two columns deeper than the others, so after an attribute
    // whose lines the oracle's normalization drops (`Code`, `BootstrapMethods`,
    // `SourceDebugExtension`), they read as part of it and are dropped with it.
    let mut after_dropped = false;
    for info in attributes {
        let name = info
            .name()
            .ok()
            .and_then(|n| n.to_str().ok().map(|s| s.into_owned()));
        let known = name.as_deref().is_some_and(is_known);
        if name.as_deref() == Some("Code") {
            after_dropped = true;
            continue;
        }
        if after_dropped && !known {
            continue;
        }
        after_dropped = matches!(
            name.as_deref(),
            Some("BootstrapMethods" | "SourceDebugExtension")
        );
        attribute(out, &info, indent)
            .with_context(|| format!("attribute {:?}", info.name().ok()))?;
    }
    Ok(())
}

fn attribute(out: &mut Out, info: &AttributeInfo<'_>, indent: usize) -> Result<()> {
    let class = info.class();
    let i = indent;
    match info.decode()? {
        Attribute::ConstantValue(value) => {
            let value = match value {
                ConstantValue::Int(v) => format!("int {v}"),
                ConstantValue::Long(v) => format!("long {v}l"),
                ConstantValue::Float(bits) => {
                    format!("float {}f", java_float(f32::from_bits(bits)))
                }
                ConstantValue::Double(bits) => {
                    format!("double {}d", java_double(f64::from_bits(bits)))
                }
                ConstantValue::String(s) => format!("String {}", escape(&java_string(s))),
            };
            out.line(i, &format!("ConstantValue: {value}"));
        }
        Attribute::Exceptions(list) => {
            out.line(i, "Exceptions:");
            let names: Vec<String> = list
                .map(|n| Ok(dotted(&text(n?)?)))
                .collect::<Result<_>>()?;
            out.line(i + 2, &format!("throws {}", names.join(", ")));
        }
        Attribute::InnerClasses(entries) => {
            out.line(i, "InnerClasses:");
            for entry in entries {
                let entry = entry?;
                let flags = entry.access;
                let mut line = String::new();
                let interface = flags.contains(AccessFlags::INTERFACE);
                for (bit, word) in [
                    (AccessFlags::PUBLIC, "public "),
                    (AccessFlags::PRIVATE, "private "),
                    (AccessFlags::PROTECTED, "protected "),
                    (AccessFlags::STATIC, "static "),
                    (AccessFlags::FINAL, "final "),
                ] {
                    if flags.contains(bit) {
                        line.push_str(word);
                    }
                }
                if flags.contains(AccessFlags::ABSTRACT) && !interface {
                    line.push_str("abstract ");
                }
                if let Some(name) = entry.name {
                    let _ = write!(line, "{}=", text(name)?);
                }
                let _ = write!(line, "class {}", quoted(&text(entry.inner)?));
                if let Some(outer) = entry.outer {
                    let _ = write!(line, " of class {}", quoted(&text(outer)?));
                }
                out.line(i + 2, &line);
            }
        }
        Attribute::EnclosingMethod(enclosing) => {
            let mut line = format!("EnclosingMethod: {}", dotted(&text(enclosing.class)?));
            if let Some((name, _)) = enclosing.method {
                let _ = write!(line, ".{}", text(name)?);
            }
            out.line(i, &line);
        }
        Attribute::Synthetic => out.line(i, "Synthetic: true"),
        Attribute::Deprecated => out.line(i, "Deprecated: true"),
        Attribute::Signature(sig) => out.line(i, &format!("Signature: {}", text(sig)?)),
        Attribute::SourceFile(name) => out.line(i, &format!("SourceFile: \"{}\"", text(name)?)),
        Attribute::Annotations {
            visible,
            annotations,
        } => {
            let header = if visible {
                "RuntimeVisibleAnnotations:"
            } else {
                "RuntimeInvisibleAnnotations:"
            };
            out.line(i, header);
            annotation_list(out, annotations, i + 2)?;
        }
        Attribute::ParameterAnnotations {
            visible,
            parameters,
        } => {
            out.line(
                i,
                if visible {
                    "RuntimeVisibleParameterAnnotations:"
                } else {
                    "RuntimeInvisibleParameterAnnotations:"
                },
            );
            for (p, annotations) in parameters.enumerate() {
                out.line(i + 2, &format!("parameter {p}:"));
                annotation_list(out, annotations?, i + 4)?;
            }
        }
        Attribute::TypeAnnotations {
            visible,
            annotations,
        } => {
            out.line(
                i,
                if visible {
                    "RuntimeVisibleTypeAnnotations:"
                } else {
                    "RuntimeInvisibleTypeAnnotations:"
                },
            );
            for (k, annotation) in annotations.enumerate() {
                let annotation = annotation?;
                let mut line = format!("{k}: {}", target(&annotation.target));
                if !annotation.path.is_empty() {
                    let steps: Vec<String> = annotation.path.iter().map(step).collect();
                    let _ = write!(line, ", location=[{}]", steps.join(", "));
                }
                out.line(i + 2, &line);
                out.line(i + 4, "");
                pretty_annotation(out, &annotation.annotation, i + 4)?;
            }
        }
        Attribute::AnnotationDefault(value) => {
            out.line(i, "AnnotationDefault:");
            out.line(i + 2, "default_value:");
            out.line(i + 4, "");
            element_value(out, value, i + 4)?;
        }
        Attribute::MethodParameters(parameters) => {
            out.line(i, "MethodParameters:");
            out.line(i + 2, &format!("{:<31}{}", "Name", "Flags"));
            for parameter in parameters {
                let name = match parameter.name {
                    Some(name) => text(name)?,
                    None => "<no name>".to_owned(),
                };
                let mut words = Vec::new();
                for (bit, word) in [
                    (AccessFlags::FINAL, "final"),
                    (AccessFlags::MANDATED, "mandated"),
                    (AccessFlags::SYNTHETIC, "synthetic"),
                ] {
                    if parameter.access.contains(bit) {
                        words.push(word);
                    }
                }
                out.line(i + 2, &format!("{name:<31}{}", words.join(" ")));
            }
        }
        Attribute::Module(module) => {
            out.line(i, "Module:");
            out.line(
                i + 2,
                &with_flags(quoted(&text(module.name)?), module.flags.0, &MODULE_FLAGS),
            );
            version_line(out, i + 2, module.version)?;
            out.commented(i + 2, &module.requires.len().to_string(), "requires");
            for requires in &module.requires {
                out.line(
                    i + 4,
                    &with_flags(
                        quoted(&text(requires.module)?),
                        requires.flags.0,
                        &REQUIRES_FLAGS,
                    ),
                );
                version_line(out, i + 4, requires.version)?;
            }
            for (list, word) in [(&module.exports, "exports"), (&module.opens, "opens")] {
                out.commented(i + 2, &list.len().to_string(), word);
                for export in list.iter() {
                    let mut line = with_flags(text(export.package)?, export.flags.0, &EXPORT_FLAGS);
                    if !export.to.is_empty() {
                        let _ = write!(line, " to ... {}", export.to.len());
                    }
                    out.line(i + 4, &line);
                    for target in &export.to {
                        out.line(i + 6, &format!("... to {}", quoted(&text(*target)?)));
                    }
                }
            }
            out.commented(i + 2, &module.uses.len().to_string(), "uses");
            for service in &module.uses {
                out.line(i + 4, &text(*service)?);
            }
            out.commented(i + 2, &module.provides.len().to_string(), "provides");
            for provides in &module.provides {
                out.line(
                    i + 4,
                    &format!(
                        "{} with ... {}",
                        text(provides.service)?,
                        provides.with.len()
                    ),
                );
                for with in &provides.with {
                    out.line(i + 6, &format!("... with {}", text(*with)?));
                }
            }
        }
        Attribute::ModulePackages(packages) => {
            out.line(i, "ModulePackages:");
            for package in packages {
                out.line(i + 2, &dotted(&text(package?)?));
            }
        }
        Attribute::ModuleMainClass(name) => {
            out.line(i, &format!("ModuleMainClass: {}", dotted(&text(name)?)))
        }
        Attribute::NestHost(name) => out.line(i, &format!("NestHost: class {}", text(name)?)),
        Attribute::NestMembers(list) | Attribute::PermittedSubclasses(list) => {
            out.line(i, &format!("{}:", text(info.name()?)?));
            for name in list {
                out.line(i + 2, &text(name?)?);
            }
        }
        Attribute::Record(components) => {
            out.line(i, "Record:");
            for component in components {
                let name = text(component.name()?)?;
                let descriptor = text(component.descriptor()?)?;
                let signature = component.signature()?.map(text).transpose()?;
                let ty = match signature.as_deref().map(signature::parse_field) {
                    Some(Ok(sig)) => java_type(&sig),
                    Some(Err(error)) => {
                        return Err(anyhow!(error).context("a record component's Signature"));
                    }
                    None => field_type(&descriptor::parse_field(&descriptor)?),
                };
                out.line(i + 2, &format!("{ty} {name};"));
                out.line(i + 4, &format!("descriptor: {descriptor}"));
                attributes(out, component.attributes(), i + 4)?;
            }
        }
        Attribute::ModuleResolution(value) => {
            out.line(i, "ModuleResolution:");
            let mut words = String::new();
            for (bit, word) in [
                (0x0001, "DO_NOT_RESOLVE_BY_DEFAULT"),
                (0x0002, "WARN_DEPRECATED"),
                (0x0004, "WARN_DEPRECATED_FOR_REMOVAL"),
                (0x0008, "WARN_INCUBATING"),
            ] {
                if value & bit != 0 {
                    words.push(' ');
                    words.push_str(word);
                }
            }
            out.commented(i + 2, &format!("{value:x}"), &words);
        }
        Attribute::Other { name, bytes } => other(out, class, &text(name)?, bytes, i)?,
    }
    Ok(())
}

const MODULE_FLAGS: [(u16, &str); 3] = [
    (0x0020, "ACC_OPEN"),
    (0x1000, "ACC_SYNTHETIC"),
    (0x8000, "ACC_MANDATED"),
];
const REQUIRES_FLAGS: [(u16, &str); 4] = [
    (0x0020, "ACC_TRANSITIVE"),
    (0x0040, "ACC_STATIC_PHASE"),
    (0x1000, "ACC_SYNTHETIC"),
    (0x8000, "ACC_MANDATED"),
];
const EXPORT_FLAGS: [(u16, &str); 2] = [(0x1000, "ACC_SYNTHETIC"), (0x8000, "ACC_MANDATED")];

/// The attributes javap prints in their own format, rather than as an unknown attribute's bytes.
fn is_known(name: &str) -> bool {
    matches!(
        name,
        "ConstantValue"
            | "Code"
            | "Exceptions"
            | "InnerClasses"
            | "EnclosingMethod"
            | "Synthetic"
            | "Signature"
            | "SourceFile"
            | "SourceDebugExtension"
            | "Deprecated"
            | "RuntimeVisibleAnnotations"
            | "RuntimeInvisibleAnnotations"
            | "RuntimeVisibleParameterAnnotations"
            | "RuntimeInvisibleParameterAnnotations"
            | "RuntimeVisibleTypeAnnotations"
            | "RuntimeInvisibleTypeAnnotations"
            | "AnnotationDefault"
            | "BootstrapMethods"
            | "MethodParameters"
            | "Module"
            | "ModulePackages"
            | "ModuleMainClass"
            | "NestHost"
            | "NestMembers"
            | "Record"
            | "PermittedSubclasses"
            | "ModuleResolution"
            | "ModuleTarget"
            | "ModuleHashes"
    )
}

/// A class or module name as javap's comments print it: in quotes when it has a character that
/// can't be in a Java identifier, other than the `/` between packages (`"java.base"`,
/// `"okio/internal/-FileSystem$1"`, but `cglib` and `java/lang/Object`).
fn quoted(name: &str) -> String {
    if name
        .chars()
        .all(|c| c == '/' || c == '$' || c == '_' || c.is_alphanumeric())
    {
        name.to_owned()
    } else {
        format!("\"{name}\"")
    }
}

fn with_flags(mut text: String, flags: u16, names: &[(u16, &str)]) -> String {
    for &(bit, name) in names {
        if flags & bit != 0 {
            text.push(' ');
            text.push_str(name);
        }
    }
    text
}

fn version_line(out: &mut Out, indent: usize, version: Option<Name<'_>>) -> Result<()> {
    match version {
        Some(version) => out.line(indent, &text(version)?),
        None => out.line(indent, "#0"),
    }
    Ok(())
}

/// Attributes the model keeps as bytes: printed from the bytes as javap does.
fn other(out: &mut Out, class: &ClassFile, name: &str, bytes: &[u8], i: usize) -> Result<()> {
    let u2 = |at: usize| -> Result<u16> {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .context("truncated")
    };
    match name {
        "BootstrapMethods" | "SourceDebugExtension" => out.line(i, &format!("{name}:")),
        "ModuleTarget" => {
            out.line(i, "ModuleTarget:");
            let platform = class.optional_utf8(u2(0)?)?;
            let platform = platform
                .map(text)
                .transpose()?
                .unwrap_or_else(|| "#0".to_owned());
            out.line(i + 2, &format!("target_platform: {platform}"));
        }
        "ModuleHashes" => {
            out.line(i, "ModuleHashes:");
            out.line(i + 2, &format!("algorithm: {}", text(class.utf8(u2(0)?)?)?));
            let count = u2(2)?;
            out.commented(i + 2, &count.to_string(), "hashes");
            let mut at = 4;
            for _ in 0..count {
                out.line(i + 2, &text(class.module_name(u2(at)?)?)?);
                let len = usize::from(u2(at + 2)?);
                let hash = bytes.get(at + 4..at + 4 + len).context("truncated")?;
                out.line(i + 2, &format!("hash_length: {len}"));
                let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
                out.line(i + 2, &format!("hash: [{hex}]"));
                at += 4 + len;
            }
        }
        "Code" => {}
        _ => {
            out.line(
                i + 2,
                &format!("{name}: length = 0x{:X} (unknown attribute)", bytes.len()),
            );
            for chunk in bytes.chunks(16) {
                let hex: String = chunk.iter().map(|b| format!(" {b:02X}")).collect();
                out.line(i + 2, &hex);
            }
        }
    }
    Ok(())
}

fn annotation_list(out: &mut Out, annotations: Annotations<'_>, indent: usize) -> Result<()> {
    for (k, annotation) in annotations.enumerate() {
        out.line(indent, &format!("{k}:"));
        out.line(indent + 2, "");
        pretty_annotation(out, &annotation?, indent + 2)?;
    }
    Ok(())
}

/// An annotation as javap's readable form: `Type`, or `Type(` with each element on its own line
/// and `)`.
fn pretty_annotation(out: &mut Out, annotation: &Annotation<'_>, indent: usize) -> Result<()> {
    out.text(&type_name(&text(annotation.type_descriptor()?)?));
    let elements = annotation.elements();
    if elements.is_empty() {
        return Ok(());
    }
    out.text("(");
    for element in elements {
        let (name, value) = element?;
        out.line(indent + 2, &format!("{}=", text(name)?));
        element_value(out, value, indent + 2)?;
    }
    out.line(indent, ")");
    Ok(())
}

/// `Ljava/lang/Deprecated;` as `java.lang.Deprecated`.
fn type_name(descriptor: &str) -> String {
    match descriptor
        .strip_prefix('L')
        .and_then(|d| d.strip_suffix(';'))
    {
        Some(name) => dotted(name),
        None => descriptor.to_owned(),
    }
}

fn element_value(out: &mut Out, value: ElementValue<'_>, indent: usize) -> Result<()> {
    match value {
        ElementValue::Byte(v) => out.text(&format!("(byte) {}", v as i8)),
        ElementValue::Char(v) => {
            let c = char::from_u32(v as u16 as u32).unwrap_or(char::REPLACEMENT_CHARACTER);
            out.text(&format!("'{c}'"));
        }
        ElementValue::Int(v) => out.text(&v.to_string()),
        ElementValue::Short(v) => out.text(&format!("(short) {}", v as i16)),
        ElementValue::Boolean(v) => out.text(if v != 0 { "true" } else { "false" }),
        ElementValue::Long(v) => out.text(&format!("{v}l")),
        ElementValue::Float(bits) => out.text(&format!("{}f", java_float(f32::from_bits(bits)))),
        ElementValue::Double(bits) => out.text(&format!("{}d", java_double(f64::from_bits(bits)))),
        ElementValue::String(s) => out.text(&format!("\"{}\"", escape(&java_string(s)))),
        ElementValue::Enum {
            type_descriptor,
            name,
        } => {
            out.text(&format!("{}.{}", text(type_descriptor)?, text(name)?));
        }
        ElementValue::Class(name) => out.text(&format!("class {}", text(name)?)),
        ElementValue::Annotation(annotation) => {
            out.text("@");
            pretty_annotation(out, &annotation, indent)?;
        }
        ElementValue::Array(values) => {
            out.text("[");
            for (k, value) in values.enumerate() {
                if k > 0 {
                    out.text(",");
                }
                element_value(out, value?, indent)?;
            }
            out.text("]");
        }
    }
    Ok(())
}

fn target(target: &TypeAnnotationTarget) -> String {
    match *target {
        TypeAnnotationTarget::ClassTypeParameter { index } => {
            format!("CLASS_TYPE_PARAMETER, param_index={index}")
        }
        TypeAnnotationTarget::MethodTypeParameter { index } => {
            format!("METHOD_TYPE_PARAMETER, param_index={index}")
        }
        TypeAnnotationTarget::Supertype { index } => format!("CLASS_EXTENDS, type_index={index}"),
        TypeAnnotationTarget::ClassTypeParameterBound {
            type_parameter,
            bound,
        } => {
            format!("CLASS_TYPE_PARAMETER_BOUND, param_index={type_parameter}, bound_index={bound}")
        }
        TypeAnnotationTarget::MethodTypeParameterBound {
            type_parameter,
            bound,
        } => {
            format!(
                "METHOD_TYPE_PARAMETER_BOUND, param_index={type_parameter}, bound_index={bound}"
            )
        }
        TypeAnnotationTarget::Field => "FIELD".to_owned(),
        TypeAnnotationTarget::Return => "METHOD_RETURN".to_owned(),
        TypeAnnotationTarget::Receiver => "METHOD_RECEIVER".to_owned(),
        TypeAnnotationTarget::FormalParameter { index } => {
            format!("METHOD_FORMAL_PARAMETER, param_index={index}")
        }
        TypeAnnotationTarget::Throws { index } => format!("THROWS, type_index={index}"),
    }
}

fn step(step: &TypePathStep) -> String {
    match step {
        TypePathStep::Array => "ARRAY".to_owned(),
        TypePathStep::Nested => "INNER_TYPE".to_owned(),
        TypePathStep::Wildcard => "WILDCARD".to_owned(),
        TypePathStep::TypeArgument(index) => format!("TYPE_ARGUMENT({index})"),
    }
}

/// A string constant as UTF-16 code units, which is what javap escapes.
fn java_string(s: JavaStr<'_>) -> Vec<u16> {
    s.to_utf16()
        .unwrap_or_else(|_| s.to_string_lossy().encode_utf16().collect())
}

/// Escapes a string as javap does: `\t`, `\n`, `\r`, `\b`, `\f`, `\"`, `\'`, `\\`, and other control
/// characters (below U+0020, and U+007F to U+009F) as `\uXXXX`; everything else as it is. A lone surrogate becomes U+FFFD, as the
/// oracle writes it.
fn escape(units: &[u16]) -> String {
    let mut out = String::new();
    for c in char::decode_utf16(units.iter().copied()) {
        let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
        match c {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '"' => out.push_str("\\\""),
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// `Double.toString`: the shortest digits that round-trip, as decimal notation from 10^-3 up to
/// 10^7, and computerized scientific notation (`1.0E10`) outside.
pub fn java_double(value: f64) -> String {
    java_number(
        value.is_nan(),
        value.is_infinite(),
        value.is_sign_negative(),
        value == 0.0,
        &format!("{:e}", value.abs()),
        &format!("{:.1e}", value.abs()),
    )
}

/// `Float.toString`, as [`java_double`] with `float` precision.
pub fn java_float(value: f32) -> String {
    java_number(
        value.is_nan(),
        value.is_infinite(),
        value.is_sign_negative(),
        value == 0.0,
        &format!("{:e}", value.abs()),
        &format!("{:.1e}", value.abs()),
    )
}

/// `shortest` is Rust's shortest round-tripping form (`d.ddde-5`), and `two` the closest with two
/// digits: Java never chooses fewer than two digits, so when the shortest has one, it takes the
/// closest two-digit decimal (`4.9E-324`, not `5.0E-324`), then drops a trailing zero.
fn java_number(
    nan: bool,
    infinite: bool,
    negative: bool,
    zero: bool,
    shortest: &str,
    two: &str,
) -> String {
    if nan {
        return "NaN".to_owned();
    }
    let sign = if negative { "-" } else { "" };
    if infinite {
        return format!("{sign}Infinity");
    }
    if zero {
        return format!("{sign}0.0");
    }
    let digits_of = |scientific: &str| -> (String, i32) {
        let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific, "0"));
        let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
        (digits, exponent.parse().unwrap_or(0))
    };
    let (mut digits, mut exponent) = digits_of(shortest);
    if digits.len() == 1 {
        (digits, exponent) = digits_of(two);
        while digits.len() > 1 && digits.ends_with('0') {
            digits.pop();
        }
    }
    if (-3..7).contains(&exponent) {
        let point = exponent + 1;
        let (integer, fraction) = if point <= 0 {
            ("0".to_owned(), "0".repeat((-point) as usize) + &digits)
        } else if (point as usize) >= digits.len() {
            (
                digits.clone() + &"0".repeat(point as usize - digits.len()),
                String::new(),
            )
        } else {
            (
                digits[..point as usize].to_owned(),
                digits[point as usize..].to_owned(),
            )
        };
        let fraction = if fraction.is_empty() {
            "0".to_owned()
        } else {
            fraction
        };
        format!("{sign}{integer}.{fraction}")
    } else {
        let fraction = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{fraction}E{exponent}", &digits[..1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_numbers() {
        assert_eq!(java_double(1.5), "1.5");
        assert_eq!(java_double(1e300), "1.0E300");
        assert_eq!(java_double(1e-5), "1.0E-5");
        assert_eq!(java_double(0.001), "0.001");
        assert_eq!(java_double(std::f64::consts::PI), "3.141592653589793");
        assert_eq!(java_double(1e7), "1.0E7");
        assert_eq!(java_double(9999999.0), "9999999.0");
        assert_eq!(java_double(100.0), "100.0");
        assert_eq!(java_double(-0.0), "-0.0");
        assert_eq!(java_double(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(java_double(f64::NAN), "NaN");
        assert_eq!(java_double(f64::MAX), "1.7976931348623157E308");
        assert_eq!(java_double(f64::MIN_POSITIVE), "2.2250738585072014E-308");
        assert_eq!(java_float(1.0e10), "1.0E10");
        assert_eq!(java_float(0.1), "0.1");
        assert_eq!(java_float(f32::MAX), "3.4028235E38");
        assert_eq!(java_double(f64::from_bits(1)), "4.9E-324");
        assert_eq!(java_float(f32::from_bits(1)), "1.4E-45");
        assert_eq!(java_double(0.5), "0.5");
        assert_eq!(java_double(2e-3), "0.002");
    }

    /// javap 27's lines for flags a class file's version doesn't define, as it prints them for
    /// ASM's varargs methods in `accessors-smart-1.1.jar` (version 46) and the JDK 11 image's
    /// `DirectMethodHandle$Holder` (`ACC_PRIVATE` on a class of version 52).
    #[test]
    fn flags_the_version_does_not_define() {
        let at = |major, minor| Defined::at(Version { major, minor });
        let public_static_varargs = 0x0089;
        assert_eq!(
            flags_line(
                public_static_varargs,
                at(46, 0).method,
                &METHOD_FLAGS,
                false
            ),
            "flags: (0x0089) ACC_PUBLIC, ACC_STATIC"
        );
        assert_eq!(
            flags_line(
                public_static_varargs,
                at(49, 0).method,
                &METHOD_FLAGS,
                false
            ),
            "flags: (0x0089) ACC_PUBLIC, ACC_STATIC, ACC_VARARGS"
        );
        assert_eq!(
            flags_line(0x0080, at(46, 0).method, &METHOD_FLAGS, false),
            "flags: (0x0080)"
        );
        let public_strict = 0x0801;
        assert_eq!(
            flags_line(public_strict, at(45, 3).method, &METHOD_FLAGS, false),
            "flags: (0x0801) ACC_PUBLIC"
        );
        assert_eq!(
            flags_line(public_strict, at(60, 0).method, &METHOD_FLAGS, false),
            "flags: (0x0801) ACC_PUBLIC, ACC_STRICT"
        );
        assert_eq!(
            flags_line(public_strict, at(61, 0).method, &METHOD_FLAGS, false),
            "flags: (0x0801) ACC_PUBLIC"
        );
        // A preview version is the latest release.
        assert_eq!(
            flags_line(public_strict, at(56, 0xffff).method, &METHOD_FLAGS, false),
            "flags: (0x0801) ACC_PUBLIC"
        );
        let synthetic_enum = 0x5000;
        assert_eq!(
            flags_line(synthetic_enum, at(48, 0).field, &FIELD_FLAGS, false),
            "flags: (0x5000)"
        );
        assert_eq!(
            flags_line(synthetic_enum, at(49, 0).field, &FIELD_FLAGS, false),
            "flags: (0x5000) ACC_SYNTHETIC, ACC_ENUM"
        );
        assert_eq!(
            flags_line(0x0032, at(52, 0).class, &CLASS_FLAGS, true),
            "flags: (0x0032) ACC_FINAL, ACC_SUPER, 0x2"
        );
        assert_eq!(
            flags_line(0x9021, at(48, 0).class, &CLASS_FLAGS, true),
            "flags: (0x9021) ACC_PUBLIC, ACC_SUPER, 0x8000, 0x1000"
        );
    }

    #[test]
    fn escapes() {
        let units: Vec<u16> = "tab\there\"q\" back\\ nul\0 u\u{1} e\u{e9} \u{2028}"
            .encode_utf16()
            .collect();
        let expected = "tab\\there\\\"q\\\" back\\\\ nul\\u0000 u\\u0001 e\u{e9} \u{2028}";
        assert_eq!(escape(&units), expected);
        assert_eq!(escape(&[0x61, 0xd800]), "a\u{fffd}");
    }
}
