//! Tests of the type relations: the JLS's examples for each section, and each observation of
//! javac, with javac's answer beside it. "Observation N" is W0's, in
//! docs/design/types-oracle-normalization.md; "W1's observation N" is in
//! docs/design/semantic-type-relations.md.
//! javac's answers were observed on Temurin 27.0.1, on the same source with the real JDK, through
//! `javax.lang.model.util.Types` or the types oracle; the classes the tests name are
//! [`RELATIONS_JDK`](crate::testing::RELATIONS_JDK)'s stand-ins for the JDK's.

use crate::summary::Primitive;
use crate::symbols::TypeKey;
use crate::testing::relations_input;
use crate::types::{Context, Conversion, Printed, Tri, Type, TypeContext, TypeId, Why};
use crate::{Engine, SourceSetId};

/// An engine over `source`, one file `Q.java` in the default package.
fn engine(source: &str) -> Engine {
    Engine::new(relations_input(&format!("//- file main Q.java\n{source}")))
}

fn context(engine: &Engine) -> (crate::Snapshot,) {
    (engine.snapshot(),)
}

fn class(cx: &mut TypeContext<'_>, binary: &str) -> TypeKey {
    cx.lang(binary)
        .unwrap()
        .unwrap_or_else(|| panic!("no class {binary}"))
}

/// The declared type of the field `field` of the class `binary`.
fn field_of(cx: &mut TypeContext<'_>, binary: &str, field: &str) -> TypeId {
    let key = class(cx, binary);
    let index = cx
        .field_index(key, field)
        .unwrap()
        .unwrap_or_else(|| panic!("no field {field} in {binary}"));
    cx.field_type(key, index).unwrap()
}

/// The type of `Q`'s field `field`.
fn f(cx: &mut TypeContext<'_>, field: &str) -> TypeId {
    field_of(cx, "Q", field)
}

fn text(cx: &mut TypeContext<'_>, t: TypeId) -> String {
    cx.type_string(t, Printed::Captures).unwrap()
}

fn yes(answer: Tri) -> bool {
    match answer {
        Tri::Yes(()) => true,
        Tri::No => false,
        Tri::Unknown(why) => panic!("unknown: {why:?}"),
    }
}

