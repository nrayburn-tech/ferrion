//! Type strings: a type as text in the types oracle's form
//! (docs/design/types-oracle-normalization.md#type-strings), for tests, dumps, and the javac
//! comparison. Classes are binary names (a local class `local <file>:<byte offset>`, which the
//! comparison converts to javac's UTF-16 offsets), type variables `T:<owner>`, capture variables
//! `capture#<n>` with their bounds at their first appearance.

use std::rc::Rc;

use super::{MethodType, R, Type, TypeArg, TypeContext, TypeId, Why};
use crate::Cancelled;
use crate::body::Map;
use crate::summary::{Primitive, WildcardBound};
use crate::symbols::{TypeKey, TypeVarOwner};

/// How a type string writes capture variables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Printed {
    /// `capture#1 extends java/lang/Number`, as javac's own type strings write them.
    Captures,
    /// As the wildcards they captured, the way `javax.lang.model`'s `asMemberOf` and
    /// `directSupertypes` substitute a wildcard-parameterized type's arguments without capture
    /// (observations 16 and 17): `? extends java/lang/Number`, and inside `? extends`, the
    /// wildcard's bound, or for `?` its type parameter's declared bound.
    Wildcards,
}

/// Where a type is written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Plain,
    /// As the bound of `? extends`.
    ExtendsBound,
}

/// Texts the printer has looked up.
#[derive(Default)]
pub(crate) struct Texts {
    binaries: Map<TypeKey, Rc<str>>,
    simple: Map<TypeKey, Rc<str>>,
    owners: Map<TypeVarOwner, Rc<str>>,
    vars: Map<(TypeVarOwner, u32), Rc<str>>,
}

fn primitive(p: Primitive) -> &'static str {
    match p {
        Primitive::Boolean => "boolean",
        Primitive::Byte => "byte",
        Primitive::Char => "char",
        Primitive::Short => "short",
        Primitive::Int => "int",
        Primitive::Long => "long",
        Primitive::Float => "float",
        Primitive::Double => "double",
    }
}

struct Writer {
    out: String,
    captures: Vec<u32>,
    printed: Printed,
}

