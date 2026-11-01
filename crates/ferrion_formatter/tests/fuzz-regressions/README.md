Reduced failures of the `format` fuzz target, each named for what it covers. `cargo test
--package ferrion_formatter --test fuzz_regressions` checks the safety properties on each, at
every release where it parses without errors (see CONTRIBUTING.md, "Fuzzing").
