# Doc comment parsing

Phase 2 of [ROADMAP.md](../ROADMAP.md) calls for doc comment parsers that produce structured data,
for traditional comments (`/** */`) and Markdown comments (`///`, Java 23 and later).
`crates/ferrion_doc_comments` has both. This document records what they take, what they produce,
how Unicode escapes are handled, and where they differ from javadoc.

The syntax is the Documentation Comment Specification for the Standard Doclet's, JEP 467's for
Markdown, and JEP 413's for `{@snippet}`. Where those are silent, javadoc's behavior was observed
by running javadoc 27 on small comments written for the purpose, never by reading its source. The
unit tests say which cases were observed. When javadoc's behavior is in doubt, run it on a comment
that shows it, and update the parser, its tests, and this document together.

## Input

The parsers work on comments, not files, and don't depend on the Java parser:

- `parse_javadoc(text, span)` parses the traditional comment at `span` of `text`, a
  `SourceText`'s text. `span` is a `DocComment` token's span from the lexer.
- `parse_markdown(text, lines)` parses the Markdown comment whose lines are at `lines`: the spans of
  the `DocLineComment` tokens that form it, in source order.
- `markdown_comment_runs(tokens)` groups a file's `DocLineComment` tokens into those runs, the way
  JEP 467 describes and javadoc does (see "Comments" in
  [lexer-oracle-normalization.md](lexer-oracle-normalization.md)): a run continues while the next
  line, after any whitespace, starts with `///`, so a blank line, a line of code, or another comment
  ends it.

Which comment documents which declaration is the Java parser's business (comment attachment, a
Phase 2 design decision), and so is the Java version: below Java 23, `///` comments are plain line
comments (SCOPE.md), so the caller doesn't ask for them to be parsed. Neither doc comment parser
looks at the version.

The parsers never fail and never panic. A span that is out of bounds or splits a character is
shrunk to fit; a traditional comment without `/**` or `*/` is read from the start or to the end of
its span; Markdown lines out of order are skipped. Whatever is between two Markdown lines' spans
stands for a line break, so lines that aren't a run still parse.

## Content and positions

Each parser first builds the comment's *content*: the text javadoc reads, with the comment syntax
taken out and Unicode escapes translated. Every part of the content knows where it is written, so
any range of it maps back to a span of the source text (`content.rs`).

- **Traditional comments:** the text between `/**` and `*/`. Asterisks right after `/**` go (javadoc
  reads `/*** x` as ` x`). On later lines, whitespace followed by asterisks goes, with the
  asterisks; whitespace that isn't followed by an asterisk stays, and so does whitespace after the
  asterisks, which matters in `<pre>`.
- **Markdown comments:** each line after its leading whitespace and `///`, shifted left by the least
  indentation of the lines that aren't blank (JEP 467). A tab counts as one character, as in
  `String.stripIndent`.
- **Line breaks:** each line terminator (CR, LF, or CRLF, written directly or as an escape) becomes
  one `\n` in the content, standing for the terminator and the next line's leading syntax: its
  whitespace and asterisks, or its whitespace, `///`, and indentation. The parsers' line break
  nodes (`Inline::LineBreak`, `markdown::Inline::SoftBreak`) have spans that cover all of it, so
  the formatter can tell content from decoration.

### Unicode escapes

javac translates Unicode escapes (JLS §3.3) before javadoc sees a comment, and javadoc 27 shows it:
`{@code \u0041\u007d} done` renders as `<code>A</code>} done`, so the escaped `}` closes the tag.
The parsers do the same: they read the translated text, so escapes can form tags, close them, and
end lines, exactly as they would for javadoc.

- **Spans cover escapes as written.** A span never starts or ends inside an escape, or inside a
  character. The fuzz targets check this.
- **Text values are translated.** `Text::text` is what javadoc reads; `Text::span` is what is
  written. In a traditional comment, a `Text` never crosses a line break, so its value is exactly
  its span's text with escapes translated (the fuzz targets check this too). A formatter that
  rewraps text can compare the two: where they differ, the text has escapes, and the formatter
  should keep its source text rather than the value.
- **Escape rules match the lexer's:** the even-backslash rule, multiple `u`s, surrogate pairs joined
  into one character, and a lone surrogate read as U+FFFD.
- **One difference from the lexer:** a malformed escape (`\u` without four hex digits) stays as
  written in the content, where the lexer drops it. The lexer has already reported it, and
  keeping it means no text of the comment is lost.

## Data model

Both parsers produce the same shape: a main description, block tags, and diagnostics.

```text
Javadoc         { span, description: Vec<Inline>, tags: Vec<BlockTag<Vec<Inline>>>, diagnostics }
MarkdownComment { span, description: Vec<Block>,  tags: Vec<BlockTag<Vec<Block>>>,  diagnostics }
```

The model is a tree of typed enums and structs (`model.rs`, `markdown.rs`). Every node has a source
span. Siblings are in source order and don't overlap, and children are within their parents; the
fuzz targets check both.

