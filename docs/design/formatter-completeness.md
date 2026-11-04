# Formatter completeness

Stage A2 of the [roadmap](../ROADMAP.md) starts with this document, written before any A2 code. It
covers the formatter's rewrites, imports and doc comments, and range formatting, with the edits
range formatting and the language server are built on. What the formatter does is fixed in
[SCOPE.md](../SCOPE.md#formatter); this document is about how. It builds on the
[formatter design](formatter.md), and changes it only where it says so.

GJF's behavior below was learned by running GJF 2.0.0 on Temurin 27 on inputs written for this
document, never by reading its source (see
[CONTRIBUTING.md](../../CONTRIBUTING.md#clean-room-policy)).

## Decisions made at the start

These were settled before this document was written:

- **The gate:** this document first, then a review, before any A2 code. The work runs on the local
  `a2/integration` branch, cut from `main` at 12bb01c.
- **Javadoc matches GJF 2.0.0's observed output.** The comparison drops
  `--skip-javadoc-formatting`. Departures are made, as in A1, only where GJF's output is unstable,
  changes meaning, or goes against SCOPE.md.
- **Imports are ordered as GJF orders them,** with `import module` in a group of its own.
- **Range formatting applies a rewrite only when the whole construct is in the range:** the whole
  import section, the whole modifier run, the whole doc comment. Otherwise only whitespace inside
  the range changes.

## Pipeline

A2 adds two steps to [A1's pipeline](formatter.md#pipeline), between finding disable regions and
building the document:

1. Parse (`format` only).
2. Classify comments.
3. Find disable regions.
4. **Plan the import section** (below): which imports stay, in what order, with which comments.
   It needs the file's uses, from one pass over its tokens and its attached doc comments.
5. **Parse doc comments** that will be reflowed, and lay out what doesn't depend on the column.
   Step 4 reads the same parses for Javadoc references.
6. Build the document. The compilation unit rule emits the planned import section.
7. Print, reflowing doc comments at their columns, and recording edits when asked for.
8. Compare.

Both rewrites are decided from the input before layout, so the document already holds their
result, and a second run plans the same section and fills the same comments (SCOPE.md's single
pass).

## Imports

### The section

The import section runs from the first import to the end of the last one, including comments
trailing it on its line. Comments before the first import and after the last are outside it: GJF
leaves comments before the first import where they are, and Ferrion does too. Comments after the
last import lead what follows, by the [classification](parser-comments.md#classification). GJF
attaches a comment on the line after the last import to that import, so a comment meant for the
class can end up among the imports (observed); Ferrion doesn't.

The section is **rewritten only when nothing pins it:** it isn't touched when it overlaps a
disable region or holds a formatter marker or a `ferrion-disable*` comment. Those directives are
never moved (SCOPE.md), and a sort can turn a region inside out: GJF, which ignores the markers,
moved `// @formatter:on` above `// @formatter:off` (observed). The same rule as for modifier runs.

### Order

GJF's order (observed), which Ferrion matches:

- **Groups:** static imports, then `import module`, then all other imports, with a blank line
  between groups and none inside one. `java`, `javax`, and the rest share the last group. The
  section has a blank line before and after it, as in A1.
- **Within a group,** imports sort by the whole name compared character by character, as written
  with its dots (and, for a static import, its member): not segment by segment, so `a.B$` sorts
  before `a.B.C`, and `$` < `*` < `.` < digits < uppercase < `_` < lowercase < non-ASCII. Ferrion
  compares UTF-16 code units, as Java strings compare. Whether GJF does wasn't observed: a name
  outside the Basic Multilingual Plane will settle it. A name is compared after Unicode escapes are
  translated; GJF rejects a file with an escape in an import (observed), so it isn't compared.
- **Duplicates** of an import, with the same name, are merged and the first is kept (observed).
- **Long imports** never break, as in A1.

### Removal

An import is **removable** if one of these holds:

- **Unused** (single-type or single static only, not on demand, not a module import): its simple
  name has no use in the file (below).
- **Own package** (single-type only): its qualifier is the file's package, such as `p.q.A` in `package p.q;`. A nested
  type, `p.q.A.B`, isn't covered.
- **A duplicate** of an import kept earlier, of any kind: on demand, module, and `java.lang`
  imports too (decided in review).

…and **no comment belongs to it** (SCOPE.md). GJF removes an import with a comment and moves the
comment to the import before it (observed); Ferrion keeps the import. Every removal is safe from
the syntax alone: an unused name can't be needed by the compiler, an own-package import names a
type the package already sees, and a duplicate adds nothing.

GJF also removes `java.lang` imports (`java.lang.String`), the package's own wildcard (`p.q.*`),
and `java.lang.*`, and unused single static imports (all observed). SCOPE.md says `java.lang` and
wildcard imports are never removed, because doing it safely needs type information: removing
`java.lang.String` changes the program if the package declares a `String`. Ferrion keeps them, a
departure. Unused single static imports are removed too, as GJF removes them (decided in
review): no identifier with the member's name means no use.

**A use** is, as GJF counts it (observed):

- any identifier in the code that isn't a declaration's own name, a label, the member after `::`,
  or a later segment of a qualified name (`Pkg.a.b()` uses `Pkg`; `x.Sel.y()` doesn't use `Sel`).
  Declarations around it don't matter: a local variable `Local` keeps `import u.Local`;
- in the doc comment attached to a declaration (the one `ferrion_syntax::comments::doc_comment`
  returns), the first segment of each type in a reference: `{@link X}`, `{@linkplain X}`,
  `{@link #m(P1, P2[])}` (`P1` and `P2`), `{@value X#F}`, `@see X`, `@throws X`, `@exception X`,
  `@uses X`, `@provides X`, and in Markdown comments `[X]` and `[text][X]` too.
  `Reference::types` lists them. Labels, prose, `{@code}`, `{@literal}`, snippets, and comments
  that aren't attached don't count, and neither does `{@value X}` without a `#`.

Counting too much only keeps an import, so the rules can be generous where GJF is; counting too
little would break the program, and `verify` checks it independently (below).

### Comments

SCOPE.md's rule: a comment belongs to an import when it is on the same line after it, with no
code after it on that line, or directly above it with no blank line between. A comment followed by
code on its line belongs to that code, as the comment map places it, so the import doesn't keep it
(decided after the ranges work; keeping the import broke idempotence, since the first run moves
the comment to the code's line). It moves with that import, and the import is kept. A comment
that belongs to no import, such as a group header followed by a blank line, moves to the top of
the section, keeping its order.

GJF's rule differs (observed): a `//` comment belongs to the import **above** it, so a header for
the next import stays with the one before it, and a header after a blank line, a block comment
anywhere between imports, or a comment inside an import makes GJF reject the whole file ("Imports
not contiguous"). It also drops a duplicate's comment. On a section GJF has already sorted the two
rules put every comment in the same place, because each comment sits between the same two imports
either way, so files formatted with GJF don't change. Ferrion follows SCOPE.md (decided in
review):

- **Owned:** a comment trailing an import on its line, the comments directly above it with no
  blank line (any kind: `//`, `/* */`, `/** */`), and comments inside it (`import /* c */ a.B;`)
  move with it, and its line breaks around them are kept. Comments above the first import are
  outside the section and stay at its top, as in GJF, so a license header written right above
  the first import doesn't move into the list.
- **Floating:** any other comment in the section moves to the top of the section, in order, with
  a blank line after the last.
- **Never lost:** an import with a comment is never removed, so no comment is dropped; a
  duplicate with a comment is kept.

### Emitting it

The import plan is a list of `(import, owned comments)` in output order plus the floating
comments. The compilation unit rule emits it through a new helper on the `Formatter`, as
`reordered_run` emits a modifier run: the floating comments, then each kept import with its
comments, then the token cursor and the comment cursor move past the whole section. The debug
assertion that tokens are emitted in source order allows the section as one unit, and checks that
each kept import is emitted once and each removed import's tokens are not.

### Checking it

`verify`'s equivalence compares the sections as multisets of import names, and checks each import
missing from the output **independently of the plan**: an import of the same kind and name is
still there; or it is a single-type import of the file's own package; or it is a single-type or
single static import whose simple name is no identifier in the output outside the imports that
may refer to one, and no reference in an attached doc comment. An identifier that may refer to an
import is any but a declaration's own name, a label, and a name after `.` or `::`; the doc
comments are the ones javac attaches (`ferrion_syntax::comments::doc_comment`), and each type in
each of their references counts by its first segment. That still counts more than the plan does,
so it can't pass a removal the plan got wrong. A first version counted every identifier and every
word in every doc comment; fuzzing showed it rejected correct removals (a field named like the
import, link labels, `{@code}` text, an unattached doc comment), so it was narrowed (decided in
review).

## Doc comments

### Which comments

GJF reformats every `/** */` comment, wherever it is: on declarations, but also in method bodies,
inside expressions, dangling at the end of a class, and on imports, `package`, and module
directives (observed). It reformats a `///` run as Markdown only when the run is a declaration's
doc comment, `package` included; elsewhere, and when a blank line cuts the run off from the
declaration, `///` lines are line comments (observed). Ferrion does the same, with three
differences that follow from SCOPE.md and A1:

- **Disable regions:** GJF reformats doc comments inside `@formatter:off` regions; Ferrion copies
  the region as written, as for everything else in it.
- **`///` below Java 23** is a line comment (`ferrion_syntax::comments::doc_comment` already reads
  the release from the tree), so it is never reflowed.
- **`///` that isn't a doc comment** is a line comment, and line comments aren't wrapped (SCOPE.md,
  and A1's departure for long `//` comments). GJF wraps it with a `//` continuation line.

GJF leaves `/***/`, `/*****/`, `/***  text */`, `/*** … ***/` banners, and `////` alone
(observed). These are block comments or line comments to the lexer, or doc comments starting
with more stars, and Ferrion re-indents them as A1 does. Which `/**` comments count is fixed by
one function, so the GJF comparison can find any it misses.

### Where reflow happens

A doc comment's lines depend on the column the comment starts at, which only the printer knows.
So reflow is done by the printer, not the builder:

- **A new kind of copied text, `VerbatimKind::Doc`,** carries every traditional doc comment, one
  line or more (built in the core step, printing as A1 did). The Javadoc workstream gives it a
  layout: the parsed comment (`ferrion_doc_comments`), cut into words, breaks, and verbatim runs
  (see below), in a side table of the document. The builder does everything that doesn't depend
  on the column, once. A Markdown comment is several `DocLineComment` tokens, so the Markdown
  workstream makes a run one piece of copied text.
- **The printer fills it** at the column it is at, with the 100-column limit counted in UTF-16 code
  units, as for code. A doc comment always starts and ends a line (GJF moves `int y = /** x */ 2;`
  to put the comment on a line of its own, observed), so it forces its groups to break and the
  column it starts at is its line's indentation. The fill is greedy, like GJF's (it breaks only
  where the next word doesn't fit), which keeps it linear and stable: filling filled text gives the
  same lines.
- **No minimum width.** At deep indentation GJF wraps one word per line (observed); so does
  Ferrion. The expression indent cap of A1 bounds the column for comments inside expressions.

### Javadoc

Ferrion matches these GJF rules (each observed, with its snapshot to come in the doc comment
suite):

1. **Fill.** Lines of a paragraph are joined and refilled; runs of whitespace become one space. A
   line breaks only at whitespace, so a long word, URL, or hyphenated word is never split and may
   overflow. **No break goes before a word starting with `@`**, which would start a block tag; the
   word before it moves down too, or the line overflows. A line of exactly 100 columns fits.
2. **Inline tags** may break at their spaces, right after the tag name included, and their
   whitespace is collapsed: `{@code a    b}` gives `{@code a b}`, and a `{@code}` that spans lines
   outside `<pre>` is joined. **`<code>…</code>` is the exception:** its content is copied as
   written, line breaks included, and never broken; a break may come right after `<code>`.
3. **Shape.** The comment is one line, `/** text */`, exactly when its body is one line that
   fits and has no block tag; otherwise `/**`, the lines, then `*/` on its own line. An empty
   comment, blank lines and all, is `/** */`. Text on the `/**` line moves into the body, extra
   stars before `*/` go (`/** a **/` gives `/** a */`), a line without a star gets one, and leading,
   trailing, and doubled blank lines go.
4. **Paragraphs.** A blank line between paragraphs starts the next with `<p>`, after a blank line;
   a `<p>` within a line starts a new paragraph; `<p>`, `<P>`, and `<p/>` are written `<p>`,
   attributes kept; `</p>` goes; a `<p>` at the start of the first paragraph goes; a `<p>` alone on
   its line stays alone, with blank lines around it. No `<p>` is added after a block element.
   `<br>` in any spelling is written `<br>` and ends its line.
5. **Lists.** `<li>`, `<dt>`, and `<dd>` go 2 past their list's tag, and their continuation lines
   and nested blocks 4 past the `<li>`; `</li>` goes; a list gets a blank line before and after.
   Tag case is kept.
6. **Blocks** (`<pre>`, `<table>`, `<blockquote>`, `<h1>`–`<h6>`, `<dl>`, `{@snippet}`) get a
   blank line before and after, and an inline `<pre>…</pre>` or `{@snippet}` becomes its own
   paragraph. `<table>` and HTML comments are copied as written; a heading's inner whitespace is
   collapsed; `<div>`, `<hr>`, and unknown tags are inline text.
7. **`<pre>`** content is copied apart from the stars, trailing whitespace, and indentation (see
   the departure below).
8. **Block tags** keep their order, get a blank line before the first, and none between them.
   Continuation lines are at +4, and a tag's later paragraphs start with `<p>` at +4 with no blank
   line before. A tag whose text ends in a block gets a blank line after it. Empty tags are kept.
9. **Kept as written:** Unicode escapes, entities, `{@inheritDoc}`, unknown inline tags (their
   whitespace collapsed), and stray `<`, `>`, and braces. A word holding a Unicode escape is copied
   from the source, as `ferrion_doc_comments`'s `Text::span` allows, and measured as written, as
   the printer measures escapes in code.

**Fallback: re-indent only.** GJF leaves a comment unreflowed when it has an unclosed `<pre>`,
`<code>`, or `<table>`, an unclosed inline tag, or a line inside `<pre>` or an inline tag starting
with `@` and a lowercase letter (which javadoc reads as a block tag). An unclosed `<ul>`,
`<blockquote>`, `<a>`, `<b>`, or `<!--`, and a stray end tag, don't stop it (all observed). Those
are SCOPE.md's examples, and Ferrion uses the same test: the parser's `UnclosedInlineTag`
diagnostic, a scan of the HTML start and end tags for the three elements, and the block-tag rule
the parser already applies inside `<pre>`. A fallback comment gets A1's re-indenting, which is
GJF's (observed): stars aligned, trailing whitespace removed, a star added to lines without one,
the text after each star kept, and the shape kept.

**Departure: `<pre>` indentation is kept.** GJF removes a margin from `<pre>` content: the least
indentation after `* ` over every line of the comment, and inside `<pre>{@code`, also the code's
common indentation (observed). javadoc keeps whitespace after the stars, so that shifts what the
rendered `<pre>` shows, which is a change of meaning. Ferrion keeps each `<pre>` line's text after
its star as written. Files GJF has already formatted don't change, since their margins are
already gone. (Decided in review.)

### Markdown

GJF reflows Markdown doc comments with these rules (observed), which Ferrion matches:

1. **Fill** as in Javadoc, the `@` rule included; `///` is followed by one space (`///x` gives
   `/// x`); leading and trailing empty `///` lines go. No `<p>` is added. A blank `///` goes before
   the first block tag, and tag continuation lines are at +2.
2. **Lists:** continuation lines line up with the item's text (`- ` gives 2, `1. ` 3, `10. ` 4);
   markers and numbers are kept; lazy continuation lines are joined into their item. A block
   quote's `>` lines are joined.
3. **Blocks** get a blank line around them, inside lists too: ATX headings (inner whitespace
   collapsed, a closing `##` kept), fenced code blocks, GFM tables, and HTML blocks (`<pre>`,
   `<table>`, `<!-- -->`), which are copied as written.

**Which comments.** GJF also leaves a run whose first line starts with `////`, and the doc comment
of a field declaring several variables, as they are (observed, matched).

**Fallback.** GJF leaves a comment with a setext heading, an indented code block, a thematic break,
a link reference definition, or an unclosed inline tag unreflowed (observed), and SCOPE.md names
setext headings. Ferrion uses the same test, from the parsed blocks and diagnostics.

**Other shapes (observed, matched).** A quote starting with a list or another quote starts with an
empty `>` line, and one starting with a heading, fenced code, a table, or an HTML block with two. A
fenced code block the comment ends before its closing fence gets one.

GJF's Markdown reflow changes meaning or isn't stable in some cases (observed), so Ferrion departs
from it there:

- **No break before a word that would start a block.** GJF breaks before words like `#`, `-`,
  `+`, `*`, `1.`, `>`, and `` ``` ``, so the next line starts a heading, list item, quote, or code
  fence: the meaning changes, and for `#` and `` ``` `` a second run changes the output again.
  Ferrion treats these words as it treats `@`: the word before moves down with them. Which words
  count is decided by asking `pulldown-cmark` whether the line would start a block, not by a list.
- **Code spans are copied as written.** GJF collapses the spaces in `` `code   span` ``, which
  CommonMark keeps. Ferrion keeps a code span whole, as GJF keeps `<code>` in Javadoc.
- **A two-space hard break is written with `\`.** GJF drops a line's two trailing spaces and joins
  it with the next, losing the hard break. Keeping the spaces would go against trimming trailing
  whitespace, and re-indenting alone removes them, so a second run would reflow the comment (the
  first design had this fallback; it wasn't idempotent). A `\` means the same. A `\` hard break in
  the input is text and already kept (observed).
- **Fallback re-indents every line.** GJF moves only a fallback comment's first line and leaves
  the other `///` lines at their old columns (observed: stable, but misaligned). Ferrion puts each
  `///` at the comment's column, so misaligned lines become aligned. (Decided in review.)
- **A tab after `///`** is read by CommonMark as indentation (a tab stops at 4), so a line that
  starts with one may be an indented code block. The parser strips a tab that's common indentation,
  and where one is left and reads as code, GJF's output and Ferrion's are the same.
- **A heading stays on one line.** GJF wraps it, and on its next run the rest is a paragraph.
- **One space follows a list marker.** GJF keeps a second but continues the item at the column one
  gives, and on its next run the item's later paragraphs leave it.
- **An item's first block starts on its marker's line.** GJF puts the marker alone before a blank
  line, and has no fixed point.
- **HTML blocks are copied whole,** `<ul>` and `<li>` included, to the blank line CommonMark ends
  them at. GJF reflows `<div>`, `<p>`, and `<ul>`, and drops a quote's `>` from a copied block.
- **The hyphen rules are Javadoc's:** no break after a hyphen-ended word, and one at a line's end
  joins the next line's first word.
- **Re-indented only:** a `{@snippet}`, a tag body starting with a fence or list, HTML in a tag, a
  Unicode escape for other than a letter or digit, and the mostly fuzz-found cases docs/style.md
  lists.

**The layout's self-check.** Each layout is rendered at columns 0 and 96 and read back through
`verify`'s content comparison, and any mismatch makes the comment fall back. Fuzzing found about 20
crashes before this, and 30-second runs since have found none, though the columns between aren't
checked, so a failure there is still possible. A bug in `verify`'s comparison can only make a comment fall back.
Reflow with the check costs about 50 µs a comment on the `///` seed file, and the check alone wasn't
measured.

### Checking it

Reflow changes a comment's text, so safety property 3 compares doc comments by their parsed
content (formatter.md, [Safety properties](formatter.md#safety-properties)), normalized for exactly
the changes above: whitespace collapsed outside `<pre>`, `<code>`, code spans, fenced code, tables,
and HTML comments; `<p>` and `</p>`, `</li>`, and blank lines between paragraphs and blocks
ignored; `<br>` spellings and `<p>` spellings made one. What's left (the words, the tags and their
arguments, the inline tags' kinds and references, the HTML tags, and every copied run) must be the
same, in order. A comment that fell back is compared as A1 compares block comments. The
normalization lives in `verify`, written from this list, not from the reflow code.

Idempotence follows from the fill being greedy and from the shape rules giving a comment the rules
leave alone; the doc comment suite and the fuzz target check it, and the cases above where GJF
isn't stable are in the suite.

## Edits

Range formatting, `--lines` (A3), and the language server (A5) all want edits, not a new file.
Output tokens are input tokens in order (formatter.md), so the difference between input and output
is a list of changes between adjacent tokens, plus the rewritten constructs. The printer makes the
list as it writes:

```rust
pub struct Edit {
    pub range: Range<u32>,   // bytes of SourceText::text that are replaced
    pub text: String,        // what replaces them, with the file's line ending
}

/// The edits that turn `source` into what `format_tree` would give, in order, none touching a
/// token that doesn't move. `None` for a tree with a syntax error, which `format_tree` skips.
pub fn format_edits(tree: &SyntaxTree, source: &SourceText) -> Option<Vec<Edit>>;
```

- **Gaps.** For each pair of adjacent pieces of text written (tokens and copied text), the
  whitespace written between them is compared with the input's; where they differ, that is one
  edit, without the bytes the two share at either end. Comments are pieces too, so an edit never
  spans a comment that doesn't change.
- **Rewritten constructs** are one edit each, over their whole input span: a reordered modifier
  run, the import section when it changes, and a comment whose text changes (reflow,
  re-indenting, or the space after `//`). A rewrite is atomic, so an edit list never holds
  half of one. A rule marks a rewrite with an `Element::Rewrite { start, end }` container around
  what it emits, which prints nothing of its own.
- **Whole files:** `format_tree` and `format_edits` print the same document, and the edits
  applied to the input give `format_tree`'s output. A debug assertion in `format_edits`, the unit
  and snapshot tests, the fuzz target, and `format-check verify` over the corpus check it.
- **Byte-order mark and final line ending:** the edits are on `SourceText::text`, after the mark,
  which no edit touches; adding the final line ending is an edit at the end.

The printer records, for each piece it writes, its input span and its output range; edits come
from walking that record once. Only the edit API asks for it.

## Ranges

```rust
/// Formats the part of `tree` in `range` (bytes of the text), for a tree that may have syntax
/// errors outside it. Returns the edits inside the range, or why it can't.
pub fn format_range(
    tree: &SyntaxTree,
    source: &SourceText,
    range: Range<u32>,
) -> Result<Vec<Edit>, RangeError>;

pub enum RangeError {
    /// A syntax error affects the range: the error's unit overlaps it.
    Affected { error: Range<u32> },
}
```

`--lines` and an editor's selection are converted to byte ranges by their callers (A3, A5), and
several ranges are several calls. A range past the end of the text is cut at it, and an empty
range touches nothing. (Built in A2: `crate::range`. What any range of a tree needs, the comment
map, the units, the error units, and the whole-file edits, is in a `Ranges` value that the range
test and the fuzz target reuse for many ranges; it isn't public API.)

### What GJF does

With `--lines`, `--offset`, and `--length` (observed; the ranges workstream re-checked the cases
this section first left open, and corrected two, marked):

- **The range becomes tokens:** a range that touches no token (a blank line, indentation, the
  space inside `int  a`) changes nothing. A comment counts with the code it is placed with: a
  comment trailing code on its line (`a(); // c`) with that code, any other with the code after it
  (corrected: not always the token after). A comment on the last line of a unit is formatted with
  it, even one before more code on that line: with `a();` in range, `a();   /* c */ b();` gives
  `a(); /* c */` and `b();` on the next line, and with `class A {` in range,
  `class A { /* c */ int x;` gives the comment and `int x;` lines of their own.
- **It expands to units:** a statement in a block or a `switch` group, a `switch` label (`case 1:`)
  or rule (`case 1 -> f();`), a member, an enum constant, an import, the package declaration.
  Lambda bodies, anonymous class bodies, and `switch` expressions in a unit are part of it: a range
  inside a lambda body formats the whole statement the lambda is in, and one inside an anonymous
  class's method the whole field or statement around the class. A body that isn't a block
  (`if (a)` then `f();` on the next line) is part of its statement, which joins them. For a
  statement or declaration with a block (`if`, a method, a class), each run of its own tokens
  around its blocks is a unit: `if (a) {`, `} else {`, `} catch (E e) {`, `} while (a);`, and
  the `}` alone; each statement inside is a unit of its own. Touching one word at the end of a
  long line re-wraps the whole statement. A `}` alone is re-indented, and blank lines before it
  removed (corrected: it changes nothing only when it is already right).
- **Whitespace around the unit:** the gap before it is the whole-file output's, its blank lines,
  its indentation, and the trailing whitespace of the line before. The gap after it gets the
  whole-file output's line breaks, so the blank lines after it and the trailing whitespace on its
  last line, but the line after keeps its indentation. Code after the unit on its last line moves
  to a line of its own, indented (`a(); b(); c();` with `b` in range gives three lines, `c();`
  otherwise as written), and code that whole-file formatting puts on the unit's last line moves up
  onto it (an enum's `;` on the line after its last constant).
- **A declaration's doc comment, modifiers, and annotations** belong to its unit. GJF reorders
  modifiers only when all of the run is in the range given, not the expanded one: with only a
  field's annotation, on the line above, in range, the field is formatted and `final public
  static` is left (observed).
- **Imports are always sorted and pruned,** whatever the range, and blank lines around the import
  section change with them.
- **Any syntax error** in the file makes GJF refuse it.

### What a range changes

Ferrion expands a range as GJF does (decided in review), and keeps the rewrites to the range, as
decided at the start:

- **Units.** The range is expanded to the units its tokens are in, as GJF's are (a statement, a
  member, the header of one with a block, an import, the package declaration, a module directive),
  the comments before the unit's first token included. **The import section is one unit** when
  whole-file formatting rewrites it (sorts it, or removes or merges imports): a range touching any
  import expands to the whole section, so its rewrite applies (decided in review, 11). When the
  plan leaves the section as written (nothing to change, pinned by a directive or a disable
  region, or holding a syntax error), each import is a unit of its own and gets only its
  whitespace. Every gap inside the units and around them
  (indentation and blank lines) gets the whole-file output. A range touching no token changes
  nothing. A partial unit isn't formatted, because breaking part of a statement's lines can leave
  the rest over the limit or broken in two styles.
- **Rewrites** apply only when their whole construct is in the expanded range: the whole import
  section for sorting and removal (a departure: GJF always fixes the imports, where Ferrion fixes
  them when the range touches them), the whole run for modifier reordering, the whole comment for
  doc comment reflow. A declaration's modifiers and doc comment are in its header's unit, and the
  import section is one unit, so in practice no rewrite is cut. The modifiers are rewritten
  whenever the header is in range, where GJF reorders them only when the range given holds all of
  them (a departure, decided in review, 10).
- **Comments** go with the code formatting puts them beside: one the whole-file output keeps on
  the line of the code before it (`a(); // c`, `a(); /* c */` then `b();`) with that code, any
  other with the code after it. Deciding by the input, as GJF does, takes the file where
  whole-file formatting doesn't: with `class A { /* c */ int x;` and only `int x;` in range, the
  comment would stay on the `{` line, where formatting the whole result keeps it, though the
  whole input's formatting gives it a line of its own (a departure, decision 15).
- **A disable region** is copied as written, in a range as in a file.
- **Errors:** a range is formatted unless an error's unit overlaps its expanded range (below),
  where GJF refuses any file with an error.

### Files with syntax errors

SCOPE.md has range formatting work in files with syntax errors, as long as the range isn't
affected by an error, and the
[error recovery design](parser-error-recovery.md#what-a-syntax-error-affects) says what an error
affects: its **unit**, the innermost statement, member declaration, import, package declaration,
or module directive containing it, or the whole declaration for an error in its header.
`SyntaxNode::error_unit` and `SyntaxTree::error_units` build it in `ferrion_syntax` (A2):

- **Where each error is:** the parser's builder marks the node an error is reported in with
  `OWN_ERROR`, beside `CONTAINS_ERROR` on its ancestors, so the units start from those nodes.
- **An `Error` node in a list** of statements or members (a block, a `switch` group, a class body,
  the compilation unit), such as a stray `)` between two statements, is a unit of its own: it
  stands in an item's place, and the items around it are whole. The parser reports the error at a
  token that can't start a statement in the block, then puts the token in an `Error` node, so a
  list that is closed by its own `}`, has an `Error` item, and holds no character the lexer
  couldn't read doesn't make itself a unit for that error.
- **An error outside every unit** (a stray character between top-level declarations) is the whole
  file's, and so is the unit of an error in a node no unit contains; every range is then
  affected. So is every range when an error unit holds a disable region's start or end, which
  would leave the region's copying undefined.

The rules lay out only trees without errors, so for a range, **each error unit is copied as
written**: the formatter emits the unit's text as one `Verbatim`, its comments included, and lays
out everything else as usual. A unit always starts its own line, inside a block, a body, or the
import section, so copying it doesn't change how the code around it is laid out: the block around
it breaks in any case, and the indentation of what follows is its block's. Recovery that loses a
`}` can shift the tree's structure after the error, and then more of the file is inside the error's
unit or the units around it change; the range test measures how often.

`format_range` then:

1. builds the document for the whole tree, error units copied, and prints it with edits;
2. expands the range to units (above);
3. returns `RangeError::Affected` if an error unit overlaps the expanded range;
4. keeps the edits inside the expanded range (each rewrite's only if its construct is inside),
   the gap before it, and the line breaks of the gap after it (above).

In a file without errors, step 1 is `format_edits`. Formatting the whole tree for a range costs a
whole file's formatting (A1: under a millisecond for most corpus files), which keeps the code
paths one; a range never needs less than its file's comments and import plan anyway.

The rest of the tree is formatted as in the clean file, because nothing outside an error unit
reads inside it, with one exception: the type-use annotations the file imports (A1's
`type_use_imports`), which an error in an import, or before the imports, can lose. The range test
checks the claim on the whole corpus.

**The range test** (`cargo format-check ranges`) breaks each corpus file at tokens picked with a
seeded generator, deleting each or replacing it with one of `goto`, `)`, `]`, `}`, `;`, `=`, or
`.`, and formats every run of a unit's tokens away from the change through `format_range`. A
broken file's run is matched with the clean file's run that has the same tokens at the shifted
offsets, the same kind of unit, and the same kinds of nodes around it, and whose range expands
to the same span; it is then compared with the clean file's output there: its text, the gap
before it, and the line breaks after it (the gaps only next to units that match too, since they
depend on both sides). A run without a match is one whose structure recovery shifted. With seed 1
and two tokens per file, over the corpus's 15,724 files: 31,444 copies (3,800 still parse, 62
are affected as a whole), 2.94 million runs away from the change, 3.5% of them affected by the
error, and of the rest 5.7% shifted (25.6% after a broken brace, 3 to 8% for other tokens) and
2,673,257 the same as in the clean file; 30, in copies whose imports changed, are the type-use
exception, and none differ otherwise.

With the import section one unit (decision 13), the test compares the section as one run when
the clean file's is rewritten. Which imports the section keeps depends on the code that uses them,
which the change can delete, so a difference in a copy whose kept imports aren't the clean file's
is counted with the type-use exception, and the gaps next to a section whose imports are all
removed, or before a comment that goes with the code after, aren't compared, since the range
leaves them to that code. With the same seed and count, and comments counted by the output
(above): 2.85 million runs, 3.6% affected, 6.5% of the rest shifted (27.9% after a broken brace;
a section rewritten in one file and not in the other doesn't match), 2,570,309 the same, 151 in
the import exception, and none differ otherwise. On javac's tests: 334,681 runs, 245,559 the
same, 51 in the import exception, none differing.

**Parser follow-ups,** after A2 (decided in review, 12), found by the range test:

- **A stray token's error is reported in its block.** At a token that can't start a statement,
  `expr_stmt` reports the error with the block open, then puts the token in an `Error` node, so
  `error_units` has to guess that a closed block's own error is its `Error` item's (above).
  Reporting it inside the `Error` node would make the flag exact.
- **Unclosed-block recovery doesn't look past annotations.** A method missing its `}` followed by
  `@Test public void m() {}` isn't closed at `public`, since the statement starts with `@Test`:
  the rest of the class parses as statements, which is most of the 26% of units shifted after a
  broken brace. An annotation followed by a member-only modifier should close the block too.

## Safety properties

What A2 adds to [the four properties](formatter.md#safety-properties):

- **Equivalence** compares the import section as a set: every import in the output is one of the
  input's, and every import of the input that isn't in the output passes the check in
  [Imports](#checking-it), made without the import plan. The rest of the tree is compared as
  before.
- **Comments** compares doc comments by parsed content (above), and an import's comments as part
  of the set: each comment of the import section is in the output once, with the import it
  belongs to or at the top.
- **Ranges:** `verify_range(input, range, edits)` checks that the edits are inside the range's
  gaps and rewrites, that applying them gives a file that parses to the same tree and comments (by
  the rules above), and that formatting the result again in the same range gives no edits.

## Testing

- **Suites,** as the exit criteria ask: `crates/ferrion_formatter/tests/snapshots/imports/`,
  `javadoc/`, and `markdown/`, inputs with expected outputs, every one also run through `verify`
  and, except the departures, through `cargo format-check snapshots` against GJF. Each observed
  rule above gets a snapshot, and so does each departure and each instability. Ranges get
  `tests/ranges/`: an input, a range, and the expected output, with and without syntax errors.
- **The range test** (exit criterion): `cargo format-check ranges` takes each corpus file, picks
  tokens at random with a fixed seed, and for each one makes a broken copy (the token deleted, or
  replaced by one that can't follow it), parses it, and checks that every unit the error doesn't
  affect gives, through `format_range`, the text the clean file's `format_tree` gives there. It
  runs on one project in the PR tools job and on the whole corpus weekly, as `format-check verify`
  does.
- **Fuzzing:** the `format` target also formats a range picked from the input and checks
  `verify_range`, and that formatting the whole result gives what formatting the whole input gives
  (a range never takes the file somewhere whole-file formatting wouldn't). The doc comment
  targets in `fuzz/` (`javadoc`, `markdown`) gain a reflow check: reflowing a parsed comment at a
  random column, then again, gives the same text, and its parsed content matches. Reduced failures
  become snapshots.
- **The A1 properties** hold across the corpus and the javac tests (`cargo format-check verify`),
  now with the rewrites on.

## The GJF comparison

- **Flags:** `--skip-javadoc-formatting`, `--skip-sorting-imports`, and
  `--skip-removing-unused-imports` are dropped from `tools/format-check/gjf.toml`, each when its
  workstream lands, with `gjf-diff.toml` updated each time. The snapshots tool drops them too.
- **Left out,** as before: files GJF rejects, files with formatter markers, and files Ferrion
  skips.
- **The measure** stays files identical and lines differing per project; the doc comment
  departures are counted apart, as text blocks are, so what's left to match is visible.

## Work order

On `a2/integration`, at most four agents at once, heavy jobs through the shared slot lock.

1. **Core (one workstream, first):** the `Edit` record in the printer and `format_edits`, checked
   against `format_tree`; `VerbatimKind::Doc`, printing as A1 did so nothing changes yet; a hook in the compilation unit rule for a rewritten import section; and `verify`'s
   new comparisons as no-ops the workstreams fill in. Small, and it gives the four workstreams
   disjoint files.
2. **In parallel, one each:**
   - **Imports:** the section, ownership, order, usage, removal, the `verify` set comparison, the
     suite, and the GJF import flags.
   - **Javadoc:** the layout table, fill, the rules and fallback above, the `verify`
     normalization, the suite, the fuzz check, and the Javadoc flag. It writes the fill shared with
     Markdown.
   - **Markdown:** its rules and departures, on the Javadoc workstream's fill once that lands (it
     starts with the block structure and fallback test, which don't need it).
   - **Ranges:** `error_unit`, copied error units, `format_range`, `verify_range`, the range suite,
     `cargo format-check ranges`, and the fuzz target's range check.
3. **The GJF diff** with the three flags gone, working down the largest differences, and
   `docs/style.md`'s new sections (imports, doc comments) and departures.
   Done: 17,375 lines differ over the corpus, down from 18,176, with 95.9% of the files identical.
   Of those, 13,666 are text blocks and about 3,300 long `//` comments (SCOPE.md), most of the rest
   `<pre>` indentation and marker comments between imports (departures), and about 200 are small
   cases left as they are: mostly comments in unusual places, and a `Map.ofEntries(...)` chain
   ending in `.stream()`, whose arguments are at +8 where GJF's are at +4.
4. **Exit check** against the roadmap's A2 criteria.

## Departures proposed

Each goes in `docs/style.md` with its snapshot, once approved:

| Input | GJF 2.0.0 | Ferrion | Reason |
| --- | --- | --- | --- |
| `java.lang.String`, `java.lang.*`, or the package's own `p.*` imported | removed | kept | SCOPE.md: needs type information |
| an unused import with a comment | removed, its comment moved to the import before | kept | SCOPE.md |
| a duplicate import with a comment | merged, the comment dropped | kept | a comment is lost |
| a header comment after a blank line, a block comment between imports, or a comment inside an import | the file is rejected | header to the top; others move with their import | SCOPE.md |
| a comment on the line after the last import | moves with that import | stays with what follows | comments design |
| formatter markers or `ferrion-disable*` in the imports | sorted with the imports | the section isn't rewritten | SCOPE.md: directives never move |
| a range far from the imports | imports sorted and pruned | imports untouched | decided at the start |
| a range touching a declaration's header but not all of its modifiers | the header formatted, the modifiers not reordered | reordered | decided in review: rewrites apply inside the expanded range |
| `<pre>` content indented past the stars | a common margin removed | kept | changes the rendered `<pre>` |
| a `///` paragraph wrapped before `-`, `#`, `>`, `1.`, `` ``` `` | breaks there, making a list, heading, quote, or fence; `#` and `` ``` `` unstable | the word before moves down | changes meaning, unstable |
| `` `code   span` `` in `///` | spaces collapsed | kept | changes meaning |
| a two-space hard break in `///` | dropped, lines joined | written with `\` | changes meaning |
| a long heading in `///` | wrapped, the rest a paragraph on the next run | one line | changes meaning, unstable |
| two spaces after a list marker in `///` | kept, later paragraphs leave the item on the next run | one space | changes meaning, unstable |
| a block first in a list item in `///` | the marker alone before a blank line | on the marker's line | unstable |
| an HTML block in `///` | `<div>`, `<p>`, `<ul>` reflowed; a quote's `>` dropped | copied whole | changes meaning |
| a hyphen-ended word at a `///` line's end | breaks after it, joined on the next run | no break after it; joined | unstable |
| a `///` comment that can't be reflowed | first line moved, others left | every line moved | misaligned output |
| a `/** */` or `///` comment in a disable region | reformatted | copied | SCOPE.md |
| a long `///` comment that isn't a doc comment | wrapped with `//` | unchanged | SCOPE.md: `//` comments aren't wrapped |

## What was consulted

- Google Java Format 2.0.0's output on inputs written for this document, on Temurin 27, and its
  `--help`. Its source was not consulted.
- The Documentation Comment Specification for the Standard Doclet, JEP 467, CommonMark 0.31, and
  the JLS on imports (7.5), for what the rewrites may change.
- The A1 and Phase 2 designs this document builds on.

## Questions for review

1. **Unused single static imports.** GJF removes them, and it is safe from the syntax alone
   (no identifier with the member's name means no use). SCOPE.md says only "single-type imports",
   so removing them needs a SCOPE.md change. Recommended: remove them, as GJF does, and amend
   SCOPE.md.
2. **Import comments: SCOPE.md's rule or GJF's?** SCOPE.md has a comment belong to the import
   below it; GJF, to the import above, and it rejects files with header comments. They agree on
   files GJF has sorted. Recommended: SCOPE.md's, as written above.
3. **`<pre>` indentation.** GJF removes a common margin from `<pre>` content, which changes what
   javadoc renders. Recommended: keep it as written (a departure), at the cost of a diff on files
   GJF hasn't formatted.
4. **Markdown fallback.** GJF re-indents only the first line of a `///` comment it can't reflow.
   Recommended: re-indent every line by the same amount (a departure).
5. **Range expansion.** Recommended: expand a range to whole statements and declaration headers,
   as GJF does, with rewrites still only inside the expanded range. The alternative is to change
   only the gaps the range covers, which keeps edits smallest but can half-break a statement.
6. **Work order.** A small core step, then four agents (imports, Javadoc, Markdown, ranges), with
   Markdown waiting on the Javadoc fill for its last part. Recommended as written.

## Decisions made in review

The user accepted all six recommendations (2026-11-02):

1. **Unused single static imports are removed,** as GJF does, and SCOPE.md and the roadmap now say
   so.
2. **Import comments follow SCOPE.md:** a comment belongs to the import on its line or directly
   below it, and floating comments move to the top of the section.
3. **`<pre>` indentation is kept** as written, a departure.
4. **A Markdown comment that can't be reflowed** has every line re-indented, a departure.
5. **Ranges expand to units,** as GJF's do, with rewrites only inside the expanded range.
6. **The work order** stands: the core step, then imports, Javadoc, Markdown, and ranges in
   parallel.

From the imports workstream, also accepted (2026-11-02):

7. **Repeated imports of any kind are merged,** as GJF merges them: a repeated on-demand, module,
   or `java.lang` import adds nothing, so dropping it is safe from the syntax alone. SCOPE.md now
   says so.
8. **`verify`'s removal check** is the narrower one in "Checking it".
9. **Unused `java.lang` imports are kept,** as SCOPE.md says, a departure.

From the Javadoc workstream, also accepted (2026-11-02):

10. **Six more Javadoc departures,** each where GJF's output changes on a second run or differs
    from what javadoc reads (docs/style.md, "Doc comments"):
    - a blank line in an inline tag is a space;
    - a comment where a rule would start a line with an `@` word is re-indented only;
    - a block tag named with an uppercase letter stays a tag;
    - no line break right after a word ending with a hyphen;
    - all leading stars go;
    - a comment with a Unicode escape for other than a letter or digit is re-indented only.
11. **A hyphen-ended word at a line's end joins the next line's first word,** as GJF joins them
    (`well-` and `known` become `well-known`). `verify` allows exactly this join.

From the ranges workstream, also accepted (2026-11-02):

12. **A declaration's header in range reorders its whole modifier run,** where GJF reorders it only
    when the range given holds the whole run, a departure.
13. **A range touching the import section expands to the whole section** when formatting rewrites
    it, so the sort and removal apply; a section the plan leaves alone keeps one unit per import.
14. **The parser issues the range test found** (the stray-token error placement in `expr_stmt`,
    and unclosed-block recovery not looking past annotations) are follow-ups after A2, recorded
    in "Ranges".
15. **A comment between two units on one line goes with the code formatting puts it beside,** so
    formatting a range and then the whole file agree; GJF goes by the code beside it in the
    input, a departure (docs/style.md).

From the Markdown workstream, also accepted (2026-11-03):

16. **A two-space hard break in `///` is written with `\`,** replacing the fallback first designed,
    which wasn't idempotent.
17. **Four more Markdown departures,** where GJF's output changes on a second run or moves content:
    a heading stays on one line; one space follows a list marker; an item's first block starts on
    its marker's line; HTML blocks are copied whole.
18. **The hyphen rules of decisions 10 and 11 apply to Markdown,** and so does a re-indent-only list
    of its own (docs/style.md, "Departures in Markdown").
19. **The layout's self-check stays,** with its cost to be measured again once Java 23 code is
    common.
