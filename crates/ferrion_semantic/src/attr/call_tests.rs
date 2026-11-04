//! Tests of method invocation and class instance creation: each input was run through javac
//! (Temurin 27.0.1, the types oracle) on the same source, and javac's chosen method and type are
//! written beside it (docs/design/semantic-attribution.md#observations). The classes the tests
//! name are [`RELATIONS_JDK`](crate::testing::RELATIONS_JDK)'s stand-ins for the JDK's, and
//! classes of the input itself where javac's answer doesn't depend on the JDK's members.

use super::tests::{Expr, engine, expressions, ty};
use crate::SourceSetId;

/// One call: its source text (white space collapsed), javac's key for its method, and its type as
/// invoked.
#[derive(Debug)]
struct CallRow {
    text: String,
    method: String,
    invoked: String,
    anonymous: Option<String>,
}

fn calls(source: &str) -> Vec<CallRow> {
    let engine = engine(&format!("//- file main Q.java\n{source}"));
    let snapshot = engine.snapshot();
    let file = snapshot.files(SourceSetId::from_index(0)).unwrap()[0];
    let types = snapshot.body_types(file).unwrap();
    let tree = snapshot.tree(file).unwrap();
    types
        .calls
        .iter()
        .map(|(node, call)| {
            let text = snapshot.call_text(file, &types, *node, call).unwrap();
            CallRow {
                text: tree
                    .node(*node)
                    .text()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                method: text.method,
                invoked: text.invoked,
                anonymous: text.anonymous,
            }
        })
        .collect()
}

/// The method and type as invoked of the call whose text is `text`.
fn call(rows: &[CallRow], text: &str) -> (String, String) {
    let row = rows
        .iter()
        .find(|r| r.text == text)
        .unwrap_or_else(|| panic!("no call `{text}` in {rows:#?}"));
    (row.method.clone(), row.invoked.clone())
}

fn method(rows: &[CallRow], text: &str) -> String {
    call(rows, text).0
}

fn invoked(rows: &[CallRow], text: &str) -> String {
    call(rows, text).1
}

fn no_call(rows: &[CallRow], text: &str) {
    assert!(
        !rows.iter().any(|r| r.text == text),
        "a method chosen for `{text}`"
    );
}

