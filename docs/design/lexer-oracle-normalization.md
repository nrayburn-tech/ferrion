# Lexer oracle normalization

The Phase 1 exit criteria in [ROADMAP.md](../ROADMAP.md) say the lexer's tokens match the javac
oracle across the corpus for every supported release, after a documented normalization, and that
the tokens reproduce every corpus file byte for byte. `cargo oracle tokens` (`tools/oracle-check`)
checks both. This document is the normalization: how the lexer's output and javac's are put in the
same terms, which differences are allowed, and how files javac reports errors for are treated.

Everything about javac here was learned by running the oracle on small inputs written for the
purpose, never from javac's source. When javac's behavior is in doubt, run the oracle on an input
that shows it, and update this document and the comparison together.

## What is compared

For each file and release:

- **Ferrion:** `ferrion_lexer::lex` on the file's decoded text. The lexer doesn't depend on the
  release, so it runs once per file, whatever the number of releases.
- **javac:** the oracle's `tokens`, `comments`, and `scanDiagnostics` sections
  (`--sections tokens,comments,scanDiagnostics`). They come from scanning the file with javac's
  scanner, separately from parsing, so the parser's error recovery never affects them.

Tokens and comments are compared as one sequence of items in source order. Each item has a class
(token or comment), a kind as javac names it, and a UTF-16 span. Two files match when the
sequences are equal, item for item.

## Reading files

Files are read with `SourceText::decode`, as Ferrion reads them. Files that aren't UTF-8 or ASCII
are **skipped**: they're counted and listed with the reason, but they aren't failures, and javac
isn't run on them.

## Round trip

Independently of javac, the tokens must:

- start at byte 0, each one where the previous one ends, with no gaps or overlaps;
- fall on character boundaries within the text;
- end with exactly one `Eof`, which is empty and last, at the end of the text;
- reproduce the file's bytes when their text is concatenated after the byte-order mark, if any.

A file that fails any of these is a round-trip failure, whatever the comparison with javac finds.

## Normalizing Ferrion's tokens

### Trivia

Whitespace and line terminators are dropped. javac doesn't report them, and the round trip
already checks that the lexer accounts for every byte. Comments are kept, as comment items.

### Positions

