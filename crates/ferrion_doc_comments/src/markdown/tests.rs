//! Where a test says what javadoc does, it was observed by running javadoc 27 on a comment of the
//! same shape.

use super::*;
use crate::invariants::check_markdown;
use crate::model::{BlockTagKind, InlineTagContent, TagArgument};
use ferrion_lexer::lex;

/// Parses the Markdown comments in `text`, which must be one run of `///` lines, and checks the
/// parser's invariants.
fn parse(text: &str) -> MarkdownComment {
    let runs = markdown_comment_runs(&lex(text).tokens);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let comment = parse_markdown(text, &runs[0]);
    if let Err(message) = check_markdown(text, &comment) {
        panic!("{message}\n{comment:#?}");
    }
    comment
}

/// `///` before each line of `lines`.
fn comment(lines: &str) -> String {
    lines
        .split('\n')
        .map(|line| format!("///{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn show(nodes: &[Inline]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Inline::Text(text) => text.text.clone(),
            Inline::Code(code) => format!("`{}`", code.text),
            Inline::Emphasis { content, .. } => format!("*{}*", show(content)),
            Inline::Strong { content, .. } => format!("**{}**", show(content)),
            Inline::Link {
                target, content, ..
            } => match target {
                LinkTarget::Url(url) => format!("[{}]({url})", show(content)),
                LinkTarget::Element(reference) => {
                    format!("[{}][={}]", show(content), reference.text)
                }
            },
            Inline::Image {
                destination,
                content,
                ..
            } => format!("![{}]({destination})", show(content)),
            Inline::Html(html) => html.text.clone(),
            Inline::SoftBreak(_) => "⏎".to_owned(),
            Inline::HardBreak(_) => "⏎!".to_owned(),
            Inline::Tag(tag) => format!("{{@{}}}", tag.name.text),
        })
        .collect()
}

fn show_blocks(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph { content, .. } => format!("p: {}", show(content)),
            Block::Heading {
                level,
                setext,
                content,
                ..
            } => format!(
                "h{level}{}: {}",
                if *setext { "=" } else { "#" },
                show(content)
            ),
            Block::BlockQuote { blocks, .. } => format!("quote: {:?}", show_blocks(blocks)),
            Block::List {
                start,
                tight,
                items,
                ..
            } => {
                let items: Vec<_> = items.iter().map(|i| show_blocks(&i.blocks)).collect();
                format!("list {start:?} tight={tight}: {items:?}")
            }
            Block::CodeBlock { info, text, .. } => format!("code {info:?}: {text:?}"),
            Block::Html { content, .. } => format!("html: {}", show(content)),
            Block::ThematicBreak { .. } => "---".to_owned(),
            Block::Table { head, rows, .. } => {
                let row = |row: &TableRow| {
                    row.cells
                        .iter()
                        .map(|c| show(&c.content))
                        .collect::<Vec<_>>()
                        .join("|")
                };
                let rows: Vec<_> = rows.iter().map(row).collect();
                format!("table {} / {}", row(head), rows.join(" / "))
            }
            Block::LinkDefinition {
                label, destination, ..
            } => format!("def [{label}]: {destination}"),
        })
        .collect()
}

fn source(text: &str, span: Span) -> &str {
    &text[span.range()]
}

fn kinds(comment: &MarkdownComment) -> Vec<DiagnosticKind> {
    comment.diagnostics.iter().map(|d| d.kind).collect()
}

#[test]
fn runs_of_lines() {
    let text =
        "/// a\n///b\n  /// c\n\n/// d\n// x\n/// e\nint x; /// f\n/// g\n//// h\n/** */ /// i";
    let runs = markdown_comment_runs(&lex(text).tokens);
    let runs: Vec<Vec<&str>> = runs
        .iter()
        .map(|run| run.iter().map(|&span| source(text, span)).collect())
        .collect();
    assert_eq!(
        runs,
        [
            vec!["/// a", "///b", "/// c"],
            vec!["/// d"],
            vec!["/// e"],
            vec!["/// f", "/// g", "//// h"],
            vec!["/// i"],
        ]
    );
    // A line with spaces only ends a run too.
    let runs = markdown_comment_runs(&lex("/// a\n  \n/// b").tokens);
    assert_eq!(runs.len(), 2);
}

#[test]
fn indentation() {
    // javadoc: the least indentation of the non-blank lines goes, so `///   Indented` keeps one
    // space when the other lines have two.
    let text = comment("   Indented\n  Less\n\n      code\n  ////");
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        ["p: Indented⏎Less", "code None: \"code\\n\"", "p: ////"]
    );
    let Block::Paragraph { content, span } = &parsed.description[0] else {
        panic!()
    };
    assert_eq!(source(&text, *span), "Indented\n///  Less");
    assert_eq!(source(&text, content[1].span()), "\n///  ");
    assert_eq!(parsed.span, Span::new(0, text.len() as u32));

    // A first line indented by four more than the others is a code block, after blank lines too.
    let parsed = parse(&comment("\n     code\n text"));
    assert_eq!(
        show_blocks(&parsed.description),
        ["code None: \"code\\n\"", "p: text"]
    );
}