/// Runs `test` with a context over `source`.
fn with(source: &str, test: impl FnOnce(&mut TypeContext<'_>)) {
    let engine = engine(source);
    let (snapshot,) = context(&engine);
    let mut cx = snapshot.types(SourceSetId::from_index(0));
    test(&mut cx);
}

const COLLECTIONS: &str = "import java.util.*;\nimport java.io.Serializable;\n";

#[test]
fn translation() {
    with(
        &format!(
            "{COLLECTIONS}class Q<T extends Comparable<T>> {{ List<String> ls; T t; Map.Entry<String, Integer> me; \
             List<? extends Number>[] arrays; int[][] ints; }}"
        ),
        |cx| {
            let ls = f(cx, "ls");
            assert_eq!(text(cx, ls), "java/util/List<java/lang/String>");
            let t = f(cx, "t");
            assert_eq!(text(cx, t), "T:Q");
            let me = f(cx, "me");
            assert_eq!(
                text(cx, me),
                "java/util/Map$Entry<java/lang/String,java/lang/Integer>"
            );
            let arrays = f(cx, "arrays");
            assert_eq!(
                text(cx, arrays),
                "java/util/List<? extends java/lang/Number>[]"
            );
            let ints = f(cx, "ints");
            assert_eq!(text(cx, ints), "int[][]");
            // A variable bounded by itself translates in one step.
            let Type::Var(owner, index) = cx.get(t) else {
                panic!("a type variable");
            };
            let bounds = cx.bounds(owner, index).unwrap();
            assert_eq!(text(cx, bounds[0]), "java/lang/Comparable<T:Q>");
        },
    );
}

/// W1's observation 11: an inner class of a generic class used inside it, or inside a subclass, is a member of the
/// generic class's type there; elsewhere, written without its outer type, it is raw.
/// javac: `Outer<T:Outer>$Inner` inside `Outer`, `Outer<java/lang/String>$Inner` in `Q extends
/// Outer<String>`, `Outer$Inner` for `Outer.Inner` in a class that isn't one; the field `t` as a
/// member of each is `T:Outer`, `java/lang/String`, and `java/lang/Object`. (`Outer.Inner`
/// inside `Q` is raw too for javac, and isn't for Ferrion: B2's types don't say a raw qualifier
/// was written; see the design's departures.)
#[test]
fn implicit_outer_types() {
    with(
        "class Outer<T> { class Inner { T t; } Inner f; static class Nested {} Nested n; }\n\
         class Q extends Outer<String> { Inner g; Outer<Integer>.Inner h; }\n\
         class R { Outer.Inner raw; }",
        |cx| {
            let f = field_of(cx, "Outer", "f");
            assert_eq!(text(cx, f), "Outer<T:Outer>$Inner");
            let n = field_of(cx, "Outer", "n");
            assert_eq!(text(cx, n), "Outer$Nested");
            let inner = class(cx, "Outer$Inner");
            let t = cx.field_index(inner, "t").unwrap().unwrap();
            for (owner, field, string, member) in [
                (
                    "Q",
                    "g",
                    "Outer<java/lang/String>$Inner",
                    "java/lang/String",
                ),
                (
                    "Q",
                    "h",
                    "Outer<java/lang/Integer>$Inner",
                    "java/lang/Integer",
                ),
                ("R", "raw", "Outer$Inner", "java/lang/Object"),
            ] {
                let ty = field_of(cx, owner, field);
                assert_eq!(text(cx, ty), string);
                let Tri::Yes(m) = cx.field_type_in(ty, inner, t).unwrap() else {
                    panic!("a member");
                };
                assert_eq!(text(cx, m), member);
            }
        },
    );
}

/// JLS 4.10.1: `double > float > long > int > char`, `int > short > byte`. javac: `int <: long`
/// and `char <: int` yes; `long <: int`, `byte <: char`, `short <: char`, `char <: short` no.
#[test]
fn primitive_subtyping() {
    with("class Q {}", |cx| {
        let p = |cx: &mut TypeContext<'_>, p| cx.primitive(p);
        let (int, long, char, byte, short) = (
            p(cx, Primitive::Int),
            p(cx, Primitive::Long),
            p(cx, Primitive::Char),
            p(cx, Primitive::Byte),
            p(cx, Primitive::Short),
        );
        assert!(yes(cx.subtype(int, long).unwrap()));
        assert!(yes(cx.subtype(char, int).unwrap()));
        assert!(yes(cx.subtype(byte, short).unwrap()));
        assert!(!yes(cx.subtype(long, int).unwrap()));
        assert!(!yes(cx.subtype(byte, char).unwrap()));
        assert!(!yes(cx.subtype(short, char).unwrap()));
        assert!(!yes(cx.subtype(char, short).unwrap()));
    });
}

/// JLS 4.10.2 and 4.10.3, each pair as javac answers `isSubtype` and `isAssignable`.
#[test]
fn class_and_array_subtyping() {
    with(
        &format!(
            "{COLLECTIONS}class Q {{ ArrayList<String> als; List<String> ls; List<Object> lo; \
             List<? extends Object> lwo; Collection<? super String> csup; ArrayList rawal; List rawl; \
             String[] sa; Object[] oa; int[] ia; long[] la; Integer[] ina; Number[] na; Object o; \
             Cloneable cl; Serializable ser; List<? extends Number> lne; List<? super Integer> lsi; \
             List<Integer> li; int i; }}"
        ),
        |cx| {
            // (s, t, subtype, assignable), javac's.
            let pairs = [
                ("als", "ls", true, true),
                ("ls", "lo", false, false),
                ("ls", "lwo", true, true),
                ("ls", "csup", true, true),
                ("ls", "rawl", true, true),
                ("lo", "lsi", true, true),
                ("rawal", "ls", false, true),
                ("rawal", "rawl", true, true),
                ("rawl", "ls", false, true),
                ("sa", "oa", true, true),
                ("ia", "oa", false, false),
                ("ia", "la", false, false),
                ("ia", "o", true, true),
                ("ia", "cl", true, true),
                ("ia", "ser", true, true),
                ("ina", "na", true, true),
                ("lne", "lsi", false, false),
                ("li", "lne", true, true),
                ("li", "lsi", true, true),
                ("i", "o", false, true),
            ];
            for (s, t, subtype, assignable) in pairs {
                let (a, b) = (f(cx, s), f(cx, t));
                assert_eq!(yes(cx.subtype(a, b).unwrap()), subtype, "{s} <: {t}");
                let converted = cx.convert(Context::Assignment, a, b, None).unwrap();
                assert_eq!(yes(converted.plain()), assignable, "{s} assignable to {t}");
            }
        },
    );
}

/// JLS 4.5.1's containment, through subtyping of parameterized types.
#[test]
fn containment() {
    use crate::summary::WildcardBound::{Extends, Super};
    use crate::types::TypeArg::{Type as T, Wildcard as W};
    with("class Q { Integer i; Number n; }", |cx| {
        let (i, n) = (f(cx, "i"), f(cx, "n"));
        assert!(yes(cx.contains(W(Some((Extends, n))), T(i)).unwrap()));
        assert!(yes(cx.contains(W(Some((Super, i))), T(n)).unwrap()));
        assert!(yes(cx.contains(W(None), T(n)).unwrap()));
        assert!(yes(cx
            .contains(W(Some((Extends, n))), W(Some((Extends, i))))
            .unwrap()));
        assert!(!yes(cx.contains(W(Some((Extends, i))), T(n)).unwrap()));
        assert!(!yes(cx.contains(T(n), T(i)).unwrap()));
        assert!(!yes(cx
            .contains(W(Some((Super, n))), W(Some((Super, i))))
            .unwrap()));
    });
}

/// W1's observation 7: `?` and `? extends Object` are the same argument (JLS 4.5.1): javac's `isSubsignature` of
/// `Properties.computeIfAbsent(Object, Function<? super Object, ?>)` and `Hashtable`'s, which
/// `Properties` sees as `(Object, Function<? super Object, ? extends Object>)`, is yes.
#[test]
fn unbounded_wildcards_are_the_same() {
    with(
        &format!("{COLLECTIONS}class Q {{ List<?> a; List<? extends Object> b; List<Object> c; }}"),
        |cx| {
            let (a, b, c) = (f(cx, "a"), f(cx, "b"), f(cx, "c"));
            assert!(yes(cx.same(a, b).unwrap()));
            assert!(!yes(cx.same(a, c).unwrap()));
        },
    );
}

/// Observation 7: capture's bounds are the glb of the wildcard's and the type parameter's.
#[test]
fn capture_bounds() {
    with(
        &format!(
            "{COLLECTIONS}class G<T extends Comparable<T>> {{}}\n\
             class Q {{ List<? super Integer> ls; G<?> g; Enum<?> en; List<? extends List<? extends Number>> ll; \
             List<String> plain; Map<? super String, ? extends Integer> map; }}"
        ),
        |cx| {
            for (field, javac) in [
                (
                    "ls",
                    "java/util/List<capture#1 extends java/lang/Object super java/lang/Integer>",
                ),
                ("g", "G<capture#1 extends java/lang/Comparable<capture#1>>"),
                (
                    "en",
                    "java/lang/Enum<capture#1 extends java/lang/Enum<capture#1>>",
                ),
                (
                    "ll",
                    "java/util/List<capture#1 extends java/util/List<? extends java/lang/Number>>",
                ),
                ("plain", "java/util/List<java/lang/String>"),
                (
                    "map",
                    "java/util/Map<capture#1 extends java/lang/Object super java/lang/String,capture#2 extends java/lang/Integer>",
                ),
            ] {
                let t = f(cx, field);
                let captured = cx.capture(t).unwrap();
                assert_eq!(text(cx, captured), javac, "{field}");
                // A captured type captures to itself.
                assert_eq!(cx.fresh_capture(captured).unwrap(), captured);
            }
        },
    );
}

/// W1's observation 5: javac's glb drops a class type whose erasure is above another's erasure,
/// and orders what is left as an intersection. Each is the capture of `Q`'s field, as javac's
/// `Types.capture` gives it.
#[test]
fn glb_in_capture() {
    with(
        &format!(
            "{COLLECTIONS}interface DbS<D extends DbS<D>> {{}} interface Dml extends DbS<Dml> {{}}\n\
             class J<S extends DbS<S>> {{}} class G<T extends Comparable<T>> {{}} class M<T extends Number> {{}}\n\
             class M2<T extends Comparable<String>> {{}} class M3<T extends Serializable> {{}}\n\
             class M4<T extends Number & Comparable<T>> {{}} interface Zz {{}} class M6<T extends Zz> {{}}\n\
             class Q {{ J<? extends Dml> j; G<? extends Integer> g; M<? extends Comparable<Integer>> m; \
             M2<? extends Serializable> m2; M3<? extends Comparable<String>> m3; M4<?> m4; \
             M4<? extends Integer> m4i; M6<? extends Comparable<String>> m6; M3<? extends Number> m3n; }}"
        ),
        |cx| {
            for (field, javac) in [
                ("j", "J<capture#1 extends Dml>"),
                ("g", "G<capture#1 extends java/lang/Integer>"),
                (
                    "m",
                    "M<capture#1 extends java/lang/Number&java/lang/Comparable<java/lang/Integer>>",
                ),
                (
                    "m2",
                    "M2<capture#1 extends java/io/Serializable&java/lang/Comparable<java/lang/String>>",
                ),
                (
                    "m3",
                    "M3<capture#1 extends java/io/Serializable&java/lang/Comparable<java/lang/String>>",
                ),
                (
                    "m4",
                    "M4<capture#1 extends java/lang/Number&java/lang/Comparable<capture#1>>",
                ),
                ("m4i", "M4<capture#1 extends java/lang/Integer>"),
                (
                    "m6",
                    "M6<capture#1 extends Zz&java/lang/Comparable<java/lang/String>>",
                ),
                ("m3n", "M3<capture#1 extends java/lang/Number>"),
            ] {
                let t = f(cx, field);
                let captured = cx.capture(t).unwrap();
                assert_eq!(text(cx, captured), javac, "{field}");
            }
        },
    );
}

/// JLS 4.6.
#[test]
fn erasure() {
    with(
        &format!(
            "{COLLECTIONS}class Outer<T> {{ class Inner {{}} }}\n\
             class Q<T extends Comparable<T>, U extends Number & Comparable<U>, V> {{ List<String> ls; T t; U u; V v; \
             T[] ta; Outer<String>.Inner oi; }}"
        ),
        |cx| {
            for (field, javac) in [
                ("ls", "java/util/List"),
                ("t", "java/lang/Comparable"),
                ("u", "java/lang/Number"),
                ("v", "java/lang/Object"),
                ("ta", "java/lang/Comparable[]"),
                ("oi", "Outer$Inner"),
            ] {
                let t = f(cx, field);
                let erased = cx.erasure(t).unwrap();
                assert_eq!(text(cx, erased), javac, "{field}");
            }
        },
    );
}

fn lub_text(cx: &mut TypeContext<'_>, a: &str, b: &str) -> String {
    let (a, b) = (f(cx, a), f(cx, b));
    let lub = cx.lub(&[a, b]).unwrap();
    let reversed = cx.lub(&[b, a]).unwrap();
    assert_eq!(lub, reversed, "lub doesn't depend on the operands' order");
    text(cx, lub)
}

/// Observation 1: `lub(Integer, String)`, as `var x = b ? 1 : "x";` types it. javac computes the
/// recursion through `Comparable`'s argument once more and then writes `?`, where JLS 4.10.4
/// leaves an infinite type. W1's observation 3: nested in another lub, each lub of the same operands
/// is computed while they are on the stack at most twice (`b ? listOfInteger : listOfString`).
#[test]
fn lub_cut_off() {
    with(
        &format!(
            "{COLLECTIONS}class Q {{ Integer i; String s; Long l; StringBuilder sb; List<Integer> li; \
             List<String> ls; List<Long> ll; List<Object> lo; String[] sa; Integer[] ia; int[] pa; long[] la; }}"
        ),
        |cx| {
            let one = "java/io/Serializable&java/lang/Comparable<? extends java/io/Serializable&java/lang/Comparable<?>&java/lang/constant/Constable&java/lang/constant/ConstantDesc>&java/lang/constant/Constable&java/lang/constant/ConstantDesc";
            assert_eq!(lub_text(cx, "i", "s"), one);
            assert_eq!(
                lub_text(cx, "li", "ls"),
                format!("java/util/List<? extends {one}>")
            );
            assert_eq!(
                lub_text(cx, "ll", "li"),
                "java/util/List<? extends java/lang/Number&java/lang/Comparable<? extends java/lang/Number&java/lang/Comparable<?>&java/lang/constant/Constable&java/lang/constant/ConstantDesc>&java/lang/constant/Constable&java/lang/constant/ConstantDesc>"
            );
            assert_eq!(
                lub_text(cx, "ls", "lo"),
                "java/util/List<? extends java/lang/Object>"
            );
            assert_eq!(lub_text(cx, "sa", "ia"), format!("{one}[]"));
            assert_eq!(
                lub_text(cx, "pa", "la"),
                "java/io/Serializable&java/lang/Cloneable"
            );
            assert_eq!(
                lub_text(cx, "sb", "s"),
                "java/io/Serializable&java/lang/CharSequence&java/lang/Comparable<? extends java/io/Serializable&java/lang/CharSequence&java/lang/Comparable<?>>"
            );
        },
    );
}

/// Observation 2, and W1's observation 2: the class first, then interfaces deepest first, then
/// by qualified name, whichever operand comes first.
#[test]
fn intersection_order() {
    let source = "//- file main a/Ia.java\npackage a; public interface Ia {}\n\
        //- file main b/Ia.java\npackage b; public interface Ia {}\n\
        //- file main z/Order.java\npackage z;\n\
        interface Zed {} interface Ab {} interface Mid extends Ab {} interface Top {}\n\
        interface Deep1 extends Top {} interface Deep2 extends Deep1 {} interface Deep3 extends Deep2 {}\n\
        class Base {} class Base2 extends Base {} class Base3 extends Base2 {}\n\
        class P1 extends Base3 implements b.Ia, a.Ia {} class P2 extends Base3 implements a.Ia, b.Ia {}\n\
        class Q1 extends Base3 implements Zed, Deep3 {} class Q2 extends Base3 implements Deep3, Zed {}\n\
        class R1 implements Mid, Zed, Ab {} class R2 implements Zed, Mid {}\n\
        class S1 extends Base3 implements Zed {} class S2 extends Base2 implements Zed {}\n\
        class T1 implements Deep1, Zed {} class T2 implements Zed, Deep2 {}\n\
        class X1 implements Zed, Ab {} class X2 implements Ab, Zed {}\n\
        class Q { P1 p1; P2 p2; Q1 q1; Q2 q2; R1 r1; R2 r2; S1 s1; S2 s2; T1 t1; T2 t2; X1 x1; X2 x2; }\n";
    let engine = Engine::new(relations_input(source));
    let snapshot = engine.snapshot();
    let cx = &mut snapshot.types(SourceSetId::from_index(0));
    for (a, b, javac) in [
        ("p1", "p2", "z/Base3&a/Ia&b/Ia"),
        ("q1", "q2", "z/Base3&z/Deep3&z/Zed"),
        ("r1", "r2", "z/Mid&z/Zed"),
        ("s1", "s2", "z/Base2&z/Zed"),
        ("t1", "t2", "z/Deep1&z/Zed"),
        ("x1", "x2", "z/Ab&z/Zed"),
    ] {
        let (a, b) = (field_of(cx, "z/Q", a), field_of(cx, "z/Q", b));
        let lub = cx.lub(&[a, b]).unwrap();
        assert_eq!(text(cx, lub), javac);
    }
}

/// W1's observation 4: a conditional's operands have captured types, so `lub` of
/// wildcard-parameterized types is of their captures: `var a = b ? superInteger : extendsInteger;`
/// is javac's `java/util/List<? extends java/lang/Object>` (`lub`, then `var`'s upward
/// projection), where JLS 4.10.4's `lcta` would give `List<Integer>`, which isn't above
/// `List<? super Integer>`. The `relations` fuzz target found it.
#[test]
fn lub_of_wildcard_types() {
    with(
        &format!(
            "{COLLECTIONS}class Q {{ List<? super Integer> s; List<? extends Integer> e; List<Integer> i; \
             List<? super Number> sn; List<? extends Number> en; List<?> w; }}"
        ),
        |cx| {
            for (a, b, javac) in [
                ("s", "e", "java/util/List<? extends java/lang/Object>"),
                ("i", "s", "java/util/List<? extends java/lang/Object>"),
                ("sn", "s", "java/util/List<? extends java/lang/Object>"),
                ("en", "e", "java/util/List<? extends java/lang/Number>"),
                ("w", "i", "java/util/List<? extends java/lang/Object>"),
                ("i", "en", "java/util/List<? extends java/lang/Number>"),
            ] {
                let (x, y) = (f(cx, a), f(cx, b));
                let lub = cx.lub(&[x, y]).unwrap();
                for operand in [x, y] {
                    assert!(yes(cx.subtype(operand, lub).unwrap()), "{a}, {b}");
                }
                let projected = cx.upward_projection(lub).unwrap();
                assert_eq!(text(cx, projected), javac, "{a}, {b}");
            }
        },
    );
}

/// W1's observation 4 again: each parameterization `lub` combines is captured, so 4.10.4's
/// `lcta(? super U, V) = ? super glb(U, V)` never applies; javac's types of the conditionals,
/// after `var`'s upward projection. The `relations` fuzz target found the first.
#[test]
fn lub_captures_parameterizations() {
    with(
        &format!(
            "{COLLECTIONS}interface I<T> {{}} interface J<T> extends I<T> {{}} class A<T> implements J<T> {{}}\n\
             class B<T> extends A<List<T>> {{}}\n\
             class C extends B<String> implements Comparable<C> {{ public int compareTo(C c) {{ return 0; }} }}\n\
             class Q {{ A<? extends List<? super Comparable<? super C>>> x; C c; List<? super Comparable<? super C>> y; \
             List<String> z; List<? super Integer> u; List<? super Comparable<Integer>> w; }}"
        ),
        |cx| {
            for (a, b, javac) in [
                (
                    "x",
                    "c",
                    "A<? extends java/util/List<? extends java/lang/Object>>",
                ),
                ("y", "z", "java/util/List<? extends java/lang/Object>"),
                ("u", "w", "java/util/List<? extends java/lang/Object>"),
            ] {
                let (x, y) = (f(cx, a), f(cx, b));
                let lub = cx.lub(&[x, y]).unwrap();
                for operand in [x, y] {
                    assert!(yes(cx.subtype(operand, lub).unwrap()), "{a}, {b}");
                }
                let projected = cx.upward_projection(lub).unwrap();
                assert_eq!(text(cx, projected), javac, "{a}, {b}");
            }
        },
    );
}

/// Observation 3: only the minimal candidates.
#[test]
fn lub_minimal_candidates() {
    with(
        &format!(
            "{COLLECTIONS}class Q {{ ArrayList<String> al; LinkedList<String> ll; HashSet<String> hs; \
             List<String> list; Set<String> set; }}"
        ),
        |cx| {
            assert_eq!(
                lub_text(cx, "al", "ll"),
                "java/util/AbstractList<java/lang/String>&java/io/Serializable&java/lang/Cloneable"
            );
            assert_eq!(
                lub_text(cx, "al", "hs"),
                "java/util/AbstractCollection<java/lang/String>&java/io/Serializable&java/lang/Cloneable"
            );
            assert_eq!(
                lub_text(cx, "list", "set"),
                "java/util/Collection<java/lang/String>"
            );
        },
    );
}

/// Observation 9: `var` takes the upward projection.
#[test]
fn upward_projection() {
    with(
        &format!(
            "{COLLECTIONS}class G<T extends Comparable<T>> {{}}\n\
             class Q {{ List<? extends Number> ln; Class<?> c; Enum<?> en; G<?> g; List<String> ls; }}"
        ),
        |cx| {
            let project = |cx: &mut TypeContext<'_>, t: TypeId| {
                let p = cx.upward_projection(t).unwrap();
                text(cx, p)
            };
            // `xs.get(0)`: the capture variable.
            let ln = f(cx, "ln");
            let captured = cx.capture(ln).unwrap();
            let element = cx.arena().class_args(captured)[0];
            let crate::types::TypeArg::Type(element) = element else {
                panic!("a capture variable");
            };
            assert_eq!(project(cx, element), "java/lang/Number");
            // `en.getDeclaringClass()`: `Class<capture of Enum<?>>`.
            let en = f(cx, "en");
            let captured = cx.capture(en).unwrap();
            let crate::types::TypeArg::Type(var) = cx.arena().class_args(captured)[0] else {
                panic!("a capture variable");
            };
            let class_key = class(cx, "java/lang/Class");
            let class = cx.class_type(class_key, None, Some(&[crate::types::TypeArg::Type(var)]));
            assert_eq!(
                project(cx, class),
                "java/lang/Class<? extends java/lang/Enum<?>>"
            );
            // `g.get()`.
            let g = f(cx, "g");
            let captured = cx.capture(g).unwrap();
            let crate::types::TypeArg::Type(var) = cx.arena().class_args(captured)[0] else {
                panic!("a capture variable");
            };
            assert_eq!(project(cx, var), "java/lang/Comparable<?>");
            let ls = f(cx, "ls");
            assert_eq!(project(cx, ls), "java/util/List<java/lang/String>");
        },
    );
}

