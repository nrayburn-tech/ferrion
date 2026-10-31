//! The Java releases Ferrion supports: Java 8 through the latest GA release (SCOPE.md). A release
//! outside this range is a usage error.

use std::ops::RangeInclusive;

/// The oldest Java release Ferrion supports.
pub const OLDEST_RELEASE: u32 = 8;

/// The newest Java release Ferrion supports: the latest GA release. Raising it means regenerating
/// the lexer's Unicode tables with that JDK and installing that JDK in `.github/workflows`; this
/// crate's tests fail until both are done.
pub const LATEST_RELEASE: u32 = 27;

pub const SUPPORTED_RELEASES: RangeInclusive<u32> = OLDEST_RELEASE..=LATEST_RELEASE;

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
}
