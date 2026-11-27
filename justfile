# Ferrion task runner. `just` lists every recipe.
# Recipes use a POSIX shell. On Windows, run them from Git Bash.

_default:
    @just --list

# The final checkpoint: CI's required checks that need no JDK or corpus. Needs a clean working tree.
ready: _clean-tree fmt-check lint test deny codegen-check scancode
    @git diff --exit-code || (echo "ready: a step changed tracked files; review and commit them" && exit 1)
    @git status --short
    @echo "ready: all checks passed"

# Format Rust code
fmt:
    cargo fmt --all

# Check formatting without changing files (what CI runs)
fmt-check:
    cargo fmt --all --check

# Clippy with CI's settings
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Hermetic tests: no JDK, network or corpus
test *args:
    cargo test --workspace --locked {{ args }}

# Licenses, advisories, banned crates and sources, for both workspaces
deny:
    cargo deny check
    cargo deny --manifest-path fuzz/Cargo.toml check

# Check that the generated syntax code matches crates/ferrion_syntax/java.grammar
codegen-check:
    cargo codegen
    @git diff --exit-code -- crates/ferrion_syntax || (echo "codegen-check: the generated code was stale; review and commit it" && exit 1)

# Copied license notices and copyright holders (needs Python 3.13 and libmagic; see CONTRIBUTING.md)
scancode:
    python3 tools/scancode/scan.py

_clean-tree:
    @git rev-parse --is-inside-work-tree >/dev/null 2>&1 || (echo "ready: not a git repository" && exit 1)
    @test -z "$(git status --porcelain)" || (echo "ready: commit or stash your changes first, so the result reflects exactly what you'll push" && exit 1)
