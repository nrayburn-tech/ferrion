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
    // So may the space after a line comment's `//`, but only where the formatter adds one.
    check("class A {} //c\n", "class A {} // c\n").unwrap();
    assert_eq!(
        property(
            "class A {} //noinspection c\n",
            "class A {} // noinspection c\n"
        ),
        Property::Comments
    );
    assert_eq!(
        property("class A {} // c\n", "class A {} //c\n"),
        Property::Comments
    );
    // A disable region's comments are copied.
    let region = "// @formatter:off\nclass A {} //c\n";
    check(region, region).unwrap();
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

#[test]
fn imports_compare_as_a_set() {
    let input = "import b.B;\nimport a.A;\n\nclass C {\n  A a;\n  B b;\n}\n";
    check(
        input,
        "import a.A;\nimport b.B;\n\nclass C {\n  A a;\n  B b;\n}\n",
    )
    .unwrap();
    // An import that isn't the input's.
    let violation = check(
        input,
        "import a.A;\nimport c.B;\n\nclass C {\n  A a;\n  B b;\n}\n",
    )
    .unwrap_err();
    assert_eq!(violation.property, Property::Equivalence);
    assert!(violation.message.starts_with("import c.B;"), "{violation}");
    // The rest of the tree is still compared, with its node indices past the imports.
    assert_eq!(
        property(
            input,
            "import a.A;\nimport b.B;\n\nclass D {\n  A a;\n  B b;\n}\n"
        ),
        Property::Equivalence
    );
}

#[test]
fn a_wrong_removal_fails() {
    // `B` is used, so removing its import breaks the program.
    let violation = check(
        "import a.A;\nimport b.B;\n\nclass C {\n  B b;\n}\n",
        "class C {\n  B b;\n}\n",
    )
    .unwrap_err();
    assert_eq!(violation.property, Property::Equivalence);
    assert!(
        violation.message.contains("import b.B; was removed"),
        "{violation}"
    );
    // So is removing one named in any doc comment, or one on demand.
    assert_eq!(
        property(
            "import b.B;\n\n/** See {@link B}. */\nclass C {}\n",
            "/** See {@link B}. */\nclass C {}\n"
        ),
        Property::Equivalence
    );
    assert_eq!(
        property("import b.*;\n\nclass C {}\n", "class C {}\n"),
        Property::Equivalence
    );
    // Unused, duplicate, and own-package imports may go.
    check(
        "package p;\n\nimport a.A;\nimport b.B;\nimport b.B;\nimport p.D;\n\nclass C {\n  B b;\n  D d;\n}\n",
        "package p;\n\nimport b.B;\n\nclass C {\n  B b;\n  D d;\n}\n",
    )
    .unwrap();
}

#[test]
fn a_lost_or_moved_import_comment_fails() {
    let input = "import b.B; // why B\nimport a.A;\n\nclass C {\n  A a;\n  B b;\n}\n";
    check(
        input,
        "import a.A;\nimport b.B; // why B\n\nclass C {\n  A a;\n  B b;\n}\n",
    )
    .unwrap();
    let lost = check(
        input,
        "import a.A;\nimport b.B;\n\nclass C {\n  A a;\n  B b;\n}\n",
    )
    .unwrap_err();
    assert_eq!(lost.property, Property::Comments);
    // Left behind with the other import.
    let moved = check(
        input,
        "import a.A; // why B\nimport b.B;\n\nclass C {\n  A a;\n  B b;\n}\n",
    )
    .unwrap_err();
    assert_eq!(moved.property, Property::Comments);
    assert!(moved.message.contains("aren't with it"), "{moved}");
    // A comment of no import may go to the top.
    check(
        "import b.B;\n\n// header\n\nimport a.A;\n\nclass C {\n  A a;\n  B b;\n}\n",
        "// header\n\nimport a.A;\nimport b.B;\n\nclass C {\n  A a;\n  B b;\n}\n",
    )
    .unwrap();
}
