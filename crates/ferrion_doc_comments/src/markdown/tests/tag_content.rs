//! Each case was observed by running javadoc 27 on a `///` comment of the same shape and reading

use super::*;
use crate::model::Inline as Literal;

/// The tags of `blocks`, outside other tags, in order.
fn tags(blocks: &[Block]) -> Vec<&InlineTag<Vec<Block>>> {
    let mut found = Vec::new();
    let mut inlines: Vec<&Inline> = Vec::new();
    let mut pending: Vec<&Block> = blocks.iter().rev().collect();
    while let Some(block) = pending.pop() {
        match block {
            Block::Paragraph { content, .. }
            | Block::Heading { content, .. }
            | Block::Html { content, .. } => inlines.extend(content.iter().rev()),
            Block::BlockQuote { blocks, .. } => pending.extend(blocks.iter().rev()),
            Block::List { items, .. } => {
                pending.extend(items.iter().rev().flat_map(|item| item.blocks.iter().rev()));
            }
            _ => {}
        }
        while let Some(inline) = inlines.pop() {
            match inline {
                Inline::Tag(tag) => found.push(&**tag),
                Inline::Emphasis { content, .. }
                | Inline::Strong { content, .. }
                | Inline::Link { content, .. }
                | Inline::Image { content, .. } => inlines.extend(content.iter().rev()),
                _ => {}
            }
        }
    }
    found
}

/// A tag's descriptive content.
fn described(tag: &InlineTag<Vec<Block>>) -> &[Block] {
    match &tag.content {
        InlineTagContent::Link { label, .. } | InlineTagContent::LinkPlain { label, .. } => label,
        InlineTagContent::Index { description, .. }
        | InlineTagContent::Return(description)
        | InlineTagContent::Summary(description) => description,
        _ => &[],
    }
}

fn literal(nodes: &[Literal]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Literal::Text(text) => text.text.clone(),
            Literal::LineBreak(_) => "⏎".to_owned(),
            other => panic!("not literal text: {other:?}"),
        })
        .collect()
}

/// A tag, with its content: `{@link List => p: a *b*}`. An unclosed tag has no `}`.
fn show_tag(tag: &InlineTag<Vec<Block>>) -> String {
    let blocks = |blocks: &[Block]| show_blocks(blocks).join(" | ");
    let mut parts: Vec<String> = Vec::new();
    match &tag.content {
        InlineTagContent::Code(nodes)
        | InlineTagContent::Literal(nodes)
        | InlineTagContent::Other(nodes) => parts.push(literal(nodes)),
        InlineTagContent::Link { reference, label }
        | InlineTagContent::LinkPlain { reference, label } => {
            parts.extend(reference.as_ref().map(|r| r.text.clone()));
            if !label.is_empty() {
                parts.push(format!("=> {}", blocks(label)));
            }
        }
        InlineTagContent::Index { term, description } => {
            parts.extend(term.as_ref().map(|t| t.text.clone()));
            parts.push(format!("=> {}", blocks(description)));
        }
        InlineTagContent::Return(description) | InlineTagContent::Summary(description) => {
            parts.push(blocks(description));
        }
        InlineTagContent::Snippet(snippet) => {
            parts.push(format!(
                ": {}",
                literal(snippet.body.as_deref().unwrap_or(&[]))
            ));
        }
        InlineTagContent::Value { format, reference } => {
            parts.extend(format.as_ref().map(|f| f.text.clone()));
            parts.extend(reference.as_ref().map(|r| r.text.clone()));
        }
        InlineTagContent::InheritDoc { supertype } => {
            parts.extend(supertype.as_ref().map(|r| r.text.clone()));
        }
        InlineTagContent::SystemProperty { name } => {
            parts.extend(name.as_ref().map(|n| n.text.clone()));
        }
        InlineTagContent::DocRoot => {}
    }
    let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    let space = if parts.is_empty() { "" } else { " " };
    let close = if tag.closed { "}" } else { "" };
    format!("{{@{}{space}{}{close}", tag.name.text, parts.join(" "))
}