#[test]
fn inline_tags_are_read_before_markdown() {
    // javadoc: `{@code a*b*c}` has no emphasis, a code span hides `{@code x}`, and a tag may
    // span lines.
    let text =
        comment(" With {@code a*b*c} and *emph* and `{@code x}`.\n Over {@link List\n label}.");
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        ["p: With {@code} and *emph* and `{@code x}`.⏎Over {@link}."]
    );
    let Block::Paragraph { content, .. } = &parsed.description[0] else {
        panic!()
    };
    let Inline::Tag(tag) = &content[1] else {
        panic!()
    };
    let InlineTagContent::Code(code) = &tag.content else {
        panic!()
    };
    assert_eq!(code.len(), 1);
    assert_eq!(source(&text, tag.span), "{@code a*b*c}");
    let Inline::Tag(link) = &content[9] else {
        panic!("{content:?}")
    };
    assert_eq!(source(&text, link.span), "{@link List\n/// label}");
    let InlineTagContent::Link { reference, label } = &link.content else {
        panic!()
    };
    assert_eq!(reference.as_ref().unwrap().text, "List");
    assert_eq!(label.len(), 1);
}

#[test]
fn tags_that_are_not_tags() {
    // javadoc: `\{@code esc}` is not a tag; `{@code a`b} c`` is a tag followed by `` c` ``,
    // since the tag is found before the code span.
    let text = comment(" \\{@code esc} and {@code a`b} c`.");
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        ["p: {@code esc} and {@code} c`."]
    );
}

#[test]
fn links_to_program_elements() {
    // javadoc links `[List]`, `[List][]`, `[a list][List]`, `[List#add(Object)]`, `[ List ]`,
    // `[java.base/]`, and `[m()]`, reports "reference not found" for `[Nope]`, and leaves
    // `[see above]` and `[1]` as text.
    let text = comment(
        " [List] [List][] [a *list*][List] [List#add(Object)] [ List ] [java.base/] [m()]\n \
         [Nope] [see above] [1] [x](http://e.com) [d] [String#copyValueOf(char\\[\\])]\n\n \
         [d]: http://example.com",
    );
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        [
            "p: [List][=List] [List][=List] [a *list*][=List] [List#add(Object)][=List#add(Object)] \
             [ List ][=List] [java.base/][=java.base/] [m()][=m()]⏎[Nope][=Nope] [see above] [1] \
             [x](http://e.com) [d](http://example.com) [String#copyValueOf(char[])][=String#copyValueOf(char\\[\\])]",
            "def [d]: http://example.com"
        ]
    );
    let Block::Paragraph { content, .. } = &parsed.description[0] else {
        panic!()
    };
    let kinds: Vec<_> = content
        .iter()
        .filter_map(|n| match n {
            Inline::Link { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    {
        use LinkKind::*;
        assert_eq!(
            kinds,
            [
                Shortcut, Collapsed, Reference, Shortcut, Shortcut, Shortcut, Shortcut, Shortcut,
                Inline, Shortcut, Shortcut
            ]
        );
    }
    let Inline::Link {
        target: LinkTarget::Element(reference),
        ..
    } = &content[4]
    else {
        panic!()
    };
    assert_eq!(source(&text, reference.span), "List");
    let parameters = content
        .iter()
        .filter_map(|n| match n {
            Inline::Link {
                target: LinkTarget::Element(r),
                ..
            } => r.member.as_ref()?.parameters.as_ref(),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(parameters.list[0].ty.dimensions, 1);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
}

#[test]
fn block_tags() {
    let text = comment(
        " Main.\n\n ```\n @param notatag in fence {@code nottag}\n ```\n\n     @param indented code\n\n \
         \\@escaped at start\n\n @param <T> the *t* type\n   continued\n @see List\n @see \"text\"\n @return",
    );
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        [
            "p: Main.",
            "code Some(\"\"): \"@param notatag in fence {@code nottag}\\n\"",
            "code None: \"@param indented code\\n\"",
            "p: @escaped at start"
        ]
    );
    let kinds: Vec<_> = parsed.tags.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        [
            BlockTagKind::Param,
            BlockTagKind::See,
            BlockTagKind::See,
            BlockTagKind::Return
        ]
    );
    let param = &parsed.tags[0];
    let Some(TagArgument::Param(name)) = &param.argument else {
        panic!()
    };
    assert!(name.type_parameter);
    assert_eq!(show_blocks(&param.body), ["p: the *t* type⏎continued"]);
    assert_eq!(
        source(&text, param.span),
        "@param <T> the *t* type\n///   continued"
    );
    assert!(
        matches!(&parsed.tags[1].argument, Some(TagArgument::Reference(r)) if r.text == "List")
    );
    assert!(matches!(&parsed.tags[2].argument, Some(TagArgument::Quoted(q)) if q.text == "text"));
    assert!(parsed.tags[3].body.is_empty());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
}

#[test]
fn html_blocks_do_not_hide_block_tags() {
    // javadoc ends the description at an `@param` line inside an HTML block, but reads inline
    // tags in the block.
    let text =
        comment(" <div>\n {@code in html} and [List] and\n @param <T> in html block\n </div>");
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        ["html: <div>{@code} and [List] and"]
    );
    assert_eq!(parsed.tags.len(), 1);
    assert_eq!(
        show_blocks(&parsed.tags[0].body),
        ["p: in html block", "html: </div>"]
    );
}

#[test]
fn unclosed_inline_tags_run_to_the_end() {
    // javadoc: "unterminated inline tag", and the tag takes in the blank line and `@return`.
    let text = comment(
        " Setext heading\n ==============\n {@code a\n @param x}\n Unclosed {@code tag\n\n @return nothing",
    );
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        ["h1=: Setext heading", "p: {@code}⏎Unclosed {@code}"]
    );
    assert!(parsed.tags.is_empty());
    assert_eq!(kinds(&parsed), [DiagnosticKind::UnclosedInlineTag]);
}