- **`BlockTag<B>`:** the tag's span (from `@` to the end of its body), its `name` without the `@`,
  a `BlockTagKind` (the standard tags, and `Other` for any other name, such as `@apiNote`), an
  optional `TagArgument`, and its body `B`. Arguments are: the name of `@param` (with `<T>` for type
  parameters), the reference of `@throws`, `@exception`, `@provides`, `@uses`, and `@see`, the
  quoted text of `@see "..."`, the name and type of `@serialField`, and the URL of `@spec`. The
  body is the rest: the description, or `@see`'s label. Bodies leave out leading and trailing
  whitespace, so an empty description is an empty body.
- **`Inline`** (traditional content, and the content of inline tags in both kinds of comment):
  text (one line at most), line breaks, `@` escapes, entities, HTML start and end tags, HTML
  comments, and inline tags. HTML elements aren't matched with their end tags, as in javadoc's own
  tree: start and end tags are separate nodes, so unbalanced HTML is representable, and checking it
  is for the linter. `<pre>` content is ordinary content, with its whitespace kept; javadoc 27 reads
  a line starting with `@` inside `<pre>` as a block tag, and so do the parsers.
- **`InlineTag`:** span, name, `closed`, and an `InlineTagContent` by tag. `{@code}`, `{@literal}`,
  `{@snippet}` bodies, and unknown tags hold literal text (text and line breaks). `{@link}` and
  `{@linkplain}` hold an optional reference and a label; `{@value}` a format and a reference;
  `{@index}` a term and a description; `{@inheritDoc}` an optional supertype; `{@summary}` and
  `{@return}` inline content; `{@systemProperty}` a name; `{@snippet}` its attributes and inline
  body. Snippet markup comments (`// @highlight ...`) are left in the body's text.
- **`Reference`:** the module, the qualifier (a `RefType`: a qualified name, type arguments, `[]`s,
  and `...`), the member with its parameters (each a type and an optional name), and a `##`
  fragment, all with spans. `Reference::types` lists every type it names, in source order, for
  finding the imports a comment uses.
- **Markdown `Block`s:** paragraphs, ATX and setext headings, block quotes, lists (with their start
  number and tightness; the items of a tight list hold paragraphs too), fenced and indented code
  blocks, HTML blocks, thematic breaks, GFM tables, and link reference definitions. Markdown
  `Inline`s: text, code spans, emphasis, strong emphasis, links, images, inline HTML, soft and hard
  breaks, and inline tags. A link's target is a URL or a program element (`LinkTarget::Element`,
  with a `Reference`).
- **`Diagnostic`:** a kind and a span. Kinds cover unclosed inline tags, missing tag names, malformed
  HTML, entities, arguments, and references, missing arguments, unexpected content, and nesting
  beyond the limit. Unknown tags aren't diagnostics: custom tags are legitimate, and whether a tag
  is allowed is the linter's question.

## Traditional comments

The parser reads the content once, left to right (`inline.rs`, `javadoc.rs`).

- **Block tags** start at an `@` that begins a line, after whitespace (and, on the first line, after
  `/**`), unless it is inside an inline tag. An `@` within a line is text. An `@` followed by no
  name (`@ x`, `@1x`) still ends the previous section, with a `MissingTagName` diagnostic, as
  javadoc 27 does. Tag names may contain `.`, `:`, and `-`, as javadoc accepts for custom tags.
