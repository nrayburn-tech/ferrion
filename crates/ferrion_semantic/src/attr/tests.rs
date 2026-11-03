//! Tests of attribution: each input was run through javac (Temurin 27.0.1, the types oracle) on
//! the same source with the real JDK, and javac's type, constant, or answer is written beside it.
//! The classes the tests name are [`RELATIONS_JDK`](crate::testing::RELATIONS_JDK)'s stand-ins
//! for the JDK's, which have the JDK's names, type parameters, and supertypes.

use crate::testing::relations_input;
use crate::types::Printed;
use crate::{Engine, FileId, SourceSetId};

/// One attributed expression: its source text (white space collapsed), type, constant, and
/// conversions.
#[derive(Debug)]
struct Expr {
    text: String,
    ty: String,
    constant: Option<String>,
    conversion: Option<String>,
}

fn engine(files: &str) -> Engine {
    Engine::new(relations_input(files))
}

/// The expressions of the file named `path` in `files` (a case's `//- file` lines and texts).
fn expressions_in(files: &str, path: &str) -> Vec<Expr> {
    let engine = engine(files);
    let snapshot = engine.snapshot();
    let set = SourceSetId::from_index(0);
    let file = *snapshot
        .files(set)
        .unwrap()
        .iter()
        .find(|&&f| snapshot.file(f).unwrap().path.ends_with(path))
        .unwrap_or_else(|| panic!("no file {path}"));
    expressions_of(&snapshot, file)
}

fn expressions_of(snapshot: &crate::Snapshot, file: FileId) -> Vec<Expr> {
    let types = snapshot.body_types(file).unwrap();
    let tree = snapshot.tree(file).unwrap();
    let mut cx = snapshot.types_in(SourceSetId::from_index(0), types.arena.clone());
    types
        .types
        .iter()
        .map(|(node, ty)| {
            let syntax = tree.node(*node);
            Expr {
                text: syntax
                    .text()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                ty: cx.type_string(*ty, Printed::Captures).unwrap(),
                constant: types.constant(*node).map(|c| c.oracle_text()),
                conversion: types
                    .conversion(*node)
                    .map(|s| format!("{:?}", s.as_slice())),
            }
        })
        .collect()
}

/// The expressions of `source`, one file `Q.java` in the default package.
fn expressions(source: &str) -> Vec<Expr> {
    expressions_in(&format!("//- file main Q.java\n{source}"), "Q.java")
}

/// The `nth` expression whose text is `text`.
fn get<'e>(exprs: &'e [Expr], text: &str, nth: usize) -> &'e Expr {
    exprs
        .iter()
        .filter(|e| e.text == text)
        .nth(nth)
        .unwrap_or_else(|| panic!("no expression `{text}` #{nth} in {exprs:#?}"))
}

fn ty<'e>(exprs: &'e [Expr], text: &str) -> &'e str {
    &get(exprs, text, 0).ty
}

fn constant<'e>(exprs: &'e [Expr], text: &str) -> Option<&'e str> {
    get(exprs, text, 0).constant.as_deref()
}

