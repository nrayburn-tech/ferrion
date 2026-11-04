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

/// The variables of `source` (one file `Q.java`), each as its name and type.
fn variables(source: &str) -> Vec<(String, String)> {
    let engine = engine(&format!("//- file main Q.java\n{source}"));
    let snapshot = engine.snapshot();
    let file = snapshot.files(SourceSetId::from_index(0)).unwrap()[0];
    let types = snapshot.body_types(file).unwrap();
    let tree = snapshot.tree(file).unwrap();
    let mut cx = snapshot.types_in(SourceSetId::from_index(0), types.arena.clone());
    types
        .variables
        .iter()
        .map(|(node, ty)| {
            let name = tree
                .node(*node)
                .direct_tokens()
                .filter(|t| t.kind() == ferrion_syntax::SyntaxKind::Identifier)
                .last()
                .map_or(String::new(), |t| t.text().to_owned());
            (name, cx.type_string(*ty, Printed::Captures).unwrap())
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

/// javac: `I.super` is the interface as a supertype of the class around it,
/// `I<java/lang/String>` in `D implements I<String>` and `I<java/lang/Integer>` in an anonymous
/// `new I<Integer>() { ... }`.
#[test]
fn qualified_super() {
    let e = expressions(
        "interface I<T> { default String p() { return \"i\"; } }\n\
         class Q implements I<String> {\n\
         public String p() { return I.super.p(); }\n\
         Object m() { var x = new I<Integer>() { public String p() { return I.super.p(); } }; return x; } }",
    );
    assert_eq!(get(&e, "I.super", 0).ty, "I<java/lang/String>");
    assert_eq!(get(&e, "I.super", 1).ty, "I<java/lang/Integer>");
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

/// javac: `? super Object` captures to `Object` itself (`List<? super Object>` is
/// `java/util/List<java/lang/Object>`, and `Map<? super Object, ? extends Object>`
/// `java/util/Map<java/lang/Object,capture#1 extends java/lang/Object>`); `var` of a cast to
/// `F<String, ?>`, with `F<T, A extends Annotation>`, is `Q$F<java/lang/String,?>` (JLS 4.10.5:
/// the declared bound is below the capture's), and of an `H<String, ?>` with `H<T, A extends
/// List<T>>` `Q$H<java/lang/String,? extends java/util/List<java/lang/String>>`; a `final var`
/// with a constant initializer is a constant variable (`n` is `int 200000`); `super` in an
/// anonymous class made with `<>` is inference's (javac: `java/util/ArrayList<java/lang/Object>`).
#[test]
fn captures_and_projections() {
    let source = "import java.util.*;\nimport java.lang.annotation.Annotation;\n\
         class Q<K> { interface F<T, A extends Annotation> { } interface H<T, A extends List<T>> { }\n\
         List<? super Object> a; Map<? super Object, ? extends Object> d; F<String, ?> f; H<String, ?> h;\n\
         void m(Object o) { Object x1 = a; Object x5 = d; var v = (F<String, ?>) o; var w = f; var y = h;\n\
         final var n = 200_000; int z = n + 1;\n\
         Object anon = new ArrayList<>() { public int size() { return super.size(); } }; } }";
    let e = expressions(source);
    assert_eq!(ty(&e, "a"), "java/util/List<java/lang/Object>");
    assert_eq!(
        ty(&e, "d"),
        "java/util/Map<java/lang/Object,capture#1 extends java/lang/Object>"
    );
    assert_eq!(constant(&e, "n"), Some("int 200000"));
    assert_eq!(constant(&e, "n + 1"), Some("int 200001"));
    assert_eq!(ty(&e, "super"), "left-to-b3c inference");
    let vars = variables(source);
    let var = |name: &str| {
        vars.iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.as_str())
            .unwrap_or_else(|| panic!("no variable {name} in {vars:?}"))
    };
    assert_eq!(var("v"), "Q$F<java/lang/String,?>");
    assert_eq!(var("w"), "Q$F<java/lang/String,?>");
    assert_eq!(
        var("y"),
        "Q$H<java/lang/String,? extends java/util/List<java/lang/String>>"
    );
    assert_eq!(var("n"), "int");
}

/// javac: a multi-catch parameter's identifier and variable are the union,
/// `java/lang/InterruptedException|java/lang/IllegalStateException`, parenthesized too, while its
/// type for relations is the `lub` (`e.x` would look in `Exception`). `Outlet<X>.Shop`, with
/// `Shop` a member of `Outlet`'s superclass `Mall`, is `Q$Mall<X>$Shop`; read, a wildcard in the
/// outer type is captured (`Q$Mall<capture#1 extends java/lang/Number>$Shop<java/lang/Integer>`).
#[test]
fn multi_catch_and_written_outer_types() {
    let source = "class Q { static class Mall<T> { class Shop<U> { } } static class Outlet<T> extends Mall<T> { }\n\
         void m(Outlet<String>.Shop<Integer> a, Outlet<? extends Number>.Shop<Integer> b, Object o) {\n\
         Object x = a; Object y = b;\n\
         try { o.hashCode(); o.wait(); } catch (InterruptedException | IllegalStateException e) { Object q = e; Object r = (e); } } }";
    let e = expressions(source);
    assert_eq!(
        ty(&e, "a"),
        "Q$Mall<java/lang/String>$Shop<java/lang/Integer>"
    );
    assert_eq!(
        ty(&e, "b"),
        "Q$Mall<capture#1 extends java/lang/Number>$Shop<java/lang/Integer>"
    );
    let union = "java/lang/InterruptedException|java/lang/IllegalStateException";
    assert_eq!(ty(&e, "e"), union);
    assert_eq!(ty(&e, "(e)"), union);
    let vars = variables(source);
    assert!(vars.iter().any(|(n, t)| n == "e" && t == union), "{vars:?}");
}

/// javac: `case DESC` on a selector whose type B2 couldn't tell, with a `String` field `DESC` in
/// scope, is the enum's constant, `Op`; on a selector that is a call, it waits for the call.
#[test]
fn case_names_follow_the_selector() {
    let source = "enum Op { ASC, DESC }\n\
         class Q { static final String DESC = \"d\"; Op[] ops; Op op() { return null; }\n\
         void m() { switch (ops[0]) { case ASC: break; case DESC: break; }\n\
         switch (op()) { case DESC: break; default: break; }\n\
         switch (\"x\") { case DESC: break; default: break; } } }";
    let e = expressions(source);
    assert_eq!(get(&e, "DESC", 0).ty, "Op");
    assert_eq!(get(&e, "DESC", 1).ty, "unknown calls");
    assert_eq!(get(&e, "DESC", 2).ty, "java/lang/String");
    assert_eq!(get(&e, "DESC", 2).constant.as_deref(), Some("String \"d\""));
}

/// javac: inside an anonymous subclass of an inner class of a generic class, the inherited
/// field `cursor` is `E:D` (the inner class's outer type is `D<E>`, as the class around it sees
/// it).
#[test]
fn inner_classes_inside_local_classes() {
    let source = "abstract class D<E> { E first; abstract class It { E cursor; It(E e) { } }\n\
         Object m() { return new It(first) { E f() { return cursor; } }; } }";
    let e = expressions(source);
    assert_eq!(ty(&e, "cursor"), "E:D");
}

/// javac on more forms: `(xs).size()`'s receiver isn't captured, through the parentheses; a
/// reference conditional under a cast is standalone (`java/lang/String`), and with `var`
/// `java/lang/Integer` for `flag ? null : 1`; `flag ? c : 1` is `char`; `-129` and `-1` narrow as
/// Java's casts do; `1 + 2 + "a" + 1 + 2` folds left to right.
#[test]
fn more_forms() {
    let e = expressions(
        "class Q<T extends Number> { java.util.List<? extends Number> xs; T t; int[] arr; char c; boolean flag; Integer boxed;\n\
         void m(byte b, short s) {\n\
         int q = (xs).size(); Object f = (this).xs; (arr)[0] = 1; arr[0] += 2; c += 1; b++; s = (short) (b + 1); t = null;\n\
         String ss = null + \"a\"; String st = \"a\" + null;\n\
         String so = (String) (flag ? null : \"a\"); var vn = flag ? null : 1; var vd = (flag ? xs : null);\n\
         boolean bb = boxed == 1; boolean bc = boxed == boxed; int sh = boxed << 2; int neg = -boxed;\n\
         int tern = flag ? c : 1; long tern2 = flag ? boxed : 1L; int[][] aa = {{1}, {2, 3}}; String[] sa = {\"a\", null};\n\
         Object oa = new Object[] {1, \"x\"}; for (int i = 0, j = 1; i < j; i++, j--) { }\n\
         long big = 2147483648L; int oct = 017; int bin = 0b101; long hexl = 0xFFL; float hf = 0x1p3f; double dd = 1e3;\n\
         double d2 = 1.5d; float f2 = .5f; char uc = '\\u0041'; char esc = '\\n'; char oc2 = '\\101'; String sb = \"\\0\\377\\s\";\n\
         int cast = (int) 3.9; byte bneg = (byte) -129; char neg1 = (char) -1; long ll = (long) 1e30; boolean lt = 'a' < 98;\n\
         boolean nb = !true; boolean xor = true ^ false; boolean and = true && false; int band = 6 & 3; int bor = 6 | 3; int bxor = 6 ^ 3;\n\
         long ls = 1L >>> 1; int rem = 7 % 3; double drem = 7.5 % 2; float fm = 2f * 3;\n\
         String cs = \"\" + 1L + 1.0f + 1.0 + 'c' + (byte) 1 + (short) 2 + true; String mix = 1 + 2 + \"a\" + 1 + 2;\n\
         int intc = 'a' + 'b'; int ic = Integer.MAX_VALUE + 1; double inf = -1.0 / 0; double nan = 0.0 / 0; boolean nane = 0.0 / 0 == 0.0 / 0;\n\
         float big2 = 1e38f * 10; String fs = \"\" + 1.0E10f + 1.0E-5 + 100.0 + 1234567.0 + 12345678.0 + 0.001 + 0.0001; } }",
    );
    assert_eq!(
        get(&e, "(xs)", 0).ty,
        "java/util/List<? extends java/lang/Number>"
    );
    assert_eq!(
        get(&e, "xs", 0).ty,
        "java/util/List<? extends java/lang/Number>"
    );
    assert_eq!(
        ty(&e, "(this).xs"),
        "java/util/List<capture#1 extends java/lang/Number>"
    );
    assert_eq!(ty(&e, "(this)"), "Q<T:Q>");
    assert_eq!(ty(&e, "(arr)[0] = 1"), "int");
    assert_eq!(ty(&e, "arr[0] += 2"), "int");
    assert_eq!(ty(&e, "c += 1"), "char");
    assert_eq!(ty(&e, "b++"), "byte");
    assert_eq!(ty(&e, "s = (short) (b + 1)"), "short");
    assert_eq!(ty(&e, "t = null"), "T:Q");
    assert_eq!(ty(&e, "null + \"a\""), "java/lang/String");
    assert_eq!(constant(&e, "null + \"a\""), None);
    assert_eq!(ty(&e, "(flag ? null : \"a\")"), "java/lang/String");
    assert_eq!(ty(&e, "flag ? null : 1"), "java/lang/Integer");
    assert_eq!(
        ty(&e, "flag ? xs : null"),
        "java/util/List<capture#1 extends java/lang/Number>"
    );
    assert_eq!(ty(&e, "boxed == 1"), "boolean");
    assert_eq!(ty(&e, "boxed == boxed"), "boolean");
    assert_eq!(ty(&e, "boxed << 2"), "int");
    assert_eq!(ty(&e, "-boxed"), "int");
    assert_eq!(ty(&e, "flag ? c : 1"), "char");
    assert_eq!(ty(&e, "flag ? boxed : 1L"), "long");
    assert_eq!(ty(&e, "{{1}, {2, 3}}"), "int[][]");
    assert_eq!(ty(&e, "{1}"), "int[]");
    assert_eq!(ty(&e, "{\"a\", null}"), "java/lang/String[]");
    assert_eq!(ty(&e, "new Object[] {1, \"x\"}"), "java/lang/Object[]");
    assert_eq!(ty(&e, "j--"), "int");
    for (text, value) in [
        ("2147483648L", "long 2147483648"),
        ("017", "int 15"),
        ("0b101", "int 5"),
        ("0xFFL", "long 255"),
        ("0x1p3f", "float 0x1.0p3"),
        ("1e3", "double 0x1.f4p9"),
        ("1.5d", "double 0x1.8p0"),
        (".5f", "float 0x1.0p-1"),
        ("'\\u0041'", "char \"A\""),
        ("'\\n'", "char \"\\u000a\""),
        ("'\\101'", "char \"A\""),
        ("\"\\0\\377\\s\"", "String \"\\u0000\\u00ff \""),
        ("(int) 3.9", "int 3"),
        ("(byte) -129", "byte 127"),
        ("(char) -1", "char \"\\uffff\""),
        ("(long) 1e30", "long 9223372036854775807"),
        ("'a' < 98", "boolean true"),
        ("!true", "boolean false"),
        ("true ^ false", "boolean true"),
        ("true && false", "boolean false"),
        ("6 & 3", "int 2"),
        ("6 | 3", "int 7"),
        ("6 ^ 3", "int 5"),
        ("1L >>> 1", "long 0"),
        ("7 % 3", "int 1"),
        ("7.5 % 2", "double 0x1.8p0"),
        ("2f * 3", "float 0x1.8p2"),
        ("1 + 2 + \"a\" + 1 + 2", "String \"3a12\""),
        ("1 + 2 + \"a\"", "String \"3a\""),
        ("'a' + 'b'", "int 195"),
        ("Integer.MAX_VALUE + 1", "int -2147483648"),
        ("-1.0 / 0", "double -Infinity"),
        ("0.0 / 0", "double NaN"),
        ("0.0 / 0 == 0.0 / 0", "boolean false"),
        ("1e38f * 10", "float Infinity"),
        (
            "\"\" + 1L + 1.0f + 1.0 + 'c' + (byte) 1 + (short) 2 + true",
            "String \"11.01.0c12true\"",
        ),
        (
            "\"\" + 1.0E10f + 1.0E-5 + 100.0 + 1234567.0 + 12345678.0 + 0.001 + 0.0001",
            "String \"1.0E101.0E-5100.01234567.01.2345678E70.0011.0E-4\"",
        ),
    ] {
        assert_eq!(constant(&e, text), Some(value), "{text}");
    }
}

/// What each context applies, as JLS chapter 5 says (javac doesn't report conversions).
#[test]
fn conversions_are_recorded() {
    let source = "class Q { long r() { return 15; }\n\
         void m(Boolean b, byte t, char c, Integer boxed, int[] a, String s, Object ob) {\n\
         if (b) { } int[] n = new int[t]; int e = a[c];\n\
         long w = 11; Integer bi = 12; Object o = 13; char k = 14; double d = boxed;\n\
         String ss = \"a\" + t; t += 1; Object up = (Object) s; String back = (String) ob; } }";
    let e = expressions(source);
    let conversion = |text: &str, nth: usize| get(&e, text, nth).conversion.as_deref();
    assert_eq!(conversion("b", 0), Some("[Unboxing]"));
    assert_eq!(conversion("t", 0), Some("[WideningPrimitive]"));
    assert_eq!(conversion("c", 0), Some("[WideningPrimitive]"));
    assert_eq!(conversion("11", 0), Some("[WideningPrimitive]"));
    assert_eq!(conversion("12", 0), Some("[Boxing]"));
    assert_eq!(conversion("13", 0), Some("[Boxing, WideningReference]"));
    assert_eq!(conversion("14", 0), Some("[NarrowingPrimitive]"));
    assert_eq!(conversion("15", 0), Some("[WideningPrimitive]"));
    assert_eq!(
        conversion("boxed", 0),
        Some("[Unboxing, WideningPrimitive]")
    );
    assert_eq!(conversion("t", 1), Some("[String]"));
    assert_eq!(conversion("s", 0), Some("[WideningReference]"));
    assert_eq!(conversion("ob", 0), Some("[NarrowingReference]"));

    let engine = engine(&format!("//- file main Q.java\n{source}"));
    let snapshot = engine.snapshot();
    let file = snapshot.files(SourceSetId::from_index(0)).unwrap()[0];
    let dump = snapshot.body_types_dump(file).unwrap();
    assert!(dump.contains("compound [NarrowingPrimitive]"), "{dump}");
}
