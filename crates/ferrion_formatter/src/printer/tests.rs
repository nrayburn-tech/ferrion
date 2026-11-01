use std::sync::Arc;

use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::{SyntaxKind, TokenId};

use super::*;
use crate::document::{DocumentBuilder, IndentKind, Verbatim};

/// A document over the tokens of `text`, which needn't be Java, built by `build` with a function
/// that emits the `n`th token that isn't whitespace.
fn print_with(
    text: &str,
    line_ending: LineEnding,
    bom: bool,
    build: impl FnOnce(&mut DocumentBuilder, &dyn Fn(&mut DocumentBuilder, usize)),
) -> String {
    let tree = parse(Arc::from(text), ParseOptions { release: 27 }).tree;
    let tokens: Vec<TokenId> = tree
        .tokens()
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::Whitespace | SyntaxKind::LineTerminator
            )
        })
        .map(|t| t.id())
        .collect();
    let emit = |b: &mut DocumentBuilder, n: usize| {
        let token = tree.token(tokens[n]);
        let text = token.text();
        if matches!(
            token.kind(),
            SyntaxKind::BlockComment | SyntaxKind::DocComment | SyntaxKind::TextBlock
        ) {
            let span = token.span();
            let mut lines = Lines::new(text);
            let first = lines.next().unwrap();
            b.verbatim(Verbatim {
                start: span.start,
                end: span.end,
                kind: if token.kind() == SyntaxKind::TextBlock {
                    VerbatimKind::Copy { to_end: false }
                } else {
                    VerbatimKind::Comment
                },
                first_width: width(first),
                breaks: lines.next().is_some(),
            });
        } else {
            b.token(token.id(), text.len() as u32, width(text));
        }
    };
    let mut b = DocumentBuilder::new();
    build(&mut b, &emit);
    let document = b.finish();
    let output = print(
        &document,
        &Source {
            tree: &tree,
            line_ending,
            bom,
        },
    );
    String::from_utf8(output).unwrap()
}

fn print_lf(
    text: &str,
    build: impl FnOnce(&mut DocumentBuilder, &dyn Fn(&mut DocumentBuilder, usize)),
) -> String {
    print_with(text, LineEnding::Lf, false, build)
}

/// Tokens `0..n` of a document, separated by `Line`s, in one group.
fn line_group(b: &mut DocumentBuilder, emit: &dyn Fn(&mut DocumentBuilder, usize), n: usize) {
    b.start_group();
    for i in 0..n {
        if i > 0 {
            b.line();
        }
        emit(b, i);
    }
    b.end();
}

#[test]
fn a_group_that_fits_is_flat() {
    let output = print_lf("a b c", |b, emit| line_group(b, emit, 3));
    assert_eq!(output, "a b c\n");
}

#[test]
fn a_group_that_doesnt_fit_breaks_every_line() {
    let word = "x".repeat(40);
    let text = format!("{word} {word} {word}");
    let output = print_lf(&text, |b, emit| line_group(b, emit, 3));
    assert_eq!(output, format!("{word}\n{word}\n{word}\n"));
}

#[test]
fn exactly_the_limit_fits() {
    let a = "a".repeat(49);
    let b = "b".repeat(50);
    let text = format!("{a} {b}");
    let output = print_lf(&text, |doc, emit| line_group(doc, emit, 2));
    assert_eq!(output, format!("{a} {b}\n"));
    let b = "b".repeat(51);
    let text = format!("{a} {b}");
    let output = print_lf(&text, |doc, emit| line_group(doc, emit, 2));
    assert_eq!(output, format!("{a}\n{b}\n"));
}

#[test]
fn outer_groups_break_first() {
    // `first` then a group of the other three, which fits on its own line.
    let words: Vec<String> = (0..4).map(|i| format!("w{i}{}", "x".repeat(28))).collect();
    let text = words.join(" ");
    let output = print_lf(&text, |b, emit| {
        b.start_group();
        emit(b, 0);
        b.line();
        b.start_indent(IndentKind::Continuation);
        b.start_group();
        for i in 1..4 {
            if i > 1 {
                b.line();
            }
            emit(b, i);
        }
        b.end();
        b.end();
        b.end();
    });
    assert_eq!(
        output,
        format!("{}\n    {} {} {}\n", words[0], words[1], words[2], words[3])
    );
}

#[test]
fn the_rest_of_the_line_counts() {
    // The group fits alone, but not with the text after it, up to the next line element.
    let a = "a".repeat(45);
    let tail = format!("\"{}\"", "t".repeat(18));
    let text = format!("{a} {a} {tail}");
    let output = print_lf(&text, |b, emit| {
        line_group(b, emit, 2);
        emit(b, 2);
    });
    assert_eq!(output, format!("{a}\n{a}{tail}\n"));
}