/// Parses `lines` as a comment, and checks its description's blocks and its tags.
#[track_caller]
fn check(lines: &str, blocks: &[&str], expected_tags: &[&str]) -> MarkdownComment {
    let parsed = parse(&comment(lines));
    assert_eq!(show_blocks(&parsed.description), blocks, "{lines}");
    let shown: Vec<_> = tags(&parsed.description)
        .into_iter()
        .map(show_tag)
        .collect();
    assert_eq!(shown, expected_tags, "{lines}");
    parsed
}

#[test]
fn descriptive_content_is_markdown() {
    // Link labels are Markdown.
    check(
        " {@link List the *list* and **bold** and `code`} and {@linkplain List a *b*}.",
        &["p: {@link} and {@linkplain}."],
        &[
            "{@link List => p: the *list* and **bold** and `code`}",
            "{@linkplain List => p: a *b*}",
        ],
    );
    // So are summaries and returns.
    check(
        " {@summary The *summary*. Second.} {@return the *value* and `code`}",
        &["p: {@summary} {@return}"],
        &[
            "{@summary p: The *summary*. Second.}",
            "{@return p: the *value* and `code`}",
        ],
    );
    // An index entry's description is Markdown (the search index reads "a *x* b c"), but its term
    // isn't ("t *u*").
    check(
        r#" {@index term a \*x\* `b` **c**} {@index "t *u*" d}."#,
        &["p: {@index} {@index}."],
        &[
            "{@index term => p: a *x* `b` **c**}",
            "{@index t *u* => p: d}",
        ],
    );
    // No label.
    check(
        " {@link List} and {@linkplain List   }.",
        &["p: {@link} and {@linkplain}."],
        &["{@link List}", "{@linkplain List}"],
    );
    // Links to program elements, links, and HTML.
    check(
        " {@summary see [List] and [x](http://e.com) and <b>html</b>.} Rest.",
        &["p: {@summary} Rest."],
        &["{@summary p: see [List][=List] and [x](http://e.com) and <b>html</b>.}"],
    );
    // Hard line breaks.
    check(
        " top  \n break {@link List a  \n b} and {@link List c\\\n d}.",
        &["p: top⏎!break {@link} and {@link}."],
        &["{@link List => p: a⏎!b}", "{@link List => p: c⏎!d}"],
    );
}

#[test]
fn descriptive_content_is_a_document() {
    // Content has blocks of its own.
    check(
        " {@link List a\n # b\n - c\n\n d} e.",
        &["p: {@link} e."],
        &["{@link List => p: a | h1#: b | list None tight=true: [[\"p: c\"]] | p: d}"],
    );
    check(
        " {@link List a\n   - b\n   - c} e.",
        &["p: {@link} e."],
        &["{@link List => p: a | list None tight=true: [[\"p: b\"], [\"p: c\"]]}"],
    );
    // From its first line.
    check(
        " {@link List  # x} {@linkplain List - a} {@link List > q}.",
        &["p: {@link} {@linkplain} {@link}."],
        &[
            "{@link List => h1#: x}",
            "{@linkplain List => list None tight=true: [[\"p: a\"]]}",
            "{@link List => quote: [\"p: q\"]}",
        ],
    );
    // The content starts after whitespace and line breaks, so `code` isn't a code block, but later
    // lines keep their indentation.
    check(
        " {@summary\n     code\n } c.",
        &["p: {@summary} c."],
        &["{@summary p: code}"],
    );
    check(
        " {@link List a\n\n     code\n } c.",
        &["p: {@link} c."],
        &["{@link List => p: a | code None: \"code\\n\"}"],
    );
    // The section's link reference definitions don't apply in it, so `[a]` is a link to a
    // program element (javadoc: "reference not found"), but its own do.
    check(
        " {@link List see [a] and [List]} b.\n\n [a]: http://example.com",
        &["p: {@link} b.", "def [a]: http://example.com"],
        &["{@link List => p: see [a][=a] and [List][=List]}"],
    );
    check(
        " {@link List [a]\n\n [a]: http://x} c.",
        &["p: {@link} c."],
        &["{@link List => p: [a](http://x) | def [a]: http://x}"],
    );
}

