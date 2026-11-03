use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{ParseOptions, parse};

use crate::{Formatted, format_tree};

/// Formats `text` at `release`, without the safety checks, which `verify`'s tests cover.
fn formatted_at(text: &str, release: u32) -> String {
    let source = SourceText::decode(text).unwrap();
    let parsed = parse(Arc::from(source.text()), ParseOptions { release });
    match format_tree(&parsed.tree, &source) {
        Formatted::Unchanged => text.to_owned(),
        Formatted::Changed(output) => String::from_utf8(output).unwrap(),
        Formatted::Skipped(errors) => panic!("skipped: {errors:?}"),
    }
}

fn formatted(text: &str) -> String {
    formatted_at(text, 27)
}

#[test]
fn groups_sort_by_the_whole_name() {
    assert_eq!(
        formatted(
            "package p;\nimport z.Z;\nimport a.b.C;\nimport a.B$;\n\nimport static q.R.r;\n\
             import module java.base;\nimport a.B.C2;\nclass T { Z z; C c; B$ d; C2 e; void f() \
             { r(); } }\n"
        ),
        "package p;\n\nimport static q.R.r;\n\nimport module java.base;\n\nimport a.B$;\n\
         import a.B.C2;\nimport a.b.C;\nimport z.Z;\n\nclass T {\n  Z z;\n  C c;\n  B$ d;\n  \
         C2 e;\n\n  void f() {\n    r();\n  }\n}\n"
    );
}

#[test]
fn utf16_code_units_order_names() {
    // U+1D538 is a surrogate pair starting at 0xD835, before U+FF5A.
    let output = formatted(
        "import a.b\u{FF5A};\nimport a.b\u{1D538};\nclass T { b\u{1D538} x; b\u{FF5A} y; }\n",
    );
    assert!(
        output.starts_with("import a.b\u{1D538};\nimport a.b\u{FF5A};\n"),
        "{output}"
    );
}

#[test]
fn escapes_are_translated_for_the_key_and_the_uses() {
    let escaped = format!(
        "import z.{}u0041;\nimport z.B;\nclass T {{ A a; B b; }}\n",
        '\\'
    );
    assert_eq!(
        formatted(&escaped),
        format!(
            "import z.{}u0041;\nimport z.B;\n\nclass T {{\n  A a;\n  B b;\n}}\n",
            '\\'
        )
    );
    let used = format!("import z.A;\nclass T {{ {}u0041 a; }}\n", '\\');
    assert!(formatted(&used).starts_with("import z.A;\n"));
}

#[test]
fn unused_own_package_and_duplicate_imports_go() {
    assert_eq!(
        formatted(
            "package p.q;\n\nimport x.Unused;\nimport p.q.A;\nimport p.q.A.B;\nimport x.Y;\n\
             import x.Y;\nimport static x.S.unused;\nimport static x.S.used;\n\nclass T { A a; \
             B b; Y y; int i = used; }\n"
        ),
        "package p.q;\n\nimport static x.S.used;\n\nimport p.q.A.B;\nimport x.Y;\n\nclass T {\n  \
         A a;\n  B b;\n  Y y;\n  int i = used;\n}\n"
    );
}

#[test]
fn wildcards_modules_and_java_lang_stay() {
    let input = "package p;\n\nimport static a.B.*;\n\nimport module java.base;\n\nimport \
                 java.lang.String;\nimport java.util.*;\nimport p.*;\n\nclass T {\n  String s;\n}\n";
    assert_eq!(formatted(input), input);
}

#[test]
fn what_counts_as_a_use() {
    let output = formatted(
        "package p;\n\nimport u.Local;\nimport u.Decl;\nimport u.Lbl;\nimport u.Sel;\nimport \
         u.Pkg;\nimport u.MRef;\nimport u.Member;\nimport u.Name;\nimport u.Lam;\n\nclass T {\n  \
         @A(Name = 1)\n  void Decl(int Local) {\n    Local++;\n    Lbl:\n    for (; ; ) break \
         Lbl;\n    x.Sel.y();\n    Pkg.a.b();\n    Runnable r = MRef::Member;\n    \
         java.util.function.IntUnaryOperator o = Lam -> Lam;\n  }\n}\n",
    );
    assert!(
        output.starts_with(
            "package p;\n\nimport u.Lam;\nimport u.Local;\nimport u.MRef;\nimport u.Name;\nimport \
             u.Pkg;\n\n"
        ),
        "{output}"
    );
}