#[test]
fn a_hard_line_breaks_the_groups_around_it() {
    let output = print_lf("a b c", |b, emit| {
        b.start_group();
        emit(b, 0);
        b.line();
        emit(b, 1);
        b.hard_line();
        emit(b, 2);
        b.end();
    });
    assert_eq!(output, "a\nb\nc\n");
}

#[test]
fn line_breaks_merge_and_blank_lines_win() {
    let output = print_lf("a b c d", |b, emit| {
        emit(b, 0);
        b.hard_line();
        b.hard_line();
        emit(b, 1);
        b.hard_line();
        b.blank_line();
        b.hard_line();
        emit(b, 2);
        b.blank_line();
        b.blank_line();
        emit(b, 3);
    });
    assert_eq!(output, "a\nb\n\nc\n\nd\n");
}

#[test]
fn no_line_ends_with_whitespace_and_no_line_starts_with_a_space() {
    let output = print_lf("a b", |b, emit| {
        b.start_indent(IndentKind::Block);
        emit(b, 0);
        b.space();
        b.hard_line();
        b.space();
        emit(b, 1);
        b.end();
    });
    assert_eq!(output, "a\n  b\n");
}

#[test]
fn nothing_is_written_before_the_first_text() {
    let output = print_lf("a", |b, emit| {
        b.blank_line();
        b.space();
        emit(b, 0);
    });
    assert_eq!(output, "a\n");
    assert_eq!(print_lf("", |b, _| b.hard_line()), "");
}

#[test]
fn indentation_is_taken_where_the_text_starts() {
    let output = print_lf("{ x }", |b, emit| {
        emit(b, 0);
        b.start_indent(IndentKind::Block);
        b.hard_line();
        emit(b, 1);
        b.hard_line();
        b.end();
        emit(b, 2);
    });
    assert_eq!(output, "{\n  x\n}\n");
}

#[test]
fn a_hug_keeps_a_block_on_the_calls_line() {
    // f ( { s } ): the arguments group fits up to the `{`, so it stays flat, and the block's
    // lines break inside the hug.
    let output = print_lf("f ( { s } )", |b, emit| {
        emit(b, 0);
        emit(b, 1);
        let group = b.start_group();
        b.start_indent_if_broken(group);
        b.soft_line();
        b.start_hug();
        emit(b, 2);
        b.start_indent(IndentKind::Block);
        b.hard_line();
        emit(b, 3);
        b.end();
        b.hard_line();
        emit(b, 4);
        b.end();
        b.end();
        b.end();
        emit(b, 5);
    });
    assert_eq!(output, "f({\n  s\n})\n");
}

#[test]
fn an_unhugged_block_breaks_its_group() {
    let output = print_lf("f ( { s } )", |b, emit| {
        emit(b, 0);
        emit(b, 1);
        let group = b.start_group();
        b.start_indent_if_broken(group);
        b.soft_line();
        emit(b, 2);
        b.start_indent(IndentKind::Block);
        b.hard_line();
        emit(b, 3);
        b.end();
        b.hard_line();
        emit(b, 4);
        b.end();
        b.end();
        emit(b, 5);
    });
    assert_eq!(output, "f(\n    {\n      s\n    })\n");
}

#[test]
fn a_hug_falls_back_when_its_first_line_doesnt_fit() {
    let name = "n".repeat(99);
    let text = format!("{name} ( {{ s }} )");
    let output = print_lf(&text, |b, emit| {
        emit(b, 0);
        emit(b, 1);
        let group = b.start_group();
        b.start_indent_if_broken(group);
        b.soft_line();
        b.start_hug();
        emit(b, 2);
        b.start_indent(IndentKind::Block);
        b.hard_line();
        emit(b, 3);
        b.end();
        b.hard_line();
        emit(b, 4);
        b.end();
        b.end();
        b.end();
        emit(b, 5);
    });
    assert_eq!(output, format!("{name}(\n    {{\n      s\n    }})\n"));
}

#[test]
fn a_fill_breaks_only_before_an_item_that_doesnt_fit() {
    let items: Vec<String> = (0..12)
        .map(|i| format!("i{i:02}{}", "x".repeat(10)))
        .collect();
    let text = items.join(" ");
    let output = print_lf(&text, |b, emit| {
        b.start_fill();
        for i in 0..items.len() {
            if i > 0 {
                b.fill_break(false);
            }
            emit(b, i);
        }
        b.end();
    });
    // Seven 13-column items and their spaces make 97 columns.
    let first = items[..7].join(" ");
    let second = items[7..].join(" ");
    assert_eq!(output, format!("{first}\n{second}\n"));
}

#[test]
fn a_starred_comment_is_aligned_one_column_after_its_slash() {
    let output = print_lf("x /**\n        * a\n   *   b  \n         */", |b, emit| {
        b.start_indent(IndentKind::Block);
        emit(b, 0);
        b.hard_line();
        emit(b, 1);
        b.end();
    });
    assert_eq!(output, "x\n  /**\n   * a\n   *   b\n   */\n");
}

