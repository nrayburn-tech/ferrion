//! What a comparison with javac is made of, so each phase can add its own (tokens, syntax errors,
//! and tree positions so far) and share the corpus, oracle, cache, and report.

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

    /// Whether [`Check::compare`] can give different outcomes for the same javac output at two
    /// releases. When it can't, a file whose output is the same at several releases, as most are,
    /// is compared once.
    fn depends_on_release(&self) -> bool {
        false
    }

    /// Compares a prepared file with javac's output for it at one release. The output is never a
    /// failure ([`Output::failure`]); those are reported without calling this.
    fn compare(&self, prepared: &Self::Prepared, release: u32, output: &Output) -> Outcome;
}

/// The result of comparing one file at one release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub status: Status,
    /// Why Ferrion's result doesn't reproduce the file, if it doesn't.
    pub round_trip: Option<Located>,
    /// The first error javac reported, when the comparison stopped there or left the file out.
    pub javac_error: Option<Located>,
    pub allowed: Counts,
    /// What javac reported, for checks that summarize it.
    pub summary: Summary,
}

/// What javac reported for a file at a release, summarized across a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// The codes of the errors and warnings javac reported while parsing, in its order.
    pub codes: Vec<String>,
    /// Tree nodes, and those javac gave no start or no end position.
    pub nodes: u64,
    pub no_start: u64,
    pub no_end: u64,
    /// The depth of the deepest tree node, where the compilation unit is 0.
    pub depth: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Matched,
    Mismatched(Box<Mismatch>),
    /// The file isn't compared, for the reason given: it isn't UTF-8 or ASCII, or the check
    /// leaves out files javac rejects.
    Skipped(String),
    /// javac produced no output for the file.
    JavacFailed(String),
    /// javac's side ran, but Ferrion has nothing to compare it with yet. Not a failure.
    NotCompared,
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
