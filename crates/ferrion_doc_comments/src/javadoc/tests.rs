//! Where a test says what javadoc does, it was observed by running javadoc 27 on a comment of the
//! same shape.

use super::*;
use crate::invariants::check_javadoc;
use crate::model::{BlockTagKind, DiagnosticKind, InlineTag, InlineTagContent, TagArgument};

/// Parses `text`, a whole comment, and checks the parser's invariants.
fn parse(text: &str) -> Javadoc {
    let javadoc = parse_javadoc(text, Span::new(0, text.len() as u32));
    if let Err(message) = check_javadoc(text, &javadoc) {
        panic!("{message}\n{javadoc:#?}");
    }
    javadoc
}

/// A compact rendering of inline content.
fn show(nodes: &[Inline]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| match node {
            Inline::Text(text) => text.text.clone(),
            Inline::LineBreak(_) => "⏎".to_owned(),
            Inline::Escape(escape) => format!("esc({})", escape.ch),
            Inline::Entity(entity) => format!("&{};", entity.name),
            Inline::StartElement(element) => {
                let attributes: Vec<String> = element
                    .attributes
                    .iter()
                    .map(|a| match &a.value {
                        Some(value) => format!(" {}={:?}", a.name.text, show(&value.content)),
                        None => format!(" {}", a.name.text),
                    })
                    .collect();
                let close = if element.self_closing { "/>" } else { ">" };
                format!("<{}{}{close}", element.name.text, attributes.concat())
            }
            Inline::EndElement(element) => format!("</{}>", element.name.text),
            Inline::HtmlComment(comment) => format!("<!--{}-->", show(&comment.content).concat()),
            Inline::Tag(tag) => format!("{{@{}}}", tag.name.text),
        })
        .collect()
}

fn kinds(javadoc: &Javadoc) -> Vec<DiagnosticKind> {
    javadoc.diagnostics.iter().map(|d| d.kind).collect()
}

fn source(text: &str, span: Span) -> &str {
    &text[span.range()]
}

fn only_tag(nodes: &[Inline]) -> &InlineTag {
    let tags: Vec<_> = nodes
        .iter()
        .filter_map(|n| match n {
            Inline::Tag(tag) => Some(tag),
            _ => None,
        })
        .collect();
    assert_eq!(tags.len(), 1, "{nodes:#?}");
    tags[0]
}

#[test]
fn leading_whitespace_and_asterisks() {
    let text = "/**\n * a\n *** b\n *\n   c\n *   d\n */";
    let javadoc = parse(text);
    // Whitespace before the asterisks goes, and whitespace after them stays; whitespace not
    // followed by an asterisk stays too (javadoc keeps `   c`'s indentation).
    assert_eq!(
        show(&javadoc.description),
        ["a", "⏎", " b", "⏎", "⏎", "   c", "⏎", "   d"]
    );
    let Inline::LineBreak(line_break) = javadoc.description[1] else {
        panic!()
    };
    assert_eq!(source(text, line_break), "\n ***");
    assert!(javadoc.tags.is_empty());
    assert!(javadoc.diagnostics.is_empty());
}

#[test]
fn asterisks_after_the_opening_delimiter() {
    // javadoc reads `/*** First` as `First`.
    let javadoc = parse("/*** First\n */");
    assert_eq!(show(&javadoc.description), ["First"]);
    let javadoc = parse("/***/");
    assert!(javadoc.description.is_empty());
    let javadoc = parse("/** */");
    assert!(javadoc.description.is_empty());
}

#[test]
fn text_spans_cover_what_is_written() {
    let text = "/** Hello,\r\n *  world. */";
    let javadoc = parse(text);
    assert_eq!(show(&javadoc.description), ["Hello,", "⏎", "  world."]);
    let spans: Vec<_> = javadoc
        .description
        .iter()
        .map(|n| source(text, n.span()))
        .collect();
    assert_eq!(spans, ["Hello,", "\r\n *", "  world."]);
}