#[test]
fn an_unstarred_comment_keeps_its_relative_indentation() {
    let output = print_lf("x\n      /* a\n        b\n\n      c\n   */", |b, emit| {
        b.start_indent(IndentKind::Block);
        emit(b, 0);
        b.hard_line();
        emit(b, 1);
        b.end();
    });
    // The least indented later line, `*/`, moves to the comment's column.
    assert_eq!(output, "x\n  /* a\n       b\n\n     c\n  */\n");
}

#[test]
fn text_blocks_are_copied_with_the_files_line_ending() {
    let text = "s = \"\"\"\r\n    a  \n  b\"\"\" ;";
    let output = print_with(text, LineEnding::CrLf, false, |b, emit| {
        b.start_indent(IndentKind::Block);
        emit(b, 0);
        b.space();
        emit(b, 1);
        b.hard_line();
        emit(b, 2);
        emit(b, 3);
        b.end();
    });
    assert_eq!(output, "s =\r\n  \"\"\"\r\n    a  \r\n  b\"\"\";\r\n");
}

#[test]
fn line_endings_and_the_byte_order_mark_are_the_files() {
    let output = print_with("a b", LineEnding::Cr, true, |b, emit| {
        emit(b, 0);
        b.hard_line();
        emit(b, 1);
    });
    assert_eq!(output, "\u{feff}a\rb\r");
}

#[test]
fn widths_are_utf16_code_units() {
    // A string of 32 emoji is 66 columns, and one of 32 CJK characters 34.
    let emoji = format!("\"{}\"", "😀".repeat(32));
    let cjk = format!("\"{}\"", "中".repeat(32));
    let filler = "f".repeat(34);
    let output = print_lf(&format!("{emoji} {filler}"), |b, emit| {
        line_group(b, emit, 2)
    });
    assert_eq!(output, format!("{emoji}\n{filler}\n"));
    let output = print_lf(&format!("{cjk} {filler}"), |b, emit| line_group(b, emit, 2));
    assert_eq!(output, format!("{cjk} {filler}\n"));
}

#[test]
fn tokens_that_would_run_together_get_a_space() {
    // Not adjacent in the source, so printing them side by side would make other tokens.
    let output = print_lf("- - x int y 1 . a / /* c */", |b, emit| {
        for i in 0..10 {
            emit(b, i);
        }
    });
    assert_eq!(output, "- -x int y 1 .a/ /* c */\n");
    // Adjacent in the source, they stay adjacent.
    let output = print_lf(">>=", |b, emit| {
        emit(b, 0);
        emit(b, 1);
    });
    assert_eq!(output, ">>=\n");
}

#[test]
fn a_hanging_indent_keeps_a_break_from_before_it_out() {
    // `x =`, a break, then `a` at +4 and `+ b` at +8: the hanging indent starts after the break.
    let output = print_lf("x = a + b", |b, emit| {
        emit(b, 0);
        b.space();
        emit(b, 1);
        b.start_indent(IndentKind::Continuation);
        b.hard_line();
        b.start_hanging(IndentKind::Continuation);
        emit(b, 2);
        b.hard_line();
        emit(b, 3);
        b.space();
        emit(b, 4);
        b.end();
        b.end();
    });
    assert_eq!(output, "x =\n    a\n        + b\n");
}

/// `ffffffff ( x -> body )`, with the arguments' group given the alternative of breaking after `->`.
fn hugged_lambda(b: &mut DocumentBuilder, emit: &dyn Fn(&mut DocumentBuilder, usize)) {
    emit(b, 0);
    emit(b, 1);
    let group = b.start_group_with_alternative();
    b.start_indent_if_broken(group);
    b.soft_line();
    b.alternative_tail(group);
    b.start_hug_in_group();
    b.start_group();
    emit(b, 2);
    b.space();
    emit(b, 3);
    b.alternative_head(group);
    b.start_indent(IndentKind::Continuation);
    b.line();
    emit(b, 4);
    b.end();
    b.end();
    b.end();
    emit(b, 5);
    b.end();
    b.end();
}

#[test]
fn a_hugged_lambda_breaks_after_its_arrow_when_it_doesnt_fit_on_a_line_of_its_own() {
    let body = "b".repeat(95);
    let output = print_lf(&format!("ffffffff ( x -> {body} )"), hugged_lambda);
    assert_eq!(output, format!("ffffffff(x ->\n    {body})\n"));
}

#[test]
fn a_hugged_lambda_that_fits_on_a_line_of_its_own_goes_there() {
    let body = "b".repeat(88);
    let output = print_lf(&format!("ffffffff ( x -> {body} )"), hugged_lambda);
    assert_eq!(output, format!("ffffffff(\n    x -> {body})\n"));
}