- **Escapes** `@@`, `@*`, and `@/` are read only where the character after `@` would otherwise mean
  something, as the specification says and javadoc 27 shows: `@@` at the start of a line or after
  `{`, `@*` at the start of a line (after any whitespace), and `@/` after `*`, inside inline tags
  too (but not in literal text, such as `{@code}`'s). Elsewhere they are text. They are `Inline::Escape` nodes, so text values never differ from their spans for them.
- **Inline tags** run to the `}` that matches their `{`, counting nested braces. An unclosed tag
  runs to the end of the comment, taking in blank lines and block tags, as in javadoc 27. `{@code}`
  and `{@literal}` drop one space or tab after the name and keep a line break (`{@code   x}` is
  `  x`). `{@` with no name is text, with a diagnostic.
- **HTML and entities:** a `<` or `&` that doesn't begin a well-formed tag, comment, or entity is
  text, with a diagnostic, as javadoc 27 reports "malformed HTML" and "bad HTML entity". Tags may
  span lines. Attribute values may hold inline tags, as in `href="{@docRoot}/x.html"`.
- **References** end at whitespace outside parentheses or at `}`. Whitespace, including line
  breaks, is allowed inside the parentheses. javadoc 27 accepts parameter names, generic types, and
  `Type.member(params)` without `#`, and rejects `C#` and `C.`; so does the parser.

## Markdown comments

`markdown.rs` puts javadoc's tags on top of `pulldown-cmark`, with CommonMark and GFM tables
enabled (the dialect JEP 467 specifies), in four steps:

1. **Code.** The whole content is parsed once as Markdown to find code spans and code blocks, where
   tags aren't recognized.
2. **Inline tags.** Every `{@` outside code, and not escaped as `\{@`, is read as an inline tag with
   the traditional parser in Markdown mode (no HTML, entities, or `@` escapes). A tag runs to its
   matching `}`, across lines and Markdown structure, and to the end of the comment when unclosed,
   as in javadoc 27. Tags are found before code spans that start inside them: javadoc 27 reads
   ``{@code a`b} c` `` as a tag followed by `` c` ``, and so does the parser.
3. **Block tags** start at an `@` that begins a line, after any whitespace, outside code and inline
   tags. HTML blocks don't hide them: javadoc 27 ends the description at an `@param` line inside a
   `<div>`. `\@` and `&#064;` are a literal `@`.
4. **Sections.** The main description and each tag's body are separate Markdown documents, so a link
   reference definition applies within its section, as the specification says. Before a section is
   parsed, each inline tag in it is replaced by inert text of the same length (a Private Use Area
   character, repeated and padded), which keeps every offset and stands in for the tag: emphasis
   and links form around it as they would around a word. The tree is then built from the parser's
   events, with the placeholders replaced by the tags. Where the structure has no room for a tag
   (inside a link destination, say), its source text is put back instead.

**Links to program elements** are reference links (`[List]`, `[List][]`, `[a list][List]`) whose
label has no link reference definition and reads as a reference. javadoc 27 links `[List]`,
`[ List ]`, `[java.base/]`, and `[m()]`, reports "reference not found" for an identifier that
resolves to nothing, such as `[Nope]`, and leaves `[see above]` and `[1]` as text. The parser makes
the first two kinds links (resolving is for consumers) and leaves the rest as text. Array
parameters are written `[String#copyValueOf(char\[\])]`, and the reference parser reads `\[\]` as
`[]`.

## Nesting and time

Nothing recurses without a bound. Inline tags, HTML attribute values, type arguments in references,
and Markdown blocks and inlines nest at most `MAX_NESTING` (32) deep; deeper content is read flat,
with a `NestingTooDeep` diagnostic: an inline tag as literal text, a type argument as a malformed
reference, and Markdown content as part of the deepest container. The tree is built from the
Markdown parser's events with an explicit stack, and the depth limit also bounds the recursion of
dropping, comparing, and printing the tree.

Reading is linear in the comment's length. A malformed HTML tag fails before its attribute values'
content is read, and searches for closing quotes and `-->` are cached, so many unclosed tags don't
make reading quadratic or exponential. Unit tests and the fuzz targets' 10-second timeout check
this.

## Fuzzing

The `javadoc` and `markdown` fuzz targets in `fuzz/` parse arbitrary bytes as a comment's content
and check `ferrion_doc_comments::invariants`: spans are in bounds, on character boundaries, and
don't split escapes; nodes nest and come in order; traditional text matches its span; diagnostics
are in order; and parsing is deterministic. Reduced failures are kept in
`crates/ferrion_doc_comments/tests/fuzz-regressions/`. See "Fuzzing" in
[CONTRIBUTING.md](../../CONTRIBUTING.md).

## Known differences from javadoc

- **Malformed Unicode escapes** stay as written in the content; javac reports them and drops them.
- **`@throws` without an exception** has a `MissingArgument` diagnostic; javadoc 27 reports nothing.
- **Quoted strings** (`@see "..."`, `{@index "..."}`, snippet attribute values) end at the end of
  their line.
- **Identifiers** in references and parameter names are Unicode letters, digits, `_`, and `$`, an
  approximation of Java's identifier characters that leaves out identifier-ignorable characters.
- **Inline tag content in Markdown comments** (the label of `{@link X the *label*}`, the text of
  `{@summary}`) is read as in traditional comments, as text and nested tags. javadoc 27 reads it as
  Markdown, rendering `*label*` as emphasis. The model's inline tag content is traditional
  `Inline`s, so representing it would need Markdown content in inline tags.
- **Markdown links with array references written with plain brackets,** such as `[Object\[\]]`:
  javadoc 27 treats the label as a reference and reports "reference not found"; the parser doesn't
  read it as a reference (a type in that position has no `[]`), so it stays text.
- **Duplicate link reference definitions:** `pulldown-cmark` keeps the first definition of a label
  and reports nothing for later ones, so they aren't in the structure. A formatter must not drop
  them; it can find them as source text outside every block.
- **The Markdown implementation** is `pulldown-cmark`, not the one javadoc uses, so corner cases of
  CommonMark may differ. Both follow the CommonMark specification and its test suite.
- **Images** whose label reads as a reference (`![List]`) are images with an empty destination.

## Changing this

When javadoc's observed behavior, the specification, or a consumer's needs change what the parsers
produce, update the parsers, their unit tests (noting what javadoc was seen to do), and this
document together.
