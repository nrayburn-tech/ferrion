use super::*;

const OPTIONS: FormatOptions = FormatOptions { release: 27 };

/// Formats `text` and checks the safety properties of the result, which it returns as text.
fn formatted(text: &str) -> String {
    let source = SourceText::decode(text).unwrap();
    let output = match format(&source, &OPTIONS) {
        Formatted::Unchanged => source.to_bytes(),
        Formatted::Changed(output) => output,
        Formatted::Skipped(errors) => panic!("skipped: {errors:?}"),
    };
    if let Err(violation) = verify(&source, &output, &OPTIONS) {
        panic!(
            "{violation}\n--- output ---\n{}",
            String::from_utf8_lossy(&output)
        );
    }
    let output = String::from_utf8(output).unwrap();
    assert_eq!(
        apply_edits(source.text(), &edits(text)),
        output.trim_start_matches('\u{feff}'),
        "the edits don't give the output"
    );
    output
}

/// The edits for `text`.
fn edits(text: &str) -> Vec<Edit> {
    let source = SourceText::decode(text).unwrap();
    let parsed = parse(
        Arc::from(source.text()),
        ParseOptions {
            release: OPTIONS.release,
        },
    );
    format_edits(&parsed.tree, &source).unwrap()
}

#[test]
fn edits_touch_only_what_changes() {
    let edit = |start: u32, end: u32, text: &str| Edit {
        range: start..end,
        text: text.to_owned(),
    };
    assert_eq!(edits("class A {\n  int x;\n}\n"), []);
    // Gaps lose only what differs, and the final line ending is added at the end.
    assert_eq!(
        edits("class   A {\n  int  x;\n}"),
        [edit(6, 8, ""), edit(18, 19, ""), edit(23, 23, "\n")]
    );
    // A reordered modifier run is one edit, over the whole run.
    assert_eq!(
        edits("class A {\n  final static int x = 1;\n}\n"),
        [edit(12, 24, "static final")]
    );
    // A re-indented comment is one edit over the comment.
    assert_eq!(
        edits("class A {\n    /* a\n     */\n  int x;\n}\n"),
        [edit(12, 14, ""), edit(14, 26, "/* a\n   */")]
    );
    // A line ending in a gap is the file's.
    assert_eq!(
        edits("class A {\r\nint x;\r\n}"),
        [edit(11, 11, "  "), edit(20, 20, "\r\n")]
    );
}

#[test]
fn formatted_files_are_unchanged() {
    let text = "class A {\n  int x;\n}\n";
    let source = SourceText::decode(text).unwrap();
    assert_eq!(format(&source, &OPTIONS), Formatted::Unchanged);
    assert_eq!(formatted("class   A{int x;}"), text);
}

#[test]
fn files_with_errors_are_skipped() {
    let source = SourceText::decode("class A { int }").unwrap();
    let Formatted::Skipped(errors) = format(&source, &OPTIONS) else {
        panic!("not skipped");
    };
    assert!(!errors.is_empty());
    // A version error skips the file too: records are from 16.
    let source = SourceText::decode("record R() {}").unwrap();
    assert!(matches!(
        format(&source, &FormatOptions { release: 11 }),
        Formatted::Skipped(errors) if !errors.is_empty()
    ));
}

#[test]
fn the_first_line_ending_is_used_throughout() {
    assert_eq!(
        formatted("class A {\r\nint x;\nint y;\r}"),
        "class A {\r\n  int x;\r\n  int y;\r\n}\r\n"
    );
    assert_eq!(formatted("class A { int x; }"), "class A {\n  int x;\n}\n");
}

#[test]
fn a_byte_order_mark_is_kept_and_none_is_added() {
    let bom = "\u{feff}";
    let output = formatted(&format!("{bom}class A {{}}"));
    assert_eq!(output, format!("{bom}class A {{}}\n"));
    assert!(!formatted("class A {}").starts_with(bom));
}