/// javac: `PLUS int`, `PLUS long`, `UNARY_MINUS int` for `-c`, `PLUS int` for `boxed + 1` with
/// `boxed` `java/lang/Integer`, `LESS_THAN boolean`, `"a" + i + c` and `"a" + i`
/// `java/lang/String`, `PLUS_ASSIGNMENT int` for `i += 2.5`, `POSTFIX_INCREMENT char` for `c++`,
/// `LEFT_SHIFT int` for `1 << l` and `long` for `l << i`, the literal of `byte by = 10` `int`,
/// `TYPE_CAST short`, `AND long` for `i & l`, `BITWISE_COMPLEMENT long`, `POSTFIX_INCREMENT
/// java/lang/Integer` for `boxed++`, `PLUS_ASSIGNMENT java/lang/String` for `s += 1`, `PLUS long`
/// for `'a' + 1L`.
#[test]
fn operators() {
    let e = expressions(
        "class Q { void m(int i, long l, char c, Integer boxed, boolean b, boolean b2, String s) {\n\
         int sum = i + 1; long w = i + l; int ch = c + 1; int u = -c; int bx = boxed + 1;\n\
         boolean lt = i < l; String cat = \"a\" + i + c; i += 2.5; c++; int sh = 1 << l;\n\
         long sh2 = l << i; byte by = 10; short sb = (short) i; long il = i & l; long t = ~l;\n\
         boolean and = b & b2; boolean nb = !b; boxed++; s += 1; long lc = 'a' + 1L; int p = +i;\n\
         int us = i >>> 2; long sr = l >> i; i >>>= 1; } }",
    );
    assert_eq!(ty(&e, "i + 1"), "int");
    assert_eq!(ty(&e, "i + l"), "long");
    assert_eq!(
        get(&e, "i", 1).conversion.as_deref(),
        Some("[WideningPrimitive]")
    );
    assert_eq!(ty(&e, "c + 1"), "int");
    assert_eq!(ty(&e, "-c"), "int");
    assert_eq!(ty(&e, "boxed + 1"), "int");
    assert_eq!(ty(&e, "boxed"), "java/lang/Integer");
    assert_eq!(
        get(&e, "boxed", 0).conversion.as_deref(),
        Some("[Unboxing]")
    );
    assert_eq!(ty(&e, "i < l"), "boolean");
    assert_eq!(ty(&e, "\"a\" + i + c"), "java/lang/String");
    assert_eq!(ty(&e, "\"a\" + i"), "java/lang/String");
    assert_eq!(ty(&e, "i += 2.5"), "int");
    assert_eq!(ty(&e, "c++"), "char");
    assert_eq!(ty(&e, "1 << l"), "int");
    assert_eq!(ty(&e, "l << i"), "long");
    assert_eq!(ty(&e, "10"), "int");
    assert_eq!(
        get(&e, "10", 0).conversion.as_deref(),
        Some("[NarrowingPrimitive]")
    );
    assert_eq!(ty(&e, "(short) i"), "short");
    assert_eq!(ty(&e, "i & l"), "long");
    assert_eq!(ty(&e, "~l"), "long");
    assert_eq!(ty(&e, "b & b2"), "boolean");
    assert_eq!(ty(&e, "!b"), "boolean");
    assert_eq!(ty(&e, "boxed++"), "java/lang/Integer");
    assert_eq!(ty(&e, "s += 1"), "java/lang/String");
    assert_eq!(ty(&e, "'a' + 1L"), "long");
    assert_eq!(constant(&e, "'a' + 1L"), Some("long 98"));
    assert_eq!(ty(&e, "+i"), "int");
    assert_eq!(ty(&e, "i >>> 2"), "int");
    assert_eq!(ty(&e, "l >> i"), "long");
    assert_eq!(ty(&e, "i >>>= 1"), "int");
}

/// javac: identifiers of a wildcard-parameterized variable are captured
/// (`java/util/List<capture#1 extends java/lang/Number>` for `xs`, `capture#1 extends
/// java/lang/Object` for `List<?>`), but not as an assignment's target (`xs = null`, whose
/// `ASSIGNMENT` is captured) or a qualifier; a field access is captured (`this.xs`); an array
/// access isn't (`la[0]` is `java/util/List<?>`); a cast is (`(List<?>) o`); a parenthesized
/// expression is its operand's.
#[test]
fn capture_of_variables() {
    let e = expressions(
        "import java.util.*;\n\
         class Q { List<? extends Number> xs; List<?> any; List<?>[] la;\n\
         void m(Object o) { Object a = xs; Object b = any; xs = null; Object c = this.xs;\n\
         Object d = la[0]; Object f = (List<?>) o; Object g = (xs); boolean h = xs == null; } }",
    );
    assert_eq!(
        ty(&e, "xs"),
        "java/util/List<capture#1 extends java/lang/Number>"
    );
    assert_eq!(
        ty(&e, "any"),
        "java/util/List<capture#1 extends java/lang/Object>"
    );
    assert_eq!(
        get(&e, "xs", 1).ty,
        "java/util/List<? extends java/lang/Number>",
        "an assignment's target"
    );
    assert_eq!(
        ty(&e, "xs = null"),
        "java/util/List<capture#1 extends java/lang/Number>"
    );
    assert_eq!(
        ty(&e, "this.xs"),
        "java/util/List<capture#1 extends java/lang/Number>"
    );
    assert_eq!(ty(&e, "this"), "Q");
    assert_eq!(ty(&e, "la[0]"), "java/util/List<?>");
    assert_eq!(ty(&e, "la"), "java/util/List<?>[]");
    assert_eq!(
        ty(&e, "(List<?>) o"),
        "java/util/List<capture#1 extends java/lang/Object>"
    );
    assert_eq!(
        ty(&e, "(xs)"),
        "java/util/List<capture#1 extends java/lang/Number>"
    );
    assert_eq!(ty(&e, "xs == null"), "boolean");
    assert_eq!(ty(&e, "null"), "null");
}