#[test]
fn literal_content_is_not_markdown() {
    // `{@code}`, `{@literal}`, and snippets are literal text.
    check(
        " {@code a *b* `c`} and {@literal x *y* `z`}.",
        &["p: {@code} and {@literal}."],
        &["{@code a *b* `c`}", "{@literal x *y* `z`}"],
    );
    check(
        " a {@snippet :\n int *x* = 1; // `y`\n }",
        &["p: a {@snippet}"],
        &["{@snippet : int *x* = 1; // `y`⏎}"],
    );
    // So are unknown tags, and code spans in literal text don't hide its braces.
    check(
        " {@foo the *custom* `tag`} after.",
        &["p: {@foo} after."],
        &["{@foo the *custom* `tag`}"],
    );
    check(" {@foo a `}` b} c.", &["p: {@foo}` b} c."], &["{@foo a `}"]);
    check(
        " {@code a `}` b} c.",
        &["p: {@code}` b} c."],
        &["{@code a `}"],
    );
    // Nor do backslashes.
    check(
        " {@code a\\}b} c}.",
        &["p: {@code}b} c}."],
        &["{@code a\\}"],
    );
    check(
        " {@literal a \\} b} c.",
        &["p: {@literal} b} c."],
        &["{@literal a \\}"],
    );
    // Nor are tags without descriptive content.
    check(
        " {@value #C} and {@value %x #C} and {@inheritDoc} and {@systemProperty a.b}.",
        &["p: {@value} and {@value} and {@inheritDoc} and {@systemProperty}."],
        &[
            "{@value #C}",
            "{@value %x #C}",
            "{@inheritDoc}",
            "{@systemProperty a.b}",
        ],
    );
}

#[test]
fn markdown_does_not_cross_tag_boundaries() {
    // Emphasis forms around a tag, but its `*`s don't pair with the label's.
    check(
        " *a {@link List b* c} d*.",
        &["p: *a {@link} d*."],
        &["{@link List => p: b* c}"],
    );
    // Nor with `*`s after it.
    check(
        " {@link List *a} b*.",
        &["p: {@link} b*."],
        &["{@link List => p: *a}"],
    );
    // Links, autolinks, and HTML don't hide braces.
    check(
        " {@link List a [b}](http://x) c} d.",
        &["p: {@link}](http://x) c} d."],
        &["{@link List => p: a [b}"],
    );
    check(
        " {@link List <http://x.com/}> b} c.",
        &["p: {@link}> b} c."],
        &["{@link List => p: <http://x.com/}"],
    );
    check(
        " {@link List a <!-- } --> b} c.",
        &["p: {@link} --> b} c."],
        &["{@link List => p: a <!--}"],
    );
    check(
        " {@link List a <b title=\"}\">x</b>} c.",
        &["p: {@link}\">x</b>} c."],
        &["{@link List => p: a <b title=\"}"],
    );
    // Braces nest.
    check(
        " {@link List a {b} c} d.",
        &["p: {@link} d."],
        &["{@link List => p: a {b} c}"],
    );
    check(
        " {@link List a {`}`} c} d.",
        &["p: {@link} d."],
        &["{@link List => p: a {`}`} c}"],
    );
}

