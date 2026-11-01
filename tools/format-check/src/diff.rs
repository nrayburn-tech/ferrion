//! Line diffs: how many lines differ between two files, and the hunks that show where.
//!
//! The diff is a longest common subsequence of lines, found with Myers's linear-space algorithm
//! (Eugene W. Myers, "An O(ND) Difference Algorithm and Its Variations", 1986). Its cost grows
//! with the number of differing lines, and a file whose every line is re-indented differs in all of
//! them, so two steps shrink the problem first: the common prefix and suffix are matched
//! directly, and lines that occur on only one side are set aside, since they can't be in any
//! common subsequence. Neither changes the length of the longest one, so the counts are exact.
//!
//! Lines are split at `\n`, so a `\r` before it is part of the line, and a last line without a
//! line ending differs from the same line with one.

use std::collections::HashMap;
use std::ops::Range;

/// A line without its `\n`, and whether it had one: only a file's last line can lack it.
type Line<'a> = (&'a [u8], bool);

/// A line diff of `old` and `new`.
pub struct Diff<'a> {
    old: Vec<Line<'a>>,
    new: Vec<Line<'a>>,
    /// The common lines, as indexes into `old` and `new`, in increasing order.
    matches: Vec<(usize, usize)>,
}

/// A part of a diff: lines `old` of the old file and `new` of the new one.
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

impl<'a> Diff<'a> {
    pub fn new(old: &'a [u8], new: &'a [u8]) -> Diff<'a> {
        let old = lines(old);
        let new = lines(new);
        let matches = common_lines(&old, &new);
        Diff { old, new, matches }
    }

    /// Insertions plus deletions: the lines only in the new file and those only in the old one.
    pub fn changed(&self) -> usize {
        self.old.len() + self.new.len() - 2 * self.matches.len()
    }

    /// The hunks, each with up to `context` common lines before and after its changes, merged
    /// where their context would overlap.
    pub fn hunks(&self, context: usize) -> Vec<Hunk> {
        let mut hunks: Vec<Hunk> = Vec::new();
        let (mut i, mut j) = (0, 0);
        let end = (self.old.len(), self.new.len());
        for &(mi, mj) in self.matches.iter().chain([&end]) {
            // Lines i..mi of the old file and j..mj of the new one are changes.
            if mi > i || mj > j {
                let start = (i.saturating_sub(context), j.saturating_sub(context));
                let end = ((mi + context).min(end.0), (mj + context).min(end.1));
                match hunks.last_mut() {
                    Some(last) if last.old.end >= start.0 => {
                        last.old.end = end.0;
                        last.new.end = end.1;
                    }
                    _ => hunks.push(Hunk {
                        old: start.0..end.0,
                        new: start.1..end.1,
                    }),
                }
            }
            (i, j) = (mi + 1, mj + 1);
        }
        hunks
    }

    /// A hunk in unified diff form: a header, then each line with ` `, `-`, or `+`.
    pub fn show(&self, hunk: &Hunk) -> String {
        let mut out = format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old.start + 1,
            hunk.old.len(),
            hunk.new.start + 1,
            hunk.new.len()
        );
        let mut line = |prefix: char, (text, ended): Line| {
            let text = String::from_utf8_lossy(text);
            out.push(prefix);
            out.push_str(text.strip_suffix('\r').unwrap_or(&text));
            out.push('\n');
            if !ended {
                out.push_str("\\ No newline at end of file\n");
            }
        };
        let (mut i, mut j) = (hunk.old.start, hunk.new.start);
        let first = self.matches.partition_point(|&(mi, _)| mi < i);
        for &(mi, mj) in &self.matches[first..] {
            if mi >= hunk.old.end {
                break;
            }
            for &old in &self.old[i..mi] {
                line('-', old);
            }
            for &new in &self.new[j..mj] {
                line('+', new);
            }
            line(' ', self.old[mi]);
            (i, j) = (mi + 1, mj + 1);
        }
        for &old in &self.old[i..hunk.old.end] {
            line('-', old);
        }
        for &new in &self.new[j..hunk.new.end] {
            line('+', new);
        }
        out
    }
}

/// The lines of `bytes`. A final line ending doesn't start another line.
fn lines(bytes: &[u8]) -> Vec<Line<'_>> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let (body, ended) = match bytes.strip_suffix(b"\n") {
        Some(body) => (body, true),
        None => (bytes, false),
    };
    let mut lines: Vec<Line> = body.split(|&b| b == b'\n').map(|l| (l, true)).collect();
    lines.last_mut().unwrap().1 = ended;
    lines
}

