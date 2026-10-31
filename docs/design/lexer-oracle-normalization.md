# Lexer oracle normalization

The Phase 1 exit criteria in [ROADMAP.md](../ROADMAP.md) say the lexer's tokens match the javac
oracle across the corpus for every supported release, after a documented normalization, and that
the tokens reproduce every corpus file byte for byte. `cargo oracle tokens` (`tools/oracle-check`)
checks both. This document is the normalization: how the lexer's output and javac's are put in the
same terms, which differences are allowed, how files javac reports errors for are treated, and
the known differences on input the corpus doesn't have.

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

## Known differences on malformed input

The corpus has no lexical errors, and the comparison stops at javac's first one (see
[Files javac reports errors for](#files-javac-reports-errors-for)). How the lexer and javac
differ on malformed input is therefore checked separately, with one small input per difference in
`tools/oracle-check/differences/`. Next to each input, `<case>.expected` is a snapshot of both
sides in the comparison's terms: javac's items and scanner errors at the latest release, the
lexer's items and errors, the allowed differences that applied, and the first item where they
differ.

- `cargo oracle differences` regenerates every snapshot with javac and fails if one changed. It
  runs in CI's `tools` job, so a new JDK that changes javac's recovery is noticed.
  `cargo oracle differences --bless` writes the new snapshots; update this section with them.
- `cargo test` checks the lexer's side of every snapshot without a JDK, and that this section
  links every case.

Positions below are omitted; the snapshots have them. Each table names the SCOPE.md category its
rows fall in.

### Same tokens, different kinds

javac makes a malformed literal or comment an `ERROR` token. The lexer makes a token of the same
span with the literal's or comment's own kind, and reports the error separately, so the parser,
the formatter, and the language server still see a literal where one was meant. The spans are
the same; only the kind differs. Category: javac error recovery.

| Case | Input | javac | Lexer |
| --- | --- | --- | --- |
| [`unclosed-char-literal.java`](../../tools/oracle-check/differences/unclosed-char-literal.java) | `'a;` | `ERROR 'a` | `CharLiteral 'a`, `UnterminatedCharLiteral` |
| [`empty-char-literal.java`](../../tools/oracle-check/differences/empty-char-literal.java) | `'';` | `ERROR ''` | `CharLiteral ''`, `EmptyCharLiteral` |
| [`unclosed-string-literal.java`](../../tools/oracle-check/differences/unclosed-string-literal.java) | `"abc;` | `ERROR "abc;` | `StringLiteral "abc;`, `UnterminatedStringLiteral` |
| [`invalid-string-escape.java`](../../tools/oracle-check/differences/invalid-string-escape.java) | `"a\qb"` | `ERROR "a\qb"` | `StringLiteral "a\qb"`, `InvalidEscape` on `\q` |
| [`unclosed-text-block.java`](../../tools/oracle-check/differences/unclosed-text-block.java) | `"""` then lines with no closing `"""` | `ERROR` from `"""` to the end of its first content line | `TextBlock` with the same span, `UnterminatedTextBlock` |
| [`unclosed-block-comment.java`](../../tools/oracle-check/differences/unclosed-block-comment.java) | `/* abc` to the end of the file | `ERROR` to the end of the file | `BlockComment` to the end of the file, `UnterminatedComment` |
| [`char-literal-escaped-line-break.java`](../../tools/oracle-check/differences/char-literal-escaped-line-break.java) | `'\` and a line break, then `';` | `ERROR` from `'` across the line break to `'`, `illegal.esc.char` | `CharLiteral` with the same span, `InvalidEscape` on `\` and the line break |
| [`malformed-numbers.java`](../../tools/oracle-check/differences/malformed-numbers.java) | `0x.p1` | `ERROR 0x.` | `DoubleLiteral 0x.`, `MalformedNumber` |

In the same file, `0x`, `1e+`, `0b1012`, and `1_` have javac's spans and kinds. The lexer's
error points at the bad character where there is one (the `2` in `0b1012`, the `_` in `1_`). javac's
scanner reports nothing for `0b1012`; its parser reports "illegal digit in a binary literal".
The lexer's one `MalformedNumber` kind covers what javac reports as `invalid.hex.number`,
`malformed.fp.lit`, `illegal.underscore`, and illegal digits.

### Deliberate departures in recovery

Where javac's recovery splits one mistake into several errors, the lexer keeps one token and
reports one error at the mistake. Category: deliberate departure.

| Case | Input | javac | Lexer | Why |
| --- | --- | --- | --- | --- |
| [`multiple-chars-in-char-literal.java`](../../tools/oracle-check/differences/multiple-chars-in-char-literal.java) | `'ab';` | `ERROR 'a`, `IDENTIFIER b`, `ERROR ';`, two `unclosed.char.lit` errors; the `;` is lost | `CharLiteral 'ab'`, `MultipleCharsInCharLiteral`; the `;` is kept | Most likely a string written with single quotes. One error where the mistake is, and the parser still sees a complete statement. |
| [`text-block-opened-mid-line.java`](../../tools/oracle-check/differences/text-block-opened-mid-line.java) | `"""abc""";` | `ERROR """`, `abc` lexed as code, `ERROR """`, two `illegal.text.block.open` errors | one `TextBlock` up to the closing `"""`, `TextBlockOpeningNotOnOwnLine` on `abc` | javac lexes the block's content as code, where a `'` or `"` in it starts a new literal and errors cascade. The intent is clear, so the block stays one token. |
| [`dot-dot.java`](../../tools/oracle-check/differences/dot-dot.java) | `a..b` | `ERROR ..`, `illegal.dot` | `Dot`, `Dot`, no lexer error | The JLS has no `..` token, and longest match gives two `.` (JLS §3.2, §3.11). The parser reports the error, with a message about what it expected. |

### javac recovery the lexer doesn't follow

After some errors, javac's `ERROR` token runs to the end of the file. The lexer ends a character
or string literal at the end of its line, since a line terminator can't be in one (JLS §3.10.4,
§3.10.5), so an error never changes how later lines are lexed. That matters to the language
server, which lexes files mid-edit. The one exception follows javac: a character literal whose
character is a line terminator, or `\` and a line terminator, takes in a `'` that starts the next
line, which would otherwise open a literal of its own. Category: javac error recovery.

