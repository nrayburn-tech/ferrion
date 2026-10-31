//! The Java releases Ferrion supports: Java 8 through the latest GA release (SCOPE.md). A release
//! outside this range is a usage error.
//!
//! Every supported release is accepted and version-checked. Testing focuses on the primary
//! releases: the LTS releases and the latest GA release. Other releases are tested where a
//! language feature's first release falls on them.

use std::ops::RangeInclusive;

/// The oldest Java release Ferrion supports.
pub const OLDEST_RELEASE: u32 = 8;

/// The newest Java release Ferrion supports: the latest GA release. Raising it means regenerating
/// the lexer's Unicode tables with that JDK, installing that JDK in `.github/workflows`, and
/// updating [`PRIMARY_RELEASES`] (and [`LTS_RELEASES`] when the new release is an LTS); this
/// crate's tests fail until all are done.
pub const LATEST_RELEASE: u32 = 27;

pub const SUPPORTED_RELEASES: RangeInclusive<u32> = OLDEST_RELEASE..=LATEST_RELEASE;

/// The long-term support releases from [`OLDEST_RELEASE`] on, oldest first.
pub const LTS_RELEASES: &[u32] = &[8, 11, 17, 21, 25];

/// The releases testing focuses on: [`LTS_RELEASES`] and [`LATEST_RELEASE`], oldest first. Test
/// matrices, oracle comparisons, and fuzzing run at these.
pub const PRIMARY_RELEASES: &[u32] = &[8, 11, 17, 21, 25, 27];

pub fn is_supported(release: u32) -> bool {
    SUPPORTED_RELEASES.contains(&release)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range() {
        assert!(is_supported(OLDEST_RELEASE) && is_supported(LATEST_RELEASE));
        assert!(!is_supported(OLDEST_RELEASE - 1) && !is_supported(LATEST_RELEASE + 1));
    }

    #[test]
    fn primary_releases_are_the_lts_releases_and_the_latest() {
        let mut expected = LTS_RELEASES.to_vec();
        if expected.last() != Some(&LATEST_RELEASE) {
            expected.push(LATEST_RELEASE);
        }
        assert_eq!(PRIMARY_RELEASES, expected);
        assert!(LTS_RELEASES.is_sorted() && LTS_RELEASES.iter().all(|&r| is_supported(r)));
        assert_eq!(LTS_RELEASES.first(), Some(&OLDEST_RELEASE));
    }
}
