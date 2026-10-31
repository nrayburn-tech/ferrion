//! What a comparison with javac is made of, so each phase can add its own (tokens now; tree
//! positions and diagnostics later) and share the corpus, oracle, cache, and report.

use ferrion_diagnostics::SourceText;
use serde::Serialize;

use crate::allowed::Counts;
use crate::oracle::{Output, Section};

pub trait Check: Sync {
    /// What [`Check::prepare`] produces for a file.
    type Prepared;

    /// The subcommand that runs the check.
    fn name(&self) -> &'static str;

    /// The oracle sections the check reads.
    fn sections(&self) -> &'static [Section];

    /// The work that doesn't depend on the release, done once per file however many releases it's
    /// compared at. `bytes` are the file as read, and `source` is what they decode to.
    fn prepare(&self, source: &SourceText, bytes: &[u8]) -> Self::Prepared;

    /// Compares a prepared file with javac's output for it at one release. The output is never a
    /// failure ([`Output::failure`]); those are reported without calling this.
    fn compare(&self, prepared: &Self::Prepared, output: &Output) -> Outcome;
}

/// The result of comparing one file at one release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub status: Status,
    /// Why Ferrion's result doesn't reproduce the file, if it doesn't.
    pub round_trip: Option<Located>,
    /// The first lexical error javac reported, when the comparison stopped there.
    pub javac_error: Option<Located>,
    pub allowed: Counts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Matched,
    Mismatched(Box<Mismatch>),
    /// The file isn't UTF-8 or ASCII, so it isn't compared.
    Skipped(String),
    /// javac produced no output for the file.
    JavacFailed(String),
    /// Ferrion panicked.
    Panicked(String),
}

impl Outcome {
    pub fn with_status(status: Status) -> Outcome {
        Outcome {
            status,
            ..Outcome::default()
        }
    }
}

/// A message about a place in a file. Lines and columns start at 1, and columns count UTF-16 code
/// units, as javac's do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Located {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

/// Where Ferrion's result and javac's first differ.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Mismatch {
    pub line: u32,
    pub column: u32,
    pub summary: String,
    /// Ferrion's items around the difference, one per line, `None` past the end.
    pub ferrion: Vec<Option<String>>,
    /// javac's items around the difference.
    pub javac: Vec<Option<String>>,
    /// The index of the first differing item in `ferrion` and `javac`.
    pub at: usize,
}