/// JLS 4.10's direct supertypes, which javac's `Types.directSupertypes` departs from
/// (observation 16; the relations comparison converts): an array's three, a wildcard type's
/// through capture, an interface's `Object` only without superinterfaces.
#[test]
fn direct_supertypes() {
    with(
        &format!(
            "{COLLECTIONS}interface I {{}} class Q {{ int[] ia; String[] sa; List<? extends Number> ln; I i; List raw; }}"
        ),
        |cx| {
            let supers = |cx: &mut TypeContext<'_>, field: &str| -> Vec<String> {
                let t = f(cx, field);
                let Tri::Yes(supers) = cx.direct_supertypes(t).unwrap() else {
                    panic!("supertypes");
                };
                supers.into_iter().map(|s| text(cx, s)).collect()
            };
            assert_eq!(
                supers(cx, "ia"),
                [
                    "java/lang/Object",
                    "java/lang/Cloneable",
                    "java/io/Serializable"
                ]
            );
            assert_eq!(
                supers(cx, "sa"),
                [
                    "java/lang/Object[]",
                    "java/io/Serializable[]",
                    "java/lang/Comparable<java/lang/String>[]",
                    "java/lang/CharSequence[]",
                    "java/lang/constant/Constable[]",
                    "java/lang/constant/ConstantDesc[]"
                ]
            );
            assert_eq!(
                supers(cx, "ln"),
                ["java/util/SequencedCollection<capture#1 extends java/lang/Number>"]
            );
            assert_eq!(supers(cx, "i"), ["java/lang/Object"]);
            assert_eq!(supers(cx, "raw"), ["java/util/SequencedCollection"]);
        },
    );
}