/// The common lines of `old` and `new`, as index pairs in increasing order.
fn common_lines<'a>(old: &[Line<'a>], new: &[Line<'a>]) -> Vec<(usize, usize)> {
    // Number the distinct lines, and count each one's occurrences on each side.
    let mut ids: HashMap<Line<'a>, usize> = HashMap::new();
    let mut counts: Vec<[usize; 2]> = Vec::new();
    let mut number = |line: Line<'a>, side: usize| {
        let next = ids.len();
        let id = *ids.entry(line).or_insert(next);
        if id == counts.len() {
            counts.push([0, 0]);
        }
        counts[id][side] += 1;
        id
    };
    let old_ids: Vec<usize> = old.iter().map(|&line| number(line, 0)).collect();
    let new_ids: Vec<usize> = new.iter().map(|&line| number(line, 1)).collect();
    // Only lines on both sides can be common. Each kept line remembers its index.
    let keep = |ids: &[usize]| -> (Vec<usize>, Vec<usize>) {
        ids.iter()
            .enumerate()
            .filter(|&(_, &id)| counts[id].iter().all(|&n| n > 0))
            .map(|(i, &id)| (id, i))
            .unzip()
    };
    let (a, a_index) = keep(&old_ids);
    let (b, b_index) = keep(&new_ids);
    let mut matches = Vec::new();
    lcs(&a, 0, &b, 0, &mut matches);
    matches
        .into_iter()
        .map(|(i, j)| (a_index[i], b_index[j]))
        .collect()
}

/// Appends a longest common subsequence of `a` and `b` to `out`, as index pairs offset by
/// `a_at` and `b_at`, in increasing order.
fn lcs(a: &[usize], a_at: usize, b: &[usize], b_at: usize, out: &mut Vec<(usize, usize)>) {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    out.extend((0..prefix).map(|i| (a_at + i, b_at + i)));
    let (a, b) = (&a[prefix..], &b[prefix..]);
    let (a_at, b_at) = (a_at + prefix, b_at + prefix);
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[..a.len() - suffix], &b[..b.len() - suffix]);
    if !a.is_empty() && !b.is_empty() {
        // Without the common prefix and suffix, the two differ in at least two lines, so both
        // parts around the middle snake are smaller problems than this one.
        let (x0, y0, x1, y1) = middle_snake(a, b);
        lcs(&a[..x0], a_at, &b[..y0], b_at, out);
        out.extend((0..x1 - x0).map(|i| (a_at + x0 + i, b_at + y0 + i)));
        lcs(&a[x1..], a_at + x1, &b[y1..], b_at + y1, out);
    }
    out.extend((0..suffix).map(|i| (a_at + a.len() + i, b_at + b.len() + i)));
}

