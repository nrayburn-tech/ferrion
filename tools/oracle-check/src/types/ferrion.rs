//! Ferrion's side of the relations comparison (B3a, docs/design/semantic-type-relations.md#checking):
//! each of the oracle's relation queries asked of `ferrion_semantic::types`, and its answer
//! written as javac's is.
//!
//! - **The sampled types** are rebuilt from the declaration site the oracle names (`field p/A.f
//!   type`, `method p/A.m(I)V parameter 0`, `type p/A ... superclass`): the field's, parameter's,
//!   or result's type in the class's entry, or its superclass or interface, translated as
//!   attribution translates them. The other types the queries ask about (a type's direct
//!   supertypes, which the oracle asks subtyping of) are parsed from javac's type strings.
//! - **The API's shape:** `Types.directSupertypes` and `Types.asMemberOf` depart from the JLS
//!   (observations 16 and 17 in docs/design/types-oracle-normalization.md, and 8 and 9 in
//!   docs/design/semantic-type-relations.md), so their answers are
//!   converted: a wildcard-parameterized type's supertypes and members are substituted without
//!   capture, an interface's supertypes start with `Object`, a type variable's are its bounds
//!   (after `Object` if the first is an interface), and an array's is one type, the array of its
//!   component's superclass, or `Serializable&Cloneable` for an array of `Object` or a primitive.
//! - **Answers** are type strings, `yes` or `no`, or a comma-separated list for supertypes;
//!   `Unknown` is no answer, so it counts as missing, but where a class or member Ferrion
//!   couldn't find is one a Lombok-annotated class's annotations make, which is
//!   `left-to-b4 lombok`.
//!
//! Each project's engine is the names comparison's ([`Ferrion`]), so a run loads it once. The
//! files' entries are [`super::attribution`]'s.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use ferrion_semantic::summary::{Primitive, WildcardBound};
use ferrion_semantic::symbols::{TypeKey, TypeVarOwner};
use ferrion_semantic::types::{
    Context, DEFAULT_FUEL, Printed, Tri, Type, TypeArg, TypeContext, TypeId,
};
use ferrion_semantic::{Cancelled, Snapshot, SourceSetId};

use super::attribution::{self, Localizer};
use super::compare::{Attributed, Typer};
use super::oracle::{Query, Relations};
use crate::names::ferrion::Ferrion;
use crate::names::plan::SourceSet;

/// What a source set's relations cost Ferrion: the most fuel one query used, and the arena's
/// size at the end.
#[derive(Clone, Debug, Default)]
pub struct Cost {
    pub fuel: u64,
    pub fuel_query: String,
    pub arena: u64,
}

/// Ferrion's relations over each project's engine.
pub struct FerrionTyper {
    ferrion: Ferrion,
    costs: Mutex<HashMap<String, Cost>>,
    /// How many files at a time to attribute, or `None` to compare the relation queries alone.
    attribution: Option<usize>,
}

impl FerrionTyper {
    pub fn new(ferrion: Ferrion, attribution: Option<usize>) -> FerrionTyper {
        FerrionTyper {
            ferrion,
            costs: Mutex::new(HashMap::new()),
            attribution,
        }
    }
}

impl Typer for FerrionTyper {
    fn attribute(
        &self,
        _: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
        relations: &Relations,
    ) -> Result<Option<Attributed>, String> {
        let start = Instant::now();
        let (answers, cost, files) = self.ferrion.with_set(set, |engine, id, _| {
            let cancelled = |_| "cancelled with no edit running".to_owned();
            let snapshot = engine.snapshot();
            let (answers, cost) = Relater::new(&snapshot, id, corpus)
                .answer(relations)
                .map_err(cancelled)?;
            let files = match self.attribution {
                Some(threads) => attribution::set_files(&snapshot, id, corpus, threads),
                None => Ok(HashMap::new()),
            };
            Ok((answers, cost, files.map_err(cancelled)?))
        })?;
        let mut costs = self.costs.lock().unwrap_or_else(|e| e.into_inner());
        costs.insert(set.display(), cost.clone());
        Ok(Some(Attributed {
            files,
            relations: answers,
            seconds: start.elapsed().as_secs_f64(),
            fuel: cost.fuel,
            fuel_query: cost.fuel_query,
            arena: cost.arena,
        }))
    }
}

