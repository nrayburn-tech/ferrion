//! The Markdown layout on single comments. The rules' cases against GJF are the `markdown/`
//! snapshot suite; these pin the pieces: which words may start a line, the containers' prefixes,
//! the fallback, and idempotence.

use ferrion_doc_comments::Span;

use super::{layout, render, starts_block};

/// The spans of the `///` lines of `comment`, one per line.
fn spans(comment: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut at = 0;
    for line in comment.split('\n') {
        let start = at + (line.len() - line.trim_start().len());
        spans.push(Span::new(start as u32, (at + line.len()) as u32));
        at += line.len() + 1;
    }
    spans
}

/// `comment`, `///` lines joined with LF, reflowed with its first `///` at `column`, or `None` if
/// it falls back.
fn reflow(comment: &str, column: u32) -> Option<String> {
    let layout = layout(comment, &spans(comment))?;
    Some(render(&layout, comment, column).join("\n"))
}

/// `comment` reflowed at column 2, which must reflow, and must reflow to itself again.
fn reflowed(comment: &str) -> String {
    let once = reflow(comment, 2).unwrap_or_else(|| panic!("{comment:?} fell back"));
    let again = reflow(&once, 2).unwrap_or_else(|| panic!("{once:?} fell back"));
    assert_eq!(again, once, "reflowing again changes it");
    once
}

#[test]
fn joins_and_refills_lines() {
    assert_eq!(
        reflowed("/// One\n  /// two   three."),
        "/// One two three."
    );
    assert_eq!(reflowed("///x\n  ///   y"), "/// x y");
    let long = format!("/// {}", ["word"; 30].join(" "));
    assert_eq!(
        reflowed(&long),
        format!(
            "/// {}\n  /// {}",
            ["word"; 19].join(" "),
            ["word"; 11].join(" ")
        )
    );
}

#[test]
fn leading_trailing_and_doubled_blank_lines_go() {
    assert_eq!(
        reflowed("///\n  /// a\n  ///\n  ///\n  /// b\n  ///"),
        "/// a\n  ///\n  /// b"
    );
    assert_eq!(reflowed("///\n  ///"), "///");
}

#[test]
fn words_that_would_start_a_block_stay_on_their_line() {
    for word in [
        "-", "+", "*", "1.", "1)", "#", ">", "```", "~~~", "---", "===", "|", "<pre>",
    ] {
        assert!(starts_block(word), "{word}");
    }
    for word in [
        "10.", "2)", "#hash", "word", "-x", "<b>", "`code`", "{@code",
    ] {
        assert!(!starts_block(word), "{word}");
    }
    let words = ["word"; 18].join(" ");
    assert_eq!(
        reflowed(&format!("/// {words} abc - dash tail")),
        format!("/// {words}\n  /// abc - dash tail")
    );
}

#[test]
fn no_break_before_at() {
    let words = ["word"; 18].join(" ");
    assert_eq!(
        reflowed(&format!("/// {words} abc @tag tail")),
        format!("/// {words}\n  /// abc @tag tail")
    );
}

#[test]
fn code_spans_are_kept_whole() {
    assert_eq!(reflowed("/// a `x   y` b"), "/// a `x   y` b");
    assert_eq!(reflowed("/// a `x\n  /// y` b"), "/// a `x y` b");
}

#[test]
fn backslash_hard_breaks_keep_their_line_break() {
    assert_eq!(reflowed("/// a   b\\\n  /// c   d"), "/// a b\\\n  /// c d");
}

#[test]
fn two_space_hard_breaks_are_written_with_a_backslash() {
    assert_eq!(reflowed("/// a   b  \n  /// c   d"), "/// a b\\\n  /// c d");
}

#[test]
fn hyphen_ended_words_join_the_next_line() {
    assert_eq!(reflowed("/// a well-\n  /// known"), "/// a well-known");
}

#[test]
fn lists_continue_past_their_markers() {
    let words = ["word"; 20].join(" ");
    assert_eq!(
        reflowed(&format!(
            "/// - {words}\n  ///   - nested\n  /// 10. ten {words}"
        )),
        format!(
            "/// - {}\n  ///   {}\n  ///   - nested\n  /// 10. {}\n  ///     {}",
            ["word"; 18].join(" "),
            ["word"; 2].join(" "),
            ["ten"]
                .into_iter()
                .chain(["word"; 17])
                .collect::<Vec<_>>()
                .join(" "),
            ["word"; 3].join(" ")
        )
    );
    // One space after the marker.
    assert_eq!(reflowed("/// -   a\n  ///     b"), "/// - a b");
}

#[test]
fn block_quotes_keep_their_markers() {
    let words = ["word"; 20].join(" ");
    assert_eq!(
        reflowed(&format!(
            "/// > {words}\n  /// > more\n  /// >\n  /// > two"
        )),
        format!(
            "/// > {}\n  /// > {} more\n  /// >\n  /// > two",
            ["word"; 18].join(" "),
            ["word"; 2].join(" ")
        )
    );
    assert_eq!(reflowed("/// > - a"), "/// >\n  /// > - a");
}

#[test]
fn blocks_get_blank_lines_and_are_copied() {
    assert_eq!(
        reflowed("/// a\n  /// ```java\n  /// int   x;\n  ///   y();\n  /// ```\n  /// b"),
        "/// a\n  ///\n  /// ```java\n  /// int   x;\n  ///   y();\n  /// ```\n  ///\n  /// b"
    );
    assert_eq!(
        reflowed("/// #   Head   one  ##\n  /// text"),
        "/// # Head one ##\n  ///\n  /// text"
    );
    // A list item's fence stays on its marker's line.
    assert_eq!(
        reflowed("/// - ```\n  ///   code\n  ///   ```"),
        "/// - ```\n  ///   code\n  ///   ```"
    );
}

#[test]
fn unclosed_fences_are_closed() {
    assert_eq!(
        reflowed("/// a\n  /// ```\n  /// code\n  ///"),
        "/// a\n  ///\n  /// ```\n  /// code\n  /// ```"
    );
}

#[test]
fn block_tags() {
    let words = ["word"; 20].join(" ");
    assert_eq!(
        reflowed(&format!(
            "/// Text.\n  /// @param x {words}\n  ///\n  ///   two\n  /// @return   r"
        )),
        format!(
            "/// Text.\n  ///\n  /// @param x {}\n  ///   {}\n  ///\n  ///   two\n  /// @return r",
            ["word"; 17].join(" "),
            ["word"; 3].join(" ")
        )
    );
}

#[test]
fn some_comments_fall_back() {
    for comment in [
        "/// Setext\n  /// ---",
        "/// a\n  ///\n  ///     indented code",
        "/// a\n  ///\n  /// ***",
        "/// [a]: http://x",
        "/// {@code unclosed",
        "/// {@snippet :\n  /// x\n  /// }",
        "//// four",
    ] {
        assert_eq!(reflow(comment, 2), None, "{comment:?}");
    }
}
