//! A summary as text, a line per declaration, for snapshot tests and debugging.

use std::fmt::Write as _;

use super::{
    Annotation, Dim, Directive, ImplicitMember, ImportKind, Method, MethodKind, Modifiers,
    Parameter, QualifiedName, Summary, TypeKind, TypeParameter, TypeRef, UnitKind,
};
use crate::Interner;

impl Summary {
    /// The summary, a line per declaration: the unit's kind, its package and imports, then each
    /// type by path with its header and members indented below it, then the module. Fingerprints
    /// are shown as `(…)`, since their values aren't meaningful to read. `names` is the interner
    /// the summary was made with.
    pub fn debug_dump(&self, names: &Interner) -> String {
        let mut out = String::new();
        let kind = match self.kind {
            UnitKind::Ordinary => "ordinary",
            UnitKind::Compact => "compact",
            UnitKind::PackageInfo => "package-info",
            UnitKind::ModuleInfo => "module-info",
        };
        let text = |name| names.text(name);
        writeln!(out, "unit {kind}").unwrap();
        if let Some(package) = &self.package {
            writeln!(
                out,
                "package {}{}",
                annotations(&package.annotations, names),
                package.name.text(names)
            )
            .unwrap();
        }
        for import in &self.imports {
            let kind = match import.kind {
                ImportKind::SingleType => "single-type",
                ImportKind::OnDemand => "on-demand",
                ImportKind::SingleStatic => "single-static",
                ImportKind::StaticOnDemand => "static-on-demand",
                ImportKind::Module => "module",
            };
            writeln!(out, "import {kind} {}", import.name.text(names)).unwrap();
        }
        for (index, ty) in self.types.iter().enumerate() {
            let kind = match ty.kind {
                TypeKind::Class => "class",
                TypeKind::Interface => "interface",
                TypeKind::Enum => "enum",
                TypeKind::Record => "record",
                TypeKind::Annotation => "@interface",
                TypeKind::Implicit => "implicit-class",
            };
            let head = format!(
                "{kind} {}{}{}{}",
                annotations(&ty.annotations, names),
                modifiers(ty.modifiers),
                self.path(index, names),
                type_parameters(&ty.type_parameters, names).unwrap_or_default()
            );
            // An implicitly declared class has no name to follow its kind.
            out.push_str(head.trim_end());
            if !ty.record_components.is_empty() {
                let components: Vec<String> = ty
                    .record_components
                    .iter()
                    .map(|c| {
                        format!(
                            "{}{}{} {}",
                            annotations(&c.annotations, names),
                            c.ty.display(names),
                            varargs(c.varargs.as_deref(), names),
                            text(c.name)
                        )
                    })
                    .collect();
                write!(out, "({})", components.join(", ")).unwrap();
            }
            for (keyword, types) in [
                ("extends", &ty.extends),
                ("implements", &ty.implements),
                ("permits", &ty.permits),
            ] {
                if !types.is_empty() {
                    write!(out, " {keyword} {}", list(types, names)).unwrap();
                }
            }
            if ty.generated {
                out.push_str(" [generated]");
            }
            out.push('\n');
            for constant in &ty.enum_constants {
                let body = if constant.has_body { " {…}" } else { "" };
                writeln!(
                    out,
                    "  constant {}{}{body}",
                    annotations(&constant.annotations, names),
                    text(constant.name)
                )
                .unwrap();
            }
            for field in &ty.fields {
                let constant = if field.constant.is_some() {
                    " = (…)"
                } else {
                    ""
                };
                writeln!(
                    out,
                    "  field {}{}{} {}{}{constant}",
                    annotations(&field.annotations, names),
                    modifiers(field.modifiers),
                    field.ty.display(names),
                    text(field.name),
                    dims(&field.dims, names)
                )
                .unwrap();
            }
            for method in &ty.methods {
                writeln!(out, "  {}", self::method(method, names)).unwrap();
            }
            for implicit in &ty.implicit {
                let component = |i: u32| text(ty.record_components[i as usize].name);
                let text = match implicit {
                    ImplicitMember::DefaultConstructor => "default constructor".to_owned(),
                    ImplicitMember::EnumValues => "values()".to_owned(),
                    ImplicitMember::EnumValueOf => "valueOf(String)".to_owned(),
                    ImplicitMember::CanonicalConstructor => "canonical constructor".to_owned(),
                    ImplicitMember::Accessor(i) => format!("accessor {}()", component(*i)),
                    ImplicitMember::ComponentField(i) => format!("field {}", component(*i)),
                    ImplicitMember::RecordEquals => "equals(Object)".to_owned(),
                    ImplicitMember::RecordHashCode => "hashCode()".to_owned(),
                    ImplicitMember::RecordToString => "toString()".to_owned(),
                };
                writeln!(out, "  implicit {text}").unwrap();
            }
        }
        if let Some(module) = &self.module {
            let open = if module.open { "open " } else { "" };
            writeln!(
                out,
                "module {}{open}{}",
                annotations(&module.annotations, names),
                module.name.text(names)
            )
            .unwrap();
            let list = |list: &[QualifiedName]| {
                list.iter()
                    .map(|n| n.text(names).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            for directive in &module.directives {
                let text = match directive {
                    Directive::Requires {
                        transitive,
                        is_static,
                        module,
                    } => format!(
                        "requires {}{}{}",
                        if *transitive { "transitive " } else { "" },
                        if *is_static { "static " } else { "" },
                        module.text(names)
                    ),
                    Directive::Exports { package, to } if to.is_empty() => {
                        format!("exports {}", package.text(names))
                    }
                    Directive::Exports { package, to } => {
                        format!("exports {} to {}", package.text(names), list(to))
                    }
                    Directive::Opens { package, to } if to.is_empty() => {
                        format!("opens {}", package.text(names))
                    }
                    Directive::Opens { package, to } => {
                        format!("opens {} to {}", package.text(names), list(to))
                    }
                    Directive::Uses { service } => format!("uses {}", service.text(names)),
                    Directive::Provides { service, with } => {
                        format!("provides {} with {}", service.text(names), list(with))
                    }
                };
                writeln!(out, "  {text}").unwrap();
            }
        }
        out
    }
}

fn method(method: &Method, names: &Interner) -> String {
    let kind = match method.kind {
        MethodKind::Method => "method",
        MethodKind::Constructor => "constructor",
        MethodKind::CompactConstructor => "compact constructor",
    };
    let result = method
        .result
        .as_ref()
        .map_or_else(String::new, |r| format!("{} ", r.display(names)));
    let mut text = format!(
        "{kind} {}{}{}{result}{}",
        annotations(&method.annotations, names),
        modifiers(method.modifiers),
        type_parameters(&method.type_parameters, names).map_or_else(String::new, |p| p + " "),
        names.text(method.name)
    );
    if method.kind != MethodKind::CompactConstructor {
        let parameters: Vec<String> = method
            .parameters
            .iter()
            .map(|p| parameter(p, names))
            .collect();
        write!(
            text,
            "({}){}",
            parameters.join(", "),
            dims(&method.dims, names)
        )
        .unwrap();
    }
    if !method.throws.is_empty() {
        write!(text, " throws {}", list(&method.throws, names)).unwrap();
    }
    if method.default_value.is_some() {
        text.push_str(" default (…)");
    }
    text.push_str(if method.has_body { " {…}" } else { ";" });
    text
}

fn parameter(parameter: &Parameter, names: &Interner) -> String {
    format!(
        "{}{}{}{} {}{}",
        annotations(&parameter.annotations, names),
        modifiers(parameter.modifiers),
        parameter.ty.display(names),
        varargs(parameter.varargs.as_deref(), names),
        names.text(parameter.name),
        dims(&parameter.dims, names)
    )
}

fn varargs(varargs: Option<&[Annotation]>, names: &Interner) -> String {
    match varargs {
        Some([]) => "...".to_owned(),
        Some(list) => format!(" {}...", annotations(list, names)),
        None => String::new(),
    }
}

/// `<T, U extends V>`, or nothing.
fn type_parameters(parameters: &[TypeParameter], names: &Interner) -> Option<String> {
    if parameters.is_empty() {
        return None;
    }
    let parameters: Vec<String> = parameters
        .iter()
        .map(|p| {
            let mut text = format!(
                "{}{}",
                annotations(&p.annotations, names),
                names.text(p.name)
            );
            if !p.bounds.is_empty() {
                let bounds: Vec<String> = p
                    .bounds
                    .iter()
                    .map(|b| b.display(names).to_string())
                    .collect();
                write!(text, " extends {}", bounds.join(" & ")).unwrap();
            }
            text
        })
        .collect();
    Some(format!("<{}>", parameters.join(", ")))
}

/// Each annotation and a space after it.
fn annotations(annotations: &[Annotation], names: &Interner) -> String {
    annotations
        .iter()
        .map(|a| format!("{} ", a.display(names)))
        .collect()
}

/// Each modifier's keyword and a space after it.
fn modifiers(modifiers: Modifiers) -> String {
    Modifiers::KEYWORDS
        .iter()
        .filter(|(m, _)| modifiers.contains(*m))
        .map(|(_, keyword)| format!("{keyword} "))
        .collect()
}

fn dims(dims: &[Dim], names: &Interner) -> String {
    dims.iter()
        .map(|d| format!("{}[]", annotations(&d.annotations, names)))
        .collect()
}

fn list(types: &[TypeRef], names: &Interner) -> String {
    types
        .iter()
        .map(|t| t.display(names).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