/// Observation 17, W1's observations 9 and 10, and JLS 4.8: members as seen from a type. javac's `asMemberOf` substitutes
/// without capture: `Box<? extends Number>`'s `get` is `()? extends java/lang/Number`, `take`'s
/// `(java/util/List<? super ? extends java/lang/Number>)void`, `all`'s
/// `()? extends java/lang/Number[]`; a raw `Box`'s are erased.
#[test]
fn member_types() {
    with(
        &format!(
            "{COLLECTIONS}class Box<T> {{ T get() {{ return null; }} void put(List<? extends T> l) {{}} \
             void take(List<? super T> l) {{}} T[] all() {{ return null; }} <U extends T> U as(Class<U> c) {{ return null; }} \
             static <S> S make() {{ return null; }} }}\n\
             class Q {{ Box<? extends Number> bn; Box rawb; Box<String> bs; }}"
        ),
        |cx| {
            let key = class(cx, "Box");
            let methods: Vec<(String, u32)> = (0..cx.method_count(key).unwrap() as u32)
                .map(|i| (cx.method_key(key, i).unwrap(), i))
                .collect();
            let method = |name: &str| {
                methods
                    .iter()
                    .find(|(k, _)| k.contains(&format!(".{name}(")))
                    .unwrap()
                    .1
            };
            let member = |cx: &mut TypeContext<'_>, of: &str, name: &str, printed| {
                let of = f(cx, of);
                let Tri::Yes(m) = cx.method_type_in(of, key, method(name)).unwrap() else {
                    panic!("a member");
                };
                cx.method_string(&m, printed).unwrap()
            };
            let w = Printed::Wildcards;
            assert_eq!(member(cx, "bn", "get", w), "()? extends java/lang/Number");
            assert_eq!(
                member(cx, "bn", "put", w),
                "(java/util/List<? extends java/lang/Number>)void"
            );
            assert_eq!(
                member(cx, "bn", "take", w),
                "(java/util/List<? super ? extends java/lang/Number>)void"
            );
            assert_eq!(member(cx, "bn", "all", w), "()? extends java/lang/Number[]");
            // As the JLS has it, through the capture.
            assert_eq!(
                member(cx, "bn", "get", Printed::Captures),
                "()capture#1 extends java/lang/Number"
            );
            assert_eq!(member(cx, "rawb", "get", w), "()java/lang/Object");
            assert_eq!(member(cx, "rawb", "put", w), "(java/util/List)void");
            assert_eq!(
                member(cx, "rawb", "as", w),
                "(java/lang/Class)java/lang/Object"
            );
            assert_eq!(member(cx, "bs", "get", w), "()java/lang/String");
            // A static method isn't substituted or erased.
            assert_eq!(
                member(cx, "rawb", "make", w),
                "<S:Box.make()Ljava/lang/Object;>()S:Box.make()Ljava/lang/Object;"
            );
        },
    );
}

