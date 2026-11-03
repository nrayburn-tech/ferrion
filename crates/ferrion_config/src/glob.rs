//! Globs, matched against paths relative to the working directory with `/` separators.
//!
//! - `*` matches any run of characters except `/`, and `?` one character except `/`; `**` as a
//!   whole segment matches any number of segments; `[abc]`, `[a-z]`, `[!a]`, and `{a,b}` are as
//!   usual.
//! - A glob is anchored to the working directory: `*.java` matches only top-level files.
//! - A trailing `/` means the folder and everything under it: `build/` is `build/**`.
//! - A leading `./` is removed.
//! - Matching is case-sensitive.
//! - `\` escapes the next character. Before a character that needs no escaping, it is an error,
//!   so that a Windows-style path isn't silently misread.
//! - An absolute path, a `..` segment, a leading `!`, an empty glob, and a malformed one are
//!   errors.

use ferrion_diagnostics::Span;
use globset::{ErrorKind, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};

/// Characters that `\` may escape: those with a meaning in a glob.
const ESCAPABLE: &[char] = &['*', '?', '[', ']', '{', '}', ',', '!', '-', '^', '\\'];

/// A glob from the config.
#[derive(Clone, Debug)]
pub struct Glob {
    /// The glob as written.
    pub raw: String,
    /// The glob as matched: without a leading `./`, and with `/**` for a trailing `/`.
    pub pattern: String,
    /// The span of the glob's string in the config, quotes included.
    pub span: Span,
    glob: globset::Glob,
    matcher: GlobMatcher,
}

impl Glob {
    /// Checks and compiles a glob. `span` is where it is in the config.
    pub fn new(raw: &str, span: Span) -> Result<Glob, GlobError> {
        let pattern = normalize(raw)?;
        let glob = build(&pattern).map_err(|e| from_globset(e.kind()))?;
        Ok(Glob {
            raw: raw.to_owned(),
            pattern,
            span,
            matcher: glob.compile_matcher(),
            glob,
        })
    }

    /// Whether the glob matches `path`, a normalized path relative to the working directory.
    pub fn is_match(&self, path: &str) -> bool {
        self.matcher.is_match(path)
    }

    /// For a glob that ends in `/**`, the glob before it: a folder it matches can be skipped
    /// whole, since the glob matches everything under it.
    pub(crate) fn folder_prefix(&self) -> Option<globset::Glob> {
        let prefix = self.pattern.strip_suffix("/**")?;
        build(prefix).ok()
    }

    pub(crate) fn globset_glob(&self) -> &globset::Glob {
        &self.glob
    }
}

/// Compiles globs into one set, matched in one pass.
pub(crate) fn set<'g>(globs: impl IntoIterator<Item = &'g globset::Glob>) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        builder.add(glob.clone());
    }
    // Every glob was compiled on its own already, so the set can't fail.
    builder.build().expect("checked globs form a set")
}

fn build(pattern: &str) -> Result<globset::Glob, globset::Error> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .case_insensitive(false)
        .build()
}

/// Why a glob is invalid, for a [`ConfigError`](crate::ConfigError).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobError {
    pub message: String,
    pub label: Option<String>,
    pub help: Option<String>,
    /// A glob that would fix the error, when one is known.
    pub suggestion: Option<String>,
}

impl GlobError {
    fn new(message: impl Into<String>) -> GlobError {
        GlobError {
            message: message.into(),
            label: None,
            help: None,
            suggestion: None,
        }
    }

    fn label(mut self, label: impl Into<String>) -> GlobError {
        self.label = Some(label.into());
        self
    }

    fn help(mut self, help: impl Into<String>) -> GlobError {
        self.help = Some(help.into());
        self
    }
}

