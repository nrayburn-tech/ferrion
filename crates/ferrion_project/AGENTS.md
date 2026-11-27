# ferrion_project

Turns command-line flags, the environment and an optional project model (JSON with a format
version) into source sets with their roots and releases, then builds each source set's classpath
and validates the JDK, jars and directories it was given. It works in two stages: `Project::load`
needs no types, and `TypeInputs::open` runs only when an enabled rule needs types. It does not
parse the command line. `ferrion_cli` fills `InputFlags` and `Environment`; `ferrion_semantic` and
the corpus tool depend on it.

**Design:** [project-inputs.md](../../docs/design/project-inputs.md), within
[semantic.md](../../docs/design/semantic.md)

**Risk level:** Deep: see [docs/agents/high-risk-areas.md](../../docs/agents/high-risk-areas.md)
§7 for any change to the project model's format, an input's meaning, or a message users read.
Standard for internal refactors that leave the model, flags and messages unchanged.

## Invariants

- Loading is a pure function of `InputFlags`, `Environment`, the model file and the classpath
  file. It reads nothing else from the process.
- Every value carries its `Origin`, and every `InputError` names it.
- An unset classpath differs from an empty one (`Some("")`).
- A model of an earlier format version still loads (version 1 without compile options).

## Where tests go

| Kind of change             | Add the test here                              | Command                                                |
| -------------------------- | ---------------------------------------------- | ------------------------------------------------------ |
| Loading, validation, model | `src/project/tests.rs`                         | `cargo test --package ferrion_project`                 |
| Classpath, JDK and jar opening | `src/types/tests.rs`                       | `cargo test --package ferrion_project`                 |
| Path handling              | inline `#[cfg(test)]` in `src/paths.rs`        | `cargo test --package ferrion_project`                 |
| Model to engine inputs     | `crates/ferrion_semantic/tests/project_input.rs` | `cargo test --package ferrion_semantic --test project_input` |
| Corpus projects load       | the `corpus` package's tests                   | `cargo test --package corpus`                          |
| Every project's type inputs open | ignored test in `corpus`                 | `cargo test --package corpus -- --ignored type_inputs` |

The crate has no `tests/` directory. `src/testing.rs` has scratch directories, jars and JDKs,
written with `ferrion_classfile::testing`. Use this crate's existing test setup. Don't add new
test frameworks, helper crates or test styles without asking.

## Don't

- Don't change the model's format, a flag's meaning or a message users read without a human's yes
  (see [docs/design/cli.md](../../docs/design/cli.md)).
- Don't read the process environment or the working directory inside this crate; take them from
  `Environment`.
- Don't use real JDKs or jars in tests; write them with `ferrion_classfile::testing`.
- Don't run a project's build to load it; the model comes from a file.
- Don't print a message that lacks the value's origin.
