# Unicode tables

`UnicodeTables.java` generates `crates/ferrion_lexer/src/unicode_tables.rs`, the tables the lexer
uses to recognize identifiers (JLS §3.8). The JLS defines a "Java letter" and a "Java letter or
digit" by `Character.isJavaIdentifierStart` and `Character.isJavaIdentifierPart`, so the tables
record what those methods return on a real JDK, for every code point. A third table records
`Character.isIdentifierIgnorable`, since javac ignores those characters when comparing names.

The lexer doesn't depend on the Java version, so the tables follow the newest GA JDK, the latest
release Ferrion supports (`LATEST_RELEASE` in `tools/oracle-check/src/main.rs`). The generated file
records the JDK feature version and the Unicode version it came from.

Two checks keep the tables current:

- `cargo test` fails unless the recorded JDK feature version is `LATEST_RELEASE`, and unless every
  workflow's `java-version` is too. Raising `LATEST_RELEASE` means regenerating the tables and
  updating the workflows in the same change.
- CI's `tools` job regenerates the tables with that JDK and fails if the result differs from the
  committed file.

To regenerate the tables, run the program with the JDK of the latest release:

```bash
java tools/unicode-tables/UnicodeTables.java > crates/ferrion_lexer/src/unicode_tables.rs
```
