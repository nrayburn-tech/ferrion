# Ferrion

**A Java formatter, linter and language server, written in Rust.** Ferrion builds on its own
lexer, parser and type checker. It aims to replace Google Java Format for formatting, and
Checkstyle, PMD and Error Prone for static analysis, without a JVM and without network access.

> **Status: in development, not released.** The formatter comes first. See
> [docs/ROADMAP.md](docs/ROADMAP.md) for the order of work and [docs/SCOPE.md](docs/SCOPE.md) for
> what Ferrion is and is not.

## Documentation

- [docs/SCOPE.md](docs/SCOPE.md): goals, principles, components and non-goals
- [docs/ROADMAP.md](docs/ROADMAP.md): phases and their exit criteria
- [docs/style.md](docs/style.md): the formatter's style, and its departures from Google Java
  Format
- [docs/design/](docs/design/): design documents for each component

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), including the clean-room policy every contribution follows.
Coding agents should start with [AGENTS.md](AGENTS.md).

## License

Copyright © 2026 Nicholas Rayburn. Licensed under either of
[Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