The lexer's spans are byte offsets into the text after any byte-order mark. javac's are UTF-16
offsets into the file as javac reads it, which includes the mark as one character (see
[Byte-order mark](#byte-order-mark)). Offsets are converted with `LineIndex::utf16_offset`, and 1
is added in a file with a mark. Reports show positions in javac's terms, with lines and columns
starting at 1 and columns in UTF-16 units.

### Kind names

Each `TokenKind` has the javac name of the same token (`javac_name` in
`tools/oracle-check/src/tokens.rs`, whose `match` has no catch-all, so a new kind can't be missed).
Keywords and literal keywords are their spelling in capitals (`CLASS`, `TRUE`, `NULL`), and the rest
are:

| Lexer | javac | Lexer | javac | Lexer | javac |
| --- | --- | --- | --- | --- | --- |
| `Identifier` | `IDENTIFIER` | `At` | `MONKEYS_AT` | `Minus` | `SUB` |
| `Underscore` | `UNDERSCORE` | `ColonColon` | `COLCOL` | `MinusMinus` | `SUBSUB` |
| `IntLiteral` | `INTLITERAL` | `Question` | `QUES` | `MinusEq` | `SUBEQ` |
| `LongLiteral` | `LONGLITERAL` | `Pipe` | `BAR` | `PipePipe` | `BARBAR` |
| `FloatLiteral` | `FLOATLITERAL` | `PipeEq` | `BAREQ` | `Error` | `ERROR` |
| `DoubleLiteral` | `DOUBLELITERAL` | `CharLiteral` | `CHARLITERAL` | `Eof` | `EOF` |
| `StringLiteral` | `STRINGLITERAL` | `TextBlock` | `STRINGLITERAL` | | |

The other separators and operators have the obvious names: `LPAREN`, `SEMI`, `ELLIPSIS`, `ARROW`,
`EQEQ`, `LTLTEQ`, `PERCENTEQ`, and so on. javac has no kind for text blocks: they're
`STRINGLITERAL`s. Of javac's kinds, only `STRINGFRAGMENT` (string templates, which never left
preview) and `CUSTOM` have no lexer kind; javac doesn't produce them for Java source.

### `>` and `>=`

The lexer never makes a token longer than `>=` that starts with `>`, and the parser joins adjacent
ones. javac's scanner takes the longest of `>>>=`, `>>>`, `>>=`, `>>`, `>=`, and `>`, and its
parser splits them. A run of adjacent `Gt` tokens, optionally ending in a `GtEq`, is joined the way
javac's scanner reads the same characters:

| Lexer | javac |
| --- | --- |
| `Gt Gt` | `GTGT` |
| `Gt Gt Gt` | `GTGTGT` |
| `Gt GtEq` | `GTGTEQ` |
| `Gt Gt GtEq` | `GTGTGTEQ` |
| `Gt Gt Gt Gt` | `GTGTGT GT` |
| `Gt Gt Gt GtEq` | `GTGTGT GTEQ` |
| `GtEq Gt` | `GTEQ GT` |

Tokens are adjacent when nothing, not even a comment or whitespace, is between them. javac lexes
`List<List<String>>` with a `GTGT`, like any other `>>`.

### Contextual keywords

javac's scanner reports every contextual keyword (`var`, `record`, `yield`, `sealed`, `permits`,
`when`, `module`, ...) as an `IDENTIFIER`, as the lexer does. `non-sealed` is
`IDENTIFIER SUB IDENTIFIER` in both. `_` is an `UNDERSCORE` at every release, 8 included, where
javac's parser, not its scanner, warns about it; the lexer's `Underscore` matches. No normalization
is needed.

### Comments

| Lexer | javac |
| --- | --- |
| `LineComment` | `LINE` |
| `BlockComment` | `BLOCK` (but see [`/**/`](#empty-block-comment)) |
| `DocComment` | `JAVADOC_BLOCK` |
| a run of `DocLineComment`s | one `JAVADOC_LINE` |

javac reports consecutive `///` lines as one comment, from the start of the first line to the end
of the last. A run continues while the next line, after any whitespace, starts with a
`DocLineComment`. Observed:

- a blank line ends a run, even one with spaces or tabs on it;
- a line that is only `///` continues it;
- a `///` comment after code on the same line starts a run, which continues on the next line;
- a `//` comment ends it, and `/** */ /// a` starts a run after the block comment;
- `////` and `/////` lines are `JAVADOC_LINE`s too, alone or within a run.

So the lexer's tokens `DocLineComment [Whitespace] LineTerminator [Whitespace] DocLineComment`
become one item, and so on for longer runs.

Markdown documentation comments arrived in Java 23, but javac 27's scanner reports `///` comments
as `JAVADOC_LINE` at every release from 8 on, and groups them the same way at every release. The
lexer's `DocLineComment` doesn't depend on the release either, so no release-specific mapping is
needed.

Comment ends come from javac (`Comment.getPos().getEndPosition()`), so a line comment ended by an
escaped line terminator (`\u000a`) ends before the escape, as the lexer's does.

## Normalizing javac's output

### Byte-order mark

The oracle reads files as UTF-8, which keeps a byte-order mark as the character U+FEFF. javac's
scanner reports it as an `ERROR` token at `0..1`, with an `illegal.char` error. In a file with a
mark, that token and that error are dropped, and the
[`byte-order-mark`](#allowed-differences) difference is counted. If javac's first token isn't that
`ERROR`, nothing is dropped, and the comparison reports the difference. No corpus file has a mark
today.

## Files javac reports errors for

SCOPE.md excludes code where javac itself reports errors from agreement. For tokens:

- **Errors from parsing** don't affect the comparison: javac's tokens come from a separate scan, so
  a Java 17 file compared at release 8 is compared in full, records and all, even though javac
  rejects it. Diagnostics will be compared in Phase 2.
- **Language-version errors from the scanner** (codes starting `compiler.err.feature.not.supported`
  or `compiler.err.preview.feature.disabled`, such as "text blocks are not supported in -source 8")
  don't affect it either. javac lexes the text the same way at every release, and the comparison
  would show it if it didn't.
- **Lexical errors** (every other error the scanner reports, such as `illegal.char`,
  `unclosed.str.lit`, or `illegal.esc.char`) stop the comparison at the start of the javac token
  holding the first one. What javac produces from there on depends on how it recovers, which
  Ferrion doesn't follow. Items before that point must still match. These comparisons are counted
  in the `javac-errors` column and listed as notes, but aren't failures in themselves.

Warnings never affect the comparison. When this was written, no corpus file had a lexical error at
any release, and 59 files had text-block version errors at each of releases 8 through 14.

## Allowed differences

Disagreements with javac are allowed only as this list, in SCOPE.md's categories. Each one is in
`tools/oracle-check/src/allowed.rs`, and the report counts how often it applied.

| Id | Category | Difference |
| --- | --- | --- |
| `byte-order-mark` | deliberate departure | javac reads a UTF-8 byte-order mark as an illegal character. Ferrion records the mark, so the formatter can keep it, and lexes the text after it. See [Byte-order mark](#byte-order-mark). |
| <a id="empty-block-comment"></a>`empty-block-comment` | deliberate departure | javac styles `/**/` as `JAVADOC_BLOCK`, like `/***/` and `/** */`. It has no content, and the lexer makes it a `BlockComment`, so it isn't taken for documentation. Before comparing, the lexer's `/**/` is given javac's style. |
| `line-comment-at-end-of-file` | javac quirk | javac doesn't report a `//` comment that ends the file with no line terminator after it (`class A {}\n// x`). It does report one followed by a line terminator, and `///` and `/* */` comments in the same place. The lexer's comment is left out before comparing. |

The javac error recovery category has no entries. A new entry needs an input
that shows javac's behavior, a reason, and a row here.

## Changing this

When the lexer's API, javac's behavior (a new JDK), or the corpus changes what's compared, update
the comparison in `tools/oracle-check`, its unit tests, and this document together.
