use std::sync::Arc;

use ferrion_parser::{ParseOptions, parse};

use super::*;
use crate::{apply_edits, format_edits};

fn tree(text: &str) -> SyntaxTree {
    parse(Arc::from(text), ParseOptions { release: 27 }).tree
}

/// The text of the units `needle`'s first occurrence in `text` expands to, and of the gaps
/// before and after them.
fn expanded(text: &str, needle: &str) -> Option<(String, String, String)> {
    let tree = tree(text);
    let source = SourceText::decode(text).unwrap();
    let start = text.find(needle).unwrap() as u32;
    let range = start..start + needle.len() as u32;
    let e = Ranges::new(&tree, &source).expand(range)?;
    let part = |r: Range<u32>| text[r.start as usize..r.end as usize].to_owned();
    Some((
        part(e.before..e.units.start),
        part(e.units.clone()),
        part(e.units.end..e.after),
    ))
}

/// `text` with the edits `format_range` gives for `needle`'s first occurrence applied.
fn formatted(text: &str, needle: &str) -> Result<String, RangeError> {
    let tree = tree(text);
    let source = SourceText::decode(text).unwrap();
    let start = text.find(needle).unwrap() as u32;
    let edits = format_range(&tree, &source, start..start + needle.len() as u32)?;
    Ok(apply_edits(text, &edits))
}

const METHOD: &str = "class A {\n  void f(int a) {\n    if (a  >  1) {\n      x(  );\n    }   \
                      else {\n      // c\n      y(  ); // t\n    }\n  }\n}\n";

#[test]
fn a_statement_with_blocks_is_its_runs() {
    assert_eq!(
        expanded(METHOD, "a  >"),
        Some(("\n    ".into(), "if (a  >  1) {".into(), "\n      ".into()))
    );
    assert_eq!(
        expanded(METHOD, "else"),
        Some(("\n    ".into(), "}   else {".into(), "\n      ".into()))
    );
    assert_eq!(
        expanded(METHOD, "x"),
        Some(("\n      ".into(), "x(  );".into(), "\n    ".into()))
    );
}

#[test]
fn comments_count_with_their_code() {
    // A comment leading a statement is in its unit, one trailing it too.
    assert_eq!(
        expanded(METHOD, "// c"),
        Some((
            "\n      ".into(),
            "// c\n      y(  ); // t".into(),
            "\n    ".into()
        ))
    );
    assert_eq!(expanded(METHOD, "// t"), expanded(METHOD, "y(  )"));
}

#[test]
fn whitespace_alone_expands_to_nothing() {
    assert_eq!(expanded(METHOD, "  "), None);
    assert_eq!(expanded("class A {}\n\n", "\n\n"), None);
}

#[test]
fn a_range_in_an_expression_formats_the_statement_around_it() {
    let text = "class A {\n  void f() {\n    run(() -> {\n      int   y;\n    });\n  }\n}\n";
    assert_eq!(
        expanded(text, "y;").map(|(_, units, _)| units),
        Some("run(() -> {\n      int   y;\n    });".into())
    );
}

#[test]
fn the_gap_after_keeps_the_next_lines_indentation() {
    let text = "class A {\n  void f() {\n    int  a = 1;   \n\n\n      int  b = 2;\n  }\n}\n";
    assert_eq!(
        formatted(text, "a ").unwrap(),
        "class A {\n  void f() {\n    int a = 1;\n\n      int  b = 2;\n  }\n}\n"
    );
    // Code on the same line moves to a line of its own.
    let text = "class A {\n  void f() {\n    int  a = 1; int  b = 2;\n  }\n}\n";
    assert_eq!(
        formatted(text, "a ").unwrap(),
        "class A {\n  void f() {\n    int a = 1;\n    int  b = 2;\n  }\n}\n"
    );
}

#[test]
fn a_comment_goes_with_the_code_formatting_puts_it_beside() {
    // Formatting gives this comment a line of its own, before `int x;`, so it goes with that: a
    // range on the class header leaves both, and one on the field moves both. Moving only the
    // comment would leave `/* c */ int x;`, which formatting the whole file keeps on one line.
    let text = "class A { /* c */ int x;\n}\n";
    assert_eq!(formatted(text, "class").unwrap(), text);
    assert_eq!(
        formatted(text, "int").unwrap(),
        "class A {\n  /* c */\n  int x;\n}\n"
    );
    // This one stays after `a();`, so it goes with that.
    let text = "class A {\n  void f() {\n    a();   /* c */ b();\n  }\n}\n";
    assert_eq!(
        formatted(text, "a()").unwrap(),
        "class A {\n  void f() {\n    a(); /* c */\n    b();\n  }\n}\n"
    );
}

#[test]
fn a_comment_on_the_line_of_the_code_before_goes_with_that_code() {
    let text = "class A {\n  void f() {\n    a();   /* c */ b(  );\n  }\n}\n";
    assert_eq!(
        formatted(text, "b(").unwrap(),
        "class A {\n  void f() {\n    a();   /* c */\n    b();\n  }\n}\n"
    );
    assert_eq!(expanded(text, "/* c */"), expanded(text, "a()"));
}

#[test]
fn a_clean_files_edits_are_whole_file_formattings() {
    let source = SourceText::decode(METHOD).unwrap();
    let tree = tree(METHOD);
    let all = format_edits(&tree, &source).unwrap();
    let range = format_range(&tree, &source, 0..METHOD.len() as u32).unwrap();
    assert_eq!(range, all);
    // Part of the file gets part of the edits, the gap after it cut to its line breaks.
    let at = METHOD.find("a  >").unwrap() as u32;
    let part = format_range(&tree, &source, at..at + 1).unwrap();
    assert_eq!(
        apply_edits(METHOD, &part),
        "class A {\n  void f(int a) {\n    if (a > 1) {\n      x(  );\n    }   \
         else {\n      // c\n      y(  ); // t\n    }\n  }\n}\n"
    );
}

#[test]
fn an_error_unit_overlapping_the_range_affects_it() {
    let text = "class A {\n  void f() {\n    a(;\n    b(  );\n  }\n}\n";
    let start = text.find("a(;").unwrap() as u32;
    assert_eq!(
        formatted(text, "a("),
        Err(RangeError::Affected {
            error: start..start + 3
        })
    );
    assert_eq!(
        formatted(text, "b(").unwrap(),
        "class A {\n  void f() {\n    a(;\n    b();\n  }\n}\n"
    );
    // A range over both is affected.
    assert!(formatted(text, "a(;\n    b").is_err());
}

#[test]
fn an_error_unit_holding_a_region_marker_affects_every_range() {
    let text = "class A {\n  void f() {\n    a( // @formatter:off\n    ;\n  }\n  int  x;\n}\n";
    assert!(formatted(text, "x").is_err());
}