#[test]
fn code_hides_braces() {
    // A code span in the content hides a `}`, as does one over a line break.
    check(
        " {@link List a `}` b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a `}` b}"],
    );
    check(
        " {@link List a `b} c` d} e.",
        &["p: {@link} e."],
        &["{@link List => p: a `b} c` d}"],
    );
    check(
        " {@link List a `b} c} d` e} f.",
        &["p: {@link} f."],
        &["{@link List => p: a `b} c} d` e}"],
    );
    check(
        " {@link List a ``b`} c`` d} e.",
        &["p: {@link} e."],
        &["{@link List => p: a `b`} c` d}"],
    );
    check(
        " {@link List `a}`} b.",
        &["p: {@link} b."],
        &["{@link List => p: `a}`}"],
    );
    check(
        " {@link List a `b\n } c` d} e.",
        &["p: {@link} e."],
        &["{@link List => p: a `b } c` d}"],
    );
    // Within a block quote or list item too.
    check(
        " {@link List a\n > `b\n > }` c} d.",
        &["p: {@link} d."],
        &["{@link List => p: a | quote: [\"p: `b }` c\"]}"],
    );
    // A backtick without a match is text.
    check(
        " {@link List a `b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a `b}"],
    );
    // A code span doesn't continue past a blank line or into a heading, list, block quote, or
    // fence.
    check(
        " {@link List a `b} c.\n\n Para `text` here.",
        &["p: {@link} c.", "p: Para `text` here."],
        &["{@link List => p: a `b}"],
    );
    check(
        " {@link List a `b} c\n # H ` d} e.",
        &["p: {@link} c", "h1#: H ` d} e."],
        &["{@link List => p: a `b}"],
    );
    check(
        " {@link List a `b} c\n - H ` d} e.",
        &["p: {@link} c", "list None tight=true: [[\"p: H ` d} e.\"]]"],
        &["{@link List => p: a `b}"],
    );
    // Code blocks hide braces.
    check(
        " {@link List a\n ```\n }\n ```\n b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a | code Some(\"\"): \"}\\n\" | p: b}"],
    );
    check(
        " {@link List a\n ~~~\n }\n ~~~\n b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a | code Some(\"\"): \"}\\n\" | p: b}"],
    );
    check(
        " {@link List a\n\n     }\n\n b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a | code None: \"}\\n\" | p: b}"],
    );
    // And tags.
    check(
        " {@link List a `{@code x}` b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a `{@code x}` b}"],
    );
    check(
        " {@summary a `b {@code c} d` e.} f.",
        &["p: {@summary} f."],
        &["{@summary p: a `b {@code c} d` e.}"],
    );
    check(
        " {@link List a\n\n ```\n {@code x}\n ```\n } b.",
        &["p: {@link} b."],
        &["{@link List => p: a | code Some(\"\"): \"{@code x}\\n\"}"],
    );
    // A code span that starts inside a literal tag hides nothing after it.
    check(
        " {@code `} {@link List x `y} z}.",
        &["p: {@code} {@link} z}."],
        &["{@code `}", "{@link List => p: x `y}"],
    );
    // But a code span or code block can make the tag unclosed.
    let parsed = check(
        " {@link List a `b} c.\n More `text` here.",
        &["p: {@link}"],
        &["{@link List => p: a `b} c. More `text` here."],
    );
    assert_eq!(kinds(&parsed), [DiagnosticKind::UnclosedInlineTag]);
    let parsed = check(
        " {@link List a\n\n ```\n }\n b} c.",
        &["p: {@link}"],
        &["{@link List => p: a | code Some(\"\"): \"}\\nb} c.\""],
    );
    assert_eq!(kinds(&parsed), [DiagnosticKind::UnclosedInlineTag]);
}