/// Observation 18 and JLS 8.4.2: `n2(List)` is a subsignature of `n(List<String>)` and of
/// `<T> m(List<T>)` (the erasure rule), and not the reverse; JLS 8.4.5's
/// return-type-substitutability; JLS 8.4.8.1's overriding.
#[test]
fn signatures() {
    with(
        &format!(
            "{COLLECTIONS}abstract class A<T> {{ void n(List<String> l) {{}} <T> void m(List<T> l) {{}} void n2(List l) {{}} \
             Object r() {{ return null; }} List<String> r2() {{ return null; }} int r3() {{ return 0; }} \
             void p(T t) {{}} static void s() {{}} private void q() {{}} protected void pr() {{}} }}\n\
             class B extends A<String> {{ String r() {{ return null; }} List r2() {{ return null; }} void p(String t) {{}} \
             static void s() {{}} void q() {{}} protected void pr() {{}} }}\n\
             interface Cl {{ Cl clone(); boolean equals(Object o); }}\n\
             class Q {{}}"
        ),
        |cx| {
            let a = class(cx, "A");
            let b = class(cx, "B");
            let find = |cx: &mut TypeContext<'_>, key: TypeKey, name: &str| -> u32 {
                (0..cx.method_count(key).unwrap() as u32)
                    .find(|&i| {
                        cx.method_key(key, i)
                            .unwrap()
                            .contains(&format!(".{name}("))
                    })
                    .unwrap_or_else(|| panic!("no method {name}"))
            };
            let declared = |cx: &mut TypeContext<'_>, key, name| {
                let i = find(cx, key, name);
                cx.method_type(key, i).unwrap()
            };
            let (n, m, n2) = (
                declared(cx, a, "n"),
                declared(cx, a, "m"),
                declared(cx, a, "n2"),
            );
            assert!(yes(cx.subsignature(&n2, &n).unwrap()));
            assert!(yes(cx.subsignature(&n2, &m).unwrap()));
            assert!(!yes(cx.subsignature(&n, &n2).unwrap()));
            assert!(!yes(cx.subsignature(&m, &n2).unwrap()));
            assert!(yes(cx.override_equivalent(&n, &n2).unwrap()));
            // Results.
            let (ar, br) = (declared(cx, a, "r"), declared(cx, b, "r"));
            assert!(yes(cx.return_substitutable(&br, &ar).unwrap()));
            assert!(!yes(cx.return_substitutable(&ar, &br).unwrap()));
            let (ar2, br2) = (declared(cx, a, "r2"), declared(cx, b, "r2"));
            assert!(yes(cx.return_substitutable(&br2, &ar2).unwrap()));
            let ar3 = declared(cx, a, "r3");
            assert!(!yes(cx.return_substitutable(&ar3, &ar).unwrap()));
            // Overriding, as `Elements.overrides` answers it.
            let overrides = |cx: &mut TypeContext<'_>, name: &str| {
                let (bm, am) = (find(cx, b, name), find(cx, a, name));
                yes(cx.overrides((b, bm), (a, am), b).unwrap())
            };
            assert!(
                overrides(cx, "p"),
                "p(String) overrides p(T) as A<String>'s"
            );
            assert!(overrides(cx, "r"));
            assert!(overrides(cx, "pr"));
            assert!(!overrides(cx, "s"), "a static method hides");
            assert!(!overrides(cx, "q"), "a private method isn't inherited");
            // W1's observation 6: an interface's `clone()` doesn't override `Object`'s, which is
            // protected; its `equals` does.
            let (cl, object) = (class(cx, "Cl"), class(cx, "java/lang/Object"));
            let clone = find(cx, cl, "clone");
            let equals = find(cx, cl, "equals");
            let object_equals = find(cx, object, "equals");
            let object_clone = find(cx, object, "clone");
            assert!(!yes(cx
                .overrides((cl, clone), (object, object_clone), cl)
                .unwrap()));
            assert!(yes(cx
                .overrides((cl, equals), (object, object_equals), cl)
                .unwrap()));
        },
    );
}

