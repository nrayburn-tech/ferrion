//! The layout and the fill on single comments. The rules' cases against GJF are the `javadoc/`
//! snapshot suite; these pin the pieces: the fill's breaking, the shape, the fallback, and
//! idempotence.

use super::javadoc::{layout, render};
use super::{Columns, Glue, Layout, Paragraph, Word, WordText, fill};

/// `comment` reflowed with its `/**` at `column`, its lines joined with LF, or `None` if it falls
/// back.
fn reflow(comment: &str, column: u32) -> Option<String> {
    let layout = layout(comment, 0, comment.len() as u32)?;
    Some(render(&layout, comment, column).join("\n"))
}

fn reflowed(comment: &str) -> String {
    reflow(comment, 2).unwrap_or_else(|| panic!("{comment:?} fell back"))
}

#[test]
fn joins_and_refills_lines() {
    assert_eq!(
        reflowed("/**\n * One\n * two   three.\n */"),
        "/** One two three. */"
    );
    let long = format!("/** {} */", ["word"; 30].join(" "));
    assert_eq!(
        reflowed(&long),
        format!(
            "/**\n   * {}\n   * {}\n   */",
            ["word"; 19].join(" "),
            ["word"; 11].join(" ")
        )
    );
}

#[test]
fn a_line_of_exactly_the_limit_fits() {
    // `   * ` is 5 columns, so 95 more end at column 100.
    let word = "a".repeat(93);
    assert_eq!(
        reflowed(&format!("/**\n * {word} b\n */")),
        format!("/**\n   * {word} b\n   */")
    );
    assert_eq!(
        reflowed(&format!("/**\n * {word}a b\n */")),
        format!("/**\n   * {word}a\n   * b\n   */")
    );
}

#[test]
fn no_break_before_at() {
    // The word before `@foo` moves down with it.
    let words = ["word"; 18].join(" ");
    assert_eq!(
        reflowed(&format!("/**\n * {words} aaaaaaaaaaaa @foo\n */")),
        format!("/**\n   * {words}\n   * aaaaaaaaaaaa @foo\n   */")
    );
}

#[test]
fn block_tags_continue_at_four() {
    let words = ["word"; 25].join(" ");
    assert_eq!(
        reflowed(&format!(
            "/**\n * Summary.\n * @param x {words}\n * @return r\n */"
        )),
        format!(
            "/**\n   * Summary.\n   *\n   * @param x {}\n   *     {}\n   * @return r\n   */",
            ["word"; 17].join(" "),
            ["word"; 8].join(" ")
        )
    );
}

#[test]
fn paragraphs_get_p() {
    assert_eq!(
        reflowed("/**\n * One.\n *\n * Two.\n * <P>Three.\n */"),
        "/**\n   * One.\n   *\n   * <p>Two.\n   *\n   * <p>Three.\n   */"
    );
    assert_eq!(reflowed("/** <p>Starts. */"), "/** Starts. */");
}

#[test]
fn empty_comments() {
    assert_eq!(reflowed("/**\n *\n *\n */"), "/** */");
    assert_eq!(reflowed("/** a **/"), "/** a */");
}

#[test]
fn pre_keeps_its_lines() {
    assert_eq!(
        reflowed("/**\n * Text\n * <pre>\n *   x  =  1;\n      no star\n * </pre>\n */"),
        "/**\n   * Text\n   *\n   * <pre>\n   *   x  =  1;\n   *      no star\n   * </pre>\n   */"
    );
}

#[test]
fn falls_back() {
    for comment in [
        "/**\n * <pre>\n * unclosed\n */",
        "/**\n * <code>unclosed\n */",
        "/**\n * <table>\n */",
        "/**\n * {@code unclosed\n */",
        "/**\n * <pre>\n * @param x\n * </pre>\n */",
        "/**\n * {@code\n * @param x}\n */",
        "/**\n * Text<br>@foo\n */",
        "/** \\u0040param x */",
        "/***\n * banner\n */",
    ] {
        assert_eq!(reflow(comment, 2), None, "{comment:?}");
    }
    // An uppercase `@` line in `<pre>` is its text.
    assert!(reflow("/**\n * <pre>\n * @Override\n * </pre>\n */", 2).is_some());
}

#[test]
fn deep_indentation_has_a_word_per_line() {
    assert_eq!(reflow("/** aa bb */", 96).unwrap(), {
        let margin = " ".repeat(96);
        format!("/**\n{margin} * aa\n{margin} * bb\n{margin} */")
    });
}

#[test]
fn filling_filled_text_gives_the_same_lines() {
    for comment in [
        "/**\n * A long paragraph of words that goes on and on and on, well past the end of a single \
         line, and then on and on some more for good measure.\n *\n * <ul><li>an item with words \
         that also goes on and on, well past the end of a single line, and then some.</ul>\n * \
         <code>a\n *   b</code> and @tag and {@code x  y}\n * @param x a parameter <pre>\n * \
         kept\n * </pre>\n */",
    ] {
        for column in [0, 2, 40, 90] {
            let once = reflow(comment, column).unwrap();
            assert_eq!(
                reflow(&once, column).as_deref(),
                Some(once.as_str()),
                "{once}"
            );
        }
    }
}

#[test]
fn fill_breaks_only_where_glue_allows() {
    // Words a (10), b (10) bound to a, c (10), at a limit that holds two.
    let text = "aaaaaaaaaabbbbbbbbbbcccccccccc";
    let word = |i: u32, glue| Word {
        text: WordText::Source(i * 10, i * 10 + 10),
        width: 10,
        glue,
    };
    let layout = Layout {
        paragraphs: vec![Paragraph {
            gap: 0,
            indent: 0,
            hanging: 2,
            words: (0, 3),
        }],
        words: vec![
            word(0, Glue::Space),
            word(1, Glue::Bound),
            word(2, Glue::Space),
        ],
        lines: Vec::new(),
        tags: false,
        prefixes: Vec::new(),
    };
    let lines = fill(
        &layout,
        text,
        Columns {
            text: 0,
            copied: 0,
            limit: 21,
        },
    );
    let texts: Vec<_> = lines.iter().map(|l| (l.indent, l.text.as_str())).collect();
    assert_eq!(texts, [(0, "aaaaaaaaaa bbbbbbbbbb"), (2, "cccccccccc")]);
}
