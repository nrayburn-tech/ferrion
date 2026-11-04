//! Member tables (docs/design/semantic-attribution.md#member-tables): each type's member fields
//! and methods in a view, declared and inherited, with hiding and overriding worked out on
//! signatures as seen from the type (JLS 8.3, 8.4.8, 8.4.8.4, 9.2, 9.3, 9.4.1).
//!
//! A table is a derived entry per type and view, shared by every file whose bodies search the
//! type. It reads the type's declaration and its direct supertypes' tables through [`Reads`], and
//! the declarations of the methods it compares, so an edit inside a body never invalidates it,
//! and a changed signature recomputes the tables that read it, whose readers are cut off where a
//! table comes out the same. A local or anonymous class's table is its file's, made by the
//! attribution walk with the same function ([`compute`]).

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::engine::derived::{Dep, Memo, Reads};
use crate::resolve::package_of;
use crate::symbols::{FieldRef, MethodRef, TypeKey};
use crate::types::{Tri, Type, TypeContext, TypeId};
use crate::{Cancelled, Fingerprint, Name, Snapshot, SourceSetId};

/// A type's member fields and methods in a view, declared and inherited, by name.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Members {
    /// Sorted by name id.
    pub fields: Box<[(Name, FieldMember)]>,
    /// Every member method of each name, sorted by name id: the declared ones, then those
    /// inherited and not overridden or hidden (JLS 8.4.8), each with the direct supertype it was
    /// inherited through. A method's type as seen from the type is
    /// [`TypeContext::method_type_in`] of the type's declared type, which substitutes along the
    /// path to its owner.
    pub methods: Box<[(Name, Box<[MemberMethod]>)]>,
    /// A supertype didn't resolve, is in a cycle, or has an incomplete hierarchy itself, or the
    /// type is Lombok-annotated (until B4): a lookup that finds nothing, or that might have found
    /// another overload, has no answer.
    pub incomplete: bool,
}

/// A field member by name.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum FieldMember {
    One(FieldRef),
    /// Inherited from two or more supertypes as different fields: an error only where it is
    /// used (JLS 8.3.3).
    Ambiguous(Box<[FieldRef]>),
}

/// A member method, and the direct supertype it was inherited through (`None` if declared).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MemberMethod {
    pub method: MethodRef,
    pub through: Option<TypeKey>,
}

impl Members {
    pub fn field(&self, name: Name) -> Option<&FieldMember> {
        let i = self.fields.binary_search_by_key(&name, |(n, _)| *n).ok()?;
        Some(&self.fields[i].1)
    }

    pub fn methods(&self, name: Name) -> &[MemberMethod] {
        match self.methods.binary_search_by_key(&name, |(n, _)| *n) {
            Ok(i) => &self.methods[i].1,
            Err(_) => &[],
        }
    }
}

impl Snapshot {
    /// A type's member fields and methods in `view` (a source type's own set, whatever `view`
    /// is), declared and inherited. A local or anonymous class's is made from its file's body
    /// result, and not kept.
    pub fn members(&self, view: SourceSetId, key: TypeKey) -> Result<Arc<Members>, Cancelled> {
        self.check()?;
        if let TypeKey::Local { .. } = key {
            let mut reads = Reads::untracked(self);
            let mut cx = self.types(view);
            let mut locals = |reads: &mut Reads<'_>, k: TypeKey| -> Result<_, Cancelled> {
                Ok(Some(reads.snapshot.members(view, k)?))
            };
            return Ok(Arc::new(compute(&mut cx, &mut reads, key, &mut locals)?));
        }
        Ok(self
            .members_memo(view, key)?
            .map_or_else(|| Arc::new(cyclic()), |m| m.value.clone()))
    }

    pub(crate) fn members_memo(
        &self,
        view: SourceSetId,
        key: TypeKey,
    ) -> Result<Option<Arc<Memo<Members>>>, Cancelled> {
        let view = key.home(view);
        self.shared()
            .members
            .get_or_cycle(self, &(view, key), |reads| {
                let mut cx = reads.snapshot.recording_types(view);
                let mut supers = |reads: &mut Reads<'_>, k: TypeKey| reads.members(view, k);
                let members = compute(&mut cx, reads, key, &mut supers)?;
                reads.extend(cx.take_reads());
                if reads.in_cycle() {
                    return Ok(cyclic());
                }
                Ok(members)
            })
    }
}

impl Reads<'_> {
    /// A type's member table in `view`, or `None` if asking is a cycle. Records it.
    pub(crate) fn members(
        &mut self,
        view: SourceSetId,
        key: TypeKey,
    ) -> Result<Option<Arc<Members>>, Cancelled> {
        let memo = self.snapshot.members_memo(view, key)?;
        self.record(
            Dep::Members(key.home(view), key),
            memo.as_ref().map_or(Fingerprint::CYCLE, |m| m.fingerprint),
        );
        Ok(memo.map(|m| m.value.clone()))
    }
}

/// The table of a type in a cycle of supertypes: nothing, incomplete.
fn cyclic() -> Members {
    Members {
        incomplete: true,
        ..Members::default()
    }
}

