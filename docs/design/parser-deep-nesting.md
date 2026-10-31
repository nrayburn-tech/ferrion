# Deep nesting

How Ferrion handles deeply nested input without overflowing the stack: in the parser, when a tree
is dropped, when it is walked, and in the phases after parsing. Part of the
[parser design](parser.md).

SCOPE.md requires that deeply nested input (long `+` chains, long `else if` chains, generated code)
never overflows the stack, and that the parser never panics. The Phase 2 exit criteria add javac's
own deep-nesting tests, and fuzzing runs the parser on arbitrary input. A stack overflow in Rust
aborts the process, so it would take down every file in a CLI run and the whole language server.

## Decision

A mix, with each part where it is cheapest:

1. **Flat data.** Every tree-shaped structure whose depth follows the input is stored flat, in
   arrays with indices: the syntax tree, doc comment trees, the formatter's document, and later
   structures of the same kind. Dropping one frees a few vectors and never recurses.
2. **Loops for chains.** The parser parses every construct that repeats at one level with a loop,
   not recursion: binary operators, member selection and calls, `else if`, prefix operators,
   `?:` in its else branch, statement and member lists. A 100,000-operand `+` chain costs no stack.
3. **Stack growth for real nesting.** Recursion is kept where the grammar nests (parentheses,
   blocks, lambdas, classes, array initializers, type arguments). Each recursive function, in the
   parser and in every later phase, calls one helper, `ensure_stack`, that moves to a new stack
   segment on the heap when the current one is nearly used up. It wraps the `stacker` crate.
4. **A nesting limit as a resource limit.** The parser stops at a fixed nesting depth, 10,000, with
   a "too deeply nested" diagnostic. It bounds memory and time, not stack use.
5. **Iterative library walks.** The tree's own iterators (descendants, ancestors, preorder with
   enter and leave events) are loops over its arrays.

Thread stack sizes are not relied on. Ferrion's worker threads get a generous fixed stack so that
growth is rare, but the guarantee holds in any thread, including test threads and the fuzzing
harness.

## What javac does

Measured with the oracle and with `javac` (JDK 27, default stack) on generated methods. "Depth" is
the deepest level in the oracle's tree.

| Input | Parses (oracle) | Compiles (`javac`) |
| --- | --- | --- |
| `+` chain of 10,000 string literals | yes: folded into 11 nodes | yes |
| `+` chain of 100,000 string literals | yes | "constant string too long" |
| `+` chain of 3,000 identifiers | yes, depth 3,004 | yes |
| `+` chain of 10,000 identifiers | stack overflow | stack overflow |
| 3,000 nested parentheses | yes, depth 3,005 | yes |
| 5,000 nested parentheses | stack overflow | stack overflow |
| `else if` chain of 2,000 | yes, depth 2,007 | yes |
| `else if` chain of 10,000 | stack overflow | stack overflow |
| 2,000 nested blocks | yes | yes |
| 10,000 nested blocks | stack overflow | not run |
| `?:` chain of 3,000 | yes, depth 3,006 | yes |
| 2,000 chained calls (`.append(1)`) | stack overflow | stack overflow |
| 3,000 nested `-(...)`, lambdas, or array initializers | stack overflow | not run |

So javac, with its default stack, handles nesting in the low thousands, and its tree can be as deep
as the input is long. javac's own deep-nesting tests must fit within what javac handles, possibly
with a larger stack. Ferrion has to handle at least that, and fuzzing will find anything deeper.

The oracle recurses over javac's tree to print it, and javac recurses while parsing, so on deep
inputs it reports a `StackOverflowError` as a crash (`{"crash": ...}`). Comparing positions on deep
inputs needs the oracle run with a larger `-Xss`; with `-Xss64m`, 4,000 chained calls parse.

## Why a mix

- **Tree depth isn't bounded by parser recursion.** A `+` chain parsed with a loop still gives a
  left-nested tree as deep as the chain is long, as javac's does. Every later phase that recurses
  over the tree (the formatter building its document, attribution, constant folding, flow analysis)
  meets that depth. Bounding parser recursion alone isn't enough.
