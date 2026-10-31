# Comments

How comments relate to nodes: what the tree stores, how a comment is associated with a node, which
comment is a declaration's documentation, and what the formatter's `@formatter:off` regions and the
linter's suppression comments need. Part of the [parser design](parser.md).

## Decision

- **Comments stay tokens.** Comments and whitespace are trivia tokens in the tree's token array
  (see [syntax tree](parser-syntax-tree.md)). No node owns a comment, and the parser makes no
  attachment decision.
- **Attachment is computed on demand.** `ferrion_syntax::comments` classifies each comment as
  leading, trailing, or dangling, relative to a node or a token, from the tree and the line breaks
  around the comment. The formatter and the linter use the same classification, so they agree on
  which declaration or statement a comment belongs to.
- **Documentation comments follow javac.** A declaration's doc comment is the last doc comment
  before its first token, as javac decides it (see [Doc comments](#doc-comments)).
- **Directives are found by their consumers.** The formatter finds `@formatter:off` regions, and the
  linter finds suppression comments, by scanning comment tokens. The tree provides in-order token
  iteration and node ranges; nothing about directives is stored in it.

The reasons:

- **Lossless by construction.** A comment can't be lost or duplicated by an attachment bug, because
  attachment never moves it.
- **One parse, several policies.** The formatter, the import sorter, and suppression comments each
  want a different answer to "whose comment is this" (see below). Baking one answer into the tree
  would force the others to undo it.
- **Attachment depends on line breaks**, which the formatter is about to change; deciding it from
  the input, on demand, keeps that dependency visible.
- **No cost when unused.** Syntax-only lint rules that ignore comments pay nothing, and the
  classification is one linear pass when it is needed.

## Classification

For each comment token, in source order:

1. **Enclosing node:** the innermost node whose range contains the comment. By the tree's
   invariants, that is the node owning the trivia around the comment. Its **elements** are its
   child nodes and its direct tokens other than trivia.
2. **Preceding** and **following:** the last element ending before the comment, and the first
   starting after it. An opening `(`, `[`, or `{` doesn't count as preceding below, and a closing
   `)`, `]`, `}`, or the `Eof` doesn't count as following, except where the trailing rule says.
3. Then:
   - With neither, **dangling** in the enclosing node: `{ /* nothing */ }`, `f(/* none */)`, a
     file of comments.
   - **Trailing** of the preceding element, delimiters included, if the comment starts on the line
     where that element ends, isn't a documentation comment, and is a line comment, is followed by
     a line break before the next code, or has no following element: `foo(); // why`,
     `{ // why`, `f(a /* why */)`.
   - Otherwise **leading** of the following element: `// why` on the line above `foo();`.
   - Otherwise trailing of the preceding element (a comment at the end of a block, after the last
     statement).

A line break is a line terminator, or a block comment that spans lines. The following element is
the outermost node starting at the next token, so a comment above `foo();` leads the statement, not
the call inside it. A documentation comment (`/** */`, or `///` from release 23) never trails, so a
declaration's doc comment leads it even after code on the same line (`int x; /** y */` then
`int y;` on the next line).

The preceding and following elements can be tokens as well as nodes: in `if /* c */ (x)`, the
comment leads the `(` token, and in `f(a, // c` it trails the `,`. The formatter needs that to keep
it between `if` and `(`.

The result is a `CommentMap`: for each comment token, its placement, its element, and its enclosing
node, plus, for each node and token with comments, the list of its leading, trailing, and dangling
comments, found by binary search. It also records the number of line breaks before and after each
comment, up to the nearest token that isn't whitespace, which is what "on its own line" and
"directly above with no blank line" mean.

Building it is one pass over the tokens with a stack of open nodes, then a sort of the comments by
element; nothing recurses. On a generated 20 MB file with 1.2 million comments (5.6 million tokens),
it took 33 ms in a release build on an Apple M1 Pro, and 38 ms for 2 million comments leading one
declaration; finding every comment's list again took about 0.2 µs per comment.

### What consumers build on it

- **Formatter:** prints leading comments before an element, trailing comments after it, and
  dangling comments inside its delimiters. Safety property 3 (every comment appears exactly once) is
  checked by comparing comment tokens of the input and the output.
- **Imports (A2):** a comment belongs to an import if it is trailing it, or leading it with no blank
  line between. Other comments in the import section (a group header followed by a blank line) are
  "floating" and move to the top of the section. That is the formatter's rule, on top of the
  classification.
- **Modifier reordering:** a comment inside a modifier list (leading or trailing a modifier token)
  pins the modifiers around it; the formatter's rules decide how.

## Doc comments

javac attaches at most one doc comment to a declaration. Which one it attached was observed with
javac 27 through the Compiler Tree API (`DocTrees.getDocComment` on each tree of a parsed file).
`-Xlint:dangling-doc-comments` shows the rule too, but doesn't report every comment javac leaves
unattached: none in method bodies, before a closing `}`, or at the end of the file.

| Input | Doc comment of the declaration |
| --- | --- |
| `/** a */ /** b */ void m()` | `/** b */`; `/** a */` is dangling |
| `/** a */ /* x */ void m()` | `/** a */` |
| `/** a */ // y` then `void m()` on the next line | `/** a */` |
| `/** a */ @Deprecated void m()` | `/** a */` |
| `@Deprecated /** a */ void m()` | none; the comment is dangling |
| `public /** a */ void m()` | none; the comment is dangling |
| `/** a */ <T> void m()` | `/** a */`; after the `<T>`, none |
| `/// a` then `/** b */` then `void m()` | `/** b */`; `/// a` is dangling |
| `/** a */` then `/// b` then `void m()` | `/// b` |
| `/// a`, a blank line, `/// b`, then `void m()` | `/// b`: a blank line ends a run |
| `/** a */ ; void m()` in a class body | none |
| `/** a */ int x = 1;` inside a method | none, and the lint doesn't report it |
| `/** a */ class L {}` inside a method | `/** a */` |
| `record R(/** a */ int x)` | none, and the lint reports it |

So the doc comment of a declaration is the **last** doc comment before the declaration's first
token, modifiers and annotations included, with only whitespace and other non-doc comments between
them. `ferrion_syntax::comments::doc_comment` implements that rule, and each declaration view
offers it as `doc_comment()`: `ast::ClassDecl::doc_comment()`, and so on for packages
(`package-info.java`), modules, classes, interfaces, enums, records, and annotation types (local
ones included), fields, methods and annotation type elements, constructors, compact constructors,
and enum constants, and for the `TypeDecl` and `Member` enums. javac attaches none to record
components, initializers, parameters, local variables, statements, imports, or module directives,
so those have none.

Where Ferrion differs from javac 27:

- **A stray `;` at the start of a file.** For `/** a */ ; class T {}`, javac attaches `/** a */` to
  the class, and in `/** a */ ; /** b */ class T {}` it prefers `/** a */`. Only the file's first
  token behaves this way: after a package or another declaration, the `;` leaves the comment
  dangling, as it does in a class body. Ferrion treats the `;` as code everywhere, so the class
  has no doc comment in the first case and `/** b */` in the second.
- **The compilation unit.** javac also records a doc comment before the first import, when there
  is no package declaration, as the compilation unit's, and the lint still reports it as
  dangling. Nothing reads it, and Ferrion's compilation units have no doc comment.
- **`/**/`** is an empty block comment to the lexer, not a doc comment (see
  [lexer oracle normalization](lexer-oracle-normalization.md#empty-block-comment)), so it never
  documents anything. javac attaches it like `/***/`.
- **`///` below release 23,** described next.

**`///` comments** are documentation only at Java 23 and later. Below 23 they are plain line
comments: they are never a declaration's doc comment, and the formatter doesn't reflow them. That
is what javac 21 does; javac 27 attaches them at every `--release`, but a project on an older
release builds with an older javac, and its javadoc ignores them. The tree records the release it
was parsed for, so `doc_comment()` answers for that release. A run of `///` lines is one doc
comment, grouped as javac groups them (see
[lexer oracle normalization](lexer-oracle-normalization.md#comments)) and as
`ferrion_doc_comments::markdown_comment_runs` does.

The parser doesn't parse a doc comment's contents. `doc_comment()` returns the comment's tokens and
span, and `ferrion_doc_comments` parses them when a consumer needs it: `parse_javadoc` with the
span of a `/** */` comment, and `parse_markdown` with the spans of a run's lines.

Doc comments in the classification are ordinary comments: a declaration's doc comment is also its
leading comment. Dangling doc comments are ordinary comments too; a lint rule can report them, and
the formatter re-indents them without reflowing.

## Formatter disable regions

`@formatter:off` and `@formatter:on`, in `//` or `/* */` comments, mark text left exactly as
written. An `off` with no matching `on` runs to the end of the file.

- **Finding regions** is a linear scan of comment tokens, done by the formatter. A region runs from
  the start of the `off` comment to the end of the `on` comment, or to the end of the file.
- **Printing:** regions don't follow node boundaries; an `off` can sit in one method and its `on`
  in the next. The formatter walks every token in order, so it can switch to copying text verbatim
  at any token and switch back after the region, whatever the nesting. Nodes that straddle a
  boundary are formatted outside the region and copied inside it.
- **Modifiers are never reordered across a marker:** the formatter checks whether a marker comment
  lies inside a modifier list's range.
- **Directives are never reflowed or moved:** marker and `ferrion-disable*` comments are excluded
  from any comment rewriting by their text.

The tree's part is in-order token iteration across node boundaries and node ranges, which it has.

## Suppression comments

`// ferrion-disable-next <rule> -- <reason>` applies to the next statement or declaration,
excluding the contents of any nested block. The linter parses comment text; the tree answers the
structural questions:

- **Target:** the first statement or declaration that starts after the comment and has the
  comment's enclosing node as its parent. Usually the comment is that statement's leading comment.
  A comment before the `}` that closes a block has no target, and the linter reports it as
  suppressing nothing. Several stacked `ferrion-disable-next` comments have the same target, so
  each applies to it.
- **Coverage:** the target's range minus the ranges of its nested bodies: block bodies of methods,
  constructors, initializers, lambdas, and control statements, and class bodies. Above a method, the
  signature is covered and the body isn't; above an `if`, the condition is covered and the branches
  aren't. Typed views list each kind's body children, so the linter computes coverage without
  kind-specific code of its own.
- `ferrion-disable` and `ferrion-enable` regions and `ferrion-disable-file` are text ranges, found
  by scanning comments, like formatter regions.

## Blank lines

Blank lines are not stored either. The number of line breaks between two tokens is a count of
`LineTerminator` tokens in the trivia between them, which the formatter reads where its style keeps
or collapses blank lines.