/// javac: `t`, an inherited field of `Base<String>`, is `java/lang/String`, as are `super.t`
/// and `this.t`; `super` is `Base<java/lang/String>`, `this` and `A.this` `A<E:A>`; a type
/// variable's field `e` is `E:A`; `arr.length` is `int`; `Base.S` and `S` `int`.
#[test]
fn names_fields_this_super() {
    let e = expressions(
        "class Base<T> { T t; static int S = 1; protected int p; }\n\
         class Q<E> extends Base<String> { int[] arr; E e;\n\
         void m() { String s = t; String s2 = super.t; String s3 = this.t; int q = super.p;\n\
         Object self = this; Object outer = Q.this; int n = arr.length; E ee = e; int a = S;\n\
         int b = Base.S; int c = this.arr.length; } }",
    );
    assert_eq!(ty(&e, "t"), "java/lang/String");
    assert_eq!(ty(&e, "super.t"), "java/lang/String");
    assert_eq!(ty(&e, "super"), "Base<java/lang/String>");
    assert_eq!(ty(&e, "this.t"), "java/lang/String");
    assert_eq!(ty(&e, "super.p"), "int");
    assert_eq!(ty(&e, "this"), "Q<E:Q>");
    assert_eq!(ty(&e, "Q.this"), "Q<E:Q>");
    assert_eq!(ty(&e, "arr.length"), "int");
    assert_eq!(ty(&e, "arr"), "int[]");
    assert_eq!(ty(&e, "e"), "E:Q");
    assert_eq!(ty(&e, "S"), "int");
    assert_eq!(ty(&e, "Base.S"), "int");
    assert_eq!(ty(&e, "this.arr.length"), "int");
    assert_eq!(ty(&e, "this.arr"), "int[]");
    // `Base` is a type name, not an expression.
    assert!(e.iter().all(|x| x.text != "Base" && x.text != "Q"));
}

/// javac: `String.class` is `java/lang/Class<java/lang/String>`, `int.class`
/// `java/lang/Class<java/lang/Integer>`, `void.class` `java/lang/Class<java/lang/Void>`,
/// `int[].class` `java/lang/Class<int[]>`; a raw `List.class` `java/lang/Class<java/util/List>`.
#[test]
fn class_literals() {
    let e = expressions(
        "import java.util.*;\n\
         class Q { void m() { Object a = String.class; Object b = int.class; Object c = void.class;\n\
         Object d = int[].class; Object f = List.class; } }",
    );
    assert_eq!(ty(&e, "String.class"), "java/lang/Class<java/lang/String>");
    assert_eq!(ty(&e, "int.class"), "java/lang/Class<java/lang/Integer>");
    assert_eq!(ty(&e, "void.class"), "java/lang/Class<java/lang/Void>");
    assert_eq!(ty(&e, "int[].class"), "java/lang/Class<int[]>");
    assert_eq!(ty(&e, "List.class"), "java/lang/Class<java/util/List>");
}

/// javac: `new int[3]` `int[]`, `new int[2][]` `int[][]`, `{1, 2}` `int[]`, `new String[] {"a"}`
/// `java/lang/String[]`, `arr[0]` `int`; an enhanced `for`'s `var` over `List<? extends
/// Number>` is `java/lang/Number` (its expression captured), over `int[]` `int`.
#[test]
fn arrays_and_for_each() {
    let e = expressions(
        "import java.util.*;\n\
         class Q { int[] arr; List<? extends Number> xs;\n\
         void m(List<String> ls) { int[] na = new int[3]; int[][] nb = new int[2][]; int[] ni = {1, 2};\n\
         String[] sa = new String[] {\"a\"}; int el = arr[0];\n\
         for (var n : xs) { Number k = n; } for (var a : arr) { int z = a; }\n\
         for (String it : ls) { String y = it; } } }",
    );
    assert_eq!(ty(&e, "new int[3]"), "int[]");
    assert_eq!(ty(&e, "new int[2][]"), "int[][]");
    assert_eq!(ty(&e, "{1, 2}"), "int[]");
    assert_eq!(ty(&e, "new String[] {\"a\"}"), "java/lang/String[]");
    assert_eq!(ty(&e, "arr[0]"), "int");
    assert_eq!(ty(&e, "n"), "java/lang/Number");
    assert_eq!(ty(&e, "a"), "int");
    assert_eq!(ty(&e, "it"), "java/lang/String");
    assert_eq!(
        get(&e, "xs", 0).ty,
        "java/util/List<capture#1 extends java/lang/Number>"
    );
}