#[test]
fn attached_doc_comments_count() {
    let output = formatted(
        "package p;\n\nimport u.Link;\nimport u.Param;\nimport u.Value;\nimport u.ValueOnly;\n\
         import u.See;\nimport u.Code;\nimport u.Loose;\nimport u.Label;\nimport u.Before;\n\
         import u.Local;\nimport static u.S.m;\n\n/** {@link Before} */\n/** {@link \
         Link#f(Param)} {@value Value#X} {@value ValueOnly} {@code Code} {@link m} {@link String \
         Label}\n * @see See\n */\nclass T {\n  void f() {\n    /** {@link Local} */\n    int \
         x;\n  }\n\n  /** {@link Loose} */\n}\n",
    );
    // Only the comment right before a declaration is attached: not `Before`, `Local` (a local
    // variable's), or `Loose` (before the `}`), as GJF counts them (observed).
    assert!(
        output.starts_with(
            "package p;\n\nimport static u.S.m;\n\nimport u.Link;\nimport u.Param;\nimport \
             u.See;\nimport u.Value;\n\n"
        ),
        "{output}"
    );
}

#[test]
fn markdown_references_count_from_release_23() {
    let input = "import u.Md;\n\n/// [Md]\nclass T {}\n";
    assert_eq!(formatted_at(input, 23), input);
    assert_eq!(formatted_at(input, 21), "/// [Md]\nclass T {}\n");
}

#[test]
fn comments_move_with_their_imports() {
    assert_eq!(
        formatted(
            "// license\n\nimport z.Z; // trailing Z\n// above Y\nimport y.Y;\n\n// header\n\n\
             import x.X;\nimport /* in */ w.W;\n// after the last import\nclass T { Z z; Y y; X \
             x; W w; }\n"
        ),
        "// license\n\n// header\n\nimport /* in */ w.W;\nimport x.X;\n// above Y\nimport y.Y;\n\
         import z.Z; // trailing Z\n\n// after the last import\nclass T {\n  Z z;\n  Y y;\n  X \
         x;\n  W w;\n}\n"
    );
}

#[test]
fn an_import_with_a_comment_stays() {
    assert_eq!(
        formatted("import b.B;\n// why\nimport a.A;\nimport a.A; // again\nclass T { B b; }\n"),
        "// why\nimport a.A;\nimport a.A; // again\nimport b.B;\n\nclass T {\n  B b;\n}\n"
    );
}

#[test]
fn comments_above_the_first_import_stay_on_top() {
    // They keep it while it stays first, as they would in a second run.
    assert_eq!(
        formatted("// header\nimport a.Unused;\nimport b.B;\nclass T { B b; }\n"),
        "// header\nimport a.Unused;\nimport b.B;\n\nclass T {\n  B b;\n}\n"
    );
    assert_eq!(
        formatted("// header\nimport c.Unused;\nimport b.B;\nclass T { B b; }\n"),
        "// header\nimport b.B;\n\nclass T {\n  B b;\n}\n"
    );
    assert_eq!(
        formatted("// header\n\nimport a.Unused;\nimport b.B;\nclass T { B b; }\n"),
        "// header\n\nimport b.B;\n\nclass T {\n  B b;\n}\n"
    );
}

#[test]
fn directives_and_regions_pin_the_section() {
    for input in [
        "import b.B;\n// @formatter:off\nimport a.A;\n// @formatter:on\nclass T {}\n",
        "import b.B;\n// ferrion-disable-next x -- why\nimport a.A;\n\nclass T {}\n",
        "// @formatter:off\nimport b.B;\nimport a.A;\nclass T {}\n",
    ] {
        assert_eq!(formatted(input), input);
    }
}

#[test]
fn removing_every_import_leaves_one_blank_line() {
    assert_eq!(
        formatted("package p;\n\nimport x.A;\n\n\nclass T {}\n"),
        "package p;\n\nclass T {}\n"
    );
    assert_eq!(formatted("\nimport x.A;\n\nclass T {}\n"), "class T {}\n");
}
