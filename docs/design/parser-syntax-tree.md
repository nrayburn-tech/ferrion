# Syntax tree

How the parser's output is laid out in memory, how consumers reach nodes and tokens, how nodes are
identified, and how the tree gives the positions javac's oracle reports. Part of the
[parser design](parser.md).

## Decision

The syntax tree is a **flat, lossless concrete syntax tree** held in two arrays:

- **tokens:** the lexer's tokens, unchanged and in order, trivia included. Every byte of the text is
  in exactly one token, so the tree reproduces the file byte for byte.
- **nodes:** one entry per node, in pre-order. A node covers a contiguous range of tokens and
  records its kind, its parent, and where its subtree ends.

Typed views (`ast::IfStmt`, `ast::BinaryExpr`, ...) are generated from a grammar file and sit on
top: each is a node handle plus accessor methods. A node is identified by its index, a `NodeId`,
which is dense and stable for the life of one tree.

This is a CST like a green tree (option a below) in what it holds, and like an arena (option c) in
how it is stored. The main reasons:

- it is the smallest and fastest of the options measured, by a wide margin for building and walking;
- dropping it never recurses, so a deep tree can't overflow the stack (see
  [deep nesting](parser-deep-nesting.md));
- `NodeId` is a dense index, so semantic analysis keys side tables by node with a `Vec`, not a hash
  map;
- it is plain data, so it is `Send` and `Sync`, and the language server shares one tree across
  threads behind an `Arc`;
- it needs no `unsafe` code, which the workspace denies.

## What the tree must serve

- **Round trip:** the tree reproduces the file byte for byte, including comments, whitespace, and
  malformed tokens.
- **Formatter:** visits every token, trivia included, in source order, with the node structure
  around it. Reads, never mutates: import removal and modifier reordering are decisions made while
  printing.
- **Linter:** syntax-only rules match node shapes; fixes are text edits computed from token spans,
  and the fix engine reparses after each pass. No rule edits a tree.
- **Language server:** reparses a file on each change and shares the result across request threads.
- **Semantic analysis (Track B):** keys data (scopes, symbols, attributed types) by node; extracts
  per-file declaration summaries quickly.
