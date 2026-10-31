# Parser design

Phase 2 of the [roadmap](../ROADMAP.md) starts with design decisions, recorded before any parser
code is written. This document is the overview: what the parser has to serve, the shape of
`ferrion_parser`, how the special file kinds fit, and the decisions made in review. Each decision
has its own document.

| Decision | Document | In short |
| --- | --- | --- |
| Memory layout, node identity | [Syntax tree](parser-syntax-tree.md) | A flat, lossless CST: the lexer's tokens and a pre-order node array. Typed views generated from a grammar file. `NodeId` is a dense index, valid for one tree. |
| Comments | [Comments](parser-comments.md) | Comments stay trivia tokens. Leading, trailing, and dangling are computed on demand. Doc comments follow javac's attachment rule. |
| Stack strategy | [Deep nesting](parser-deep-nesting.md) | Flat data so drops never recurse, loops for chains, stack growth (`stacker`) for real nesting, and a nesting limit as a resource limit. |
| Every version's syntax | [Language versions](parser-language-versions.md) | One grammar for all releases plus a table-driven version check. One release-dependent reading (`yield`). Contextual keywords decided by context. |
| Error recovery, syntax errors | [Error recovery](parser-error-recovery.md) | Recovery in lists with recovery sets, errors local to their statement or member, quality measured on mutated corpus files, and diagnostics anchored where javac reports them. |

How Ferrion's syntax errors and tree positions are compared with javac's oracle is in
`parser-oracle-normalization.md`, written separately. These documents only make sure the tree and
diagnostics can produce javac's positions.

## What the parser serves

- **Round trip:** the tree reproduces every file byte for byte, comments, whitespace, and malformed
  tokens included.
- **The formatter (Track A):** every token in order with the structure around it, a comment
  classification, `@formatter:off` regions, and which parts of a broken file are unaffected, for
  range formatting.
- **The linter (Track C):** node shapes for syntax-only rules, token spans for fixes, the targets of
  suppression comments, and which parts of a file a syntax error affects.
- **Semantic analysis (Track B):** a dense id per node for side tables, a fast walk over
  declarations for per-file summaries, and the release a file was parsed for.
- **The language server:** a full reparse on each change, fast enough not to need incremental
  reparsing, and a tree that can be shared across threads.
- **Verification:** javac's tree positions and parsing-phase errors, per release, and no panic or
  stack overflow on any input.

## The crate

`ferrion_parser` is internal (`publish = false`, SCOPE.md), so its API serves the workspace and can
change freely. Its shape:

```rust
pub struct ParseOptions {
    pub release: u32, // in ferrion_java_version::SUPPORTED_RELEASES
}

/// Lexes and parses `text`, a `SourceText`'s text. Never fails and never panics.
pub fn parse(text: Arc<str>, options: ParseOptions) -> Parse;

pub struct Parse {
    pub tree: SyntaxTree,                   // immutable, Send + Sync
    pub diagnostics: Vec<ParseDiagnostic>,  // lexical, syntax, and version, in source order
}

// The tree and its cursors (parser-syntax-tree.md).
pub struct SyntaxTree;              // tokens + pre-order nodes + text + release
pub struct NodeId(u32);
pub struct TokenId(u32);
pub enum SyntaxKind;                // token kinds, contextual keyword kinds, node kinds (generated)
pub struct SyntaxNode<'t>;          // &tree + NodeId, Copy
pub struct SyntaxToken<'t>;         // &tree + TokenId, Copy
pub struct NodeMap<T>;              // side table indexed by NodeId
pub struct SyntaxPtr;               // kind + span, to find a node again in a reparsed tree

pub mod ast;      // typed views (generated): CompilationUnit, ClassDecl, IfStmt, BinaryExpr, ...
pub mod comments; // CommentMap: leading, trailing, dangling; doc comment lookup
pub mod version;  // Feature table and the version check

pub struct ParseDiagnostic;         // kind, severity, span, javac anchor (parser-error-recovery.md)

#[doc(hidden)]
pub mod invariants; // tree validation for tests and fuzz targets, like the lexer's
```