/// Where a table's computation finds its supertypes' tables: `None` for one in a cycle.
pub(crate) type Supers<'a, 's> =
    dyn FnMut(&mut Reads<'s>, TypeKey) -> Result<Option<Arc<Members>>, Cancelled> + 'a;

/// A method candidate while a table is made.
#[derive(Clone, Copy)]
struct Candidate {
    member: MemberMethod,
    is_abstract: bool,
    /// Declared in an interface.
    interface: bool,
    /// Inherited from the superclass, not an interface.
    from_superclass: bool,
}

/// Makes `key`'s table with `cx`, its supertypes' tables from `supers` (`None` for one whose
/// table is a cycle).
pub(crate) fn compute<'s>(
    cx: &mut TypeContext<'s>,
    reads: &mut Reads<'s>,
    key: TypeKey,
    supers: &mut Supers<'_, 's>,
) -> Result<Members, Cancelled> {
    let Some(decl) = cx.decl_info(key)? else {
        return Ok(cyclic());
    };
    let names = reads.snapshot.names().clone();
    // A local class's package is the one its top level class is in.
    let package = package_of(cx.outermost(key)?, &names);
    let mut incomplete = decl.incomplete;
    let this = cx.declared_type(key)?;

    // Direct supertypes and their tables; an interface without superinterfaces has `Object`'s
    // public methods (JLS 9.2).
    let mut parents: Vec<(TypeKey, Arc<Members>, bool)> = Vec::new();
    for s in cx.declared_supertype_list(key)? {
        match cx.get(s) {
            Type::Class { key: k, .. } => match supers(reads, k)? {
                Some(table) => {
                    incomplete |= table.incomplete;
                    let interface = cx.is_interface(k)?;
                    parents.push((k, table, !interface));
                }
                None => incomplete = true,
            },
            _ => incomplete = true,
        }
    }
    let mut object_methods = false;
    if decl.interface && parents.is_empty() {
        match cx.lang("java/lang/Object")? {
            Some(object) if object != key => match supers(reads, object)? {
                Some(table) => {
                    parents.push((object, table, false));
                    object_methods = true;
                }
                None => incomplete = true,
            },
            Some(_) => {}
            None => incomplete = true,
        }
    }

    // Fields: the declared ones hide every inherited one of their names (JLS 8.3).
    let mut fields: Vec<(Name, FieldMember)> = Vec::new();
    for (index, name) in decl.fields.iter().enumerate() {
        if !fields.iter().any(|(n, _)| n == name) {
            let field = FieldRef {
                owner: key,
                index: index as u32,
            };
            fields.push((*name, FieldMember::One(field)));
        }
    }
    let declared_fields = fields.len();
    for (_, table, _) in parents.iter().filter(|_| !object_methods) {
        for (name, member) in table.fields.iter() {
            if fields[..declared_fields].iter().any(|(n, _)| n == name) {
                continue;
            }
            let refs: Vec<FieldRef> = match member {
                FieldMember::One(f) => vec![*f],
                FieldMember::Ambiguous(fs) => fs.to_vec(),
            };
            let mut inherited = Vec::new();
            for f in refs {
                if cx.inheritable_field(f, package)? {
                    inherited.push(f);
                }
            }
            if inherited.is_empty() {
                continue;
            }
            match fields[declared_fields..]
                .iter_mut()
                .find(|(n, _)| n == name)
            {
                Some((_, existing)) => {
                    let mut all: Vec<FieldRef> = match existing {
                        FieldMember::One(f) => vec![*f],
                        FieldMember::Ambiguous(fs) => fs.to_vec(),
                    };
                    for f in inherited {
                        if !all.contains(&f) {
                            all.push(f);
                        }
                    }
                    *existing = if all.len() == 1 {
                        FieldMember::One(all[0])
                    } else {
                        FieldMember::Ambiguous(all.into_boxed_slice())
                    };
                }
                None => fields.push((
                    *name,
                    if inherited.len() == 1 {
                        FieldMember::One(inherited[0])
                    } else {
                        FieldMember::Ambiguous(inherited.into_boxed_slice())
                    },
                )),
            }
        }
    }
    fields.sort_by_key(|(n, _)| *n);

    // Methods, by name.
    let mut by_name: Vec<(Name, Vec<Candidate>)> = Vec::new();
    let slot = |by_name: &mut Vec<(Name, Vec<Candidate>)>, name: Name| -> usize {
        match by_name.iter().position(|(n, _)| *n == name) {
            Some(i) => i,
            None => {
                by_name.push((name, Vec::new()));
                by_name.len() - 1
            }
        }
    };
    let declared_counts: Vec<(Name, usize)>;
    {
        for (index, method) in decl.methods.iter().enumerate() {
            let Some(method) = method else { continue };
            let i = slot(&mut by_name, method.name);
            by_name[i].1.push(Candidate {
                member: MemberMethod {
                    method: MethodRef {
                        owner: key,
                        index: index as u32,
                    },
                    through: None,
                },
                is_abstract: method.is_abstract,
                interface: decl.interface,
                from_superclass: false,
            });
        }
        declared_counts = by_name.iter().map(|(n, c)| (*n, c.len())).collect();
    }
    for (parent, table, superclass) in &parents {
        for (name, methods) in table.methods.iter() {
            for m in methods.iter() {
                // A supertype's table is of its own source set's view.
                let Some(method) = cx.method_in_view(m.method)? else {
                    continue;
                };
                let m = MemberMethod {
                    method,
                    through: m.through,
                };
                let Some(info) = cx.method_info(m.method)? else {
                    continue;
                };
                // Private methods and an interface's static methods aren't inherited, nor a
                // package-access method of another package; through an interface, only
                // `Object`'s public instance methods are (JLS 8.4.8, 9.2, 9.4.1).
                let other_package = info.package_access
                    && package_of(cx.outermost(m.method.owner)?, &names) != package;
                if info.is_private
                    || (info.is_static && info.interface)
                    || other_package
                    || (object_methods && (!info.is_public || info.is_static))
                {
                    continue;
                }
                let i = slot(&mut by_name, *name);
                if by_name[i].1.iter().any(|c| c.member.method == m.method) {
                    continue;
                }
                by_name[i].1.push(Candidate {
                    member: MemberMethod {
                        method: m.method,
                        through: Some(*parent),
                    },
                    is_abstract: info.is_abstract,
                    interface: info.interface,
                    from_superclass: *superclass,
                });
            }
        }
    }
    let mut methods: Vec<(Name, Box<[MemberMethod]>)> = Vec::with_capacity(by_name.len());
    for (name, candidates) in by_name {
        let declared = declared_counts
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(0, |(_, c)| *c);
        // Each name's comparisons on their own budget: a type like jedis's `Jedis` has thousands
        // of methods, whose names have nothing to do with one another. Running out keeps methods
        // an overrider could have removed, so the table can't be trusted (it has no answers).
        cx.refuel(crate::types::DEFAULT_FUEL);
        let kept = prune(cx, this, decl.interface, &candidates, declared)?;
        incomplete |= cx.out_of_fuel();
        methods.push((name, kept.into_boxed_slice()));
    }
    methods.sort_by_key(|(n, _)| *n);
    Ok(Members {
        fields: fields.into_boxed_slice(),
        methods: methods.into_boxed_slice(),
        incomplete,
    })
}

