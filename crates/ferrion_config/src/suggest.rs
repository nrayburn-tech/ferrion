//! "Did you mean" suggestions for misspelt names.

/// The candidate closest to `name`, if one is close enough: within an edit distance of 2, or of 1
/// for candidates of four characters or fewer. A difference in case is an edit like any other.
/// Of equally close candidates, the first wins.
pub fn did_you_mean<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let limit = if candidate.chars().count() <= 4 { 1 } else { 2 };
        let distance = edit_distance(name, candidate);
        if distance <= limit && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The Levenshtein distance between `a` and `b`, in characters: the fewest insertions, deletions,
/// and substitutions that turn one into the other.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("ignore", "ignores"), 1);
        assert_eq!(edit_distance("Files", "files"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("ab", "ba"), 2);
        assert_eq!(edit_distance("é", "e"), 1);
    }

    #[test]
    fn suggestions() {
        let keys = ["name", "files", "ignores", "rules", "format"];
        assert_eq!(did_you_mean("ignore", keys), Some("ignores"));
        assert_eq!(did_you_mean("FILES", keys), None);
        assert_eq!(did_you_mean("Files", keys), Some("files"));
        assert_eq!(did_you_mean("fles", keys), Some("files"));
        assert_eq!(did_you_mean("rule", keys), Some("rules"));
        assert_eq!(did_you_mean("formats", keys), Some("format"));
        assert_eq!(did_you_mean("xyz", keys), None);
        // Four letters or fewer allow one edit.
        let levels = ["off", "info", "warn", "error"];
        assert_eq!(did_you_mean("wan", levels), Some("warn"));
        assert_eq!(did_you_mean("of", levels), Some("off"));
        assert_eq!(did_you_mean("wrn", levels), Some("warn"));
        assert_eq!(did_you_mean("wr", levels), None);
        assert_eq!(did_you_mean("err", levels), Some("error"));
        assert_eq!(did_you_mean("eror", levels), Some("error"));
        assert_eq!(did_you_mean("warning", levels), None);
        // The closest wins, and the first of equally close ones.
        assert_eq!(did_you_mean("abd", ["abc", "abd2", "abe"]), Some("abc"));
        assert_eq!(did_you_mean("abcdx", ["abcdef", "abcd"]), Some("abcd"));
    }
}