/// Checks the rules that globset doesn't, and returns the glob to compile.
fn normalize(raw: &str) -> Result<String, GlobError> {
    if raw.is_empty() {
        return Err(GlobError::new("a glob can't be empty"));
    }
    if raw.starts_with('!') {
        return Err(GlobError::new("a glob can't start with `!`")
            .label("negation isn't supported")
            .help("to leave files out of an entry, put a glob for them in its `ignores`"));
    }
    if is_absolute(raw) {
        return Err(GlobError::new("a glob can't be an absolute path")
            .label("globs are relative to the working directory"));
    }
    check_escapes(raw)?;
    let mut pattern = raw.to_owned();
    if pattern.ends_with('/') {
        pattern.push_str("**");
    }
    let mut pattern = pattern.as_str();
    while let Some(rest) = pattern.strip_prefix("./") {
        pattern = rest;
    }
    if pattern.split('/').any(|segment| segment == "..") {
        return Err(GlobError::new("a glob can't contain `..`")
            .label("globs are relative to the working directory, and can't match outside it"));
    }
    if pattern.is_empty() {
        return Err(GlobError::new("a glob can't be empty"));
    }
    Ok(pattern.to_owned())
}

/// A path from the root (`/x`), or with a drive (`C:/x`, `C:\x`, `C:x`).
fn is_absolute(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.first() == Some(&b'/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// Every `\` must escape a character with a meaning in a glob, and can't be last.
fn check_escapes(raw: &str) -> Result<(), GlobError> {
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            continue;
        }
        match chars.next() {
            Some(next) if ESCAPABLE.contains(&next) => {}
            Some(next) => {
                let mut error = GlobError::new(format!("`\\{next}` isn't an escape in a glob"))
                    .label(
                        "`\\` makes the character after it literal, and this one needs no escaping",
                    );
                // A Windows-style path: every `\` in it is meant as a separator.
                let fixed = raw.replace('\\', "/");
                error = error.help(format!("folders are separated by `/`: `{fixed}`"));
                error.suggestion = Some(fixed);
                return Err(error);
            }
            None => {
                return Err(GlobError::new("a glob can't end with `\\`")
                    .label("`\\` makes the character after it literal, and there is none"));
            }
        }
    }
    Ok(())
}

