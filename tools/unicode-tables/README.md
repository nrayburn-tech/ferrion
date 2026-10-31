# Unicode tables

`UnicodeTables.java` generates `crates/ferrion_lexer/src/unicode_tables.rs`, the tables the lexer
uses to recognize identifiers (JLS §3.8). The JLS defines a "Java letter" and a "Java letter or
digit" by `Character.isJavaIdentifierStart` and `Character.isJavaIdentifierPart`, so the tables
record what those methods return on a real JDK, for every code point. A third table records
`Character.isIdentifierIgnorable`, since javac ignores those characters when comparing names.

The lexer doesn't depend on the Java version, so the tables follow the newest GA JDK. The generated
file records the JDK and Unicode versions it came from.

To regenerate the tables, for example after a new JDK release updates the Unicode version, run the
program with that JDK:

```bash
java tools/unicode-tables/UnicodeTables.java > crates/ferrion_lexer/src/unicode_tables.rs
```