#[test]
fn backslashes_hide_two_characters() {
    // Before `{@`, one or two backslashes, or a backslash and any character, hide the tag; three
    // don't. (The Markdown renders `\\\` before the tag as `\\`.)
    check(" a \\{@code x} b.", &["p: a {@code x} b."], &[]);
    check(" a \\\\{@code x} b.", &["p: a \\{@code x} b."], &[]);
    check(
        " a \\\\\\{@code x} b.",
        &["p: a \\\\{@code} b."],
        &["{@code x}"],
    );
    check(" a \\\\\\\\{@code x} b.", &["p: a \\\\{@code x} b."], &[]);
    check(" a \\x{@code x} b.", &["p: a \\x{@code x} b."], &[]);
    check(
        " a \\xy{@code x} b.",
        &["p: a \\xy{@code} b."],
        &["{@code x}"],
    );
    check(" x\\ {@code x} b.", &["p: x\\ {@code x} b."], &[]);
    check(" a \\*{@code x}* b.", &["p: a *{@code x}* b."], &[]);
    // The same before a `}` in descriptive content.
    check(
        " {@link List a \\*b\\* \\} c} d.",
        &["p: {@link} d."],
        &["{@link List => p: a *b* } c}"],
    );
    check(
        " {@summary a \\} b.} c.",
        &["p: {@summary} c."],
        &["{@summary p: a } b.}"],
    );
    check(
        " {@return a `}` b \\} c}",
        &["p: {@return}"],
        &["{@return p: a `}` b } c}"],
    );
    check(
        " {@link List a \\\\} b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a \\} b}"],
    );
    check(
        " {@link List a \\\\\\} b} c.",
        &["p: {@link} b} c."],
        &["{@link List => p: a \\\\}"],
    );
    check(
        " {@link List a \\*} b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a *} b}"],
    );
    check(
        " {@link List a \\x} b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a \\x} b}"],
    );
    check(
        " {@link List a \\ } b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a \\ } b}"],
    );
    check(
        " {@link List a \\xy} b} c.",
        &["p: {@link} b} c."],
        &["{@link List => p: a \\xy}"],
    );
    check(
        " {@link List a \\é} b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a \\é} b}"],
    );
    check(
        " {@link List \\}} b} c.",
        &["p: {@link} c."],
        &["{@link List => p: }} b}"],
    );
    // A line break is one of the two.
    check(
        " {@link List a\\\n } b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a⏎!} b}"],
    );
    // Before a `{`, which then doesn't nest, or a nested tag.
    check(
        " {@link List a \\{ b} c}.",
        &["p: {@link} c}."],
        &["{@link List => p: a { b}"],
    );
    check(
        " {@link List a \\\\{ b} c} d.",
        &["p: {@link} c} d."],
        &["{@link List => p: a \\{ b}"],
    );
    check(
        " {@link List a \\{@code x} b} c.",
        &["p: {@link} b} c."],
        &["{@link List => p: a {@code x}"],
    );
    // Before a backtick, which then doesn't begin a code span.
    check(
        " {@link List a \\`} b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a `} b}"],
    );
    check(
        " {@link List a \\`b} c` d} e.",
        &["p: {@link} c` d} e."],
        &["{@link List => p: a `b}"],
    );
    check(
        " {@link List a \\\\`b} c` d} e.",
        &["p: {@link} c` d} e."],
        &["{@link List => p: a \\`b}"],
    );
    // But backslashes in code spans are text.
    check(
        " a `b \\` {@code x} c.",
        &["p: a `b \\` {@code} c."],
        &["{@code x}"],
    );
    check(
        " {@link List a `b \\`} c` d} e.",
        &["p: {@link} c` d} e."],
        &["{@link List => p: a `b \\`}"],
    );
}

#[test]
fn nested_tags() {
    // Tags nest in descriptive content, which is Markdown at every level, and emphasis forms around
    // them.
    let parsed = check(
        " {@summary a {@link List *b* `c`} d.}",
        &["p: {@summary}"],
        &["{@summary p: a {@link} d.}"],
    );
    let summary = tags(&parsed.description)[0];
    let nested: Vec<_> = tags(described(summary)).into_iter().map(show_tag).collect();
    assert_eq!(nested, ["{@link List => p: *b* `c`}"]);

    for (line, outer, inner) in [
        (
            " {@link List a {@code x*y*} *em* b}.",
            "{@link List => p: a {@code} *em* b}",
            "{@code x*y*}",
        ),
        (
            " {@summary a {@link List b `}` c} d.}",
            "{@summary p: a {@link} d.}",
            "{@link List => p: b `}` c}",
        ),
        (
            " {@summary *a {@code *} b*}",
            "{@summary p: *a {@code} b*}",
            "{@code *}",
        ),
        (
            " {@link List a {@link List b} c}.",
            "{@link List => p: a {@link} c}",
            "{@link List => p: b}",
        ),
        (
            " {@link List *a {@code x} b*} c.",
            "{@link List => p: *a {@code} b*}",
            "{@code x}",
        ),
    ] {
        let parsed = parse(&comment(line));
        let outer_tags = tags(&parsed.description);
        assert_eq!(outer_tags.len(), 1, "{line}");
        assert_eq!(show_tag(outer_tags[0]), outer, "{line}");
        let inner_tags: Vec<_> = tags(described(outer_tags[0]))
            .into_iter()
            .map(show_tag)
            .collect();
        assert_eq!(inner_tags, [inner], "{line}");
    }
}

