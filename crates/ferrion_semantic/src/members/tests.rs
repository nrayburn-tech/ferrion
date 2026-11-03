//! Tests of member tables, on an input whose members javac was asked for
//! (`Elements.getAllMembers`, Temurin 27.0.1). That API lists more than JLS 8.3 and 8.4.8 make
//! members: a field hidden by another of its name (`A.x` beside `B.x`), and an interface's
//! abstract method that a concrete method of the superclass implements (`I.f()` beside `R.f()`
//! in `S`); the tables follow the JLS there, as overload resolution does, and agree with javac
//! everywhere else, which each test notes.

use super::{FieldMember, Members};
use crate::symbols::TypeKey;
use crate::testing::relations_input;
use crate::{Engine, Snapshot, SourceSetId};

const SOURCE: &str = "import java.util.*;\n\
    interface I { int F = 1; void f(); default void d() {} static void s() {} }\n\
    interface J extends I { int F = 2; void f(); }\n\
    interface K { int F = 3; void f(); default void d() {} }\n\
    abstract class A implements Comparable<A> { int x; private int hidden; public int compareTo(A o) { return 0; } abstract void g(); void h() {} static void st() {} }\n\
    class B extends A implements J { int x; void g() {} public void f() {} }\n\
    abstract class C extends A implements I, K { public void d() {} }\n\
    class D implements I, J { public void f() {} }\n\
    interface E { boolean equals(Object o); String toString(); }\n\
    interface Cl { Cl clone(); }\n\
    class G<T> { T t; void p(T t) {} }\n\
    class H extends G<String> { void p(String s) {} }\n\
    class R { public void f() {} }\n\
    class S extends R implements I { }\n";

fn table(snapshot: &Snapshot, binary: &str) -> (TypeKey, std::sync::Arc<Members>) {
    let view = SourceSetId::from_index(0);
    let mut cx = snapshot.types(view);
    let key = cx.lang(binary).unwrap().unwrap();
    (key, snapshot.members(view, key).unwrap())
}

/// The owners of the member methods `name`, as `Owner.name`, in order.
fn methods(snapshot: &Snapshot, members: &Members, name: &str) -> Vec<String> {
    let name = snapshot.names().intern(name);
    members
        .methods(name)
        .iter()
        .map(|m| {
            let owner = snapshot.binary_text(m.method.owner).unwrap();
            format!("{owner}.{}", snapshot.names().text(name))
        })
        .collect()
}

fn field(snapshot: &Snapshot, members: &Members, name: &str) -> Vec<String> {
    let name = snapshot.names().intern(name);
    let owners = match members.field(name) {
        Some(FieldMember::One(f)) => vec![f.owner],
        Some(FieldMember::Ambiguous(fs)) => fs.iter().map(|f| f.owner).collect(),
        None => Vec::new(),
    };
    owners
        .into_iter()
        .map(|o| snapshot.binary_text(o).unwrap())
        .collect()
}

fn with(test: impl FnOnce(&Snapshot)) {
    let engine = Engine::new(relations_input(&format!("//- file main Q.java\n{SOURCE}")));
    test(&engine.snapshot());
}

/// javac: `B` has `B.f()`, `B.g()`, `I.d()`, `A.h()`, `A.st()`, `A.compareTo(A)`, and `Object`'s;
/// not `I.s()` (an interface's static method isn't inherited) or `A.hidden`.
#[test]
fn declared_and_inherited() {
    with(|s| {
        let (_, b) = table(s, "B");
        assert_eq!(methods(s, &b, "f"), ["B.f"]);
        assert_eq!(methods(s, &b, "g"), ["B.g"]);
        assert_eq!(methods(s, &b, "d"), ["I.d"]);
        assert_eq!(methods(s, &b, "h"), ["A.h"]);
        assert_eq!(methods(s, &b, "st"), ["A.st"]);
        assert_eq!(methods(s, &b, "compareTo"), ["A.compareTo"]);
        assert_eq!(methods(s, &b, "equals"), ["java/lang/Object.equals"]);
        assert!(methods(s, &b, "s").is_empty());
        assert!(field(s, &b, "hidden").is_empty());
        // JLS 8.3: `B.x` hides `A.x`, and `J.F` hides `I.F` in `J` (javac's `getAllMembers`
        // lists the hidden ones too).
        assert_eq!(field(s, &b, "x"), ["B"]);
        assert_eq!(field(s, &b, "F"), ["J"]);
        assert!(!b.incomplete);
    });
}