#[test]
fn structure() {
    let text = comment(
        " # Title\n\n > quoted *text*\n\n 1. one\n 2. two\n    - nested\n\n - loose\n\n - list\n\n \
         | a | b |\n |---|:-:|\n | 1 | {@code |} |\n\n ***\n\n Hard  \n break\\\n here ![img](i.png) <b>x</b>",
    );
    let parsed = parse(&text);
    assert_eq!(
        show_blocks(&parsed.description),
        [
            "h1#: Title",
            "quote: [\"p: quoted *text*\"]",
            "list Some(1) tight=true: [[\"p: one\"], [\"p: two\", \"list None tight=true: [[\\\"p: nested\\\"]]\"]]",
            "list None tight=false: [[\"p: loose\"], [\"p: list\"]]",
            "table a|b / 1|{@code}",
            "---",
            "p: Hard⏎!break⏎!here ![img](i.png) <b>x</b>"
        ]
    );
    let Block::Table { alignments, .. } = &parsed.description[4] else {
        panic!()
    };
    assert_eq!(alignments, &[Alignment::None, Alignment::Center]);
}

#[test]
fn unicode_escapes() {
    // An escaped `///` is still the comment's prefix, and escapes in the content are translated.
    let text = "\\u002f// \\u0041 {@code \\u007d\n///   b";
    let parsed = parse(text);
    assert_eq!(show_blocks(&parsed.description), ["p: A {@code}⏎b"]);
    let Block::Paragraph { content, .. } = &parsed.description[0] else {
        panic!()
    };
    assert_eq!(source(text, content[0].span()), "\\u0041 ");
    assert_eq!(source(text, content[1].span()), "{@code \\u007d");
}

#[test]
fn nesting_is_bounded() {
    for line in [
        " ".to_owned() + &">".repeat(5_000) + " deep",
        " ".to_owned() + &"- ".repeat(2_000) + "deep",
        " ".to_owned() + &"*a ".repeat(5_000) + &"b* ".repeat(5_000),
        " ".to_owned() + &"[".repeat(5_000) + &"](u)".repeat(5_000),
        " ".to_owned() + &"{@summary ".repeat(5_000),
    ] {
        let parsed = parse(&comment(&line));
        let _ = parsed;
    }
    let parsed = parse(&comment(&(" ".to_owned() + &">".repeat(100) + " deep")));
    assert!(kinds(&parsed).contains(&DiagnosticKind::NestingTooDeep));
}

#[test]
fn any_spans_are_read() {
    let text = "/// a\n/// é\n";
    for lines in [
        vec![Span::new(0, 99)],
        vec![Span::new(7, 8)],
        vec![Span::new(6, 12), Span::new(0, 5)],
        vec![Span::new(99, 3)],
        vec![],
    ] {
        let parsed = parse_markdown(text, &lines);
        check_markdown(text, &parsed).unwrap();
    }
}

#[test]
fn long_inputs_take_linear_time() {
    let n = 50_000;
    for line in [
        "{@a ".repeat(n / 4),
        "[x] ".repeat(n / 4),
        "`{@a` ".repeat(n / 6),
        "[a][b".repeat(n / 5),
        "<a title=\"{@a}".repeat(n / 14),
    ] {
        parse(&comment(&line));
    }
    parse(&comment(&" {@code x}\n".repeat(n / 11)));
}