| Case | Input | javac | Lexer |
| --- | --- | --- | --- |
| [`char-literal-invalid-escape-unclosed.java`](../../tools/oracle-check/differences/char-literal-invalid-escape-unclosed.java) | `'\ ;` | one `ERROR` from `'` to the end of the file, `illegal.esc.char` | `CharLiteral '\ ` (`UnterminatedCharLiteral`, `InvalidEscape`), then the rest of the file as usual |
| [`char-literal-three-quotes.java`](../../tools/oracle-check/differences/char-literal-three-quotes.java) | `''';` | `ERROR ''`, then one `ERROR` to the end of the file, `empty.char.lit` | `CharLiteral ''` (`EmptyCharLiteral`), `CharLiteral ';` (`UnterminatedCharLiteral`), then the rest of the file as usual |

### Tokens javac doesn't produce

Every byte is in exactly one of the lexer's tokens, which javac's tokens don't promise. Category:
deliberate departure.

| Case | Input | javac | Lexer |
| --- | --- | --- | --- |
| [`invalid-unicode-escape-alone.java`](../../tools/oracle-check/differences/invalid-unicode-escape-alone.java) | `int a; \u12 int b;` | no token covers `\u12`; `illegal.unicode.esc` | `Error \u12`, `InvalidUnicodeEscape` |
| [`sub-at-end-of-file.java`](../../tools/oracle-check/differences/sub-at-end-of-file.java) | a SUB (control-Z) ending the file | `EOF` before the SUB | the SUB is a `Whitespace` token (JLS §3.5), so `EOF` is after it |

When an invalid Unicode escape follows another token's text, both put it in that token
(`1\u12` is one `IntLiteral`), so only an escape after whitespace, a line terminator, or at the
start of the file becomes an `Error` token.

## Allowed differences

Disagreements with javac are allowed only as this list, in SCOPE.md's categories. Each one is in
`tools/oracle-check/src/allowed.rs`, and the report counts how often it applied.

| Id | Category | Difference |
| --- | --- | --- |
| `byte-order-mark` | deliberate departure | javac reads a UTF-8 byte-order mark as an illegal character. Ferrion records the mark, so the formatter can keep it, and lexes the text after it. See [Byte-order mark](#byte-order-mark). No corpus file has a mark; [`byte-order-mark.java`](../../tools/oracle-check/differences/byte-order-mark.java) checks it. |
| <a id="empty-block-comment"></a>`empty-block-comment` | deliberate departure | javac styles `/**/` as `JAVADOC_BLOCK`, like `/***/` and `/** */`. It has no content, and the lexer makes it a `BlockComment`, so it isn't taken for documentation. Before comparing, the lexer's `/**/` is given javac's style. No corpus file has one; [`empty-block-comment.java`](../../tools/oracle-check/differences/empty-block-comment.java) checks it. |
| `line-comment-at-end-of-file` | javac quirk | javac doesn't report a `//` comment that ends the file with no line terminator after it (`class A {}\n// x`). It does report one followed by a line terminator, and `///` and `/* */` comments in the same place. The lexer's comment is left out before comparing. |

The javac error recovery category has no entries here: the comparison stops at javac's first
lexical error, so recovery differences never reach it. They're listed under
[Known differences on malformed input](#known-differences-on-malformed-input) instead. A new entry
needs an input that shows javac's behavior, a reason, and a row here.

## Changing this

When the lexer's API, javac's behavior (a new JDK), or the corpus changes what's compared, update
the comparison in `tools/oracle-check`, its unit tests, and this document together.
