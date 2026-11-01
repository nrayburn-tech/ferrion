use super::*;

const OPTIONS: FormatOptions = FormatOptions { release: 27 };

fn check(input: &str, output: &str) -> Result<(), Violation> {
    verify(
        &SourceText::decode(input).unwrap(),
        output.as_bytes(),
        &OPTIONS,
    )
}

fn property(input: &str, output: &str) -> Property {
    check(input, output).unwrap_err().property
}

#[test]
fn the_formatters_output_passes() {
    check("class A{int x;}", "class A {\n  int x;\n}\n").unwrap();
}

#[test]
fn unformatted_output_isnt_idempotent() {
    let violation = check("class A{int x;}", "class A{int x;}").unwrap_err();
    assert_eq!(violation.property, Property::Idempotence);
    assert_eq!(violation.offset, Some(7));
}

#[test]
fn a_changed_token_isnt_equivalent() {
    let violation = check("class A { int x; }", "class A {\n  int y;\n}\n").unwrap_err();
    assert_eq!(violation.property, Property::Equivalence);
    assert_eq!(violation.offset, Some(16));
    assert!(
        violation.message.contains("Identifier \"x\""),
        "{violation}"
    );
}

#[test]
fn a_changed_tree_isnt_equivalent() {
    // The same tokens, nested differently: `a - (b - c)` against `(a - b) - c` would add tokens,
    // but `x = y` as a statement and as a field differ in their nodes.
    assert_eq!(
        property("class A { int x; }", "class A {\n  int x;\n\n  ;\n}\n"),
        Property::Equivalence
    );
}

#[test]
fn modifier_runs_compare_as_sorted() {
    check(
        "class A { static public int x; }",
        "class A {\n  public static int x;\n}\n",
    )
    .unwrap();
    // Across an annotation, they're two runs.
    assert_eq!(
        property(
            "class A { static @D public int x; }",
            "class A {\n  public @D static int x;\n}\n"
        ),
        Property::Equivalence
    );
}

#[test]
fn a_lost_or_changed_comment_is_reported() {
    let violation = check("class A {} // c\n", "class A {}\n").unwrap_err();
    assert_eq!(violation.property, Property::Comments);
    assert_eq!(
        property("class A {} // c\n", "class A {} // d\n"),
        Property::Comments
    );
    // Trailing whitespace, and the indentation of a block comment's later lines, may change.
    check(
        "class A {} // c  \n/* a\n      b */\n",
        "class A {} // c\n/* a\nb */\n",
    )
    .unwrap();
}

#[test]
fn errors_in_the_output_are_reported() {
    let violation = check("class A {}", "class A {\n").unwrap_err();
    assert_eq!(violation.property, Property::NoNewErrors);
    assert_eq!(
        verify(
            &SourceText::decode("class A {}").unwrap(),
            b"class A {} \xFF",
            &OPTIONS
        )
        .unwrap_err()
        .property,
        Property::NoNewErrors
    );
}

#[test]
fn an_input_with_errors_must_be_left_alone() {
    check("class A {", "class A {").unwrap();
    assert_eq!(property("class A {", "class A {\n"), Property::NoNewErrors);
}

#[test]
fn text_blocks_compare_apart_from_line_terminators() {
    check(
        "class A {\r\n  String s =\r\n      \"\"\"\r\n    a\r\n    \"\"\";\r\n}\r\n",
        "class A {\r\n  String s =\r\n      \"\"\"\r\n    a\r\n    \"\"\";\r\n}\r\n",
    )
    .unwrap();
    assert_eq!(
        property(
            "class A {\n  String s = \"\"\"\n    a\n    \"\"\";\n}\n",
            "class A {\n  String s = \"\"\"\n    b\n    \"\"\";\n}\n"
        ),
        Property::Equivalence
    );
}