/// javac: `i > 0 ? 1 : 2` `int`, `i > 0 ? 1 : 2L` `long`, `(i > 0 ? 1 : 2.0)` `double`,
/// `true ? 'a' : 0` `char` (a constant `int` that fits), `b ? boxed : 1` `int`,
/// `b ? (short) 1 : (byte) 1` `short`; a reference conditional in an assignment context is a
/// poly expression (`Object o = i > 0 ? "a" : 1` is typed `java/lang/Object`, the target: B3c's),
/// and with `var` its `lub` (observation 1).
#[test]
fn conditionals() {
    let e = expressions(
        "class Q { void m(int i, boolean b, Integer boxed) {\n\
         int c1 = i > 0 ? 1 : 2; long c2 = i > 0 ? 1 : 2L; Number c3 = (i > 0 ? 1 : 2.0);\n\
         char c4 = true ? 'a' : 0; int c5 = b ? boxed : 1; short c6 = b ? (short) 1 : (byte) 1;\n\
         Object c7 = i > 0 ? \"a\" : 1; var c8 = b ? 1 : \"a\"; } }",
    );
    assert_eq!(ty(&e, "i > 0 ? 1 : 2"), "int");
    assert_eq!(ty(&e, "i > 0 ? 1 : 2L"), "long");
    assert_eq!(ty(&e, "(i > 0 ? 1 : 2.0)"), "double");
    assert_eq!(ty(&e, "true ? 'a' : 0"), "char");
    assert_eq!(constant(&e, "true ? 'a' : 0"), Some("char \"a\""));
    assert_eq!(ty(&e, "b ? boxed : 1"), "int");
    assert_eq!(ty(&e, "b ? (short) 1 : (byte) 1"), "short");
    assert_eq!(ty(&e, "i > 0 ? \"a\" : 1"), "left-to-b3c inference");
    assert_eq!(
        ty(&e, "b ? 1 : \"a\""),
        "java/io/Serializable&java/lang/Comparable<? extends java/io/Serializable&java/lang/Comparable<?>&java/lang/constant/Constable&java/lang/constant/ConstantDesc>&java/lang/constant/Constable&java/lang/constant/ConstantDesc"
    );
}