- **Error recovery:** holds partial trees with error nodes and missing children.
- **javac comparison:** gives each javac tree node's start and end (see
  [Matching javac's tree positions](#matching-javacs-tree-positions)).

## Options considered

### (a) Hand-written green/red tree

Immutable green nodes (kind, text length, children), shared through `Arc` and interned so that equal
subtrees are stored once. Red nodes are created while walking, and add parent pointers and absolute
offsets. Trivia are tokens. Typed views on top.

- **Memory and allocation:** one allocation per node and per distinct token. Making it compact needs
  thin pointers and a tagged child representation, which need `unsafe`. Without them it was the
  largest option measured.
- **Typed access:** scan the children for the first that casts; cheap on green nodes, but offsets
  and parents need red nodes.
- **Identity:** a green node has no identity (it is shared), so side tables key by
  (kind, range) through a hash map.
- **Threads:** green nodes are `Send` and `Sync`. Red nodes can be too, at the cost of atomic
  reference counts.
- **Deep nesting:** dropping an `Arc` tree recurses; a 1,000,000-deep chain overflowed a 2 MiB
  stack. An iterative drop is possible but has to be written.
- **Structural sharing** makes incremental reparsing and tree editing cheap. Ferrion needs neither
  (see [Incremental reparsing](#incremental-reparsing)).
- **Code:** the tree, builder, interner, red layer, and iterators, roughly 2,000 to 3,000 lines.

### (b) The `rowan` crate

The same design as (a), maintained by the rust-analyzer project, MIT OR Apache-2.0, with its
`unsafe` code inside the dependency.

- **Memory:** compact; smaller than (a), larger than the flat tree.
- **Walking:** red nodes are allocated and reference counted as they are visited. A full walk took
  about 140 times as long as walking the flat tree.
- **Threads:** `GreenNode` is `Send` and `Sync`; `SyntaxNode`, the red node every typed view wraps,
  is neither (checked by compiling against 0.16.1). A language server thread can only receive a
  green node and rebuild the red root.
- **Deep nesting:** dropping a 100,000-deep tree overflowed a 2 MiB stack, and building a
  1,000,000-deep one overflowed the 8 MiB main stack. Building deep chains slows quadratically: a
  chain 5,000 deep took 80 ms, and one 20,000 deep took 1.25 s.
- **Identity:** as in (a).
- **Code:** none for the tree itself. Typed views still have to be written or generated.

### (c) Arena-allocated typed AST

One struct or enum per node kind (`If { cond: ExprId, then: StmtId, else_: Option<StmtId> }`),
allocated in arenas, with trivia and comments in side tables keyed by position.

- **Typed access** is a field read, the cheapest of all.
- **Identity:** arena indices, dense.
- **Lossless only with extra work:** the formatter needs every keyword and punctuation token and the
  comments between them (`if /* c */ (x)`). Each struct would have to record its tokens, or the
  formatter would have to find them again between child spans. javac positions need the same.
- **Error recovery:** every field becomes optional, and each kind needs a way to hold skipped
  tokens.
- **Generic walks** (comments, version check, "node at offset", javac positions) need a visitor over
  about 150 kinds.
- **Code:** the most of any option: the structs, a builder per kind, side tables, and visitors.

The flat CST keeps (c)'s good properties (dense ids, arrays, no reference counting) and gets the
rest from a single generic node shape.

## Measurements

A throwaway prototype, outside the repository, built each representation from the Ferrion lexer's
tokens with a crude shape parser: a node per block, parenthesized group, bracket group, statement,
and name or literal. That gives 0.61 nodes per non-trivia token; javac's tree has 0.66 on
commons-lang's main sources, and the real parser will have somewhat more than javac. Input: guava's
`guava/src`, commons-lang's `src`, and spring-framework, 10,673 files and 68.6 MB, 12.6 million
tokens (8.2 million non-trivia), 5.0 million nodes. Release build, one thread, Apple M1 Pro.
Memory is what stays allocated once the tree is built.

| Representation | Memory | Per node, tokens included | Allocations | Build | Walk every node and token | Drop |
| --- | --- | --- | --- | --- | --- | --- |
| `rowan` 0.16.1 | 347 MB | 69 B | 3.5 M | 441 ms | 713 ms | 112 ms |
| `Arc` green tree, no `unsafe` | 562 MB | 112 B | 19.1 M | 981 ms | 46 ms | 214 ms |
| flat arrays | 252 MB | 50 B | 49 k | 52 ms | 5 ms | 1 ms |

The flat tree also keeps the source text (69 MB here), since its tokens are spans; the other two
copy token text into the tree. Lexing the same files took 359 ms. The walk computes each node's
start offset and each token's end; for `rowan` it is a preorder walk with tokens over red nodes.

Deep chains, built and then dropped on a thread with a 2 MiB stack:

| Representation | Result |
| --- | --- |
| flat arrays | 1,000,000 levels: built and dropped in 8 ms |
| `Arc` green tree | 20,000 levels: fine. 1,000,000: stack overflow on drop |
| `rowan` 0.16.1 | 20,000 levels: 1.25 s. 100,000: stack overflow on drop. 1,000,000: stack overflow while building |

The flat tree's 50 bytes per node is 20 bytes of node and the rest tokens, at 12 bytes each.
Storing token starts and kinds in separate arrays (ends follow from the next start) would bring
tokens to about 5 bytes. That is an optimization for later, behind the same API.

## Layout

```rust
pub struct SyntaxTree {
    text: Arc<str>,           // the source text, after any byte-order mark
    tokens: Vec<Token>,       // the lexer's tokens: kind and span
    token_kinds: Vec<SyntaxKind>, // the kind as parsed (see "Contextual keywords")
    nodes: Vec<Node>,         // pre-order
    release: u32,             // the release it was parsed for
}

struct Node {                 // 20 bytes
    kind: SyntaxKind,         // u16: token kinds, contextual keyword kinds, node kinds
    flags: u16,               // CONTAINS_ERROR, ...
    parent: NodeId,           // u32; the root's is itself
    first_token: TokenId,     // u32
    token_end: TokenId,       // u32, exclusive
    subtree_end: NodeId,      // u32: one past the last node in this subtree
}
```

Invariants, checked by a debug validator and by the parser fuzz target:

- **Every token belongs to the tree.** The root covers every token, including leading trivia and the
  final `Eof`.
- **Nodes cover contiguous token ranges and nest:** a child's range is inside its parent's, and
  siblings don't overlap.
- **Nodes other than the root start and end on non-trivia tokens.** Trivia between two tokens of a
  node belong to the innermost node that covers both tokens. The trivia before a declaration, its
  doc comment included, are not part of it, and javac's positions don't include them either (see
  [comments](parser-comments.md) for how comments are associated with nodes).
- **No node is empty.** A missing child is absent, not an empty node, and the parser reports it.
  Tokens the parser couldn't place are inside an `ERROR` node.
- **Pre-order:** node `n`'s descendants are exactly `n + 1 .. subtree_end`. Its first child is
  `n + 1` if that is below `subtree_end`, and a child's next sibling is the child's `subtree_end` if
  that is below the parent's. Nodes are sorted by start offset, so the node at an offset is found by
  binary search and a short walk down.
- `CONTAINS_ERROR` is set on every node whose subtree holds an `ERROR` node or a missing child.

Consumers use cursors, not the arrays: `SyntaxNode<'t>` and `SyntaxToken<'t>` are a `&'t SyntaxTree`
and an index, and are `Copy`. They offer kind, span, text, parent, ancestors, children, children
with tokens (the node's direct tokens interleaved with its child nodes), first and last token,
previous and next token, and descendants. Every iterator is a loop over the arrays; none recurses.

The tree owns its text, as an `Arc<str>` shared with the caller rather than a copy, so that it is
`'static`, `Send`, and `Sync`. A thread that needs to hold a node without a borrow keeps an
`Arc<SyntaxTree>` and a `NodeId`.

### Contextual keywords

The lexer makes `record`, `var`, `yield`, `sealed`, and the others `Identifier`s. Where the parser
reads one as a keyword (a record declaration, a yield statement, `sealed` in modifiers, module
directives, a `when` guard), it records the keyword kind in `token_kinds`, so that consumers don't
repeat the parser's decision. `var` and `_` keep their identifier kind: whether `var x` declares an
inferred local or uses a type named `var` depends on the release, and is semantic analysis's call
(see [language versions](parser-language-versions.md)).

Tokens stay one to one with the lexer's, so a `TokenId` is also an index into the lexer's output and
tokens never need splitting or merging. Operators the lexer leaves in parts (`>>`, `>>>=`, and
`non-sealed`, which is three tokens) are runs of adjacent tokens inside their node; typed accessors
return them as one operator or modifier with a span covering the run.

## Building

The parser is hand-written recursive descent (CONTRIBUTING.md rules out parser generators). It
doesn't build nodes directly. It emits events: start a node of a kind, consume a token, finish the
node. A node that turns out to start before one already parsed (the left operand of `a + b`, the
target of `a.b()`) is opened with a marker that can be moved earlier, a technique described in
rust-analyzer's design notes. A builder then turns the events into the arrays in one loop with an
explicit stack. Trivia are skipped by the parser and placed by the builder, which is what keeps
node ranges free of edge trivia.

## Typed views

A grammar file (`crates/ferrion_parser/java.grammar`, in a small format of Ferrion's own) lists each
node kind and its children by role, for example:

```
IfStmt = 'if' '(' condition:Expr ')' then_branch:Stmt ('else' else_branch:Stmt)?
BinaryExpr = lhs:Expr op:BinaryOp rhs:Expr
```

A code generator in `tools/` writes `SyntaxKind`, the typed view structs, the `Expr`/`Stmt`/`Decl`
enums, and their accessors to a checked-in file, and CI checks that it is up to date, as it does for
rule documentation. Generated code is plain to read and review, and adds no proc-macro build time.

An accessor scans the node's direct children (and, for tokens, its direct tokens) for the first that
matches its role. That is linear in the number of children, which is small, over contiguous arrays,
and allocates nothing. Every accessor returns an `Option`, since recovery can leave any child
missing. A view is a `SyntaxNode` with a checked kind, so it is free to create and to convert back.

## Node identity

**Within a tree:** `NodeId` and `TokenId` are `u32` indices. They are dense, ordered by position,
and never change for the life of the tree, which is immutable. `NodeMap<T>` is a `Vec<T>` indexed by
`NodeId`, for side tables that cover most nodes; a hash map keyed by `NodeId` serves sparse ones. A
subtree is a range of ids, so "is a descendant of" is a comparison.

**Across edits:** nothing at the node level survives a change to the text. Every edit produces a new
tree, and ids from the old tree are meaningless in it. That is deliberate:

- **Language server:** it keeps, per open document, the latest version's text, tree, and results.
  Diagnostics and fixes are byte spans, converted to LSP positions with `LineIndex` when they are
  sent. A code action computed for version N is recomputed if the document has moved on. Formatting
  and fixes are computed from the current tree and returned as text edits.
- **Semantic analysis (Track B):** project-wide data must survive edits inside a method body (B2's
  condition), so it can't hold node ids or offsets. Per-file declaration summaries are keyed by a
  declaration path: the enclosing types' names, then the member's kind, name, and, for overloads,
  its parameter types as written. Body-level results (scopes, attributed types) are keyed by
  `NodeId` within one tree and recomputed when the file changes. The B2 spike decides the details;
  the tree provides a fast declaration-only walk, which skips a body by jumping to its
  `subtree_end`.
- **Pointers into a reparsed tree:** where something must find "the same node" in a new tree of
  unchanged text (a cache keyed across reparses of one version, for example), it stores a
  `SyntaxPtr`: the node's kind and span, resolved by searching the new tree.

### Incremental reparsing

Not planned. The language server reparses the whole file on each change. From the lexer's measured
throughput (190 MB/s) and the builder's (above), a 200 KB file, larger than nearly every corpus
file, lexes in about 1 ms and builds in well under that; parsing proper is expected to cost a small
multiple. If profiling of the language server shows otherwise, reparsing only the changed block is
possible later without changing the tree's API.

## Matching javac's tree positions

The oracle's `tree` section gives `[kind, start, end, depth]` for each javac tree node in
pre-order. The comparison is in `parser-oracle-normalization.md`; the tree's part is to make every
javac position computable. It does, because it keeps every token and has a node for every javac
node except where noted. Running the oracle on small inputs showed what the mapping has to handle:

- **Starts and ends:** a node starts at its first token, modifiers and annotations included, and doc
  comments excluded. Statements, variables, imports, `package`, and module directives end after
  their `;`. The compilation unit starts at its first token, after leading comments.
- **Missing positions:** empty modifiers are `-1..-1`, but a generic method's empty modifiers are an
  empty range before `<` (`<T> void g()`). Nodes javac synthesizes have no end: a compact
  constructor's parameters, an enum constant's implicit `new`. These are dropped before comparing.
- **Declarations javac splits:** `int[] f, g[];` is two `VARIABLE`s, each with the shared
  modifiers and type. The first ends after the `,`, the second starts at the shared modifiers, and
  the second's type spans `int[] f, g[]`. `for (int i = 0, j = 0; ...)` is the same. Ferrion keeps
  one declaration with two declarators, and the mapping produces javac's nodes from it.
- **Array types:** `int h[]` gives an `ARRAY_TYPE` spanning the name. `String[] m(...)[]` gives one
  spanning the parameters. In `int[][]`, the inner `ARRAY_TYPE` has the outer one's span.
- **Folding:** javac's parser joins adjacent string literals in a `+` chain into one
  `STRING_LITERAL` spanning them: `"a" + "b" + x` is `PLUS(STRING_LITERAL "a" + "b", x)`. A
  declaration initialized with 10,000 concatenated literals is an eleven-node tree.
- **Odd spans:** a cast's `INTERSECTION_TYPE` starts at the `(`; an annotation type declaration's
  modifiers include the `@` of `@interface`; `this.<T>g()`'s `MEMBER_SELECT` spans the type
  arguments.
- **Order:** the oracle lists children in javac's order, not source order: a method's return type
  comes before its type parameters.
- **Absent from javac's tree:** a `;` in a class body; names of classes, methods, and variables;
  individual modifier keywords. A `;` among top-level declarations is an `EMPTY_STATEMENT`.
- **Implicit classes:** a top-level method gets a synthesized `CLASS` with the method's span.
- **Errors:** missing expressions are empty `ERRONEOUS` nodes; an unterminated declaration's end
  runs to the start of the next token, trailing whitespace included. Files with javac errors are
  excluded from position comparison anyway.

The oracle itself recurses over the tree: on deeply nested inputs it reports a
`StackOverflowError` as a crash unless run with a larger `-Xss` (see
[deep nesting](parser-deep-nesting.md)).

## How consumers use it

- **Formatter:** walks the tree recursively from the root, building its document from each node's
  children with tokens. It asks the comment classification ([comments](parser-comments.md)) where
  each comment goes, and copies `@formatter:off` regions from the text by token range.
- **Linter rules:** visit the nodes of the kinds they register for (one pass over the node array,
  dispatching by kind), and read children through typed views. A fix is a list of text edits built
  from token spans.
- **Semantic analysis:** lowers declarations into its own symbol tables, keyed by declaration path,
  and attributes bodies into `NodeMap`s keyed by `NodeId`.
- **Language server:** holds `Arc<SyntaxTree>` per document version and runs requests on any
  thread.