/// The member methods of one name: the declared ones (the first `declared` candidates), and the
/// inherited ones no declared method overrides or hides, no inherited method from a subtype of
/// their owner overrides, and, for an abstract or default one, no concrete method from the
/// superclass overrides (JLS 8.4.8, 8.4.8.4). Signatures are compared as members of `this`; an
/// unknown answer keeps the method.
fn prune(
    cx: &mut TypeContext<'_>,
    this: TypeId,
    interface: bool,
    candidates: &[Candidate],
    declared: usize,
) -> Result<Vec<MemberMethod>, Cancelled> {
    if candidates.len() <= 1 {
        return Ok(candidates.iter().map(|c| c.member).collect());
    }
    let mut types = Vec::with_capacity(candidates.len());
    for c in candidates {
        let m = c.member.method;
        types.push(match cx.method_type_in(this, m.owner, m.index)? {
            Tri::Yes(t) => Some(t),
            _ => None,
        });
    }
    let object = cx.lang("java/lang/Object")?;
    let mut kept = vec![true; candidates.len()];
    let sub = |cx: &mut TypeContext<'_>, a: usize, b: usize| -> Result<bool, Cancelled> {
        match (&types[a], &types[b]) {
            (Some(x), Some(y)) => Ok(cx.subsignature(x, y)?.is_yes()),
            _ => Ok(false),
        }
    };
    for (i, keep) in kept.iter_mut().enumerate().skip(declared) {
        // Overridden or hidden by a declared method.
        for d in 0..declared {
            if sub(cx, d, i)? {
                *keep = false;
                break;
            }
        }
    }
    for i in declared..candidates.len() {
        if !kept[i] {
            continue;
        }
        let owner = candidates[i].member.method.owner;
        for j in declared..candidates.len() {
            if i == j || !kept[j] {
                continue;
            }
            let other = candidates[j];
            let other_owner = other.member.method.owner;
            // Overridden from a subtype of its owner. Only in an interface does an interface's
            // method override `Object`'s (observed: `toString()` of an interface extending
            // `CharSequence` and another is `CharSequence`'s); in a class, `Object`'s stays over
            // an abstract one (JLS 8.4.8.4).
            let overrides = if other.interface && Some(owner) == object {
                interface
            } else {
                cx.is_subclass(other_owner, owner)?
            };
            if other_owner != owner && overrides && sub(cx, j, i)? {
                kept[i] = false;
                break;
            }
            // An abstract or default method, and a concrete one from the superclass.
            if (candidates[i].is_abstract || candidates[i].interface)
                && other.from_superclass
                && !other.is_abstract
                && !other.interface
                && sub(cx, j, i)?
            {
                kept[i] = false;
                break;
            }
        }
    }
    Ok(candidates
        .iter()
        .zip(kept)
        .filter(|(_, k)| *k)
        .map(|(c, _)| c.member)
        .collect())
}
