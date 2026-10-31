//! Java letters and digits (JLS §3.8), by the tables in `unicode_tables.rs`.

use crate::unicode_tables::{IDENTIFIER_IGNORABLE, JAVA_IDENTIFIER_PART, JAVA_IDENTIFIER_START};

/// Whether `c` can start an identifier: a "Java letter", for which
/// `Character.isJavaIdentifierStart` is true.
pub(crate) fn is_start(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphabetic() || c == '_' || c == '$';
    }
    in_table(JAVA_IDENTIFIER_START, c)
}

/// Whether `c` can continue an identifier: a "Java letter or digit", for which
/// `Character.isJavaIdentifierPart` is true. That includes the identifier-ignorable characters.
pub(crate) fn is_part(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_' || c == '$' || is_ignorable(c);
    }
    in_table(JAVA_IDENTIFIER_PART, c)
}

/// Whether `c` is ignored when comparing identifiers: `Character.isIdentifierIgnorable`, which
/// covers most control characters and the Unicode format characters, such as a soft hyphen.
pub(crate) fn is_ignorable(c: char) -> bool {
    if c.is_ascii() {
        return matches!(c, '\0'..='\x08' | '\x0E'..='\x1B' | '\x7F');
    }
    in_table(IDENTIFIER_IGNORABLE, c)
}

fn in_table(table: &[(u32, u32)], c: char) -> bool {
    let c = c as u32;
    let i = table.partition_point(|&(_, last)| last < c);
    table.get(i).is_some_and(|&(first, _)| first <= c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_matches_the_tables() {
        for c in (0..128u8).map(char::from) {
            assert_eq!(is_start(c), in_table(JAVA_IDENTIFIER_START, c), "{c:?}");
            assert_eq!(is_part(c), in_table(JAVA_IDENTIFIER_PART, c), "{c:?}");
            assert_eq!(is_ignorable(c), in_table(IDENTIFIER_IGNORABLE, c), "{c:?}");
        }
    }

    #[test]
    fn tables_are_sorted_and_disjoint() {
        for table in [
            JAVA_IDENTIFIER_START,
            JAVA_IDENTIFIER_PART,
            IDENTIFIER_IGNORABLE,
        ] {
            assert!(table.iter().all(|&(first, last)| first <= last));
            assert!(table.windows(2).all(|w| w[0].1 + 1 < w[1].0));
        }
    }

    #[test]
    fn non_ascii() {
        assert!(is_start('é'));
        assert!(is_start('€'), "currency symbols are Java letters");
        assert!(is_start('‿'), "so are connector punctuation characters");
        assert!(is_start('𝐀'));
        assert!(!is_start('\u{200B}'));
        assert!(is_part('\u{200B}'));
        assert!(is_part('\u{0301}'), "combining marks continue identifiers");
        assert!(!is_start('\u{0301}'));
        assert!(is_part('٣'));
        assert!(!is_start('٣'));
        assert!(is_ignorable('\u{AD}'));
        assert!(!is_part('\u{A0}'));
        assert!(!is_part('\u{2028}'));
        assert!(!is_part(char::REPLACEMENT_CHARACTER));
    }
}
