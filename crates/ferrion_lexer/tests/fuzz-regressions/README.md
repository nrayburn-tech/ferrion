# Lexer fuzz regressions

Each file here, apart from this README and dotfiles, is a reduced input that once made a lexer fuzz target fail.
`tests/fuzz_regressions.rs` reads each one as a source file's bytes and runs it through the same
checks as the fuzz targets, so the failure stays fixed.

To add one, minimize the crash with `cargo fuzz tmin` and copy the result here under a short name
that says what it covers, such as `unterminated-text-block-escape.java`. See "Fuzzing" in
`CONTRIBUTING.md` for the commands, including how to turn a `lex_fragments` input into source text.
