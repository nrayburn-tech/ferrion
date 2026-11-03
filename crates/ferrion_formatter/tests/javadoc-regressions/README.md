Failures of the reflow check in the `javadoc` fuzz target, each named for what it covers: the
content of a traditional doc comment, between `/**` and `*/`, whose first byte picks the column.
`cargo test --package ferrion_formatter --test fuzz_regressions` reflows each twice and checks that
the text settles and the content is kept (see CONTRIBUTING.md, "Fuzzing").