/// javac's constants (observations 11 and 13 in docs/design/types-oracle-normalization.md and
/// this design's): `"s" + K` `String "s3"`, `Long.MAX_VALUE + 1` `long -9223372036854775808`,
/// `"" + D` with `D = 1.0E23` `String "1.0E23"`, `(char) ('a' + 1)` `char "b"`, `(byte) 300`
/// `byte 44`, `"a" == "a"` `boolean true`, `1 / 0` none, `1.0 / 0` `double Infinity`,
/// `"x" + null` none, `1 << 33` `int 2`, `-0.0` `double -0x0.0p0`, `0xFFFF_FFFF` `int -1`,
/// `0x1.8p3` `double 0x1.8p3`, a text block, `K > 2 ? K : -K` `int 3`, a `final` local; and not
/// `self.K` or `this.K`.
#[test]
fn constants() {
    let e = expressions(
        "class Q { static final int K = 3; static final double D = 1.0E23; Q self;\n\
         void m() { String s = \"s\" + K; long big = Long.MAX_VALUE + 1; String ds = \"\" + D;\n\
         String ds2 = \"\" + 2.0e-3 + 'c' + true + 1.5f; char ch = (char) ('a' + 1);\n\
         byte by = (byte) 300; boolean eq = \"a\" == \"a\"; int not = 1 / 0; double inf = 1.0 / 0;\n\
         String nul = \"x\" + null; int shift = 1 << 33; double neg = -0.0; int hex = 0xFFFF_FFFF;\n\
         double hf = 0x1.8p3; float f3 = 1.0f / 3; int min = -2147483648;\n\
         int tern = K > 2 ? K : -K; final int k = 3; int z = k + 1; int ak = self.K; int tk = this.K;\n\
         String tb = \"\"\"\n      hello\n        world\\t!\n      \"\"\"; String esc = \"q\\\"\\\\\\u00e9\\uD83D\\uDE00\"; } }",
    );
    assert_eq!(constant(&e, "\"s\" + K"), Some("String \"s3\""));
    assert_eq!(constant(&e, "K"), Some("int 3"));
    assert_eq!(
        constant(&e, "Long.MAX_VALUE + 1"),
        Some("long -9223372036854775808")
    );
    assert_eq!(
        constant(&e, "Long.MAX_VALUE"),
        Some("long 9223372036854775807")
    );
    assert_eq!(constant(&e, "\"\" + D"), Some("String \"1.0E23\""));
    assert_eq!(
        constant(&e, "\"\" + 2.0e-3 + 'c' + true + 1.5f"),
        Some("String \"0.002ctrue1.5\"")
    );
    assert_eq!(constant(&e, "2.0e-3"), Some("double 0x1.0624dd2f1a9fcp-9"));
    assert_eq!(constant(&e, "(char) ('a' + 1)"), Some("char \"b\""));
    assert_eq!(constant(&e, "('a' + 1)"), Some("int 98"));
    assert_eq!(constant(&e, "(byte) 300"), Some("byte 44"));
    assert_eq!(constant(&e, "\"a\" == \"a\""), Some("boolean true"));
    assert_eq!(constant(&e, "1 / 0"), None);
    assert_eq!(constant(&e, "1.0 / 0"), Some("double Infinity"));
    assert_eq!(constant(&e, "\"x\" + null"), None);
    assert_eq!(constant(&e, "1 << 33"), Some("int 2"));
    assert_eq!(constant(&e, "-0.0"), Some("double -0x0.0p0"));
    assert_eq!(constant(&e, "0xFFFF_FFFF"), Some("int -1"));
    assert_eq!(constant(&e, "0x1.8p3"), Some("double 0x1.8p3"));
    assert_eq!(constant(&e, "1.0f / 3"), Some("float 0x1.555556p-2"));
    assert_eq!(constant(&e, "-2147483648"), Some("int -2147483648"));
    assert_eq!(constant(&e, "K > 2 ? K : -K"), Some("int 3"));
    assert_eq!(constant(&e, "k + 1"), Some("int 4"));
    assert_eq!(constant(&e, "self.K"), None);
    assert_eq!(constant(&e, "this.K"), None);
    let tb = e.iter().find(|x| x.text.starts_with("\"\"\"")).unwrap();
    assert_eq!(
        tb.constant.as_deref(),
        Some("String \"hello\\u000a  world\\u0009!\\u000a\"")
    );
    assert_eq!(
        constant(&e, "\"q\\\"\\\\\\u00e9\\uD83D\\uDE00\""),
        Some("String \"q\\\"\\\\\\u00e9\\ud83d\\ude00\"")
    );
}

/// A constant field's value crosses files, and a cycle of constants isn't one.
#[test]
fn constants_across_files() {
    let files = "//- file main p/A.java\n\
                 package p; public class A { public static final int X = B.Y + 1; public static final String S = \"a\" + X;\n\
                 static final int C1 = C2; static final int C2 = C1; final int instance = 7; static final Integer BOXED = 1; }\n\
                 //- file main p/B.java\n\
                 package p; class B { static final int Y = 41; void m() { int a = A.X; String s = A.S; int c = A.C1; int i = new A().instance; Integer b = A.BOXED; } }\n";
    let e = expressions_in(files, "B.java");
    assert_eq!(constant(&e, "A.X"), Some("int 42"));
    assert_eq!(constant(&e, "A.S"), Some("String \"a42\""));
    assert_eq!(constant(&e, "A.C1"), None);
    assert_eq!(constant(&e, "A.BOXED"), None);
}

