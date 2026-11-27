# ferrion_classfile

Reads class files, jars, zip files, jimage module images and `ct.sym`, and finds classes by name
in classpaths and in the platform API javac sees with `--release <N>` on a JDK. Everything is
lazy, returns `Result`, and is `Send + Sync`. It does not write class files outside `testing`, and
it never runs a JDK tool. `ferrion_project` and `ferrion_semantic` depend on it.

**Design:** [classfile.md](../../docs/design/classfile.md),
[classfile-format.md](../../docs/design/classfile-format.md),
[classfile-containers.md](../../docs/design/classfile-containers.md),
[classfile-jimage.md](../../docs/design/classfile-jimage.md),
[classfile-ctsym.md](../../docs/design/classfile-ctsym.md)

**Risk level:** Deep: see [docs/agents/high-risk-areas.md](../../docs/agents/high-risk-areas.md)
§5 for every change to a reader, since all input is untrusted; also §8 for recursion that follows
the input. Standard for `testing.rs` helpers and docs that don't change what a reader accepts.

## Invariants

- No panic, hang, unbounded allocation or unbounded recursion on any input. Sizes and counts read
  from a file are checked against the bytes available before use.
- An error in one class doesn't stop the rest.
- Every entry an index lists can be found, and a read's size matches the declared one or errors
  (the promises `invariants.rs` checks).
- A signature prints back as its text.

## Where tests go

| Kind of change          | Add the test here                                    | Command                                                    |
| ----------------------- | ---------------------------------------------------- | ---------------------------------------------------------- |
| A reader's behavior     | inline `#[cfg(test)]` in that module (`src/zip.rs`, `src/jimage.rs`, ...), using `testing` writers | `cargo test --package ferrion_classfile --lib` |
| A fuzz failure          | minimized input in `tests/fuzz-regressions/<target>/` | `cargo test --package ferrion_classfile --test fuzz_seeds` |
| A new fuzz seed         | `testing::fuzz_seeds`, then rewrite `fuzz/seeds/`    | `FERRION_WRITE_FUZZ_SEEDS=1 cargo test --package ferrion_classfile --test fuzz_seeds` |
| A new promise to check  | `src/invariants.rs`, used by the fuzz targets        | `cargo test --package ferrion_classfile --lib`             |
| Agreement with the JDK  | no test file; run the javap oracle                   | `cargo oracle classfile --jdk <path>`                      |

Fuzz targets: `classfile`, `signature`, `zip`, `jar`, `jimage`, `jimage_structured`, `ctsym`.
Run them from `fuzz/` as CONTRIBUTING.md's "Fuzzing" section describes. Use this crate's existing
test setup. Don't add new test frameworks, helper crates or test styles without asking.

## Don't

- Don't copy a class file, jar or image from a JDK into a test or a seed. `testing.rs` writes them.
- Don't learn the jimage format from anything but observation: hex dumps of images, `jlink`
  output, the `jimage` tool and the jrt file system. Cite the observation in the log in
  `classfile-jimage.md`, and use only what the document says Ferrion uses.
- Don't study OpenJDK or javac source.
- Don't trust a length, count or offset read from a file; check it first.
- Don't panic or `unwrap` on input. Return an `Error`.
- Don't add a difference from javap or javac to the oracle without explaining it in
  `classfile.md`.