/// JLS 5.2 to 5.6: the conversions each context applies.
#[test]
fn conversions() {
    use Conversion::*;
    with(
        &format!(
            "{COLLECTIONS}class Q<T extends Integer> {{ byte b; short s; char c; int i; long l; double d; Integer bi; Byte bb; \
             Object o; Number n; List raw; List<String> ls; ArrayList<String> als; Collection<?> cw; String str; \
             List<Integer> li; T t; Long bl; }}"
        ),
        |cx| {
            let steps =
                |cx: &mut TypeContext<'_>, context, from: &str, to: &str, constant: Option<i32>| {
                    let (a, b) = (f(cx, from), f(cx, to));
                    let constant = constant.map(crate::constants::Constant::Int);
                    match cx.convert(context, a, b, constant.as_ref()).unwrap() {
                        Tri::Yes(steps) => Some(steps.as_slice().to_vec()),
                        Tri::No => None,
                        Tri::Unknown(why) => panic!("unknown {why:?}"),
                    }
                };
            let a = Context::Assignment;
            assert_eq!(
                steps(cx, a, "i", "b", Some(42)),
                Some(vec![NarrowingPrimitive])
            );
            assert_eq!(steps(cx, a, "i", "b", Some(200)), None);
            assert_eq!(steps(cx, a, "i", "b", None), None);
            assert_eq!(
                steps(cx, a, "i", "bb", Some(42)),
                Some(vec![NarrowingPrimitive, Boxing])
            );
            assert_eq!(steps(cx, a, "i", "l", None), Some(vec![WideningPrimitive]));
            assert_eq!(steps(cx, a, "i", "bi", None), Some(vec![Boxing]));
            assert_eq!(
                steps(cx, a, "i", "o", None),
                Some(vec![Boxing, WideningReference])
            );
            assert_eq!(steps(cx, a, "bi", "i", None), Some(vec![Unboxing]));
            assert_eq!(
                steps(cx, a, "bi", "l", None),
                Some(vec![Unboxing, WideningPrimitive])
            );
            assert_eq!(
                steps(cx, a, "t", "i", None),
                Some(vec![WideningReference, Unboxing])
            );
            assert_eq!(steps(cx, a, "raw", "ls", None), Some(vec![Unchecked]));
            assert_eq!(
                steps(cx, a, "als", "ls", None),
                Some(vec![WideningReference])
            );
            assert_eq!(steps(cx, a, "ls", "ls", None), Some(vec![Identity]));
            assert_eq!(steps(cx, a, "i", "bl", None), None);
            // Strict invocation allows no boxing; loose does.
            assert_eq!(steps(cx, Context::StrictInvocation, "i", "bi", None), None);
            assert_eq!(
                steps(cx, Context::LooseInvocation, "i", "bi", None),
                Some(vec![Boxing])
            );
            assert_eq!(
                steps(cx, Context::StrictInvocation, "i", "l", None),
                Some(vec![WideningPrimitive])
            );
            // Casts.
            let c = Context::Casting;
            assert_eq!(steps(cx, c, "l", "i", None), Some(vec![NarrowingPrimitive]));
            assert_eq!(
                steps(cx, c, "b", "c", None),
                Some(vec![WideningNarrowingPrimitive])
            );
            assert_eq!(
                steps(cx, c, "o", "bi", None),
                Some(vec![NarrowingReference])
            );
            assert_eq!(
                steps(cx, c, "o", "i", None),
                Some(vec![NarrowingReference, Unboxing])
            );
            assert_eq!(steps(cx, c, "bi", "str", None), None);
            assert_eq!(
                steps(cx, c, "ls", "als", None),
                Some(vec![NarrowingReference])
            );
            assert_eq!(
                steps(cx, c, "cw", "als", None),
                Some(vec![NarrowingReference, Unchecked])
            );
            assert_eq!(steps(cx, c, "li", "ls", None), None, "provably distinct");
            // String conversion, for `+`.
            assert_eq!(
                steps(cx, Context::String, "bi", "str", None),
                Some(vec![String])
            );
            // Numeric promotion.
            let (b, s, i, l, bi, d) = (
                f(cx, "b"),
                f(cx, "s"),
                f(cx, "i"),
                f(cx, "l"),
                f(cx, "bi"),
                f(cx, "d"),
            );
            let promoted = |cx: &mut TypeContext<'_>, x, y| match cx.binary_promotion(x, y).unwrap()
            {
                Tri::Yes(t) => text(cx, t),
                _ => panic!("promoted"),
            };
            assert_eq!(promoted(cx, b, s), "int");
            assert_eq!(promoted(cx, i, l), "long");
            assert_eq!(promoted(cx, bi, d), "double");
            let Tri::Yes(unary) = cx.unary_promotion(b).unwrap() else {
                panic!("promoted");
            };
            assert_eq!(text(cx, unary), "int");
        },
    );
}

