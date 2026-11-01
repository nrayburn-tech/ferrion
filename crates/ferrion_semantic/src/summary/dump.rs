//! A summary as text, a line per declaration, for snapshot tests and debugging.

use std::fmt::Write as _;

use super::{
    Annotation, Dim, Directive, ImplicitMember, ImportKind, Method, MethodKind, Modifiers,
    Parameter, Summary, TypeKind, TypeParameter, TypeRef, UnitKind,
};

impl Summary {
    /// The summary, a line per declaration: the unit's kind, its package and imports, then each
    /// type by path with its header and members indented below it, then the module. Fingerprints
    /// are shown as `(…)`, since their values aren't meaningful to read.
    pub fn debug_dump(&self) -> String {
        let mut out = String::new();
        let kind = match self.kind {
            UnitKind::Ordinary => "ordinary",
            UnitKind::Compact => "compact",
            UnitKind::PackageInfo => "package-info",
            UnitKind::ModuleInfo => "module-info",
        };
        writeln!(out, "unit {kind}").unwrap();
        if let Some(package) = &self.package {
            writeln!(
                out,
                "package {}{}",
                annotations(&package.annotations),
                package.name
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
            writeln!(out, "import {kind} {}", import.name).unwrap();
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
                annotations(&ty.annotations),
                modifiers(ty.modifiers),
                self.path(index),
                type_parameters(&ty.type_parameters).unwrap_or_default()
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
                            annotations(&c.annotations),
                            c.ty,
                            varargs(c.varargs.as_deref()),
                            c.name
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
                    write!(out, " {keyword} {}", list(types)).unwrap();
                }
            }
            if ty.generated {
                out.push_str(" [generated]");
            }
            if ty.lombok {
                out.push_str(" [lombok]");
            }
            out.push('\n');
            for constant in &ty.enum_constants {
                let body = if constant.has_body { " {…}" } else { "" };
                writeln!(
                    out,
                    "  constant {}{}{body}",
                    annotations(&constant.annotations),
                    constant.name
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
                    annotations(&field.annotations),
                    modifiers(field.modifiers),
                    field.ty,
                    field.name,
                    dims(&field.dims)
                )
                .unwrap();
            }
            for method in &ty.methods {
                writeln!(out, "  {}", self::method(method)).unwrap();
            }
            for implicit in &ty.implicit {
                let text = match implicit {
                    ImplicitMember::DefaultConstructor => "default constructor".to_owned(),
                    ImplicitMember::EnumValues => "values()".to_owned(),
                    ImplicitMember::EnumValueOf => "valueOf(String)".to_owned(),
                    ImplicitMember::CanonicalConstructor => "canonical constructor".to_owned(),
                    ImplicitMember::Accessor(i) => {
                        format!("accessor {}()", ty.record_components[*i as usize].name)
                    }
                    ImplicitMember::ComponentField(i) => {
                        format!("field {}", ty.record_components[*i as usize].name)
                    }
                };
                writeln!(out, "  implicit {text}").unwrap();
            }
        }
        if let Some(module) = &self.module {
            let open = if module.open { "open " } else { "" };
            writeln!(
                out,
                "module {}{open}{}",
                annotations(&module.annotations),
                module.name
            )
            .unwrap();
            for directive in &module.directives {
                let names = |names: &[super::QualifiedName]| {
                    names
                        .iter()
                        .map(|n| n.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let text = match directive {
                    Directive::Requires {
                        transitive,
                        is_static,
                        module,
                    } => format!(
                        "requires {}{}{module}",
                        if *transitive { "transitive " } else { "" },
                        if *is_static { "static " } else { "" }
                    ),
                    Directive::Exports { package, to } if to.is_empty() => {
                        format!("exports {package}")
                    }
                    Directive::Exports { package, to } => {
                        format!("exports {package} to {}", names(to))
                    }
                    Directive::Opens { package, to } if to.is_empty() => {
                        format!("opens {package}")
                    }
                    Directive::Opens { package, to } => {
                        format!("opens {package} to {}", names(to))
                    }
                    Directive::Uses { service } => format!("uses {service}"),
                    Directive::Provides { service, with } => {
                        format!("provides {service} with {}", names(with))
                    }
                };
                writeln!(out, "  {text}").unwrap();
            }
        }
        out
    }
}

fn method(method: &Method) -> String {
    let kind = match method.kind {
        MethodKind::Method => "method",
        MethodKind::Constructor => "constructor",
        MethodKind::CompactConstructor => "compact constructor",
    };
    let result = method
        .result
        .as_ref()
        .map_or_else(String::new, |r| format!("{r} "));
    let mut text = format!(
        "{kind} {}{}{}{result}{}",
        annotations(&method.annotations),
        modifiers(method.modifiers),
        type_parameters(&method.type_parameters).map_or_else(String::new, |p| p + " "),
        method.name
    );
    if method.kind != MethodKind::CompactConstructor {
        let parameters: Vec<String> = method.parameters.iter().map(parameter).collect();
        write!(text, "({}){}", parameters.join(", "), dims(&method.dims)).unwrap();
    }
    if !method.throws.is_empty() {
        write!(text, " throws {}", list(&method.throws)).unwrap();
    }
    if method.default_value.is_some() {
        text.push_str(" default (…)");
    }
    text.push_str(if method.has_body { " {…}" } else { ";" });
    text
}

fn parameter(parameter: &Parameter) -> String {
    format!(
        "{}{}{}{} {}{}",
        annotations(&parameter.annotations),
        modifiers(parameter.modifiers),
        parameter.ty,
        varargs(parameter.varargs.as_deref()),
        parameter.name,
        dims(&parameter.dims)
    )
}

fn varargs(varargs: Option<&[Annotation]>) -> String {
    match varargs {
        Some([]) => "...".to_owned(),
        Some(list) => format!(" {}...", annotations(list)),
        None => String::new(),
    }
}

/// `<T, U extends V>`, or nothing.
fn type_parameters(parameters: &[TypeParameter]) -> Option<String> {
    if parameters.is_empty() {
        return None;
    }
    let parameters: Vec<String> = parameters
        .iter()
        .map(|p| {
            let mut text = format!("{}{}", annotations(&p.annotations), p.name);
            if !p.bounds.is_empty() {
                let bounds: Vec<String> = p.bounds.iter().map(|b| b.to_string()).collect();
                write!(text, " extends {}", bounds.join(" & ")).unwrap();
            }
            text
        })
        .collect();
    Some(format!("<{}>", parameters.join(", ")))
}

/// Each annotation and a space after it.
fn annotations(annotations: &[Annotation]) -> String {
    annotations.iter().map(|a| format!("{a} ")).collect()
}

/// Each modifier's keyword and a space after it.
fn modifiers(modifiers: Modifiers) -> String {
    Modifiers::KEYWORDS
        .iter()
        .filter(|(m, _)| modifiers.contains(*m))
        .map(|(_, keyword)| format!("{keyword} "))
        .collect()
}

fn dims(dims: &[Dim]) -> String {
    dims.iter()
        .map(|d| format!("{}[]", annotations(&d.annotations)))
        .collect()
}

fn list(types: &[TypeRef]) -> String {
    types
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