- **A cap on tree depth would reject valid code.** 10,000 concatenated string literals compile with
  javac, which folds them while parsing, but Ferrion's tree doesn't fold, so it is 10,000 deep. A
  cap low enough to protect a fixed stack would report a syntax error javac doesn't.
- **Big stacks alone don't bound anything.** A thread with a 64 MiB stack still overflows on a deep
  enough input, and fuzzing produces one. They also don't cover threads Ferrion doesn't create.
- **Iteration everywhere is too costly.** Writing the formatter and the type checker with explicit
  stacks would make the two largest components harder to write and review, for inputs that are rare.
- **Stack growth is proven.** `stacker` (MIT OR Apache-2.0, maintained by the Rust project) is what
  rustc uses for the same problem, and rustc runs on every platform Ferrion ships for. Its check is
  a comparison of the stack pointer, cheap enough for every recursive call. Its cost: it contains
  `unsafe` code and, through `psm`, a little assembly per platform, built with a C toolchain, which
  the A4 cross builds must provide.

## The parts

### Flat data

The syntax tree is flat by design (see [syntax tree](parser-syntax-tree.md)). In the prototype
measured there, a 1,000,000-deep tree was built and dropped on a 2 MiB stack in 8 ms, while
`Arc`-based trees overflowed on drop at 100,000 levels (`rowan`) and 1,000,000 levels (a plain
`Arc` tree). The same rule applies to `ferrion_doc_comments` and to the formatter's document.
Structures that can't be flat get a hand-written `Drop` that frees with an explicit stack.

### The parser

- **Loops:** precedence climbing parses all binary operators in one loop per precedence level, so a
  chain of any length is one stack frame deep. Postfix operations (`.name`, calls, `[i]`, `::`,
  `++`) are a loop on the primary. `else if` is a loop that opens one `IfStmt` per `if` and closes
  them all at the end, which events make possible without recursion (see
  [syntax tree](parser-syntax-tree.md#building)). Prefix operators and casts count their operators,
  parse the operand once, then close the nodes. `a ? b : c ? d : e` loops on the else branch.
- **Recursion** remains for parentheses, blocks, statements nested in statements, lambdas,
  anonymous and local classes, array initializers, switch bodies, annotation values, and type
  arguments. Each recursive entry point calls `ensure_stack` and increments a depth counter.
- **The nesting limit:** at the limit, the parser reports `NestingTooDeep` at the token that would
  go deeper, skips to the matching closing delimiter (an iterative scan of the tokens), and puts the
  skipped tokens in an `ERROR` node. The file then has a syntax error, so the formatter skips it and
  rules treat that region as affected. The limit is a deliberate departure from javac, recorded with
  the other differences. With `ensure_stack`, the limit doesn't need to be tied to a stack size;
  10,000 is several times what javac handles with its default stack.

### Walks and later phases

- The tree's iterators never recurse.
- Recursive code over the tree anywhere in Ferrion calls `ensure_stack` on entry. A test helper runs
  each consumer (formatter, version check, comment classification, lint rules, and later
  attribution) on the stress inputs below.
- The formatter's printer and the fix engine are loops by design.

### Where the helper lives

`ensure_stack(f)` is a one-line wrapper over `stacker::maybe_grow` with Ferrion's constants (a red
zone around 128 KiB, segments of a few MiB, both to be tuned by measurement). It lives in a small
internal crate, `ferrion_stack`, so the dependency and its constants are in one place, and the doc
comment parser, formatter, and semantic analysis can use it without depending on the parser.

## Testing

- **Stress tests,** generated rather than committed: each pattern above, plus nested classes,
  nested generic types (`List<List<...>>`), nested annotations, and nested `switch` expressions, at
  depths from 1,000 to 100,000 (1,000,000 for chains). Each is parsed, checked, walked by every
  consumer, and dropped, on a thread with a deliberately small stack (256 KiB), so that a missing
  `ensure_stack` fails the test instead of passing on a large main-thread stack.
- **javac's deep-nesting tests,** fetched at test time with javac's other test inputs, are parsed
  and compared like any other input.
- **Fuzzing:** the parser fuzz targets include a fragment-based target that produces deep nesting,
  like the lexer's `lex_fragments`.
- **Time:** the stress tests also check each input against the per-file time limit.