Inside, the parser is split by area (`grammar/` with declarations, types, expressions, statements,
patterns, and modules), around a shared core of token lookahead, event emission, and recovery
helpers. The event builder, the tree, the version check, and the comment classification are
separate modules.

Dependencies: `ferrion_lexer`, `ferrion_diagnostics` (spans, `SourceText`), `ferrion_java_version`,
and a new `ferrion_stack` (see [deep nesting](parser-deep-nesting.md#where-the-helper-lives)). Doc
comments are parsed by `ferrion_doc_comments`, from the text of the tokens the tree identifies, when
a consumer asks.

`parse` takes the text rather than lexed tokens so that callers can't pair a tree with the wrong
tokens, and returns lexical errors among its diagnostics. Callers that only want tokens keep using
`ferrion_lexer`.

## File kinds

The parser doesn't look at file names, and neither does javac's: with the oracle, a module
declaration in `Mod.java` and package annotations in `Pkg.java` parse without errors. Checks that
depend on the file name belong to semantic analysis.

- **Ordinary compilation units:** an optional package declaration, imports, then type declarations.
  At 25 and later, top-level methods and fields form an implicitly declared class; the tree keeps
  them as members of the compilation unit, and javac's synthesized class is produced only by the
  position mapping (see [syntax tree](parser-syntax-tree.md#matching-javacs-tree-positions)).
- **`module-info.java`:** a compilation unit whose imports are followed by a module declaration:
  annotations, `open`, `module`, a name, and directives (`requires` with `transitive` and `static`,
  `exports` and `opens` with `to` lists, `uses`, `provides` with a `with` list). The version check
  reports it below 9. Module directive keywords are contextual (see
  [language versions](parser-language-versions.md#contextual-keywords)). A compilation unit that
  mixes a module declaration with type declarations gets a syntax error, as with javac.
- **`package-info.java`:** an ordinary compilation unit whose package declaration has annotations
  and usually a doc comment. `ast::PackageDecl::doc_comment()` finds it by the same rule as for
  other declarations.

The formatter and linter treat all three the same way; the import rules apply to module-info's
imports too.

## Performance

From the prototype in [syntax tree](parser-syntax-tree.md#measurements): building the flat tree
from tokens runs at over 1 GB/s, and the lexer at about 190 MB/s. Parsing proper isn't measured yet;
the design keeps it to one pass over the tokens with bounded lookahead and no backtracking, and no
allocation per node beyond the arrays.

## What was consulted

Everything about javac in these documents was learned by running `tools/javac-oracle` (and `javac`
itself, for later-phase errors and doc comment warnings) on small inputs written for the purpose,
never from javac's source or any other incompatible implementation's. The `rowan` crate
(MIT OR Apache-2.0) was used through its public API in a throwaway prototype, outside the
repository, to measure it. The ideas of a lossless syntax tree with trivia as tokens, typed views
over an untyped tree, and parser events with markers come from rust-analyzer's and rowan's published
design documentation (MIT OR Apache-2.0).

## Decisions made in review

The first draft of these documents left six questions for review. They were settled as follows.

1. **The parser takes the release.** `parse` takes the release and the tree records it, with a
   closed, tested list of release-dependent readings, today only `yield` (see
   [release-dependent readings](parser-language-versions.md#release-dependent-readings)).
2. **Stack growth uses `stacker`.** Every recursive function calls `ensure_stack`, in a new internal
   crate, `ferrion_stack`, that wraps it (see [deep nesting](parser-deep-nesting.md)). The cost is
   `unsafe` code and a little assembly inside the dependency, through `psm`, built with each
   target's C toolchain.
3. **A nesting limit of 10,000.** The parser stops at 10,000 nested constructs with a Ferrion-only
   diagnostic, a deliberate departure from javac. Chains (`+`, `else if`, method calls) aren't
   limited.
4. **javac's parsing-phase warnings are never shown to users.** The version check produces them only
   to compare them with javac's.
5. **Typed views are generated** into a checked-in file from a grammar file of Ferrion's own format,
   with a CI check that the file is up to date.
6. **Withdrawn preview syntax** (string templates) gets a generic syntax error, as with javac 27.