/// javac: `C` has both `K.f()` and `I.f()` (neither overrides the other), and its own `d()`;
/// `F` from both `I` and `K` is ambiguous.
#[test]
fn unrelated_interfaces() {
    with(|s| {
        let (_, c) = table(s, "C");
        assert_eq!(methods(s, &c, "f"), ["I.f", "K.f"]);
        assert_eq!(methods(s, &c, "d"), ["C.d"]);
        assert_eq!(field(s, &c, "F"), ["I", "K"]);
    });
}

/// javac: in `D implements I, J`, `J.f()` and `I.f()` are overridden by `D.f()`, and `I.d()` is
/// inherited once, through either.
#[test]
fn diamond() {
    with(|s| {
        let (_, d) = table(s, "D");
        assert_eq!(methods(s, &d, "f"), ["D.f"]);
        assert_eq!(methods(s, &d, "d"), ["I.d"]);
        let (_, j) = table(s, "J");
        assert_eq!(methods(s, &j, "f"), ["J.f"]);
        assert_eq!(field(s, &j, "F"), ["J"]);
    });
}

/// javac: an interface has `Object`'s public methods (JLS 9.2), overridden by its own
/// (`E.equals`), and not its protected `clone()`, which `Cl.clone()` doesn't override (W1's
/// observation 6).
#[test]
fn interfaces_have_objects_public_methods() {
    with(|s| {
        let (_, e) = table(s, "E");
        assert_eq!(methods(s, &e, "equals"), ["E.equals"]);
        let (_, cl) = table(s, "Cl");
        assert_eq!(methods(s, &cl, "clone"), ["Cl.clone"]);
        assert_eq!(methods(s, &cl, "equals"), ["java/lang/Object.equals"]);
    });
}

/// javac: `H.p(String)` overrides `G.p(T)` as a member of `G<String>`; `G.t` is inherited.
#[test]
fn overriding_as_seen_from_the_type() {
    with(|s| {
        let (_, h) = table(s, "H");
        assert_eq!(methods(s, &h, "p"), ["H.p"]);
        assert_eq!(field(s, &h, "t"), ["G"]);
    });
}

/// JLS 8.4.8: `S extends R implements I` inherits `R.f()`, a concrete method of its superclass,
/// and not `I.f()`, whose signature `R.f()`'s is a subsignature of; javac's `getAllMembers`
/// lists both.
#[test]
fn a_concrete_method_from_the_superclass() {
    with(|s| {
        let (_, t) = table(s, "S");
        assert_eq!(methods(s, &t, "f"), ["R.f"]);
        assert_eq!(methods(s, &t, "d"), ["I.d"]);
    });
}

/// A type whose superclass didn't resolve, or a Lombok-annotated one, has an incomplete table.
#[test]
fn incomplete() {
    let mut text = String::from("//- jar jdk.jar\n");
    for class in crate::testing::RELATIONS_JDK {
        text.push_str(&format!("//- class {class}\n"));
    }
    text.push_str("//- jar lombok.jar\n//- class annotation lombok/Data\n");
    text.push_str("//- set main 21 jdk.jar lombok.jar no-platform\n");
    text.push_str("//- file main Q.java\nclass Q extends Missing { int a; }\n");
    text.push_str("@lombok.Data class L { int b; }\n");
    let engine = Engine::new(crate::testing::case_input(&text));
    let s = engine.snapshot();
    let (_, q) = table(&s, "Q");
    assert!(q.incomplete);
    assert_eq!(field(&s, &q, "a"), ["Q"]);
    let (_, l) = table(&s, "L");
    assert!(l.incomplete);
}