/// JLS 15.12.2's phases. javac: `s1(1)` `s1(I)V`, `s1(1L)` `s1(J)V`, `s1("x")` the `Object`
/// one, `s1()` and `s1(1, 2)` the variable arity one with `(int[])void`, `s1(boxed)` the `Object`
/// one (strict beats unboxing), `s1(sh)` `s1(I)V`; `s2(1)` the `long` one over boxing; `s4(null)`
/// the `String` one; `s5()` and `s5("a")` `String...`, `s5(1)` `Object...`.
#[test]
fn the_phases() {
    let rows = calls(
        "class Q {
           static void s1(int i) {} static void s1(long l) {} static void s1(Object o) {} static void s1(int... is) {}
           static void s2(Integer i) {} static void s2(long l) {}
           static void s4(Object o) {} static void s4(String s) {}
           static void s5(Object... os) {} static void s5(String... ss) {}
           static void s6(char c) {} static void s6(int i) {}
           static void s7(byte b) {} static void s7(int i) {}
           void f(Integer boxed, short sh) {
             s1(1); s1(1L); s1(\"x\"); s1(); s1(1, 2); s1(boxed); s1(sh);
             s2(1); s2(boxed); s4(null); s5(); s5(\"a\"); s5(1); s6('a'); s6(1); s7((byte) 1); s7(1);
           }
         }",
    );
    assert_eq!(method(&rows, "s1(1)"), "method Q.s1(I)V");
    assert_eq!(invoked(&rows, "s1(1)"), "(int)void");
    assert_eq!(method(&rows, "s1(1L)"), "method Q.s1(J)V");
    assert_eq!(
        method(&rows, "s1(\"x\")"),
        "method Q.s1(Ljava/lang/Object;)V"
    );
    assert_eq!(method(&rows, "s1()"), "method Q.s1([I)V");
    assert_eq!(invoked(&rows, "s1()"), "(int[])void");
    assert_eq!(method(&rows, "s1(1, 2)"), "method Q.s1([I)V");
    assert_eq!(
        method(&rows, "s1(boxed)"),
        "method Q.s1(Ljava/lang/Object;)V"
    );
    assert_eq!(method(&rows, "s1(sh)"), "method Q.s1(I)V");
    assert_eq!(method(&rows, "s2(1)"), "method Q.s2(J)V");
    assert_eq!(
        method(&rows, "s2(boxed)"),
        "method Q.s2(Ljava/lang/Integer;)V"
    );
    assert_eq!(
        method(&rows, "s4(null)"),
        "method Q.s4(Ljava/lang/String;)V"
    );
    assert_eq!(method(&rows, "s5()"), "method Q.s5([Ljava/lang/String;)V");
    assert_eq!(
        method(&rows, "s5(\"a\")"),
        "method Q.s5([Ljava/lang/String;)V"
    );
    assert_eq!(method(&rows, "s5(1)"), "method Q.s5([Ljava/lang/Object;)V");
    assert_eq!(method(&rows, "s6('a')"), "method Q.s6(C)V");
    assert_eq!(method(&rows, "s6(1)"), "method Q.s6(I)V");
    assert_eq!(method(&rows, "s7((byte) 1)"), "method Q.s7(B)V");
    assert_eq!(method(&rows, "s7(1)"), "method Q.s7(I)V");
}

/// JLS 15.12.2.5, with its variable arity rule. javac: `obj(ss)`, `obj(os)`, `obj(null)` and `obj()`
/// the `Object...` one, `obj(is)` and `obj(1)` the `Object` one; `ns()` and `ns(1)` `int...`,
/// `ns(1L)` `long...`; `gs("a")` the `String` one. `s3(boxed, 1)`, `bx(1, 2)`, and `vs(1)` are
/// ambiguous (`compiler.err.ref.ambiguous`): `long` and `Object` have no subtyping, and `vs`'s are
/// each more specific than the other.
#[test]
fn most_specific() {
    let rows = calls(
        "class Q {
           void obj(Object o) { } void obj(Object... o) { }
           void ns(long... l) { } void ns(int... l) { }
           void gs(String s, Object... o) { } void gs(Object o, Object... os) { }
           static void s3(long a, Object b) {} static void s3(Object a, Object b) {}
           void bx(Integer... l) { } void bx(int a, long... l) { }
           void vs(int a, int... b) { } void vs(int... b) { }
           void arr(String[] ss, Object[] os, int[] is, Integer boxed) {
             obj(ss); obj(os); obj(is); obj(1); obj(); obj(null); obj(1, 2);
             ns(); ns(1); ns(1L); gs(\"a\"); gs(\"a\", 1); gs(1);
             s3(boxed, 1); bx(1, 2); vs(1);
           }
         }",
    );
    let array = "method Q.obj([Ljava/lang/Object;)V";
    let single = "method Q.obj(Ljava/lang/Object;)V";
    assert_eq!(method(&rows, "obj(ss)"), array);
    assert_eq!(method(&rows, "obj(os)"), array);
    assert_eq!(method(&rows, "obj(is)"), single);
    assert_eq!(method(&rows, "obj(1)"), single);
    assert_eq!(method(&rows, "obj()"), array);
    assert_eq!(method(&rows, "obj(null)"), array);
    assert_eq!(method(&rows, "obj(1, 2)"), array);
    assert_eq!(method(&rows, "ns()"), "method Q.ns([I)V");
    assert_eq!(method(&rows, "ns(1)"), "method Q.ns([I)V");
    assert_eq!(method(&rows, "ns(1L)"), "method Q.ns([J)V");
    let string = "method Q.gs(Ljava/lang/String;[Ljava/lang/Object;)V";
    assert_eq!(method(&rows, "gs(\"a\")"), string);
    assert_eq!(method(&rows, "gs(\"a\", 1)"), string);
    assert_eq!(
        method(&rows, "gs(1)"),
        "method Q.gs(Ljava/lang/Object;[Ljava/lang/Object;)V"
    );
    no_call(&rows, "s3(boxed, 1)");
    no_call(&rows, "bx(1, 2)");
    no_call(&rows, "vs(1)");
}

/// An ambiguous call's type is an error, with the name.
#[test]
fn ambiguous_is_an_error() {
    let e = expressions(
        "class Q { static void s3(long a, Object b) {} static void s3(Object a, Object b) {}
           void f(Integer boxed) { s3(boxed, 1); } }",
    );
    assert_eq!(ty(&e, "s3(boxed, 1)"), "error s3");
}