/// Errors and incomplete hierarchies: what is found through the supertypes that resolved is
/// `Yes`, `No` needs the whole hierarchy.
#[test]
fn unknowns() {
    with(
        &format!(
            "{COLLECTIONS}class X extends Missing implements Serializable {{}}\nclass Q {{ X x; Missing m; Serializable s; Object o; List<String> ls; }}"
        ),
        |cx| {
            let (x, m, s, o, ls) = (f(cx, "x"), f(cx, "m"), f(cx, "s"), f(cx, "o"), f(cx, "ls"));
            assert!(matches!(cx.get(m), Type::Error(_)));
            assert!(matches!(
                cx.subtype(m, o).unwrap(),
                Tri::Unknown(Why::Error(_))
            ));
            assert!(yes(cx.subtype(x, o).unwrap()));
            assert!(yes(cx.subtype(x, s).unwrap()));
            assert!(matches!(cx.subtype(x, ls).unwrap(), Tri::Unknown(_)));
        },
    );
}

/// Expansive inheritance makes subtyping undecidable in general: the query ends `Unknown(Limit)`
/// rather than hanging, and within its fuel.
#[test]
fn termination() {
    with(
        "class N<Z> {} class C<X> extends N<N<? super C<C<X>>>> {}\n\
         class Q<T> { C<T> c; N<? super C<T>> n; }",
        |cx| {
            let (c, n) = (f(cx, "c"), f(cx, "n"));
            cx.refuel(10_000);
            let answer = cx.subtype(c, n).unwrap();
            assert!(matches!(answer, Tri::Unknown(Why::Limit)), "{answer:?}");
            assert!(cx.fuel_used() <= 10_000);
            // A context that ran out answers its next query afresh.
            cx.refuel(10_000);
            let o = cx.lang("java/lang/Object").unwrap().unwrap();
            let object = cx.class_type(o, None, None);
            assert!(yes(cx.subtype(c, object).unwrap()));
        },
    );
}

