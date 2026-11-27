# ferrion_config

`ferrion.jsonc`: the JSONC parser, the config model, validation, globs, flat-config resolution
(which entries apply to a path and the settings they give it) and the JSON Schema. It loads from
text or from a file and touches no other part of the file system. Every error carries a byte span
in the config text, so the CLI and the language server render the same error. It depends on
`ferrion_diagnostics`, and `ferrion_cli` depends on it.

**Design:** [cli.md](../../docs/design/cli.md) ("Configuration", "The JSON Schema"),
[SCOPE.md](../../docs/SCOPE.md#configuration)

**Risk level:** Deep for changes to the file's keys, validation rules, error codes and wording,
resolution order, or the schema (high-risk-areas.md §7), which need a human's yes before they start.
Standard for internals that change none of those.

## Invariants

- Loading checks the whole config and collects every error (up to `MAX_ERRORS`), each with a span.
- `schemas/ferrion.schema.json` is what `cargo codegen` generates from the same key tables the
  validator uses. The validator stays the authority where the schema can't express a rule.
- Keys are camelCase and rule names kebab-case. Unknown keys and names are errors with suggestions.

## Where tests go

| Kind of change             | Add the test here                                  | Command |
| -------------------------- | -------------------------------------------------- | ------- |
| A valid or invalid config  | `tests/configs/valid/` or `tests/configs/invalid/` (`.jsonc`) | `cargo test --package ferrion_config --test configs` |
| An error message           | the catalog `tests/configs/errors.txt`, rewritten with `FERRION_BLESS=1` | `FERRION_BLESS=1 cargo test --package ferrion_config --test configs` |
| Schema acceptance          | first line `// schema: rejects`, `accepts` or `n/a` in an invalid input | `cargo test --package ferrion_config --test schema` |
| Which entries apply        | `tests/resolve.rs`                                 | `cargo test --package ferrion_config --test resolve` |
| JSONC, globs, suggestions  | inline `mod tests` in `src/jsonc.rs`, `glob.rs`, `suggest.rs` | `cargo test --package ferrion_config --lib` |
| A schema change            | regenerate with `cargo codegen`                    | `cargo test --package ferrion_config --test schema` |

Every error code must appear in `errors.txt`. Review the catalog diff as a whole. Don't add new
test frameworks without asking.

## Don't

- Don't hand-edit `schemas/ferrion.schema.json`; change the key tables and run `cargo codegen`.
- Don't add file system access beyond loading a config file.
- Don't rename a key, error code or rule name without a human's yes.
