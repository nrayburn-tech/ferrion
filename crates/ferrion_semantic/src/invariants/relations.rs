//! Properties of the type relations (docs/design/semantic-type-relations.md#checking), for the
//! `relations` fuzz target and the property tests: on types built from a small fixed hierarchy
//! (generic classes, F-bounds, inner classes of generic classes, expansive inheritance, arrays,
//! wildcards), chosen by the input's bytes,
//!
//! - subtyping is reflexive, and transitive wherever two of its answers are `Yes` (the third is
//!   then never `No`), and a subtype is assignable;
//! - `lub` is above each operand (never `No`);
//! - capture of a captured type is the same type, and erasure of an erased type;
//! - two types that are the same are subtypes of each other;
//! - every query stops within its fuel, and nothing panics.

use crate::testing::relations_input;
use crate::types::{Context, Printed, Tri, Type, TypeContext, TypeId, Why};
use crate::{Engine, SourceSetId};

/// The hierarchy the types are built from.
const HIERARCHY: &str = "import java.util.*;\n\
    interface I<T> {}\n\
    interface J<T> extends I<T> {}\n\
    class A<T> implements J<T> {}\n\
    class B<T> extends A<List<T>> {}\n\
    class C extends B<String> implements Comparable<C> {}\n\
    class F<T extends F<T>> {}\n\
    class G extends F<G> {}\n\
    class O<T> { class In {} class In2<U> extends In {} }\n\
    class P extends O<Integer> {}\n\
    class D<X> extends A<D<X>> {}\n\
    class N<Z> {}\n\
    class E<X> extends N<N<? super E<E<X>>>> {}\n";

/// Generic classes with their arity, and plain types. Every type built is within its bounds:
/// subtyping through capture is transitive only for those, so `F`, whose parameter is bounded by
/// `F` itself, is only written with arguments that are.
const GENERIC: &[(&str, usize)] = &[
    ("List", 1),
    ("Collection", 1),
    ("Map", 2),
    ("A", 1),
    ("B", 1),
    ("I", 1),
    ("J", 1),
    ("D", 1),
    ("N", 1),
    ("E", 1),
    ("Comparable", 1),
];
const PLAIN: &[&str] = &[
    "Object",
    "String",
    "Integer",
    "Number",
    "C",
    "G",
    "P",
    "int",
    "long",
    "List",
    "A",
    "F<G>",
    "F<?>",
    "F<? extends G>",
    "F<? extends F<?>>",
];

/// The most types a check builds, and the fuel each query gets.
const TYPES: usize = 8;
const FUEL: u64 = 20_000;

struct Bytes<'a> {
    bytes: &'a [u8],
}

impl Bytes<'_> {
    fn next(&mut self) -> u8 {
        match self.bytes.split_first() {
            Some((&b, rest)) => {
                self.bytes = rest;
                b
            }
            None => 0,
        }
    }

    /// A type's source text, nested at most `depth` deep.
    fn ty(&mut self, depth: u32) -> String {
        let choice = self.next();
        if depth == 0 {
            return PLAIN[choice as usize % PLAIN.len()].to_owned();
        }
        match choice % 8 {
            0..=2 => PLAIN[self.next() as usize % PLAIN.len()].to_owned(),
            3 | 4 => {
                let (name, arity) = GENERIC[self.next() as usize % GENERIC.len()];
                let args: Vec<String> = (0..arity).map(|_| self.arg(depth - 1)).collect();
                format!("{name}<{}>", args.join(", "))
            }
            5 => format!("{}[]", self.reference(depth - 1)),
            6 => format!("O<{}>.In", self.reference(depth - 1)),
            _ => format!(
                "O<{}>.In2<{}>",
                self.reference(depth - 1),
                self.arg(depth - 1)
            ),
        }
    }

    /// A reference type's text: a primitive chosen is boxed.
    fn reference(&mut self, depth: u32) -> String {
        match self.ty(depth).as_str() {
            "int" => "Integer".to_owned(),
            "long" => "Long".to_owned(),
            other => other.to_owned(),
        }
    }

    fn arg(&mut self, depth: u32) -> String {
        match self.next() % 5 {
            0 => "?".to_owned(),
            1 => format!("? extends {}", self.reference(depth)),
            2 => format!("? super {}", self.reference(depth)),
            _ => self.reference(depth),
        }
    }
}

/// Builds the hierarchy and up to [`TYPES`] types from `bytes`, and checks the properties.
pub fn check_relations(bytes: &[u8]) -> Result<(), String> {
    let mut input = Bytes { bytes };
    let count = 2 + input.next() as usize % (TYPES - 1);
    let fields: Vec<String> = (0..count).map(|_| input.ty(3)).collect();
    check_types(&fields)
}