#[test]
fn malformed_tags() {
    // An unclosed tag's content runs to the end of the comment.
    let parsed = check(
        " {@link List *a*",
        &["p: {@link}"],
        &["{@link List => p: *a*"],
    );
    assert_eq!(kinds(&parsed), [DiagnosticKind::UnclosedInlineTag]);
    let parsed = check(
        " {@link List a `b` c",
        &["p: {@link}"],
        &["{@link List => p: a `b` c"],
    );
    assert_eq!(kinds(&parsed), [DiagnosticKind::UnclosedInlineTag]);
    let parsed = check(
        " {@summary a {@link List *b*} c\n\n @return d",
        &["p: {@summary}"],
        &["{@summary p: a {@link} c | p: @return d"],
    );
    assert_eq!(kinds(&parsed), [DiagnosticKind::UnclosedInlineTag]);
    assert!(parsed.tags.is_empty());
    // `{@` without a name is text.
    let parsed = check(
        " {@link List a {@ b} c.",
        &["p: {@link} c."],
        &["{@link List => p: a {@ b}"],
    );
    assert_eq!(kinds(&parsed), [DiagnosticKind::MissingTagName]);
    // Block tags aren't read in a tag.
    check(
        " {@link List a\n @param x} c.",
        &["p: {@link} c."],
        &["{@link List => p: a⏎@param x}"],
    );
    // Nor is a missing reference supplied by the label.
    let parsed = check(
        " {@link} {@link }",
        &["p: {@link} {@link}"],
        &["{@link}", "{@link}"],
    );
    assert_eq!(
        kinds(&parsed),
        [
            DiagnosticKind::MissingArgument,
            DiagnosticKind::MissingArgument
        ]
    );
}

#[test]
fn block_tags_hold_tags_with_markdown_content() {
    let text = comment(" Main.\n @param x the {@link List *list*}\n @return {@summary a `}`}");
    let parsed = parse(&text);
    let shown: Vec<Vec<String>> = parsed
        .tags
        .iter()
        .map(|tag| tags(&tag.body).into_iter().map(show_tag).collect())
        .collect();
    assert_eq!(
        shown,
        [
            vec!["{@link List => p: *list*}"],
            vec!["{@summary p: a `}`}"]
        ]
    );
}

#[test]
fn spans() {
    // A Unicode escape closes the tag, as javac translates it before javadoc reads the comment.
    let text = "/// {@summary a *b*\n///   c \\u007d d";
    let parsed = parse(text);
    let tag = tags(&parsed.description)[0];
    assert_eq!(source(text, tag.span), "{@summary a *b*\n///   c \\u007d");
    let [Block::Paragraph { span, content }] = described(tag) else {
        panic!("{tag:?}")
    };
    assert_eq!(source(text, *span), "a *b*\n///   c");
    let spans: Vec<_> = content
        .iter()
        .map(|node| source(text, node.span()))
        .collect();
    // The line break covers the `///` and the common indentation; Markdown leaves out the rest.
    assert_eq!(spans, ["a ", "*b*", "\n/// ", "c"]);
    let Inline::Emphasis { content, .. } = &content[1] else {
        panic!()
    };
    assert_eq!(source(text, content[0].span()), "b");

    // Escapes inside the content map back to what is written.
    let text = "/// {@link List a \\u002ab\\u002a}";
    let parsed = parse(text);
    let tag = tags(&parsed.description)[0];
    let [Block::Paragraph { content, .. }] = described(tag) else {
        panic!()
    };
    let spans: Vec<_> = content
        .iter()
        .map(|node| source(text, node.span()))
        .collect();
    assert_eq!(spans, ["a ", "\\u002ab\\u002a"]);
}

#[test]
fn depth_is_shared_with_markdown() {
    // Markdown in tags counts toward the nesting limit with the Markdown around them.
    let line = " ".to_owned() + &"> ".repeat(30) + "{@summary > > > > x}";
    let parsed = parse(&comment(&line));
    assert!(kinds(&parsed).contains(&DiagnosticKind::NestingTooDeep));
    // The content is still there.
    let mut blocks = &parsed.description;
    while let [Block::BlockQuote { blocks: inner, .. }] = blocks.as_slice() {
        blocks = inner;
    }
    let shown: Vec<_> = tags(blocks).into_iter().map(show_tag).collect();
    assert_eq!(shown, ["{@summary p: x}"]);
}
