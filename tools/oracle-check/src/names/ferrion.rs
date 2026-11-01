//! Ferrion's side of the names comparison: what its name resolution must give for a source set,
//! in the comparison's terms, and the stand-in used until it exists.
//!
//! For each file, Ferrion gives a [`ResolvedName`] for every name in B2's part
//! (docs/design/semantic-names.md, "Decision"):
//!
//! - **The range** is the byte range of the javac tree for the name: an identifier's own range,
//!   and for a qualified name, from the start of its first qualifier to the end of its last
//!   identifier (`java.util.List` and its qualifiers `java.util` and `java` are three names). These
//!   are the ranges the tree comparison already maps Ferrion's nodes to (`ferrion::tree`), so a
//!   name's range is that of the node the mapping gives for its `Name` or `NameExpr`.
//! - **The key** is a string in the oracle's form (`Names.Keys` in `tools/javac-oracle`, and
//!   docs/design/semantic-oracle-normalization.md): `package p`, `module m`, `type p/A$B <origin>`
//!   with the origin `platform`, `source <file>`, or `class <classpath entry>`, `type local
//!   <file>:<offset>`, `typevar <owner key> T`, `field <owner>.<name>`, `method
//!   <owner>.<name><descriptor>`, and `local <file>:<offset>`. Files and classpath entries are
//!   relative to the corpus directory, with `/`. Offsets are those of the declaration's name, in
//!   UTF-16 code units with a byte-order mark counted as one, as javac counts them: use
//!   [`Offsets`] to convert Ferrion's byte offsets.
//! - **For a method name**, the key is the type whose members are searched for the method (JLS
//!   15.12.1), which is B2's part; the method a call chooses is B3's.
//!
//! A file with no entry, or a name javac resolves that Ferrion gives nothing for, is a mismatch
//! once Ferrion has results for the source set. Names outside B2's part that Ferrion gives anyway
//! are counted, not compared.
//!
//! `ferrion_semantic`'s `SymbolKey` (W4) is nearly this form already: [`oracle_key`] makes its
//! paths relative to the corpus directory and writes its `class platform` as `platform`. Its
//! `member` keys (a single-static import's member) never meet a javac key, since javac gives
//! those names no element.

use std::collections::HashMap;
use std::path::Path;

use ferrion_diagnostics::{LineIndex, SourceText, Span};

use super::plan::SourceSet;

/// One name, as Ferrion resolved it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedName {
    /// Bytes in the file's decoded text.
    pub span: Span,
    pub key: String,
}

/// A source set's names, by file: the path relative to the corpus directory, with `/`, as the
/// oracle names files.
pub type SetNames = HashMap<String, Vec<ResolvedName>>;

/// Ferrion's name resolution, as the comparison runs it.
pub trait Resolver: Sync {
    /// The names of every file of `set` (one of `sets`), or `None` while Ferrion has nothing to
    /// compare. Files are read from `corpus`; generated sources are already there, under
    /// [`SourceSet::generated`]. An error is reported for the source set, not compared.
    fn resolve(
        &self,
        sets: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
    ) -> Result<Option<SetNames>, String>;
}

/// Stands in for Ferrion until name resolution exists (B2's W5): nothing to compare.
pub struct StandIn;

impl Resolver for StandIn {
    fn resolve(
        &self,
        _: &[SourceSet],
        _: &SourceSet,
        _: &Path,
    ) -> Result<Option<SetNames>, String> {
        Ok(None)
    }
}

/// A `ferrion_semantic` symbol key in the oracle's form: each `source <path>` and `class <path>`
/// relative to `corpus`, with `/`, and `class platform` as `platform`. Keys nest (a type variable's
/// owner), so every occurrence is converted.
#[allow(dead_code, reason = "for W5's resolver, which replaces the stand-in")]
pub fn oracle_key(key: &str, corpus: &Path) -> String {
    let prefix = format!("{}/", corpus.display());
    let mut out = String::with_capacity(key.len());
    let mut words = key.split(' ').peekable();
    while let Some(word) = words.next() {
        if !out.is_empty() {
            out.push(' ');
        }
        match (word, words.peek()) {
            ("class", Some(&"platform")) => {
                out.push_str("platform");
                words.next();
            }
            ("class" | "source", Some(path)) if path.starts_with(&prefix) => {
                out.push_str(word);
                out.push(' ');
                out.push_str(&path[prefix.len()..].replace('\\', "/"));
                words.next();
            }
            _ => out.push_str(word),
        }
    }
    out
}

/// Converts a file's byte offsets to javac's: UTF-16 code units, with a byte-order mark as one.
pub struct Offsets {
    index: LineIndex,
    bom: bool,
}

impl Offsets {
    pub fn new(source: &SourceText) -> Offsets {
        Offsets {
            index: LineIndex::new(source.text()),
            bom: source.has_bom(),
        }
    }

    pub fn utf16(&self, byte: u32) -> i64 {
        i64::from(self.index.utf16_offset(byte) + u32::from(self.bom))
    }

    /// A span as javac's `[start, end]`.
    pub fn range(&self, span: Span) -> (i64, i64) {
        (self.utf16(span.start), self.utf16(span.end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_keys_in_the_oracles_form() {
        let corpus = Path::new("/c");
        assert_eq!(
            oracle_key("type java/lang/String class platform", corpus),
            "type java/lang/String platform"
        );
        assert_eq!(
            oracle_key(
                "typevar type p/A source /c/projects/x/src/p/A.java T",
                corpus
            ),
            "typevar type p/A source projects/x/src/p/A.java T"
        );
        assert_eq!(
            oracle_key("type q/B class /c/jars/q/b.jar", corpus),
            "type q/B class jars/q/b.jar"
        );
        assert_eq!(oracle_key("field p/A.f", corpus), "field p/A.f");
    }

    #[test]
    fn offsets_count_utf16_and_the_byte_order_mark() {
        let source =
            SourceText::decode("\u{feff}/* é😀 */ class A {}".as_bytes().to_vec()).unwrap();
        let offsets = Offsets::new(&source);
        let class = source.text().find("class").unwrap() as u32;
        // `/* `, then é (one unit), 😀 (two), ` */ `, after the mark.
        assert_eq!(offsets.utf16(class), 1 + 3 + 1 + 2 + 4);
        assert_eq!(
            offsets.range(Span {
                start: class,
                end: class + 5
            }),
            (11, 16)
        );
    }
}