/// Checks the properties on the types `fields` writes, over the hierarchy: for regression
/// tests, which keep a failure's types rather than its bytes, whose meaning changes with the
/// generator.
pub fn check_types(fields: &[String]) -> Result<(), String> {
    let count = fields.len();
    let mut source = HIERARCHY.to_owned();
    source.push_str("class Q {");
    for (i, field) in fields.iter().enumerate() {
        source.push_str(&format!(" {field} f{i};"));
    }
    source.push_str(" }\n");
    let engine = Engine::new(relations_input(&format!("//- file main Q.java\n{source}")));
    let snapshot = engine.snapshot();
    let mut cx = snapshot.types(SourceSetId::from_index(0));
    let q = cx.lang("Q").map_err(cancelled)?.ok_or("no class Q")?;
    let mut types = Vec::with_capacity(count);
    for i in 0..count {
        let index = cx
            .field_index(q, &format!("f{i}"))
            .map_err(cancelled)?
            .ok_or("no field")?;
        types.push(cx.field_type(q, index).map_err(cancelled)?);
    }
    let mut check = Check { cx, fields };
    check.all(&types)
}

fn cancelled(_: crate::Cancelled) -> String {
    "cancelled with no edit running".to_owned()
}

struct Check<'s, 'f> {
    cx: TypeContext<'s>,
    fields: &'f [String],
}

impl Check<'_, '_> {
    fn subtype(&mut self, s: TypeId, t: TypeId) -> Result<Tri, String> {
        self.cx.refuel(FUEL);
        let answer = self.cx.subtype(s, t).map_err(cancelled)?;
        if self.cx.fuel_used() > FUEL {
            return Err("a query used more than its fuel".to_owned());
        }
        Ok(answer)
    }

    fn describe(&mut self, t: TypeId) -> String {
        self.cx
            .type_string(t, Printed::Captures)
            .unwrap_or_default()
    }

    fn fail(&mut self, what: &str, types: &[TypeId]) -> String {
        let texts: Vec<String> = types.iter().map(|&t| self.describe(t)).collect();
        format!(
            "{what}: {} (fields: {})",
            texts.join(", "),
            self.fields.join("; ")
        )
    }

    fn all(&mut self, types: &[TypeId]) -> Result<(), String> {
        let answerable: Vec<TypeId> = types
            .iter()
            .copied()
            .filter(|&t| self.cx.arena().non_answer(t).is_none())
            .collect();
        for &t in &answerable {
            match self.subtype(t, t)? {
                Tri::Yes(()) => {}
                _ => return Err(self.fail("not a subtype of itself", &[t])),
            }
            self.cx.refuel(FUEL);
            let captured = self.cx.capture(t).map_err(cancelled)?;
            if self.cx.fresh_capture(captured).map_err(cancelled)? != captured {
                return Err(self.fail("capture of a captured type changed it", &[t]));
            }
            let erased = self.cx.erasure(t).map_err(cancelled)?;
            if self.cx.erasure(erased).map_err(cancelled)? != erased {
                return Err(self.fail("erasure of an erased type changed it", &[t]));
            }
        }
        // Pairs: assignability, sameness, and lub.
        let mut below = vec![vec![false; answerable.len()]; answerable.len()];
        for (i, &s) in answerable.iter().enumerate() {
            for (j, &t) in answerable.iter().enumerate() {
                let answer = self.subtype(s, t)?;
                below[i][j] = answer.is_yes();
                if answer.is_yes() {
                    self.cx.refuel(FUEL);
                    let assignable = self
                        .cx
                        .convert(Context::Assignment, s, t, None)
                        .map_err(cancelled)?;
                    if assignable.is_no() {
                        return Err(self.fail("a subtype isn't assignable", &[s, t]));
                    }
                }
                self.cx.refuel(FUEL);
                if self.cx.same(s, t).map_err(cancelled)?.is_yes()
                    && (self.subtype(s, t)?.is_no() || self.subtype(t, s)?.is_no())
                {
                    return Err(self.fail("the same types aren't subtypes of each other", &[s, t]));
                }
                let both_references =
                    self.cx.arena().is_reference(s) && self.cx.arena().is_reference(t);
                if i < j && both_references {
                    self.cx.refuel(FUEL);
                    let lub = self.cx.lub(&[s, t]).map_err(cancelled)?;
                    if matches!(self.cx.get(lub), Type::Unknown(Why::Limit)) {
                        continue;
                    }
                    for operand in [s, t] {
                        if self.subtype(operand, lub)?.is_no() {
                            return Err(self.fail("lub isn't above an operand", &[operand, lub]));
                        }
                    }
                }
            }
        }
        // Transitivity.
        for i in 0..answerable.len() {
            for j in 0..answerable.len() {
                for k in 0..answerable.len() {
                    if below[i][j] && below[j][k] && !below[i][k] {
                        let (s, u) = (answerable[i], answerable[k]);
                        if self.subtype(s, u)?.is_no() {
                            let t = answerable[j];
                            return Err(self.fail("subtyping isn't transitive", &[s, t, u]));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