#[test]
fn blank_lines_between_members_are_kept_at_most_one() {
    assert_eq!(
        formatted("class A {\n\n\n  int x;\n  int y;\n\n\n\n  int z;\n}\n"),
        "class A {\n\n  int x;\n  int y;\n\n  int z;\n}\n"
    );
}

#[test]
fn comments_keep_their_lines() {
    let text = "// file\n\npackage p; // trailing\n\n/** doc */\nclass A {\n  // leading\n  \
                int x; /* block */\n\n  /* own line */\n  int y;\n\n  void m() {\n    // only\n  \
                }\n}\n// end\n";
    assert_eq!(formatted(text), text);
}

#[test]
fn a_trailing_line_comment_ends_its_line() {
    assert_eq!(
        formatted("class A { int x = // why\n1; }"),
        "class A {\n  int x = // why\n      1;\n}\n"
    );
}

#[test]
fn line_comments_lose_trailing_whitespace() {
    assert_eq!(formatted("class A {} // c  \t\n"), "class A {} // c\n");
}

#[test]
fn block_comments_are_re_indented() {
    assert_eq!(
        formatted("class A {\n        /**\n         * Doc.\n         */\n    void m() {}\n}\n"),
        "class A {\n  /**\n   * Doc.\n   */\n  void m() {}\n}\n"
    );
}

#[test]
fn text_blocks_are_copied() {
    let text = "class A {\n  String s =\n      \"\"\"\n        a\n      b  \n    \"\"\";\n}\n";
    assert_eq!(formatted(text), text);
}

#[test]
fn disable_regions_are_copied() {
    let text = "class A {\n  // @formatter:off\n  int   x =  1;\n     int y;\n  // @formatter:on\n  \
                int   z;\n}\n";
    assert_eq!(
        formatted(text),
        "class A {\n  // @formatter:off\n  int   x =  1;\n     int y;\n  // @formatter:on\n  \
         int z;\n}\n"
    );
}

#[test]
fn an_unmatched_off_runs_to_the_end_of_the_file() {
    let text = "// @formatter:off\nclass   A  {\n}\n\n\n";
    let source = SourceText::decode(text).unwrap();
    assert_eq!(format(&source, &OPTIONS), Formatted::Unchanged);
    assert_eq!(
        formatted("class A {\n/* @formatter:off */ int   x;\n}"),
        "class A {\n  /* @formatter:off */ int   x;\n}"
    );
}

#[test]
fn modifiers_are_reordered_within_runs() {
    assert_eq!(
        formatted("class A { final static public int a; static @Deprecated public final int b; }"),
        "class A {\n  public static final int a;\n  static @Deprecated public final int b;\n}\n"
    );
    assert_eq!(
        formatted("sealed abstract class A permits B {} final non-sealed public class B {}"),
        "abstract sealed class A permits B {}\n\npublic final non-sealed class B {}\n"
    );
}

#[test]
fn a_comment_pins_the_modifiers_around_it() {
    assert_eq!(
        formatted("class A { final /* c */ static public int b; }"),
        "class A {\n  final /* c */ public static int b;\n}\n"
    );
}

#[test]
fn modifiers_with_a_marker_are_not_reordered() {
    let text = "class A {\n  static /* @formatter:on */ public int b;\n}\n";
    assert_eq!(formatted(text), text);
    let text = "class A {\n  // @formatter:off\n  static public int b;\n}\n";
    assert_eq!(formatted(text), text);
}

#[test]
fn shift_operators_stay_together() {
    assert_eq!(
        formatted("class A { int x = a >> b >>> c; { x >>= 1; x >>>= 2; } List<List<String>> l; }"),
        "class A {\n  int x = a >> b >>> c;\n\n  {\n    x >>= 1;\n    x >>>= 2;\n  }\n\n  \
         List<List<String>> l;\n}\n"
    );
}

#[test]
fn a_tree_with_errors_is_skipped() {
    let source = SourceText::decode("class A {").unwrap();
    let tree = parse(Arc::from(source.text()), ParseOptions { release: 27 }).tree;
    assert_eq!(format_tree(&tree, &source), Formatted::Skipped(Vec::new()));
}