/// Answers one source set's queries.
struct Relater<'s> {
    cx: TypeContext<'s>,
    snapshot: &'s Snapshot,
    corpus: &'s Path,
    locals: Localizer<'s>,
    owners: HashMap<String, Option<TypeKey>>,
    /// Each class's methods by their keys in the oracle's form.
    methods: HashMap<TypeKey, HashMap<String, u32>>,
    /// Each type built so far, by index, and whether building it met something Lombok makes.
    built: HashMap<usize, (Option<TypeId>, bool)>,
    /// The current query met a class or member that a Lombok-annotated class's annotations
    /// make, which Ferrion doesn't have until B4.
    lombok: bool,
}

impl<'s> Relater<'s> {
    fn new(snapshot: &'s Snapshot, view: SourceSetId, corpus: &'s Path) -> Relater<'s> {
        Relater {
            cx: snapshot.types(view),
            snapshot,
            corpus,
            locals: Localizer::new(snapshot, view, corpus),
            owners: HashMap::new(),
            methods: HashMap::new(),
            built: HashMap::new(),
            lombok: false,
        }
    }

    fn answer(&mut self, relations: &Relations) -> Result<(Vec<Option<String>>, Cost), Cancelled> {
        let mut cost = Cost::default();
        for (site, index) in &relations.sites {
            self.cx.refuel(DEFAULT_FUEL);
            self.lombok = false;
            let built = self.site(site)?;
            self.built.insert(*index, (built, self.lombok));
            if self.cx.fuel_used() > cost.fuel {
                cost.fuel = self.cx.fuel_used();
                cost.fuel_query = format!("the type of {site}");
            }
        }
        let mut answers = Vec::with_capacity(relations.queries.len());
        for query in &relations.queries {
            self.cx.refuel(DEFAULT_FUEL);
            self.lombok = false;
            let mut answer = self.query(relations, query)?;
            if answer.is_none() && self.lombok {
                answer = Some("left-to-b4 lombok".to_owned());
            }
            if self.cx.fuel_used() > cost.fuel {
                cost.fuel = self.cx.fuel_used();
                cost.fuel_query = super::compare::describe(relations, query);
            }
            answers.push(answer);
        }
        cost.arena = self.cx.arena().len() as u64;
        Ok((answers, cost))
    }

    fn query(&mut self, relations: &Relations, query: &Query) -> Result<Option<String>, Cancelled> {
        let yes_no = |answer: Tri| match answer {
            Tri::Yes(()) => Some("yes".to_owned()),
            Tri::No => Some("no".to_owned()),
            Tri::Unknown(_) => None,
        };
        Ok(match query {
            Query::Erasure(t, _) => match self.ty(relations, *t)? {
                Some(t) => {
                    let erased = self.cx.erasure(t)?;
                    self.string(erased, Printed::Captures)?
                }
                None => None,
            },
            Query::Capture(t, _) => match self.ty(relations, *t)? {
                Some(t) => {
                    let captured = self.cx.capture(t)?;
                    self.string(captured, Printed::Captures)?
                }
                None => None,
            },
            Query::Supertypes(t, _) => match self.ty(relations, *t)? {
                Some(t) => match self.api_supertypes(t)? {
                    Some(supers) => {
                        let mut texts = Vec::with_capacity(supers.len());
                        for s in supers {
                            match self.string(s, Printed::Wildcards)? {
                                Some(text) => texts.push(text),
                                None => return Ok(None),
                            }
                        }
                        Some(texts.join(","))
                    }
                    None => None,
                },
                None => None,
            },
            Query::Subtype(s, t, _) => match (self.ty(relations, *s)?, self.ty(relations, *t)?) {
                (Some(s), Some(t)) => yes_no(self.cx.subtype(s, t)?),
                _ => None,
            },
            Query::Assignable(s, t, _) => {
                match (self.ty(relations, *s)?, self.ty(relations, *t)?) {
                    (Some(s), Some(t)) => {
                        yes_no(self.cx.convert(Context::Assignment, s, t, None)?.plain())
                    }
                    _ => None,
                }
            }
            Query::Same(s, t, _) => match (self.ty(relations, *s)?, self.ty(relations, *t)?) {
                (Some(s), Some(t)) => yes_no(self.cx.same(s, t)?),
                _ => None,
            },
            Query::Member(t, member, _) => match self.ty(relations, *t)? {
                Some(t) => self.member(t, member)?,
                None => None,
            },
            Query::Subsignature(t, m, n, _) => {
                let (Some(t), Some(m), Some(n)) =
                    (self.ty(relations, *t)?, self.method(m)?, self.method(n)?)
                else {
                    return Ok(None);
                };
                match (
                    self.cx.method_type_in(t, m.0, m.1)?,
                    self.cx.method_type_in(t, n.0, n.1)?,
                ) {
                    (Tri::Yes(mt), Tri::Yes(nt)) => yes_no(self.cx.subsignature(&mt, &nt)?),
                    _ => None,
                }
            }
            Query::Overrides(m, n, owner, _) => {
                let owner = owner
                    .strip_prefix("type ")
                    .map(|rest| split_owner(rest).0.to_owned());
                let (Some(m), Some(n), Some(owner)) = (self.method(m)?, self.method(n)?, owner)
                else {
                    return Ok(None);
                };
                match self.owner(&owner)? {
                    Some(of) => yes_no(self.cx.overrides(m, n, of)?),
                    None => None,
                }
            }
            Query::Threw(..) => None,
        })
    }

    /// The type at `index` of the relations' types: a sampled type, rebuilt from its site, or
    /// another, parsed from its string.
    fn ty(&mut self, relations: &Relations, index: usize) -> Result<Option<TypeId>, Cancelled> {
        if let Some(&(built, lombok)) = self.built.get(&index) {
            self.lombok |= lombok;
            return Ok(built);
        }
        let outer = std::mem::take(&mut self.lombok);
        let parsed = match relations.types.get(index) {
            Some(text) => {
                let mut parser = Parser {
                    text,
                    at: 0,
                    relater: self,
                };
                match parser.ty()? {
                    Some(t) if parser.at == text.len() => Some(t),
                    _ => None,
                }
            }
            None => None,
        };
        self.built.insert(index, (parsed, self.lombok));
        self.lombok |= outer;
        Ok(parsed)
    }

    /// A type as javac writes it, local classes with corpus-relative paths and UTF-16 offsets.
    fn string(&mut self, t: TypeId, printed: Printed) -> Result<Option<String>, Cancelled> {
        if self.mentions_non_answer(t) {
            return Ok(None);
        }
        let text = self.cx.type_string(t, printed)?;
        Ok(Some(self.oracle_form(&text)?))
    }

    fn mentions_non_answer(&self, t: TypeId) -> bool {
        let arena = self.cx.arena();
        let mut pending = vec![t];
        let mut steps = 0;
        while let Some(t) = pending.pop() {
            steps += 1;
            if steps > 100_000 {
                return true;
            }
            match arena.get(t) {
                Type::Error(_) | Type::Unknown(_) => return true,
                Type::Class { outer, args, .. } => {
                    pending.extend(outer);
                    for arg in arena.class_args(t) {
                        match *arg {
                            TypeArg::Type(a) | TypeArg::Wildcard(Some((_, a))) => pending.push(a),
                            TypeArg::Wildcard(None) => {}
                        }
                    }
                    let _ = args;
                }
                Type::Array(c) => pending.push(c),
                Type::Intersection(list) => pending.extend(arena.list(list)),
                Type::Capture(n) => {
                    let var = arena.capture(n);
                    // A capture's bounds are written once; a bound naming it is fine.
                    if !matches!(arena.get(var.upper), Type::Capture(_)) {
                        match arena.get(var.upper) {
                            Type::Error(_) | Type::Unknown(_) => return true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn oracle_form(&mut self, text: &str) -> Result<String, Cancelled> {
        self.locals.oracle_form(text)
    }

    /// A class named as the oracle names owners: a binary name, or `local <file>:<offset>`.
    fn owner(&mut self, text: &str) -> Result<Option<TypeKey>, Cancelled> {
        if let Some(&key) = self.owners.get(text) {
            if key.is_none() {
                self.lombok |= self.made_by_lombok(text)?;
            }
            return Ok(key);
        }
        let key = match text.strip_prefix("local ") {
            Some(local) => {
                let (path, offset) = local.rsplit_once(':').unwrap_or((local, ""));
                let offset: i64 = offset.parse().unwrap_or(-1);
                let path = self.corpus.join(path);
                match self.locals.file(&path)? {
                    Some(file) => {
                        let body = self.snapshot.body_names(file)?;
                        let offsets = self.locals.offsets_of(&path)?;
                        body.local_types
                            .iter()
                            .position(|t| offsets.utf16(t.at) == offset)
                            .map(|index| TypeKey::Local {
                                file,
                                index: index as u32,
                            })
                    }
                    None => None,
                }
            }
            None => self.cx.lang(text)?,
        };
        self.owners.insert(text.to_owned(), key);
        if key.is_none() {
            self.lombok |= self.made_by_lombok(text)?;
        }
        Ok(key)
    }

    /// Whether a class is annotated from `lombok`, so that it may have members and member
    /// classes Ferrion doesn't know of until B4.
    fn is_lombok(&mut self, key: TypeKey) -> Result<bool, Cancelled> {
        Ok(self.snapshot.type_entry(key)?.is_some_and(|e| e.lombok))
    }

    /// Whether a class Ferrion can't find is a member of a Lombok-annotated class: a builder
    /// `@Builder` makes.
    fn made_by_lombok(&mut self, binary: &str) -> Result<bool, Cancelled> {
        let mut text = binary;
        while let Some((outer, _)) = text.rsplit_once('$') {
            if let Some(key) = self.cx.lang(outer)?
                && self.is_lombok(key)?
            {
                return Ok(true);
            }
            text = outer;
        }
        Ok(false)
    }

    /// A method by its key: `method p/A.m(I)V`.
    fn method(&mut self, key: &str) -> Result<Option<(TypeKey, u32)>, Cancelled> {
        let Some(rest) = key.strip_prefix("method ") else {
            return Ok(None);
        };
        let Some(paren) = rest.find('(') else {
            return Ok(None);
        };
        let Some((owner, _)) = rest[..paren].rsplit_once('.') else {
            return Ok(None);
        };
        let Some(owner) = self.owner(owner)? else {
            return Ok(None);
        };
        if !self.methods.contains_key(&owner) {
            let mut methods = HashMap::new();
            for i in 0..self.cx.method_count(owner)? as u32 {
                let text = self.cx.method_key(owner, i)?;
                methods.insert(self.oracle_form(&text)?, i);
            }
            self.methods.insert(owner, methods);
        }
        let found = self.methods[&owner].get(key).map(|&i| (owner, i));
        if found.is_none() {
            self.lombok |= self.is_lombok(owner)?;
        }
        Ok(found)
    }

    /// A sampled type, from its site.
    fn site(&mut self, site: &str) -> Result<Option<TypeId>, Cancelled> {
        if let Some(rest) = site.strip_prefix("type ") {
            let (owner, role) = split_owner(rest);
            let Some(key) = self.owner(owner)? else {
                return Ok(None);
            };
            let words: Vec<&str> = role.split_whitespace().collect();
            return Ok(match words[..] {
                [.., "superclass"] => self.cx.superclass(key)?,
                [.., "interface", n] => {
                    let n: usize = n.parse().unwrap_or(usize::MAX);
                    self.cx.interfaces(key)?.get(n).copied()
                }
                _ => None,
            });
        }
        if let Some(rest) = site.strip_prefix("field ") {
            let Some(field) = rest.strip_suffix(" type") else {
                return Ok(None);
            };
            let Some((owner, name)) = field.rsplit_once('.') else {
                return Ok(None);
            };
            let Some(key) = self.owner(owner)? else {
                return Ok(None);
            };
            let index = self.cx.field_index(key, name)?;
            if index.is_none() {
                self.lombok |= self.is_lombok(key)?;
            }
            return Ok(match index {
                Some(i) => Some(self.cx.field_type(key, i)?),
                None => None,
            });
        }
        if site.starts_with("method ") {
            let (method, role) = match site.rsplit_once(" parameter ") {
                Some((method, n)) => (method, n.parse::<usize>().ok()),
                None => match site.strip_suffix(" return") {
                    Some(method) => (method, None),
                    None => return Ok(None),
                },
            };
            let Some((key, index)) = self.method(method)? else {
                return Ok(None);
            };
            let m = self.cx.method_type(key, index)?;
            return Ok(match role {
                Some(n) => m.params.get(n).copied(),
                None => Some(m.result),
            });
        }
        Ok(None)
    }

    /// `Types.asMemberOf(t, member)`, in the API's shape: substituted without capture.
    fn member(&mut self, t: TypeId, member: &str) -> Result<Option<String>, Cancelled> {
        if let Some(field) = member.strip_prefix("field ") {
            let Some((owner, name)) = field.rsplit_once('.') else {
                return Ok(None);
            };
            let Some(key) = self.owner(owner)? else {
                return Ok(None);
            };
            let index = self.cx.field_index(key, name)?;
            if index.is_none() {
                self.lombok |= self.is_lombok(key)?;
            }
            let Some(index) = index else {
                return Ok(None);
            };
            return match self.cx.field_type_in(t, key, index)? {
                Tri::Yes(ty) => self.string(ty, Printed::Wildcards),
                _ => Ok(None),
            };
        }
        let Some((key, index)) = self.method(member)? else {
            return Ok(None);
        };
        match self.cx.method_type_in(t, key, index)? {
            Tri::Yes(m) => {
                let mut all = m.params.clone();
                all.push(m.result);
                all.extend(&m.throws);
                if all.iter().any(|&t| self.mentions_non_answer(t)) {
                    return Ok(None);
                }
                let text = self.cx.method_string(&m, Printed::Wildcards)?;
                Ok(Some(self.oracle_form(&text)?))
            }
            _ => Ok(None),
        }
    }

    /// `Types.directSupertypes(t)` in the API's shape (observation 16), or `None` where Ferrion
    /// has no answer.
    fn api_supertypes(&mut self, t: TypeId) -> Result<Option<Vec<TypeId>>, Cancelled> {
        Ok(match self.cx.get(t) {
            Type::Primitive(_) | Type::Null | Type::Void => Some(Vec::new()),
            Type::Class { key, .. } => {
                let Tri::Yes(supers) = self.cx.direct_supertypes(t)? else {
                    return Ok(None);
                };
                if self.cx.is_interface(key)? {
                    let object = self.object()?;
                    let mut out = vec![object];
                    out.extend(supers.into_iter().filter(|&s| s != object));
                    Some(out)
                } else {
                    Some(supers)
                }
            }
            Type::Var(owner, index) => {
                let bounds = self.cx.bounds(owner, index)?;
                let first_interface = match bounds.first().map(|&b| self.cx.get(b)) {
                    Some(Type::Class { key, .. }) => self.cx.is_interface(key)?,
                    _ => false,
                };
                if first_interface {
                    let mut out = vec![self.object()?];
                    out.extend(bounds);
                    Some(out)
                } else {
                    Some(bounds)
                }
            }
            Type::Array(c) => {
                let object = self.object()?;
                let primitive_or_object = !matches!(
                    self.cx.get(c),
                    Type::Class { .. } | Type::Array(_) | Type::Var(..) | Type::Capture(_)
                ) || c == object;
                if primitive_or_object {
                    let serializable = self.lang_class("java/io/Serializable")?;
                    let cloneable = self.lang_class("java/lang/Cloneable")?;
                    let both = self.cx.arena_mut().intersection(&[serializable, cloneable]);
                    Some(vec![both])
                } else {
                    match self.api_supertypes(c)? {
                        Some(supers) => match supers.first() {
                            Some(&s) => Some(vec![self.cx.arena_mut().array(s)]),
                            None => Some(Vec::new()),
                        },
                        None => None,
                    }
                }
            }
            _ => None,
        })
    }

    fn object(&mut self) -> Result<TypeId, Cancelled> {
        self.lang_class("java/lang/Object")
    }

    fn lang_class(&mut self, binary: &str) -> Result<TypeId, Cancelled> {
        Ok(match self.cx.lang(binary)? {
            Some(key) => self.cx.class_type(key, None, None),
            None => self
                .cx
                .arena_mut()
                .unknown(ferrion_semantic::types::Why::Later),
        })
    }
}

/// An owner and what follows it, in a site's `type <owner> <origin> <role>`: a local class has
/// no origin.
fn split_owner(text: &str) -> (&str, &str) {
    if let Some(local) = text.strip_prefix("local ") {
        // `local <path>:<offset>`, then the rest.
        let colon = local.find(':').map_or(local.len(), |c| c + 1);
        let digits = local[colon..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        let end = "local ".len() + colon + digits;
        return (&text[..end], &text[end..]);
    }
    match text.find(' ') {
        Some(space) => (&text[..space], &text[space..]),
        None => (text, ""),
    }
}

/// A parser of javac's type strings, building the types in the context's arena.
struct Parser<'p, 's> {
    text: &'p str,
    at: usize,
    relater: &'p mut Relater<'s>,
}

impl Parser<'_, '_> {
    fn rest(&self) -> &str {
        &self.text[self.at..]
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.at += token.len();
            true
        } else {
            false
        }
    }

    /// A name: letters, digits, `_`, `$`, and `/`.
    fn name(&mut self) -> &str {
        let start = self.at;
        let len: usize = self
            .rest()
            .chars()
            .take_while(|&c| c.is_alphanumeric() || matches!(c, '_' | '$' | '/'))
            .map(char::len_utf8)
            .sum();
        self.at += len;
        &self.text[start..self.at]
    }

    /// A type, or an intersection of types.
    fn ty(&mut self) -> Result<Option<TypeId>, Cancelled> {
        let Some(first) = self.array()? else {
            return Ok(None);
        };
        let mut parts = vec![first];
        while self.eat("&") {
            match self.array()? {
                Some(t) => parts.push(t),
                None => return Ok(None),
            }
        }
        Ok(Some(self.relater.cx.arena_mut().intersection(&parts)))
    }

    fn array(&mut self) -> Result<Option<TypeId>, Cancelled> {
        let Some(mut t) = self.primary()? else {
            return Ok(None);
        };
        while self.eat("[]") {
            t = self.relater.cx.arena_mut().array(t);
        }
        Ok(Some(t))
    }

    fn primary(&mut self) -> Result<Option<TypeId>, Cancelled> {
        if self.rest().starts_with("local ") {
            let (owner, _) = split_owner(self.rest());
            let owner = owner.to_owned();
            self.at += owner.len();
            let Some(key) = self.relater.owner(&owner)? else {
                return Ok(None);
            };
            return self.class_rest(key, owner);
        }
        if self.rest().starts_with('?') || self.rest().starts_with("capture#") {
            return Ok(None);
        }
        let start = self.at;
        let name = self.name().to_owned();
        if name.is_empty() {
            return Ok(None);
        }
        let primitive = match name.as_str() {
            "boolean" => Some(Primitive::Boolean),
            "byte" => Some(Primitive::Byte),
            "char" => Some(Primitive::Char),
            "short" => Some(Primitive::Short),
            "int" => Some(Primitive::Int),
            "long" => Some(Primitive::Long),
            "float" => Some(Primitive::Float),
            "double" => Some(Primitive::Double),
            _ => None,
        };
        if let Some(p) = primitive {
            return Ok(Some(self.relater.cx.primitive(p)));
        }
        if name == "null" {
            return Ok(Some(self.relater.cx.arena_mut().intern(Type::Null)));
        }
        if self.eat(":") {
            return self.type_var(&name);
        }
        let _ = start;
        let Some(key) = self.relater.owner(&name)? else {
            return Ok(None);
        };
        self.class_rest(key, name)
    }

    /// A class's arguments, if any, and the member classes of it that follow (`$Inner<...>`).
    fn class_rest(&mut self, key: TypeKey, binary: String) -> Result<Option<TypeId>, Cancelled> {
        let mut binary = binary;
        let mut key = key;
        let mut outer = None;
        loop {
            let args = if self.eat("<") {
                let mut args = Vec::new();
                loop {
                    match self.arg()? {
                        Some(arg) => args.push(arg),
                        None => return Ok(None),
                    }
                    if self.eat(">") {
                        break;
                    }
                    if !self.eat(",") {
                        return Ok(None);
                    }
                }
                Some(args)
            } else {
                None
            };
            let t = self.relater.cx.class_type(key, outer, args.as_deref());
            if !self.rest().starts_with('$') {
                return Ok(Some(t));
            }
            self.at += 1;
            let simple = self.name().to_owned();
            binary = format!("{binary}${simple}");
            key = match self.relater.owner(&binary)? {
                Some(key) => key,
                None => return Ok(None),
            };
            outer = Some(t);
        }
    }

    fn arg(&mut self) -> Result<Option<TypeArg>, Cancelled> {
        if self.eat("?") {
            if self.eat(" extends ") {
                return Ok(self
                    .ty()?
                    .map(|t| TypeArg::Wildcard(Some((WildcardBound::Extends, t)))));
            }
            if self.eat(" super ") {
                return Ok(self
                    .ty()?
                    .map(|t| TypeArg::Wildcard(Some((WildcardBound::Super, t)))));
            }
            return Ok(Some(TypeArg::Wildcard(None)));
        }
        Ok(self.ty()?.map(TypeArg::Type))
    }

    /// A type variable, after its name and `:`: its owner, a class or a method.
    fn type_var(&mut self, name: &str) -> Result<Option<TypeId>, Cancelled> {
        let owner_text = if self.rest().starts_with("local ") {
            let (owner, _) = split_owner(self.rest());
            let owner = owner.to_owned();
            self.at += owner.len();
            owner
        } else {
            self.name().to_owned()
        };
        let owner = if self.eat(".") {
            // A method: its name, then its descriptor.
            let start = self.at;
            let Some(paren) = self.rest().find('(') else {
                return Ok(None);
            };
            self.at += paren;
            if !self.descriptor() {
                return Ok(None);
            }
            let key = format!("method {owner_text}.{}", &self.text[start..self.at]);
            match self.relater.method(&key)? {
                Some((key, index)) => TypeVarOwner::Method(key, index),
                None => return Ok(None),
            }
        } else {
            match self.relater.owner(&owner_text)? {
                Some(key) => TypeVarOwner::Type(key),
                None => return Ok(None),
            }
        };
        Ok(match self.relater.cx.type_param_index(owner, name)? {
            Some(index) => Some(self.relater.cx.arena_mut().var(owner, index)),
            None => None,
        })
    }

    /// A method descriptor: `(...)` and a return descriptor.
    fn descriptor(&mut self) -> bool {
        if !self.eat("(") {
            return false;
        }
        while !self.eat(")") {
            if !self.field_descriptor() {
                return false;
            }
        }
        self.eat("V") || self.field_descriptor()
    }

    fn field_descriptor(&mut self) -> bool {
        while self.eat("[") {}
        match self.rest().chars().next() {
            Some('B' | 'C' | 'D' | 'F' | 'I' | 'J' | 'S' | 'Z') => {
                self.at += 1;
                true
            }
            Some('L') => match self.rest().find(';') {
                Some(end) => {
                    self.at += end + 1;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
}