/// The properties the `relations` fuzz target checks, on inputs from a fixed seed.
#[test]
fn properties() {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..300 {
        let mut bytes = Vec::with_capacity(96);
        for _ in 0..96 {
            // xorshift64*
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            bytes.push((state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8);
        }
        if let Err(message) = crate::invariants::check_relations(&bytes) {
            panic!("{message}\ninput: {bytes:?}");
        }
    }
}

/// Types the `relations` fuzz target and the property test failed on, fixed.
#[test]
fn fuzz_regressions() {
    let cases: &[(&str, &[&str])] = &[
        (
            "lub(A<? super C>, A<? extends C>) was A<C>, not above A<? super C>",
            &[
                "A<? super C>",
                "A<? extends C>",
                "A<? extends B<?>>",
                "Collection<?>",
                "A<?>",
            ],
        ),
        (
            "lub(A<? extends List<? super Comparable<? super C>>>, C) was \
             A<? extends List<? super String>>, through an erased glb",
            &[
                "A<? extends List<? super Comparable<? super C>>>",
                "C",
                "String",
                "O<Object>.In",
            ],
        ),
        (
            "lub(O<Integer>.In, O<List<?>>.In) lost the outer types' lci",
            &[
                "O<List<?>>.In2<? super O<Object>.In[]>",
                "O<G>.In2<? super Object>",
                "O<Integer>.In",
                "P",
            ],
        ),
    ];
    for (what, fields) in cases {
        let fields: Vec<String> = fields.iter().map(|f| (*f).to_owned()).collect();
        if let Err(message) = crate::invariants::check_types(&fields) {
            panic!("{what}: {message}");
        }
    }
}