fn from_globset(kind: &ErrorKind) -> GlobError {
    match kind {
        ErrorKind::UnclosedClass => GlobError::new("unclosed `[` in a glob")
            .label("a character class needs a closing `]`")
            .help("to match `[` itself, write `\\[`"),
        ErrorKind::InvalidRange(from, to) => {
            GlobError::new(format!("invalid range `{from}-{to}` in a glob"))
                .label("a range goes from the lower character to the higher")
        }
        ErrorKind::UnopenedAlternates => GlobError::new("unmatched `}` in a glob")
            .label("there is no `{` before it")
            .help("to match `}` itself, write `\\}`"),
        ErrorKind::UnclosedAlternates => GlobError::new("unclosed `{` in a glob")
            .label("alternatives need a closing `}`")
            .help("to match `{` itself, write `\\{`"),
        ErrorKind::DanglingEscape => GlobError::new("a glob can't end with `\\`")
            .label("`\\` makes the character after it literal, and there is none"),
        other => GlobError::new(format!("invalid glob: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(raw: &str) -> Glob {
        Glob::new(raw, Span::default()).unwrap_or_else(|e| panic!("{raw}: {}", e.message))
    }

    fn error(raw: &str) -> GlobError {
        Glob::new(raw, Span::default()).unwrap_err()
    }

    #[test]
    fn star_doesnt_cross_slashes() {
        let g = glob("*.java");
        assert!(g.is_match("A.java"));
        assert!(!g.is_match("src/A.java"));
        let g = glob("src/*/A.java");
        assert!(g.is_match("src/x/A.java"));
        assert!(!g.is_match("src/x/y/A.java"));
        let g = glob("src/?.java");
        assert!(g.is_match("src/A.java"));
        assert!(!g.is_match("src//.java"));
    }

    #[test]
    fn double_star_matches_any_number_of_segments() {
        let g = glob("**/*.java");
        assert!(g.is_match("A.java"));
        assert!(g.is_match("a/b/c/A.java"));
        let g = glob("**/legacy/**");
        assert!(g.is_match("legacy/A.java"));
        assert!(g.is_match("src/main/legacy/x/A.java"));
        assert!(!g.is_match("src/legacy"));
        assert!(!g.is_match("src/notlegacy/A.java"));
        let g = glob("src/**/A.java");
        assert!(g.is_match("src/A.java"));
        assert!(g.is_match("src/a/b/A.java"));
    }

    #[test]
    fn globs_are_anchored() {
        let g = glob("src/*.java");
        assert!(g.is_match("src/A.java"));
        assert!(!g.is_match("lib/src/A.java"));
        assert!(!glob("A.java").is_match("src/A.java"));
    }

    #[test]
    fn classes_and_alternatives() {
        let g = glob("[abc].java");
        assert!(g.is_match("a.java"));
        assert!(!g.is_match("d.java"));
        assert!(glob("[a-c].java").is_match("b.java"));
        let g = glob("[!a].java");
        assert!(g.is_match("b.java"));
        assert!(!g.is_match("a.java"));
        let g = glob("src/{main,test}/**");
        assert!(g.is_match("src/main/A.java"));
        assert!(g.is_match("src/test/A.java"));
        assert!(!g.is_match("src/other/A.java"));
    }

    #[test]
    fn trailing_slash_and_leading_dot_slash() {
        let g = glob("build/");
        assert_eq!(g.pattern, "build/**");
        assert_eq!(g.raw, "build/");
        assert!(g.is_match("build/a/A.java"));
        assert!(!g.is_match("builds/A.java"));
        assert_eq!(glob("./src/*.java").pattern, "src/*.java");
        assert!(glob("./src/*.java").is_match("src/A.java"));
        assert_eq!(glob("./").pattern, "**");
        assert_eq!(glob("././a").pattern, "a");
    }

    #[test]
    fn case_sensitive() {
        assert!(!glob("*.java").is_match("A.JAVA"));
        assert!(!glob("Src/**").is_match("src/A.java"));
    }

    #[test]
    fn escapes() {
        let g = glob("a\\*.java");
        assert!(g.is_match("a*.java"));
        assert!(!g.is_match("ab.java"));
        assert!(glob("\\[x\\].java").is_match("[x].java"));
        // A `\` in a file name, which only Unix allows: globset reads it as `/` on Windows.
        if cfg!(unix) {
            assert!(glob("a\\\\b").is_match("a\\b"));
        }
        assert!(glob("\\{a\\}").is_match("{a}"));
    }

    #[test]
    fn windows_style_paths_are_errors() {
        let e = error("src\\main\\**");
        assert_eq!(e.message, "`\\m` isn't an escape in a glob");
        assert_eq!(e.suggestion.as_deref(), Some("src/main/**"));
        assert_eq!(
            error("src\\test\\*\\x").suggestion.as_deref(),
            Some("src/test/*/x")
        );
        assert_eq!(
            error("C:\\src\\**").message,
            "a glob can't be an absolute path"
        );
        assert_eq!(
            error("c:/src/**").message,
            "a glob can't be an absolute path"
        );
        assert_eq!(error("a\\").message, "a glob can't end with `\\`");
    }

    #[test]
    fn errors() {
        assert_eq!(error("").message, "a glob can't be empty");
        assert_eq!(error("/src/**").message, "a glob can't be an absolute path");
        assert_eq!(error("../shared/**").message, "a glob can't contain `..`");
        assert_eq!(error("src/../x").message, "a glob can't contain `..`");
        assert_eq!(error("./../x").message, "a glob can't contain `..`");
        assert_eq!(error("!**/x").message, "a glob can't start with `!`");
        assert_eq!(error("[a").message, "unclosed `[` in a glob");
        assert_eq!(error("{a").message, "unclosed `{` in a glob");
        assert_eq!(error("a}").message, "unmatched `}` in a glob");
        assert_eq!(error("[z-a]").message, "invalid range `z-a` in a glob");
        // `..` inside a name is not a segment.
        assert!(glob("a..b/**").is_match("a..b/x"));
    }

    #[test]
    fn folder_prefixes() {
        let prefix = |raw: &str| glob(raw).folder_prefix().map(|g| g.glob().to_owned());
        assert_eq!(prefix("**/legacy/**").as_deref(), Some("**/legacy"));
        assert_eq!(prefix("build/").as_deref(), Some("build"));
        assert_eq!(prefix("build/*"), None);
        assert_eq!(prefix("**"), None);
    }
}
