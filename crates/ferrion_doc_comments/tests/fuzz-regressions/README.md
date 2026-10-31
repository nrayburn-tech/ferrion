# Doc comment fuzz regressions

Each file in `javadoc/` and `markdown/`, apart from dotfiles, is a reduced input that once made the
fuzz target of the same name fail. `tests/fuzz_regressions.rs` runs each one through the same
checks as its target, so the failure stays fixed: a `javadoc` input is the content of a `/** */`
comment, and a `markdown` input is the lines of `///` comments, without the `///`s.

To add one, minimize the crash with `cargo fuzz tmin` and copy the result into the directory of
its target, under a short name that says what it covers, such as `split-wildcard-bound`. See
"Fuzzing" in `CONTRIBUTING.md` for the commands.