/// javac: `case RED` in a switch on an enum is `Color`, `case K` `int` with its constant. (javac
/// also has the selector's parentheses, `(color)`, as a `PARENTHESIZED` tree of type `Color`,
/// which Ferrion's tree has no node for.)
#[test]
fn switch_statements() {
    let e = expressions(
        "enum Color { RED, GREEN }\n\
         class Q { static final int K = 3; void m(Color color, int i, String s) {\n\
         switch (color) { case RED: break; case GREEN: break; }\n\
         switch (i) { case 1: case K: break; }\n\
         switch (s) { case \"a\": break; } } }",
    );
    assert_eq!(ty(&e, "RED"), "Color");
    assert_eq!(ty(&e, "GREEN"), "Color");
    assert_eq!(ty(&e, "color"), "Color");
    assert_eq!(get(&e, "K", 0).constant.as_deref(), Some("int 3"));
}

/// javac: inside a lambda, `i + 1` is typed (`int`), and an explicitly typed parameter `a` is
/// `int`; the lambda itself is its target type (B3c's), and an implicitly typed parameter's uses
/// are B3c's too. A pattern `instanceof` is `boolean`, and its binding `str` the declared type.
#[test]
fn lambdas_and_patterns() {
    let e = expressions(
        "class Q { interface F { int f(int a); } void m(int i, Object o) {\n\
         Runnable r = () -> { int q = i + 1; }; F f = (int a) -> a + 1; F g = a -> a * 2;\n\
         if (o instanceof String str) { String t = str; } } }",
    );
    assert_eq!(ty(&e, "i + 1"), "int");
    assert_eq!(ty(&e, "a + 1"), "int");
    assert_eq!(ty(&e, "a * 2"), "left-to-b3c inference");
    assert_eq!(ty(&e, "() -> { int q = i + 1; }"), "left-to-b3c inference");
    assert_eq!(ty(&e, "o instanceof String str"), "boolean");
    assert_eq!(ty(&e, "str"), "java/lang/String");
}

/// Calls and creations wait for overload resolution (W3), and what depends on them with them;
/// their arguments are typed.
#[test]
fn calls_wait_for_overloads() {
    let e = expressions(
        "class Q { int f; int g(int x) { return x; } void m(int i) { int a = g(i + 1); int b = new Q().f; } }",
    );
    assert_eq!(ty(&e, "g(i + 1)"), "unknown calls");
    assert_eq!(ty(&e, "i + 1"), "int");
    assert_eq!(ty(&e, "new Q()"), "unknown calls");
    assert_eq!(ty(&e, "new Q().f"), "unknown calls");
}

/// javac: an annotation's `K + 1` is `int` with its constant, `names = {"a", "b"}`'s array
/// `java/lang/String[]`, and an element's default `{}` too.
#[test]
fn annotations() {
    let e = expressions(
        "@interface Ann { int value() default 0; String[] names() default {}; }\n\
         class Q { static final int K = 3; @Ann(K + 1) int a; @Ann(names = {\"a\", \"b\"}) int b; }",
    );
    assert_eq!(ty(&e, "K + 1"), "int");
    assert_eq!(constant(&e, "K + 1"), Some("int 4"));
    assert_eq!(ty(&e, "{\"a\", \"b\"}"), "java/lang/String[]");
    assert_eq!(ty(&e, "{}"), "java/lang/String[]");
}

/// A field of an expression's type, which B2 leaves to B3, found in the member table of the
/// expression's type: here an array element's, and a type variable's through its bound.
/// javac: `ps[0].x` `int`, `t.x` `int`, `t.g` `java/lang/String` for `T extends P<String>`.
#[test]
fn fields_left_by_names() {
    let e = expressions(
        "class P<G> { int x; G g; }\n\
         class Q<T extends P<String>> { P<Integer>[] ps; T t; void m() { int a = ps[0].x; int b = t.x; String c = t.g; Integer d = ps[0].g; } }",
    );
    assert_eq!(ty(&e, "ps[0].x"), "int");
    assert_eq!(ty(&e, "t.x"), "int");
    assert_eq!(ty(&e, "t.g"), "java/lang/String");
    assert_eq!(ty(&e, "ps[0].g"), "java/lang/Integer");
}

/// A local class's members, and a local class's constant field.
#[test]
fn local_classes() {
    let e = expressions(
        "class Q { void m() { class L { static final int C = 5; int f; } L l = null; int a = l.f; int b = L.C; int c = new Object() { int h = 2; }.h; } }",
    );
    assert_eq!(ty(&e, "l.f"), "int");
    assert_eq!(ty(&e, "L.C"), "int");
    assert_eq!(constant(&e, "L.C"), Some("int 5"));
}