impl TypeContext<'_> {
    /// A type's string.
    pub fn type_string(&mut self, t: TypeId, printed: Printed) -> Result<String, Cancelled> {
        let mut w = Writer {
            out: String::new(),
            captures: Vec::new(),
            printed,
        };
        match self.write(&mut w, t, Position::Plain) {
            Ok(()) => Ok(w.out),
            Err(super::Stop::Cancelled) => Err(Cancelled),
            Err(super::Stop::Limit) => Ok(w.out),
        }
    }

    /// A method type's string: `<T:owner>(P1,P2)R throws X`.
    pub fn method_string(&mut self, m: &MethodType, printed: Printed) -> Result<String, Cancelled> {
        let mut w = Writer {
            out: String::new(),
            captures: Vec::new(),
            printed,
        };
        let result = (|| -> R<()> {
            if !m.type_params.is_empty() {
                w.out.push('<');
                self.join(&mut w, &m.type_params, ",")?;
                w.out.push('>');
            }
            w.out.push('(');
            self.join(&mut w, &m.params, ",")?;
            w.out.push(')');
            self.write(&mut w, m.result, Position::Plain)?;
            if !m.throws.is_empty() {
                w.out.push_str(" throws ");
                self.join(&mut w, &m.throws, ",")?;
            }
            Ok(())
        })();
        match result {
            Err(super::Stop::Cancelled) => Err(Cancelled),
            _ => Ok(w.out),
        }
    }

    fn join(&mut self, w: &mut Writer, types: &[TypeId], separator: &str) -> R<()> {
        for (i, &t) in types.iter().enumerate() {
            if i > 0 {
                w.out.push_str(separator);
            }
            self.write(w, t, Position::Plain)?;
        }
        Ok(())
    }

    fn write(&mut self, w: &mut Writer, t: TypeId, position: Position) -> R<()> {
        ferrion_stack::ensure_stack(|| self.write_inner(w, t, position))
    }

    fn write_inner(&mut self, w: &mut Writer, t: TypeId, position: Position) -> R<()> {
        match self.arena.get(t) {
            Type::Primitive(p) => w.out.push_str(primitive(p)),
            Type::Void => w.out.push_str("void"),
            Type::Null => w.out.push_str("null"),
            Type::Class { key, outer, args } => {
                match outer {
                    Some(o) if !matches!(key, TypeKey::Local { .. }) => {
                        self.write(w, o, Position::Plain)?;
                        w.out.push('$');
                        let simple = self.simple_text(key)?;
                        w.out.push_str(&simple);
                    }
                    _ => {
                        let binary = self.binary_text(key);
                        w.out.push_str(&binary);
                    }
                }
                if let Some(args) = args {
                    let args = self.arena.args(args).to_vec();
                    w.out.push('<');
                    for (i, arg) in args.into_iter().enumerate() {
                        if i > 0 {
                            w.out.push(',');
                        }
                        self.write_arg(w, arg)?;
                    }
                    w.out.push('>');
                }
            }
            Type::Array(c) => {
                self.write(w, c, Position::Plain)?;
                w.out.push_str("[]");
            }
            Type::Var(owner, index) => {
                let name = self.var_text(owner, index)?;
                w.out.push_str(&name);
                w.out.push(':');
                let owner = self.owner_text(owner)?;
                w.out.push_str(&owner);
            }
            Type::Capture(n) => {
                let var = *self.arena.capture(n);
                if w.printed == Printed::Wildcards {
                    match (position, var.wildcard) {
                        (Position::ExtendsBound, TypeArg::Wildcard(None)) => {
                            let (key, index) = var.of;
                            let bounds = self.var_bounds(TypeVarOwner::Type(key), index)?.to_vec();
                            self.join(w, &bounds, "&")?;
                        }
                        (
                            Position::ExtendsBound,
                            TypeArg::Wildcard(Some((WildcardBound::Extends, b))),
                        ) => {
                            self.write(w, b, Position::ExtendsBound)?;
                        }
                        (_, arg) => self.write_arg(w, arg)?,
                    }
                    return Ok(());
                }
                if let Some(i) = w.captures.iter().position(|&c| c == n) {
                    w.out.push_str(&format!("capture#{}", i + 1));
                    return Ok(());
                }
                w.captures.push(n);
                w.out
                    .push_str(&format!("capture#{} extends ", w.captures.len()));
                self.write(w, var.upper, Position::Plain)?;
                if let Some(lower) = var.lower {
                    w.out.push_str(" super ");
                    self.write(w, lower, Position::Plain)?;
                }
            }
            Type::Intersection(list) => {
                let parts = self.arena.list(list).to_vec();
                self.join(w, &parts, "&")?;
            }
            Type::Error(e) => {
                let name = self.names.text(self.arena.error_symbol(e).name);
                w.out.push_str("error ");
                w.out.push_str(&name);
            }
            Type::Unknown(why) => w.out.push_str(match why {
                Why::Limit => "unknown limit",
                Why::Later => "unknown later",
                Why::Incomplete(_) => "unknown incomplete",
                Why::Error(_) => "unknown error",
            }),
        }
        Ok(())
    }

    fn write_arg(&mut self, w: &mut Writer, arg: TypeArg) -> R<()> {
        match arg {
            TypeArg::Type(t) => self.write(w, t, Position::Plain),
            TypeArg::Wildcard(None) => {
                w.out.push('?');
                Ok(())
            }
            TypeArg::Wildcard(Some((WildcardBound::Extends, b))) => {
                w.out.push_str("? extends ");
                self.write(w, b, Position::ExtendsBound)
            }
            TypeArg::Wildcard(Some((WildcardBound::Super, b))) => {
                w.out.push_str("? super ");
                self.write(w, b, Position::Plain)
            }
        }
    }

    /// A class's binary name: `java/util/Map$Entry`, or `local <file>:<offset>`.
    pub(crate) fn binary_text(&mut self, key: TypeKey) -> Rc<str> {
        if let Some(text) = self.texts.binaries.get(&key) {
            return text.clone();
        }
        let text: Rc<str> = Rc::from(self.snapshot.binary_text(key).unwrap_or_default());
        self.texts.binaries.insert(key, text.clone());
        text
    }

    fn simple_text(&mut self, key: TypeKey) -> R<Rc<str>> {
        if let Some(text) = self.texts.simple.get(&key) {
            return Ok(text.clone());
        }
        let text: Rc<str> = match key {
            TypeKey::Source { path, .. }
            | TypeKey::Compact {
                path: Some(path), ..
            } => Rc::from(&*self.names.text(self.names.path_last(path))),
            TypeKey::Class { .. } => match self.snapshot.class_entry(key) {
                Some(entry) => Rc::from(&*self.names.text(entry.simple)),
                None => self.binary_text(key),
            },
            TypeKey::Compact { path: None, .. } | TypeKey::Local { .. } => self.binary_text(key),
        };
        self.texts.simple.insert(key, text.clone());
        Ok(text)
    }

    fn var_text(&mut self, owner: TypeVarOwner, index: u32) -> R<Rc<str>> {
        if let Some(text) = self.texts.vars.get(&(owner, index)) {
            return Ok(text.clone());
        }
        let (key, method) = match owner {
            TypeVarOwner::Type(key) => (key, None),
            TypeVarOwner::Method(key, m) => (key, Some(m)),
        };
        let name = self.snapshot.type_param_name(key, method, index)?;
        let text: Rc<str> = Rc::from(&*self.names.text(name));
        self.texts.vars.insert((owner, index), text.clone());
        Ok(text)
    }

    fn owner_text(&mut self, owner: TypeVarOwner) -> R<Rc<str>> {
        if let Some(text) = self.texts.owners.get(&owner) {
            return Ok(text.clone());
        }
        let text: Rc<str> = match owner {
            TypeVarOwner::Type(key) => self.binary_text(key),
            TypeVarOwner::Method(key, m) => {
                let text = self.snapshot.method_text(key, m)?;
                Rc::from(text.strip_prefix("method ").unwrap_or(&text))
            }
        };
        self.texts.owners.insert(owner, text.clone());
        Ok(text)
    }
}
