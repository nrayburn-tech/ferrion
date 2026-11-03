//! Error units (`SyntaxNode::error_unit`, `SyntaxTree::error_units`) on trees `ferrion_parser`
//! recovered, as docs/design/parser-error-recovery.md, "What a syntax error affects", defines
//! them.

use std::sync::Arc;

use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::{SyntaxKind, SyntaxTree};

fn tree(text: &str) -> SyntaxTree {
    let text: Arc<str> = Arc::from(text);
    let parsed = parse(text.clone(), ParseOptions { release: 27 });
    ferrion_parser::invariants::check(&text, 27, &parsed).unwrap();
    parsed.tree
}

/// The text of each of `text`'s error units, in order.
fn units(text: &str) -> Vec<String> {
    let tree = tree(text);
    tree.error_units()
        .iter()
        .map(|unit| unit.text().to_owned())
        .collect()
}

/// `body` as a method's body, in a class.
fn in_method(body: &str) -> String {
    format!("class A {{\n  void f() {{\n{body}\n  }}\n}}\n")
}

#[test]
fn no_errors_no_units() {
    assert!(units("class A { void f() { int x = 1; } }").is_empty());
}

#[test]
fn a_statement_is_its_errors_unit() {
    assert_eq!(
        units(&in_method("    int x = 1\n    int y = 2;")),
        ["int x = 1"]
    );
    assert_eq!(
        units(&in_method("    foo(1, );\n    bar();")),
        ["foo(1, );"]
    );
}

#[test]
fn an_error_in_a_header_affects_the_whole_statement() {
    assert_eq!(
        units(&in_method("    if (a > ) {\n      b();\n    }\n    c();")),
        ["if (a > ) {\n      b();\n    }"]
    );
}

#[test]
fn the_innermost_statement_counts_in_a_lambda_body() {
    assert_eq!(
        units(&in_method(
            "    run(() -> {\n      a(;\n      b();\n    });"
        )),
        ["a(;"]
    );
}

#[test]
fn an_error_node_in_a_list_is_a_unit_of_its_own() {
    assert_eq!(units(&in_method("    a(); )\n    b();")), [")"]);
    assert_eq!(units("class A {\n  int x;\n  )\n  int y;\n}\n"), [")"]);
    assert_eq!(units("class A {}\n}\n"), ["}"]);
}

#[test]
fn an_error_in_a_declarations_header_affects_the_whole_declaration() {
    assert_eq!(
        units("class A implements B extends C {\n  int x;\n}\nclass D {}\n"),
        ["class A implements B extends C {\n  int x;\n}"]
    );
    assert_eq!(
        units("class A {\n  void f(int a b) {\n    g();\n  }\n  int y;\n}\n"),
        ["void f(int a b) {\n    g();\n  }"]
    );
}

#[test]
fn an_unclosed_body_affects_its_declaration() {
    // The block closes at `private`, which can't start a statement, so `b` is whole.
    assert_eq!(
        units("class A {\n  void a() {\n    x();\n  private void b() {}\n}\n"),
        ["void a() {\n    x();"]
    );
}

#[test]
fn an_unclosed_body_with_a_stray_token_affects_its_declaration() {
    // The block closes at `private`, so its last token is the `}` of the `if`, not its own.
    assert_eq!(
        units("class A {\n  void a() {\n    if (b) {}\n    .\n  private int c;\n}\n"),
        ["void a() {\n    if (b) {}\n    ."]
    );
}

#[test]
fn imports_and_packages_are_units() {
    assert_eq!(
        units("package p\nimport a.B;\nimport a.C\nclass A {}\n"),
        ["package p", "import a.C"]
    );
}

#[test]
fn a_lexical_error_affects_the_node_its_token_is_in() {
    assert_eq!(units("class A {\n  int x = #1;\n}\n"), ["int x = #1;"]);
    // Between members, it is the class body's.
    assert_eq!(
        units("class A {\n  int x;\n  #\n  int y;\n}\nclass B {}\n"),
        ["class A {\n  int x;\n  #\n  int y;\n}"]
    );
    // Between top-level declarations, the whole file's.
    let text = "class A {}\n#\nclass B {}\n";
    assert_eq!(units(text), [text]);
}

#[test]
fn units_inside_another_unit_are_left_out() {
    // The `catch` header's error makes the whole `try` the unit; the statement inside it is in
    // that unit.
    assert_eq!(
        units(&in_method(
            "    try {\n      a(;\n    } catch (E e {\n    }"
        )),
        ["try {\n      a(;\n    } catch (E e {\n    }"]
    );
    assert_eq!(units(&in_method("    a(;\n    b(;")), ["a(;", "b(;"]);
}

#[test]
fn error_unit_of_any_node() {
    let tree = tree(&in_method("    foo(1 +);"));
    let binary = tree
        .nodes()
        .find(|n| n.kind() == SyntaxKind::BinaryExpr)
        .unwrap();
    assert!(binary.has_own_error());
    assert_eq!(binary.error_unit().text(), "foo(1 +);");
    assert_eq!(tree.root().error_unit(), tree.root());
}