#[test]
fn unicode_escapes() {
    // javac translates escapes before javadoc reads the comment: javadoc renders
    // `{@code \u0041\u007d} done` as `<code>A</code>} done`, so the escaped `}` closes the tag.
    let text = "/** {@code \\u0041\\u007d} done */";
    let javadoc = parse(text);
    let tag = only_tag(&javadoc.description);
    assert!(tag.closed);
    let InlineTagContent::Code(code) = &tag.content else {
        panic!()
    };
    assert_eq!(show(code), ["A"]);
    assert_eq!(source(text, code[0].span()), "\\u0041");
    assert_eq!(source(text, tag.span), "{@code \\u0041\\u007d");
    assert_eq!(show(&javadoc.description), ["{@code}", "} done"]);

    // An escaped line terminator is a line break, and an escaped delimiter is a delimiter.
    let text = "\\u002f** a\\u000a * b \\u002a/";
    let javadoc = parse(text);
    assert_eq!(show(&javadoc.description), ["a", "⏎", " b"]);
    assert_eq!(source(text, javadoc.description[1].span()), "\\u000a *");

    // A malformed escape stays as written.
    let javadoc = parse("/** a\\u12 */");
    assert_eq!(show(&javadoc.description), ["a\\u12"]);
}

#[test]
fn block_tags() {
    let text = "/**
 * Main.
 * @param x the x
 *   continued
 * @param <T> the type
 * @return the result
 * @throws IllegalStateException when bad
 * @exception java.io.IOException on I/O
 * @since 1.0
 * @deprecated use {@link #y()}
 * @author A. Person
 * @version 2
 * @serial include
 * @serialData written
 * @serialField name String the name
 * @hidden
 * @provides p.Service an implementation
 * @uses p.Service a use
 * @spec https://example.com/s The Spec
 * @apiNote a custom tag
 * @foo.bar dotted @not a tag
 */";
    let javadoc = parse(text);
    assert_eq!(show(&javadoc.description), ["Main."]);
    let kinds: Vec<_> = javadoc.tags.iter().map(|t| t.kind).collect();
    use BlockTagKind::*;
    assert_eq!(
        kinds,
        [
            Param,
            Param,
            Return,
            Throws,
            Exception,
            Since,
            Deprecated,
            Author,
            Version,
            Serial,
            SerialData,
            SerialField,
            Hidden,
            Provides,
            Uses,
            Spec,
            Other,
            Other
        ]
    );
    let tag = &javadoc.tags[0];
    assert_eq!(source(text, tag.span), "@param x the x\n *   continued");
    assert_eq!(tag.name.text, "param");
    let Some(TagArgument::Param(param)) = &tag.argument else {
        panic!()
    };
    assert_eq!(
        (param.name.text.as_str(), param.type_parameter),
        ("x", false)
    );
    assert_eq!(show(&tag.body), ["the x", "⏎", "   continued"]);

    let Some(TagArgument::Param(param)) = &javadoc.tags[1].argument else {
        panic!()
    };
    assert_eq!(source(text, param.span), "<T>");
    assert_eq!(
        (param.name.text.as_str(), param.type_parameter),
        ("T", true)
    );

    let Some(TagArgument::Reference(reference)) = &javadoc.tags[4].argument else {
        panic!()
    };
    assert_eq!(reference.text, "java.io.IOException");
    assert_eq!(show(&javadoc.tags[4].body), ["on I/O"]);

    let Some(TagArgument::SerialField { name, ty }) = &javadoc.tags[11].argument else {
        panic!()
    };
    assert_eq!(name.text, "name");
    assert_eq!(ty.as_ref().unwrap().name.segments[0].text, "String");
    assert_eq!(show(&javadoc.tags[11].body), ["the name"]);

    assert!(javadoc.tags[12].body.is_empty());
    let Some(TagArgument::Url(url)) = &javadoc.tags[15].argument else {
        panic!()
    };
    assert_eq!(url.text, "https://example.com/s");
    assert_eq!(javadoc.tags[16].name.text, "apiNote");
    // javadoc accepts `.`, `:`, and `-` in custom tag names. An `@` within a line is text.
    assert_eq!(javadoc.tags[17].name.text, "foo.bar");
    assert_eq!(show(&javadoc.tags[17].body), ["dotted @not a tag"]);
    assert!(javadoc.diagnostics.is_empty(), "{:?}", javadoc.diagnostics);
}

#[test]
fn see_forms() {
    let text = "/**
 * @see \"quoted text\"
 * @see <a href=\"https://example.com\">example</a>
 * @see java.util.List#add(int, Object) the label
 * @see #m
 */";
    let javadoc = parse(text);
    let Some(TagArgument::Quoted(quoted)) = &javadoc.tags[0].argument else {
        panic!()
    };
    assert_eq!(quoted.text, "quoted text");
    assert_eq!(source(text, quoted.span), "\"quoted text\"");
    assert!(javadoc.tags[1].argument.is_none());
    assert_eq!(
        show(&javadoc.tags[1].body),
        ["<a href=[\"https://example.com\"]>", "example", "</a>"]
    );
    let Some(TagArgument::Reference(reference)) = &javadoc.tags[2].argument else {
        panic!()
    };
    assert_eq!(reference.member.as_ref().unwrap().name.text, "add");
    assert_eq!(show(&javadoc.tags[2].body), ["the label"]);
    assert!(javadoc.diagnostics.is_empty());
}

#[test]
fn tag_arguments_on_the_next_line() {
    // javadoc takes `x` as the name in `@param` followed by `x` on the next line.
    let javadoc = parse("/**\n * @param\n *   x on next line\n */");
    let Some(TagArgument::Param(param)) = &javadoc.tags[0].argument else {
        panic!()
    };
    assert_eq!(param.name.text, "x");
    assert_eq!(show(&javadoc.tags[0].body), ["on next line"]);
    // But not across a block tag.
    let javadoc = parse("/**\n * @param\n * @return r\n */");
    assert!(javadoc.tags[0].argument.is_none());
    assert_eq!(javadoc.tags.len(), 2);
    assert_eq!(kinds(&javadoc), [DiagnosticKind::MissingArgument]);
}

#[test]
fn malformed_block_tags() {
    // javadoc: "no tag name after '@'" for `@ x` and `@1abc`, "'>' expected" for `@param <T`,
    // "unexpected content" for an unclosed `@see "`, and "syntax error in reference".
    let text = "/**
 * Text.
 * @ space
 * @1abc digit
 * @param <T
 * @see \"unclosed
 * @see Foo#
 * @throws
 */";
    let javadoc = parse(text);
    assert_eq!(show(&javadoc.description), ["Text."]);
    assert_eq!(javadoc.tags.len(), 6);
    assert_eq!(javadoc.tags[0].name.text, "");
    assert_eq!(show(&javadoc.tags[0].body), ["space"]);
    assert_eq!(show(&javadoc.tags[1].body), ["1abc digit"]);
    use DiagnosticKind::*;
    assert_eq!(
        kinds(&javadoc),
        [
            MissingTagName,
            MissingTagName,
            MalformedArgument,
            MalformedArgument,
            MalformedReference,
            MissingArgument
        ]
    );
}

#[test]
fn block_tag_at_the_start() {
    let javadoc = parse("/** @return the value */");
    assert!(javadoc.description.is_empty());
    assert_eq!(javadoc.tags[0].kind, BlockTagKind::Return);
    assert_eq!(show(&javadoc.tags[0].body), ["the value"]);
    // On one line, a later `@` is text: javadoc shows `m2. @param a a` as the description.
    let javadoc = parse("/** m2. @param a a */");
    assert_eq!(show(&javadoc.description), ["m2. @param a a"]);
}

#[test]
fn escapes() {
    // javadoc: `@@foo` and `a@*b` within a line are literal, `*@/` is `*/`, `{@@code x}` is the
    // text `{@code x}`, and `@@` and `@*` start a line with `@` and `*`, after any whitespace.
    let text = "/**
 * Mid @@foo, a@*b, *@/ end, {@@code x}.
 *@* starred
 *   @@at
 * @*b1
     @*d1
 */";
    let javadoc = parse(text);
    assert_eq!(
        show(&javadoc.description),
        [
            "Mid @@foo, a@*b, *",
            "esc(/)",
            " end, {",
            "esc(@)",
            "code x}.",
            "⏎",
            "esc(*)",
            " starred",
            "⏎",
            "   ",
            "esc(@)",
            "at",
            "⏎",
            " ",
            "esc(*)",
            "b1",
            "⏎",
            "     ",
            "esc(*)",
            "d1"
        ]
    );
    assert!(javadoc.tags.is_empty());
}

#[test]
fn escapes_in_inline_tags() {
    // javadoc renders `{@link Object label\n * @@foo and\n * @*bar}` with the label
    // `label @foo and *bar`, and `{@code @@ @* @/}` as `@@ @* @/`: escapes work in inline content,
    // but not in literal text.
    let javadoc =
        parse("/**\n * {@link Object label\n * @@foo and\n * @*bar} {@code @@ @* @/}\n */");
    let tags: Vec<_> = javadoc
        .description
        .iter()
        .filter_map(|n| match n {
            Inline::Tag(tag) => Some(&tag.content),
            _ => None,
        })
        .collect();
    let InlineTagContent::Link { label, .. } = tags[0] else {
        panic!()
    };
    assert_eq!(
        show(label),
        [
            "label", "⏎", " ", "esc(@)", "foo and", "⏎", " ", "esc(*)", "bar"
        ]
    );
    let InlineTagContent::Code(code) = tags[1] else {
        panic!()
    };
    assert_eq!(show(code), ["@@ @* @/"]);
}

#[test]
fn code_and_literal() {
    // javadoc: `{@code   three}` is `  three` (one space goes), `{@code\n * nl}` keeps the line
    // break, and braces balance.
    let text =
        "/** {@code   three} {@literal a<b>} {@code {a {b}}} {@code}\n * {@code\n *    nl} */";
    let javadoc = parse(text);
    let tags: Vec<_> = javadoc
        .description
        .iter()
        .filter_map(|n| match n {
            Inline::Tag(tag) => Some(tag),
            _ => None,
        })
        .collect();
    let contents: Vec<_> = tags
        .iter()
        .map(|t| match &t.content {
            InlineTagContent::Code(nodes) | InlineTagContent::Literal(nodes) => show(nodes),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        contents,
        [
            vec!["  three"],
            vec!["a<b>"],
            vec!["{a {b}}"],
            vec![],
            vec!["⏎", "    nl"]
        ]
    );
    assert!(tags.iter().all(|t| t.closed));
}

#[test]
fn block_tags_inside_inline_tags() {
    // javadoc: an `@` starting a line inside an inline tag doesn't start a block tag.
    let text = "/**\n * {@code\n * @Override\n * void x() {}\n * }\n * @return r\n */";
    let javadoc = parse(text);
    let tag = only_tag(&javadoc.description);
    let InlineTagContent::Code(code) = &tag.content else {
        panic!()
    };
    assert_eq!(
        show(code),
        ["⏎", " @Override", "⏎", " void x() {}", "⏎", " "]
    );
    assert_eq!(javadoc.tags.len(), 1);
}

#[test]
fn unclosed_inline_tags_run_to_the_end() {
    // javadoc: "unterminated inline tag", and the tag takes in the block tags after it, blank
    // lines included.
    let text = "/**\n * Unclosed {@code x y\n * more\n *\n * @param a the a\n */";
    let javadoc = parse(text);
    let tag = only_tag(&javadoc.description);
    assert!(!tag.closed);
    assert!(javadoc.tags.is_empty());
    assert_eq!(
        source(text, tag.span),
        "{@code x y\n * more\n *\n * @param a the a\n "
    );
    assert_eq!(kinds(&javadoc), [DiagnosticKind::UnclosedInlineTag]);
    assert_eq!(source(text, javadoc.diagnostics[0].span), "{@code");

    let javadoc = parse("/** {@link Object\n * next line\n * @see Object\n */");
    let tag = only_tag(&javadoc.description);
    assert!(!tag.closed);
    assert!(javadoc.tags.is_empty());
}

#[test]
fn links() {
    // javadoc: braces in a label balance, `{@link Object label {x} more}` has the label
    // `label {x} more`.
    let text = "/** {@link Object label {x} more} {@linkplain java.util.List#add(int,\n *   Object) the <i>add</i>} {@link} */";
    let javadoc = parse(text);
    let tags: Vec<_> = javadoc
        .description
        .iter()
        .filter_map(|n| match n {
            Inline::Tag(tag) => Some(tag),
            _ => None,
        })
        .collect();
    let InlineTagContent::Link { reference, label } = &tags[0].content else {
        panic!()
    };
    assert_eq!(reference.as_ref().unwrap().text, "Object");
    assert_eq!(show(label), ["label {x} more"]);
    let InlineTagContent::LinkPlain { reference, label } = &tags[1].content else {
        panic!()
    };
    let reference = reference.as_ref().unwrap();
    assert_eq!(reference.text, "java.util.List#add(int,\n   Object)");
    assert_eq!(
        source(text, reference.span),
        "java.util.List#add(int,\n *   Object)"
    );
    let parameters = &reference
        .member
        .as_ref()
        .unwrap()
        .parameters
        .as_ref()
        .unwrap()
        .list;
    assert_eq!(parameters.len(), 2);
    assert_eq!(show(label), ["the ", "<i>", "add", "</i>"]);
    let InlineTagContent::Link {
        reference: None, ..
    } = &tags[2].content
    else {
        panic!()
    };
    assert_eq!(kinds(&javadoc), [DiagnosticKind::MissingArgument]);
}

#[test]
fn other_inline_tags() {
    let text = "/** {@docRoot} {@docRoot x} {@inheritDoc} {@inheritDoc java.util.List} \
        {@value} {@value %d C#X} {@value \"%x\" #X} {@index \"a phrase\" desc} {@index word} \
        {@summary A {@link X} summary.} {@return the {@code r}} {@systemProperty a.b} \
        {@systemProperty a.b-c} {@custom a {b} c} */";
    let javadoc = parse(text);
    let contents: Vec<_> = javadoc
        .description
        .iter()
        .filter_map(|n| match n {
            Inline::Tag(tag) => Some(&tag.content),
            _ => None,
        })
        .collect();
    assert!(matches!(contents[0], InlineTagContent::DocRoot));
    assert!(matches!(
        contents[2],
        InlineTagContent::InheritDoc { supertype: None }
    ));
    let InlineTagContent::InheritDoc { supertype: Some(s) } = contents[3] else {
        panic!()
    };
    assert_eq!(s.text, "java.util.List");
    assert!(matches!(
        contents[4],
        InlineTagContent::Value {
            format: None,
            reference: None
        }
    ));
    let InlineTagContent::Value {
        format: Some(format),
        reference: Some(reference),
    } = contents[5]
    else {
        panic!()
    };
    assert_eq!(
        (format.text.as_str(), reference.text.as_str()),
        ("%d", "C#X")
    );
    let InlineTagContent::Value {
        format: Some(format),
        ..
    } = contents[6]
    else {
        panic!()
    };
    assert_eq!(format.text, "%x");
    let InlineTagContent::Index {
        term: Some(term),
        description,
    } = contents[7]
    else {
        panic!()
    };
    assert_eq!(
        (term.text.as_str(), show(description)),
        ("a phrase", vec!["desc".to_owned()])
    );
    let InlineTagContent::Summary(summary) = contents[9] else {
        panic!()
    };
    assert_eq!(show(summary), ["A ", "{@link}", " summary."]);
    let InlineTagContent::Return(description) = contents[10] else {
        panic!()
    };
    assert_eq!(show(description), ["the ", "{@code}"]);
    let InlineTagContent::SystemProperty { name: Some(name) } = contents[11] else {
        panic!()
    };
    assert_eq!(name.text, "a.b");
    let InlineTagContent::Other(content) = contents[13] else {
        panic!()
    };
    assert_eq!(show(content), ["a {b} c"]);
    // javadoc: "unexpected content" for `{@docRoot x}`, and at the `-` of `a.b-c`.
    assert_eq!(
        kinds(&javadoc),
        [
            DiagnosticKind::UnexpectedContent,
            DiagnosticKind::UnexpectedContent
        ]
    );
    assert_eq!(source(text, javadoc.diagnostics[1].span), "-c");
}

#[test]
fn missing_inline_tag_names() {
    // javadoc: "no tag name after '@'" for `{@ foo}`, `{@}`, and `{@1x}`, which are text.
    let javadoc = parse("/** [{@ foo}] [{@}] [{@1x}] */");
    assert_eq!(
        show(&javadoc.description),
        ["[", "{@", " foo}] [", "{@", "}] [", "{@", "1x}]"]
    );
    assert_eq!(kinds(&javadoc), [DiagnosticKind::MissingTagName; 3]);
}

#[test]
fn snippets() {
    let text = "/**
 * {@snippet lang=java id='a b' region=\"r\" flag :
 *   if (x) {
 *     y(); // @highlight substring=\"y\"
 *   }
 * }
 * {@snippet : one}
 * {@snippet class=Example region=main}
 * {@snippet a=b:
 * body}
 */";
    let javadoc = parse(text);
    let snippets: Vec<_> = javadoc
        .description
        .iter()
        .filter_map(|n| match n {
            Inline::Tag(tag) => match &tag.content {
                InlineTagContent::Snippet(snippet) => Some(snippet),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let attributes: Vec<_> = snippets[0]
        .attributes
        .iter()
        .map(|a| {
            (
                a.name.text.as_str(),
                a.value.as_ref().map(|v| v.text.as_str()),
            )
        })
        .collect();
    assert_eq!(
        attributes,
        [
            ("lang", Some("java")),
            ("id", Some("a b")),
            ("region", Some("r")),
            ("flag", None)
        ]
    );
    let body = snippets[0].body.as_ref().unwrap();
    assert_eq!(
        show(body),
        [
            "   if (x) {",
            "⏎",
            "     y(); // @highlight substring=\"y\"",
            "⏎",
            "   }",
            "⏎",
            " "
        ]
    );
    // javadoc: "unexpected content" when the body starts on the `:` line.
    assert_eq!(show(snippets[1].body.as_ref().unwrap()), ["one"]);
    assert!(snippets[2].body.is_none());
    assert_eq!(snippets[2].attributes.len(), 2);
    // javadoc reads `a=b:` as `a=b` and the colon.
    assert_eq!(snippets[3].attributes[0].value.as_ref().unwrap().text, "b");
    assert_eq!(show(snippets[3].body.as_ref().unwrap()), [" body"]);
    assert_eq!(kinds(&javadoc), [DiagnosticKind::UnexpectedContent]);
}

#[test]
fn html() {
    let text = "/** text <b\n * >bold</b> <a href=\"x\"\n *  title='t' data-x=foo hidden>link</a>\n * <br/> <!-- a\n * c --> </p> */";
    let javadoc = parse(text);
    assert_eq!(
        show(&javadoc.description),
        [
            "text ",
            "<b>",
            "bold",
            "</b>",
            " ",
            "<a href=[\"x\"] title=[\"t\"] data-x=[\"foo\"] hidden>",
            "link",
            "</a>",
            "⏎",
            " ",
            "<br/>",
            " ",
            "<!-- a⏎ c -->",
            " ",
            "</p>"
        ]
    );
    let Inline::StartElement(element) = &javadoc.description[1] else {
        panic!()
    };
    assert_eq!(source(text, element.span), "<b\n * >");
    assert!(javadoc.diagnostics.is_empty());
}

#[test]
fn malformed_html_and_entities() {
    // javadoc: "malformed HTML" for `1 < 2` and `<3`, "bad HTML entity" for `&#xZZ;` and a lone
    // `&`. `&bogus;` is an entity, which doclint reports as invalid.
    let javadoc = parse("/** 1 < 2 <3 <a href=\"x> &amp; &#64; &#x40; &bogus; &#xZZ; & alone */");
    assert_eq!(
        show(&javadoc.description),
        [
            "1 < 2 <3 <a href=\"x> ",
            "&amp;",
            " ",
            "&#64;",
            " ",
            "&#x40;",
            " ",
            "&bogus;",
            " &#xZZ; & alone"
        ]
    );
    use DiagnosticKind::*;
    assert_eq!(
        kinds(&javadoc),
        [
            MalformedHtml,
            MalformedHtml,
            MalformedHtml,
            MalformedEntity,
            MalformedEntity
        ]
    );
}

#[test]
fn inline_tags_in_attribute_values() {
    // javadoc expands `{@docRoot}` in `href="{@docRoot}/x"`.
    let javadoc = parse("/** <a href=\"{@docRoot}/x.html\" title=a&amp;b>doc</a> */");
    assert_eq!(
        show(&javadoc.description),
        [
            "<a href=[\"{@docRoot}\", \"/x.html\"] title=[\"a&amp;b\"]>",
            "doc",
            "</a>"
        ]
    );
}

#[test]
fn nesting_is_bounded() {
    let depth = 10_000;
    let text = format!(
        "/** {}x{} */",
        "{@summary ".repeat(depth),
        "}".repeat(depth)
    );
    let javadoc = parse(&text);
    assert!(kinds(&javadoc).contains(&DiagnosticKind::NestingTooDeep));
    let text = format!("/** {}x */", "<a title=\"{@summary ".repeat(depth));
    parse(&text);
    let text = format!("/** {{@link A{}}} */", "<A".repeat(depth));
    let javadoc = parse(&text);
    assert_eq!(kinds(&javadoc), [DiagnosticKind::MalformedReference]);
}

#[test]
fn unterminated_comment() {
    let javadoc = parse("/** text {@code x");
    assert_eq!(show(&javadoc.description), ["text ", "{@code}"]);
    assert_eq!(kinds(&javadoc), [DiagnosticKind::UnclosedInlineTag]);
}

#[test]
fn any_span_is_read() {
    let text = "class A { /** é */ }";
    for (start, end) in [(0, 99), (11, 12), (14, 15), (99, 3), (10, 10), (0, 0)] {
        let javadoc = parse_javadoc(text, Span::new(start, end));
        check_javadoc(text, &javadoc).unwrap();
    }
    let javadoc = parse_javadoc(text, Span::new(10, 19));
    assert_eq!(show(&javadoc.description), ["é"]);
}

#[test]
fn long_inputs_take_linear_time() {
    // Each of these took quadratic or exponential time in an earlier version.
    let n = 100_000;
    for body in [
        "@".repeat(n),
        " ".repeat(n) + "x",
        "<a b ".repeat(n / 5),
        "<a title=\"".repeat(n / 10),
        "<!--".repeat(n / 4),
        "&a".repeat(n / 2),
        "{@a ".repeat(n / 4),
        "{@link #m(".repeat(n / 10),
        "\n * @".repeat(n / 5),
    ] {
        parse(&format!("/**{body}*/"));
    }
}