/// The middle snake of an optimal edit path from `a` to `b`: a run of common lines from
/// `(x0, y0)` to `(x1, y1)` that an optimal path passes through, with about half its edits on
/// either side.
fn middle_snake(a: &[usize], b: &[usize]) -> (usize, usize, usize, usize) {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let delta = n - m;
    let odd = delta % 2 != 0;
    let max = (n + m + 1) / 2;
    let offset = max + 1;
    let size = (2 * max + 3) as usize;
    // The furthest x reached on each diagonal k = x - y: forward from (0, 0), and backward from
    // (n, m) in reversed coordinates, where the forward diagonal k is delta - k.
    let mut forward = vec![0isize; size];
    let mut backward = vec![0isize; size];
    for d in 0..=max {
        for k in (-d..=d).step_by(2) {
            let at = (k + offset) as usize;
            let mut x = if k == -d || (k != d && forward[at - 1] < forward[at + 1]) {
                forward[at + 1]
            } else {
                forward[at - 1] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[x as usize] == b[y as usize] {
                (x, y) = (x + 1, y + 1);
            }
            forward[at] = x;
            let reversed = delta - k;
            if odd
                && (-(d - 1)..=d - 1).contains(&reversed)
                && x + backward[(reversed + offset) as usize] >= n
            {
                return (x0 as usize, y0 as usize, x as usize, y as usize);
            }
        }
        for k in (-d..=d).step_by(2) {
            let at = (k + offset) as usize;
            let mut x = if k == -d || (k != d && backward[at - 1] < backward[at + 1]) {
                backward[at + 1]
            } else {
                backward[at - 1] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[(n - x - 1) as usize] == b[(m - y - 1) as usize] {
                (x, y) = (x + 1, y + 1);
            }
            backward[at] = x;
            let ahead = delta - k;
            if !odd && (-d..=d).contains(&ahead) && forward[(ahead + offset) as usize] + x >= n {
                // In forward coordinates, the snake runs from (n - x, m - y) to (n - x0, m - y0).
                return (
                    (n - x) as usize,
                    (m - y) as usize,
                    (n - x0) as usize,
                    (m - y0) as usize,
                );
            }
        }
    }
    unreachable!("an edit path of length n + m always exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Insertions plus deletions by the textbook dynamic program, to check the fast one against.
    fn slow_changed(old: &[Line], new: &[Line]) -> usize {
        let mut table = vec![vec![0usize; new.len() + 1]; old.len() + 1];
        for i in (0..old.len()).rev() {
            for j in (0..new.len()).rev() {
                table[i][j] = if old[i] == new[j] {
                    table[i + 1][j + 1] + 1
                } else {
                    table[i + 1][j].max(table[i][j + 1])
                };
            }
        }
        old.len() + new.len() - 2 * table[0][0]
    }

    fn check(old: &str, new: &str) {
        let diff = Diff::new(old.as_bytes(), new.as_bytes());
        assert_eq!(
            diff.changed(),
            slow_changed(&diff.old, &diff.new),
            "{old:?} -> {new:?}"
        );
        for pair in diff.matches.windows(2) {
            assert!(pair[0].0 < pair[1].0 && pair[0].1 < pair[1].1);
        }
        for &(i, j) in &diff.matches {
            assert_eq!(diff.old[i], diff.new[j]);
        }
        // Every changed line is in a hunk.
        let shown: usize = diff
            .hunks(0)
            .iter()
            .map(|hunk| hunk.old.len() + hunk.new.len())
            .sum();
        assert_eq!(shown, diff.changed(), "{old:?} -> {new:?}");
    }

    #[test]
    fn counts() {
        assert_eq!(Diff::new(b"a\nb\nc\n", b"a\nb\nc\n").changed(), 0);
        assert_eq!(Diff::new(b"a\nb\nc\n", b"a\nc\nd\n").changed(), 2);
        assert_eq!(Diff::new(b"", b"a\n").changed(), 1);
        assert_eq!(Diff::new(b"a\n", b"").changed(), 1);
        // A missing final line ending is a difference, as is a carriage return.
        assert_eq!(Diff::new(b"a\nb", b"a\nb\n").changed(), 2);
        assert_eq!(Diff::new(b"a\r\nb\n", b"a\nb\n").changed(), 2);
    }

    #[test]
    fn matches_the_dynamic_program() {
        // Pairs of short sequences over a small alphabet, which have odd and even differences,
        // repeated lines, and lines on one side only.
        let sequences: Vec<String> = (0..4u32.pow(6))
            .map(|n| {
                let len = n % 7;
                (0..len)
                    .map(|i| ["x", "y", "z", "w"][(n / 4u32.pow(i) % 4) as usize])
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            })
            .collect();
        for (i, old) in sequences.iter().enumerate().step_by(13) {
            for new in sequences.iter().skip(i % 7).step_by(17) {
                check(old, new);
            }
        }
        check("a\nb\nc\nd\ne\nf\n", "b\na\nd\nc\nf\ne\n");
        check("    }\n  }\n}\n", "  }\n}\n");
        check("a\nb", "a\nb\n");
    }

    #[test]
    fn hunks() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nB\nc\nd\ne\nf\ng\nh\ni\nJ\n";
        let diff = Diff::new(old.as_bytes(), new.as_bytes());
        let hunks = diff.hunks(1);
        assert_eq!(hunks.len(), 2);
        assert_eq!(diff.show(&hunks[0]), "@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n");
        assert_eq!(diff.show(&hunks[1]), "@@ -9,2 +9,2 @@\n i\n-j\n+J\n");
        // Context that would overlap merges them.
        assert_eq!(diff.hunks(4).len(), 1);

        let diff = Diff::new(b"a\r\nb\n", b"a\r\nc");
        assert_eq!(
            diff.show(&diff.hunks(3)[0]),
            "@@ -1,2 +1,2 @@\n a\n-b\n+c\n\\ No newline at end of file\n"
        );
    }
}