/// Override-equivalent abstract methods inherited together: javac takes one with the most specific
/// return type, in the order it searches supertypes (the class, then interfaces deepest first,
/// then by qualified name), with the first two swapped. Observed: of `Ab`, `Mid`, `Zed` it takes
/// `Mid`, of `Ab`, `Zed` it takes `Zed`, of `AA` (a class), `Ab`, `Zed` it takes `Ab`; with returns
/// `Object`, `Object`, `String`, `String` it takes the first `String`. `Gen1<String>.put(T)` and
/// `Gen2.put(String)`, whose erasures differ, are ambiguous. A class's abstract method that
/// overrides a concrete one above it is chosen over interfaces' (`B1.same` in `C1`).
#[test]
fn merged_abstract_methods() {
    let rows = calls(
        "interface Zed { void same(); }
         interface Ab { void same(); }
         interface Mid { void same(); }
         interface P1 extends Zed, Ab { }
         interface P2 extends Ab, Zed { }
         interface P3 extends Zed, Mid, Ab { }
         interface Deep extends Zed { void same(); }
         interface P4 extends Ab, Deep { }
         abstract class AA { public abstract void same(); }
         abstract class Q3 extends AA implements Zed, Ab { }
         abstract class Q4 extends AA implements Ab { }
         abstract class BB { public void same() {} }
         abstract class Q8 extends BB implements Zed, Ab { }
         interface N1 { Object r(); }
         interface N2 { Object r(); }
         interface N3 { String r(); }
         interface N4 { String r(); }
         interface T extends N4, N3, N2, N1 { }
         class A1 { public void same() { } }
         abstract class B1 extends A1 { public abstract void same(); }
         abstract class C1 extends B1 implements Zed { }
         interface Gen1<G> { void put(G g); G got(); }
         interface Gen2 { void put(String s); String got(); }
         interface Gen3 extends Gen1<String>, Gen2 { }
         class Q { void c(P1 p1, P2 p2, P3 p3, P4 p4, Q3 q3, Q4 q4, Q8 q8, T t, Gen3 g3, C1 c1) {
           c1.same(); p1.same(); p2.same(); p3.same(); p4.same(); q3.same(); q4.same(); q8.same(); t.r();
           g3.put(\"a\"); g3.got();
         } }",
    );
    assert_eq!(method(&rows, "p1.same()"), "method Zed.same()V");
    assert_eq!(method(&rows, "p2.same()"), "method Zed.same()V");
    assert_eq!(method(&rows, "p3.same()"), "method Mid.same()V");
    assert_eq!(method(&rows, "p4.same()"), "method Ab.same()V");
    assert_eq!(method(&rows, "q3.same()"), "method Ab.same()V");
    assert_eq!(method(&rows, "q4.same()"), "method Ab.same()V");
    assert_eq!(method(&rows, "q8.same()"), "method BB.same()V");
    assert_eq!(method(&rows, "t.r()"), "method N3.r()Ljava/lang/String;");
    assert_eq!(method(&rows, "c1.same()"), "method B1.same()V");
    no_call(&rows, "g3.put(\"a\")");
    assert_eq!(
        method(&rows, "g3.got()"),
        "method Gen2.got()Ljava/lang/String;"
    );
}

/// The result: substituted, erased after an unchecked conversion, captured; the type as invoked
/// isn't captured itself. javac: `g(rawl)` is `(java/util/List<java/lang/String>)java/util/List`;
/// `w()` is `()java/util/List<?>` with the type `java/util/List<capture#1 extends
/// java/lang/Object>`; `xs.get(0)` for `G<? extends Number>` is `(int)capture#1 extends
/// java/lang/Number`; on a raw type everything is erased.
#[test]
fn the_result() {
    let source = "import java.util.*;
         class G<E> { E get(int i) { return null; } G<E> self() { return this; } void put(E e) {} }
         class KV<K, V> extends G<K> { }
         class Q {
           static List<String> g(List<String> l) { return l; }
           static List<?> w() { return null; }
           G<? extends Number> xs; G<?> q; G<? super Integer> cs; G raw; KV kv; G<String> gs;
           void f(List rawl) {
             g(rawl); w(); xs.get(0); q.get(0); cs.put(1); raw.get(0); raw.self(); kv.get(0); gs.get(0);
             gs.self().get(0).length();
           }
         }";
    let rows = calls(source);
    assert_eq!(
        invoked(&rows, "g(rawl)"),
        "(java/util/List<java/lang/String>)java/util/List"
    );
    assert_eq!(invoked(&rows, "w()"), "()java/util/List<?>");
    assert_eq!(
        invoked(&rows, "xs.get(0)"),
        "(int)capture#1 extends java/lang/Number"
    );
    assert_eq!(
        invoked(&rows, "q.get(0)"),
        "(int)capture#1 extends java/lang/Object"
    );
    assert_eq!(
        invoked(&rows, "cs.put(1)"),
        "(capture#1 extends java/lang/Object super java/lang/Integer)void"
    );
    assert_eq!(invoked(&rows, "raw.get(0)"), "(int)java/lang/Object");
    assert_eq!(invoked(&rows, "raw.self()"), "()G");
    assert_eq!(invoked(&rows, "kv.get(0)"), "(int)java/lang/Object");
    assert_eq!(
        method(&rows, "kv.get(0)"),
        "method G.get(I)Ljava/lang/Object;"
    );
    assert_eq!(invoked(&rows, "gs.get(0)"), "(int)java/lang/String");
    let e = expressions(source);
    assert_eq!(ty(&e, "g(rawl)"), "java/util/List");
    assert_eq!(
        ty(&e, "w()"),
        "java/util/List<capture#1 extends java/lang/Object>"
    );
    assert_eq!(ty(&e, "xs.get(0)"), "capture#1 extends java/lang/Number");
    assert_eq!(ty(&e, "kv.get(0)"), "java/lang/Object");
    assert_eq!(ty(&e, "cs.put(1)"), "void");
}

/// Overriding across byte arrays: a declared method overrides a default one of its signature.
/// jedis's `Jedis.sscan(byte[], byte[])` over `SetBinaryCommands`'s.
#[test]
fn declared_overrides_default() {
    let rows = calls(
        "interface S { default int m(byte[] a, byte[] b) { return 0; } default int m(String a, String b) { return 1; } }
         interface T extends S { }
         class J implements T, S { public int m(final byte[] a, final byte[] b) { return 2; } }
         class Q { void f(J j, byte[] x) { j.m(x, x); j.m(\"a\", \"b\"); } }",
    );
    assert_eq!(method(&rows, "j.m(x, x)"), "method J.m([B[B)I");
    assert_eq!(
        method(&rows, "j.m(\"a\", \"b\")"),
        "method S.m(Ljava/lang/String;Ljava/lang/String;)I"
    );
}

/// The inference line: a generic candidate without explicit type arguments, or an argument that
/// needs inference, is B3c's, and its receiver's other calls too; explicit type arguments
/// substitute. javac: `this.<String>id("a")` is `(java/lang/String)java/lang/String`, and a
/// non-generic method ignores them (`this.<String>plain()`, `()int`).
#[test]
fn the_inference_line() {
    let source = "class Q {
           <U> U id(U u) { return u; } <U> void gone(int a, U u) {} int plain() { return 0; }
           void over(int a) {} <U> void over(U u, U v) {}
           void over2(Runnable r) {} void one(Object o) {}
           <V extends Number> V bounded(V v) { return v; }
           void f() {
             this.<String>id(\"a\"); this.<String>plain(); id(\"a\"); id(\"a\").hashCode(); one(id(\"a\"));
             over(1); over2(() -> { }); one(Q::new); this.<Integer>bounded(1);
           }
         }";
    let rows = calls(source);
    let e = expressions(source);
    assert_eq!(
        invoked(&rows, "this.<String>id(\"a\")"),
        "(java/lang/String)java/lang/String"
    );
    assert_eq!(ty(&e, "this.<String>id(\"a\")"), "java/lang/String");
    assert_eq!(invoked(&rows, "this.<String>plain()"), "()int");
    assert_eq!(
        invoked(&rows, "this.<Integer>bounded(1)"),
        "(java/lang/Integer)java/lang/Integer"
    );
    no_call(&rows, "id(\"a\")");
    assert_eq!(ty(&e, "id(\"a\")"), "left-to-b3c inference");
    assert_eq!(ty(&e, "id(\"a\").hashCode()"), "left-to-b3c inference");
    assert_eq!(ty(&e, "one(id(\"a\"))"), "left-to-b3c inference");
    // The generic one has another arity, so it isn't potentially applicable.
    assert_eq!(method(&rows, "over(1)"), "method Q.over(I)V");
    assert_eq!(ty(&e, "over2(() -> { })"), "left-to-b3c inference");
    assert_eq!(ty(&e, "one(Q::new)"), "left-to-b3c inference");
}

/// A reference conditional as an argument: its own type is the target's (B3c's), but the call is
/// chosen by its operands. javac: `pr(b ? "a" : "b")` the `String` one, `pr(b ? 1 : "b")` the
/// `Object` one, `pr(b ? 'a' : 1)` the `char` one (a numeric conditional), and
/// `pr((b ? "a" : "b"))` as the first.
#[test]
fn conditional_arguments() {
    let source = "class Q {
           void pr(Object o) {} void pr(String s) {} void pr(char c) {} void pr(int i) {}
           void f(boolean b, Object o, String s) {
             pr(b ? \"a\" : \"b\"); pr(b ? 1 : \"b\"); pr(b ? 'a' : 1); pr((b ? \"a\" : \"b\"));
             pr(b ? o : s); pr(b ? null : \"b\");
           }
         }";
    let rows = calls(source);
    let string = "method Q.pr(Ljava/lang/String;)V";
    let object = "method Q.pr(Ljava/lang/Object;)V";
    assert_eq!(method(&rows, "pr(b ? \"a\" : \"b\")"), string);
    assert_eq!(method(&rows, "pr(b ? 1 : \"b\")"), object);
    assert_eq!(method(&rows, "pr(b ? 'a' : 1)"), "method Q.pr(C)V");
    assert_eq!(method(&rows, "pr((b ? \"a\" : \"b\"))"), string);
    assert_eq!(method(&rows, "pr(b ? o : s)"), object);
    assert_eq!(method(&rows, "pr(b ? null : \"b\")"), string);
    let e = expressions(source);
    assert_eq!(ty(&e, "b ? \"a\" : \"b\""), "left-to-b3c inference");
}

/// `getClass()` (JLS 4.3.2) and an array's `clone()` (JLS 10.7). javac: `getClass()`'s type as
/// invoked stays `()java/lang/Class<?>` (the stand-in has no signature), its type is `Class<capture#1 extends |T|>`: `Q` for a
/// generic class's `this`, `java/util/List` for `xs`, `java/lang/Object[]` for `T[]`; an array's
/// `clone()` is `Array.clone()Ljava/lang/Object;`, `()java/lang/Object`, and its type the array's.
#[test]
fn get_class_and_clone() {
    let source = "import java.util.*;
         class Q<T> {
           List<? extends Number> xs; int[] arr; String[][] sarr; T[] ta; T tv;
           void f() throws Exception {
             getClass(); this.getClass(); xs.getClass(); ta.getClass(); arr.getClass(); tv.getClass();
             arr.clone(); sarr.clone(); sarr[0].clone(); ta.clone(); new int[1].clone(); arr.equals(arr);
           }
         }";
    let e = expressions(source);
    let rows = calls(source);
    assert_eq!(ty(&e, "getClass()"), "java/lang/Class<capture#1 extends Q>");
    assert_eq!(
        ty(&e, "this.getClass()"),
        "java/lang/Class<capture#1 extends Q>"
    );
    assert_eq!(
        ty(&e, "xs.getClass()"),
        "java/lang/Class<capture#1 extends java/util/List>"
    );
    assert_eq!(
        ty(&e, "ta.getClass()"),
        "java/lang/Class<capture#1 extends java/lang/Object[]>"
    );
    assert_eq!(
        ty(&e, "arr.getClass()"),
        "java/lang/Class<capture#1 extends int[]>"
    );
    assert_eq!(
        ty(&e, "tv.getClass()"),
        "java/lang/Class<capture#1 extends java/lang/Object>"
    );
    assert_eq!(
        method(&rows, "arr.getClass()"),
        "method java/lang/Object.getClass()Ljava/lang/Class;"
    );
    assert_eq!(ty(&e, "arr.clone()"), "int[]");
    assert_eq!(ty(&e, "sarr.clone()"), "java/lang/String[][]");
    assert_eq!(ty(&e, "sarr[0].clone()"), "java/lang/String[]");
    assert_eq!(ty(&e, "ta.clone()"), "T:Q[]");
    assert_eq!(ty(&e, "new int[1].clone()"), "int[]");
    assert_eq!(
        method(&rows, "arr.clone()"),
        "method Array.clone()Ljava/lang/Object;"
    );
    assert_eq!(invoked(&rows, "arr.clone()"), "()java/lang/Object");
    assert_eq!(
        method(&rows, "arr.equals(arr)"),
        "method java/lang/Object.equals(Ljava/lang/Object;)Z"
    );
}

/// Forms of invocation: a simple name of an enclosing class's method, inherited ones as seen from
/// the class, `super`, `I.super`, `Outer.this`, a type's name, a static method through an
/// expression, an interface's `Object` methods, a type variable's and an intersection's bounds.
#[test]
fn forms_of_invocation() {
    let source = "class Base<X> { X got; X get() { return got; } static int st() { return 0; } }
         interface Dd { default int dm() { return 1; } static int sm() { return 2; } }
         interface Ru { void run(); }
         interface Co<C> { int compareTo(C c); }
         class DdI implements Dd { public int dm() { return Dd.super.dm(); } int sf() { return Dd.sm(); } }
         class Q<T extends Co<T> & Ru, U extends T> extends Base<String> {
           private int pp() { return 0; }
           class Ni { int pp(int a) { return Q.this.pp() + super.hashCode() + get().hashCode(); } }
           void f(T t, U u, Dd d, Ru r, Q<T, U> q) {
             get(); this.get(); super.get(); Base.st(); q.st();
             t.compareTo(t); t.run(); u.compareTo(t); u.run(); t.equals(t);
             d.equals(d); d.dm();
             ((Ru & Co<String>) r).compareTo(\"a\");
           }
         }";
    let rows = calls(source);
    let e = expressions(source);
    assert_eq!(invoked(&rows, "get()"), "()java/lang/String");
    assert_eq!(
        method(&rows, "get()"),
        "method Base.get()Ljava/lang/Object;"
    );
    assert_eq!(invoked(&rows, "this.get()"), "()java/lang/String");
    assert_eq!(invoked(&rows, "super.get()"), "()java/lang/String");
    assert_eq!(method(&rows, "Dd.super.dm()"), "method Dd.dm()I");
    assert_eq!(method(&rows, "Dd.sm()"), "method Dd.sm()I");
    assert_eq!(method(&rows, "Base.st()"), "method Base.st()I");
    assert_eq!(method(&rows, "q.st()"), "method Base.st()I");
    assert_eq!(method(&rows, "Q.this.pp()"), "method Q.pp()I");
    assert_eq!(
        method(&rows, "t.compareTo(t)"),
        "method Co.compareTo(Ljava/lang/Object;)I"
    );
    assert_eq!(invoked(&rows, "t.compareTo(t)"), "(T:Q)int");
    assert_eq!(method(&rows, "t.run()"), "method Ru.run()V");
    assert_eq!(invoked(&rows, "u.compareTo(t)"), "(T:Q)int");
    assert_eq!(method(&rows, "u.run()"), "method Ru.run()V");
    assert_eq!(
        method(&rows, "t.equals(t)"),
        "method java/lang/Object.equals(Ljava/lang/Object;)Z"
    );
    assert_eq!(
        method(&rows, "d.equals(d)"),
        "method java/lang/Object.equals(Ljava/lang/Object;)Z"
    );
    assert_eq!(
        invoked(&rows, "((Ru & Co<String>) r).compareTo(\"a\")"),
        "(java/lang/String)int"
    );
    assert_eq!(ty(&e, "get().hashCode()"), "int");
}

/// Accessibility (JLS 6.6): a private method isn't a candidate outside its top level class, a
/// package-private one outside its package, a protected one only through the subclass's own type.
/// javac: `h.hpick(1)` from another class chooses the `long` one, not the private `int` one;
/// `b.pick(1)` in `p.Sub` the public `long` one over the protected `int` one, `s.pick(1)` and
/// `pick(1)` the protected one.
#[test]
fn accessibility() {
    let engine = engine(
        "//- file main p/Base.java
package p;
public class Base { protected int pick(int i) { return 0; } public int pick(long l) { return 1; } void only() {} public void only(Object o) {} protected Base(int i) { } public Base(long l) { } }
//- file main q/Sub.java
package q;
import p.Base;
class Sub extends Base {
  Sub() { super(1); }
  void f(Base b, Sub s) { b.pick(1); s.pick(1); pick(1); this.pick(1); super.pick(1); s.only(1); new Base(1); new Base(1) { }; }
}
class H { private void hpick(int i) {} void hpick(long l) {} void g() { hpick(1); } }
class O { void g(H h) { h.hpick(1); } }
",
    );
    let snapshot = engine.snapshot();
    let set = SourceSetId::from_index(0);
    let file = *snapshot
        .files(set)
        .unwrap()
        .iter()
        .find(|&&f| snapshot.file(f).unwrap().path.ends_with("q/Sub.java"))
        .unwrap();
    let types = snapshot.body_types(file).unwrap();
    let tree = snapshot.tree(file).unwrap();
    let rows: Vec<CallRow> = types
        .calls
        .iter()
        .map(|(node, call)| {
            let text = snapshot.call_text(file, &types, *node, call).unwrap();
            CallRow {
                text: tree.node(*node).text().to_owned(),
                method: text.method,
                invoked: text.invoked,
                anonymous: text.anonymous,
            }
        })
        .collect();
    let int = "method p/Base.pick(I)I";
    let long = "method p/Base.pick(J)I";
    assert_eq!(method(&rows, "b.pick(1)"), long);
    assert_eq!(method(&rows, "s.pick(1)"), int);
    assert_eq!(method(&rows, "pick(1)"), int);
    assert_eq!(method(&rows, "hpick(1)"), "method q/H.hpick(I)V");
    assert_eq!(method(&rows, "this.pick(1)"), int);
    assert_eq!(method(&rows, "super.pick(1)"), int);
    assert_eq!(method(&rows, "h.hpick(1)"), "method q/H.hpick(J)V");
    assert_eq!(method(&rows, "super(1)"), "method p/Base.<init>(I)V");
    assert_eq!(method(&rows, "new Base(1)"), "method p/Base.<init>(J)V");
    assert_eq!(method(&rows, "new Base(1) { }"), "method p/Base.<init>(I)V");
}

/// Constructors. javac: `new G<String>("a")` chooses `G.<init>(Ljava/lang/Object;)V`, and its type
/// is as written; an anonymous class's own constructor has the parameters of the superclass's as
/// invoked (`(Ljava/lang/String;)V`), after an outer instance (`(LBase;)V`), an interface's none.
/// An implicit `super()` chooses by the superclass as declared, a generic constructor with
/// explicit arguments substitutes, and a default constructor has its class's access.
#[test]
fn creations() {
    let source = "interface Ru { void run(); }
         class G<X> { G() {} G(X x) {} protected G(int i, X x) {}
           class In<Y> { In() {} In(Y y) {} }
         }
         class H<X> { H() {} <V> H(V v, long l) {} }
         class Sub extends G<String> {
           Sub() { }
           Sub(String s) { super(s); }
           Sub(int i) { this(); }
           Sub(int i, String s) { super(i, s); }
         }
         class Q {
           void f(G<String> g) {
             new G<String>(\"a\"); new G(\"a\"); new Sub(1); new Object(); new G<>(\"a\");
             new G<String>(\"x\") { }; new G<String>(1, \"x\") { }; new Ru() { public void run() { } };
             g.new In<Integer>(1); g.new In<String>(\"a\") { }; new <String>H<Integer>(\"a\", 1L); new H<Integer>(\"a\", 1L);
           }
         }";
    let rows = calls(source);
    let e = expressions(source);
    assert_eq!(
        method(&rows, "new G<String>(\"a\")"),
        "method G.<init>(Ljava/lang/Object;)V"
    );
    assert_eq!(ty(&e, "new G<String>(\"a\")"), "G<java/lang/String>");
    assert_eq!(ty(&e, "new G(\"a\")"), "G");
    assert_eq!(ty(&e, "new G<>(\"a\")"), "left-to-b3c inference");
    no_call(&rows, "new G<>(\"a\")");
    assert_eq!(ty(&e, "new Sub(1)"), "Sub");
    assert_eq!(method(&rows, "new Sub(1)"), "method Sub.<init>(I)V");
    assert_eq!(
        method(&rows, "super(s)"),
        "method G.<init>(Ljava/lang/Object;)V"
    );
    assert_eq!(invoked(&rows, "super(s)"), "(java/lang/String)void");
    assert_eq!(method(&rows, "this()"), "method Sub.<init>()V");
    assert_eq!(invoked(&rows, "super(i, s)"), "(int,java/lang/String)void");
    // The body of `Sub() { }`: its implicit `super()`.
    assert_eq!(method(&rows, "{ }"), "method G.<init>()V");
    assert_eq!(invoked(&rows, "{ }"), "()void");
    let anonymous = |text: &str| {
        rows.iter()
            .find(|r| r.text == text)
            .and_then(|r| r.anonymous.clone())
            .map(|a| a.rsplit_once('.').unwrap().1.to_owned())
    };
    assert_eq!(
        anonymous("new G<String>(\"x\") { }").as_deref(),
        Some("<init>(Ljava/lang/String;)V")
    );
    assert_eq!(
        anonymous("new G<String>(1, \"x\") { }").as_deref(),
        Some("<init>(ILjava/lang/String;)V")
    );
    assert_eq!(
        anonymous("g.new In<String>(\"a\") { }").as_deref(),
        Some("<init>(LG;Ljava/lang/String;)V")
    );
    assert_eq!(
        method(&rows, "g.new In<Integer>(1)"),
        "method G$In.<init>(Ljava/lang/Object;)V"
    );
    assert_eq!(
        ty(&e, "g.new In<Integer>(1)"),
        "G<java/lang/String>$In<java/lang/Integer>"
    );
    assert_eq!(
        method(&rows, "new <String>H<Integer>(\"a\", 1L)"),
        "method H.<init>(Ljava/lang/Object;J)V"
    );
    // A generic constructor without explicit arguments is B3c's, but the type is the class's.
    no_call(&rows, "new H<Integer>(\"a\", 1L)");
    assert_eq!(ty(&e, "new H<Integer>(\"a\", 1L)"), "H<java/lang/Integer>");
}

/// Calls javac makes up. javac: `class D { }`'s default constructor calls `E.<init>()V`, at `class`;
/// a record's canonical one `java/lang/Record.<init>()V`; an enum's `java/lang/Enum.<init>
/// (Ljava/lang/String;I)V` with `(java/lang/String,int)void`. An enum constant chooses among its
/// enum's constructors; one with a body has the anonymous class's too.
#[test]
fn made_up_calls() {
    let rows = calls(
        "class E { E() { } }
         class D extends E { }
         record R(int a) { }
         record R2(int a) { R2 { } }
         enum Color {
           RED, BLUE(1), PINK(1, 2) { void g() { } }, TAN(\"a\");
           Color() { } Color(int i) { } Color(int... is) { } Color(Object o) { }
           void g() { }
         }
         enum Plain { A }
         class Q { }",
    );
    assert_eq!(method(&rows, "class D extends E { }"), "method E.<init>()V");
    assert_eq!(
        method(&rows, "record R(int a) { }"),
        "method java/lang/Record.<init>()V"
    );
    assert_eq!(method(&rows, "RED"), "method Color.<init>()V");
    assert_eq!(method(&rows, "BLUE(1)"), "method Color.<init>(I)V");
    assert_eq!(
        method(&rows, "TAN(\"a\")"),
        "method Color.<init>(Ljava/lang/Object;)V"
    );
    let pink = rows
        .iter()
        .find(|r| r.text.starts_with("PINK"))
        .expect("PINK's call");
    assert_eq!(pink.method, "method Color.<init>([I)V");
    assert_eq!(pink.invoked, "(int[])void");
    assert!(
        pink.anonymous
            .as_deref()
            .is_some_and(|a| a.ends_with(".<init>([I)V")),
        "{pink:?}"
    );
}

/// A `case` name follows a selector that is a call (B2's `case-label`): javac has `OPEN` as `K`'s
/// constant, whatever `String OPEN` is in scope. A selector left to inference waits with it.
#[test]
fn case_labels_of_calls() {
    let e: Vec<Expr> = expressions(
        "enum K { OPEN, SHUT }
         class Q {
           static final String OPEN = \"o\";
           K kind() { return K.OPEN; }
           String text() { return OPEN; }
           <T> T any(T t) { return t; }
           void f() {
             switch (kind()) { case OPEN: break; default: break; }
             switch (text()) { case OPEN: break; default: break; }
             switch (any(K.OPEN)) { case SHUT: break; default: break; }
           }
         }",
    );
    let open: Vec<&Expr> = e.iter().filter(|x| x.text == "OPEN").collect();
    assert!(open.iter().any(|x| x.ty == "K"), "{open:?}");
    assert!(open.iter().any(|x| x.ty == "java/lang/String"), "{open:?}");
    assert_eq!(ty(&e, "SHUT"), "left-to-b3c inference");
}

/// Statically imported methods: every single-static import of the name is a candidate, whichever
/// type declares it, and the chosen one says which (B2's `static-import-overload`).
#[test]
fn static_imports() {
    let engine = engine(
        "//- file main p/A.java
package p;
public class A { public static int over(int i) { return 0; } public void over(long l) { } }
//- file main p/B.java
package p;
public class B extends C { public static int over(String s) { return 0; } public static int hid() { return 1; } }
//- file main p/C.java
package p;
public class C { public static int over(Object o) { return 2; } public static int hid() { return 3; } }
//- file main q/Q.java
package q;
import static p.A.over;
import static p.B.*;
class Q { void f() { over(1); over(1L); over(\"a\"); hid(); } }
//- file main q/R.java
package q;
import static p.B.over;
import static p.A.over;
import static p.B.hid;
class R { void f() { over(1); over(\"a\"); over(new Object()); hid(); } }
",
    );
    let snapshot = engine.snapshot();
    let set = SourceSetId::from_index(0);
    let rows = |name: &str| -> Vec<CallRow> {
        let file = *snapshot
            .files(set)
            .unwrap()
            .iter()
            .find(|&&f| snapshot.file(f).unwrap().path.ends_with(name))
            .unwrap();
        let types = snapshot.body_types(file).unwrap();
        let tree = snapshot.tree(file).unwrap();
        types
            .calls
            .iter()
            .map(|(node, call)| {
                let text = snapshot.call_text(file, &types, *node, call).unwrap();
                CallRow {
                    text: tree.node(*node).text().to_owned(),
                    method: text.method,
                    invoked: text.invoked,
                    anonymous: text.anonymous,
                }
            })
            .collect()
    };
    let q = rows("q/Q.java");
    assert_eq!(method(&q, "over(1)"), "method p/A.over(I)I");
    no_call(&q, "over(\"a\")");
    assert_eq!(method(&q, "hid()"), "method p/B.hid()I");
    let r = rows("q/R.java");
    assert_eq!(method(&r, "over(1)"), "method p/A.over(I)I");
    assert_eq!(
        method(&r, "over(\"a\")"),
        "method p/B.over(Ljava/lang/String;)I"
    );
    assert_eq!(
        method(&r, "over(new Object())"),
        "method p/C.over(Ljava/lang/Object;)I"
    );
    assert_eq!(method(&r, "hid()"), "method p/B.hid()I");
}

/// javac records a call that is an argument, with a capture variable for its type, as the
/// variable's upper bound; through parentheses, and as a conditional's operand, where a field or
/// array element is too. Anywhere else, and inside a type, the capture stays.
#[test]
fn capture_variables_as_arguments() {
    let e = expressions(
        "class Box<B> { B f; B[] arr; B get() { return f; } Box<B> self() { return this; } }
         class Q {
           Box<?> bq; Box<? extends Number> bn; Box<? extends Box<?>> ee;
           Q(Object o) { }
           static void g(Object o) {} static void n(Number o) {}
           void f(boolean b) {
             n(bn.get());
             n((bn.self().get()));
             g(bn.self());
             g(bq.f);
             g(bq.arr[0]);
             new Q(bq.self().self().get());
             g(b ? bn.f : null);
             g(b ? bn.arr[1] : null);
             g(ee.get());
             bq.get().hashCode();
             Object a = bq.self().get();
           }
         }",
    );
    assert_eq!(ty(&e, "bn.get()"), "java/lang/Number");
    assert_eq!(ty(&e, "(bn.self().get())"), "java/lang/Number");
    assert_eq!(ty(&e, "bn.self().get()"), "java/lang/Number");
    assert_eq!(
        ty(&e, "bn.self()"),
        "Box<capture#1 extends java/lang/Number>"
    );
    assert_eq!(ty(&e, "bq.f"), "capture#1 extends java/lang/Object");
    assert_eq!(ty(&e, "bq.arr[0]"), "capture#1 extends java/lang/Object");
    assert_eq!(ty(&e, "bq.self().self().get()"), "java/lang/Object");
    assert_eq!(ty(&e, "bn.f"), "java/lang/Number");
    assert_eq!(ty(&e, "bn.arr[1]"), "java/lang/Number");
    assert_eq!(
        ty(&e, "ee.get()"),
        "Box<capture#1 extends java/lang/Object>"
    );
    assert_eq!(ty(&e, "bq.get()"), "capture#1 extends java/lang/Object");
    assert_eq!(
        ty(&e, "bq.self().get()"),
        "capture#1 extends java/lang/Object"
    );
}

/// An anonymous class made through an outer instance: what it inherits is seen through the
/// instance's type. javac: `new From<Integer>().new To<String>() { }.a()` is `(...)
/// java/lang/Integer`, `new From<Integer>() { }.new To<String>()` is `From<java/lang/Integer>$To<
/// java/lang/String>`. Inside a class made with `<>`, inherited methods wait for inference.
#[test]
fn anonymous_through_an_outer_instance() {
    let source = "class From<F> { class To<T> { F a() { return null; } T b() { return null; } } }
         class Sup<S> { S got() { return null; } }
         class Q { void f() {
           new From<Integer>().new To<String>() { }.a();
           new From<Integer>().new To<String>() { }.b();
           new From<Integer>() { }.new To<String>().a();
           new From<Integer>() { }.new To<String>();
           new Sup<>() { Object own() { return got(); } };
         } }";
    let e = expressions(source);
    assert_eq!(
        ty(&e, "new From<Integer>().new To<String>() { }.a()"),
        "java/lang/Integer"
    );
    assert_eq!(
        ty(&e, "new From<Integer>().new To<String>() { }.b()"),
        "java/lang/String"
    );
    assert_eq!(
        ty(&e, "new From<Integer>() { }.new To<String>()"),
        "From<java/lang/Integer>$To<java/lang/String>"
    );
    assert_eq!(
        ty(&e, "new From<Integer>() { }.new To<String>().a()"),
        "java/lang/Integer"
    );
    assert_eq!(ty(&e, "got()"), "left-to-b3c inference");
}

/// The members of a class and of an interface, as javac chooses them. In a class, `Object`'s
/// concrete `hashCode()` stays over an interface's abstract one (`Ac`); in an interface, one
/// interface's is chosen over `Object`'s (`Ca`, whatever else it extends), and of several
/// interfaces', `Object`'s (`Tw`).
#[test]
fn object_methods_and_interfaces() {
    let rows = calls(
        "interface Co { int hashCode(); String toString(); }
         interface Co2 { int hashCode(); }
         interface At { }
         interface Ca extends At, Co { }
         interface Cb extends Ca { }
         interface Tw extends Co, Co2 { }
         abstract class Ab implements Co { }
         abstract class Ac extends Ab implements Co { }
         class Q { void f(Ca ca, Cb cb, Tw tw, Ac ac) {
           ca.toString(); ca.hashCode(); cb.hashCode(); tw.hashCode(); tw.toString(); ac.hashCode();
         } }",
    );
    let object = "method java/lang/Object.hashCode()I";
    assert_eq!(method(&rows, "ca.hashCode()"), "method Co.hashCode()I");
    assert_eq!(method(&rows, "cb.hashCode()"), "method Co.hashCode()I");
    assert_eq!(method(&rows, "tw.hashCode()"), object);
    assert_eq!(
        method(&rows, "tw.toString()"),
        "method Co.toString()Ljava/lang/String;"
    );
    assert_eq!(method(&rows, "ac.hashCode()"), object);
}
