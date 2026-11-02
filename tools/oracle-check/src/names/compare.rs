//! Which of javac's names are in B2's part, and comparing them with Ferrion's.
//!
//! The rules are in docs/design/semantic-oracle-normalization.md, "What is compared".

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};

use super::ferrion::{Offsets, ResolvedName};
use super::oracle::{FileOutput, Name};

/// What a javac name is to the comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    /// In B2's part: compared.
    B2,
    /// Qualified by an expression's type, or a method a call on an expression chooses: B3's.
    B3,
    /// A tree javac made up, with no end or an empty range: the default constructor's `super()`,
    /// an enum constant's `new`, a record's implicit members.
    Implicit,
    /// `this`, `super`, or `class`, which javac's trees have as names and Ferrion's don't.
    Keyword,
    /// javac gives no element: an on-demand import's `*`, a static import's member.
    NoElement,
    /// Inside a Lombok-annotated type, left out until B4.
    Lombok,
}

impl Part {
    pub const ALL: [Part; 6] = [
        Part::B2,
        Part::B3,
        Part::Implicit,
        Part::Keyword,
        Part::NoElement,
        Part::Lombok,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Part::B2 => "b2",
            Part::B3 => "b3",
            Part::Implicit => "implicit",
            Part::Keyword => "keyword",
            Part::NoElement => "noElement",
            Part::Lombok => "lombok",
        }
    }
}

/// The kind of symbol a key names, for counting: its first word, with `type local` apart.
pub fn key_kind(key: &str) -> &str {
    if key.starts_with("type local ") {
        return "local type";
    }
    key.split(' ').next().unwrap_or("")
}

/// The part of each of a file's names, in order.
pub fn parts(file: &FileOutput) -> Vec<Part> {
    // The qualifier of a member select `a.b.c` is the name with the same start and the greatest
    // end before its own: `a.b`.
    let mut by_start: HashMap<i64, Vec<(i64, usize)>> = HashMap::new();
    for (i, name) in file.names.iter().enumerate() {
        by_start.entry(name.start).or_default().push((name.end, i));
    }
    for ends in by_start.values_mut() {
        ends.sort_unstable();
    }
    let qualifier_of = |i: usize| -> Option<usize> {
        let name = &file.names[i];
        let ends = by_start.get(&name.start)?;
        ends.iter()
            .rev()
            .find(|&&(end, _)| end >= 0 && end < name.end)
            .map(|&(_, j)| j)
    };
    let in_lombok = |name: &Name| {
        file.lombok
            .iter()
            .any(|&(start, end)| start <= name.start && name.start < end)
    };
    // Qualifiers come after the names they qualify in javac's pre-order, so parts are found from
    // the last name back, and a name's qualifier is settled before the name.
    let mut parts = vec![Part::B3; file.names.len()];
    for i in (0..file.names.len()).rev() {
        let name = &file.names[i];
        // A tree javac made up has no end, or for the superclass of an anonymous class that a
        // qualified `new` creates (`o.new Inner() {}`), an empty range after the name.
        parts[i] = if name.start < 0 || name.end <= name.start {
            Part::Implicit
        } else if name.context == "keyword" {
            Part::Keyword
        } else if in_lombok(name) {
            Part::Lombok
        } else if name.key == "none" {
            Part::NoElement
        } else {
            match name.context.as_str() {
                "package" | "import" | "module" | "annotation" | "annotation-element" | "type" => {
                    Part::B2
                }
                "method" => match name.qualifier.as_str() {
                    "-" | "p" | "t" | "this" => Part::B2,
                    _ => Part::B3,
                },
                _ => match name.qualifier.as_str() {
                    "-" | "p" | "t" | "this" => Part::B2,
                    // A field of a variable B2 resolved, of a declared type without arguments.
                    "v" if qualifier_of(i).is_some_and(|q| parts[q] == Part::B2) => Part::B2,
                    _ => Part::B3,
                },
            }
        };
    }
    parts
}

/// What javac's key for a name is compared with: for a method name, the type searched for it.
pub fn javac_key(name: &Name) -> &str {
    match (&name.searched, name.context.as_str()) {
        (Some(searched), "method") => searched,
        _ => &name.key,
    }
}

/// Why a name in B2's part didn't match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Ferrion has no name at javac's range.
    Missing,
    /// The keys name different kinds of symbol (a type against a package, a field against a
    /// local).
    Kind,
    /// The same type, from somewhere else: shadowing or classpath order.
    Origin,
    /// Another symbol of the same kind.
    Symbol,
    /// Ferrion leaves the name to B3 (`b3 <why>`): it needs a type B2 doesn't attribute.
    LeftToB3,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Missing => "missing",
            Category::Kind => "kind",
            Category::Origin => "origin",
            Category::Symbol => "symbol",
            Category::LeftToB3 => "b3",
        }
    }
}

pub fn categorize(javac: &str, ferrion: Option<&str>) -> Category {
    let Some(ferrion) = ferrion else {
        return Category::Missing;
    };
    if ferrion.starts_with("b3 ") {
        return Category::LeftToB3;
    }
    if key_kind(javac) != key_kind(ferrion) {
        return Category::Kind;
    }
    // `type <binary name> <origin...>`
    let binary = |key: &str| key.split(' ').nth(1).map(str::to_owned);
    if key_kind(javac) == "type" && binary(javac) == binary(ferrion) {
        return Category::Origin;
    }
    Category::Symbol
}

/// A name in B2's part whose keys differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub start: i64,
    pub end: i64,
    pub context: String,
    pub javac: String,
    pub ferrion: Option<String>,
    pub category: Category,
}

/// The comparison of one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compared {
    pub matched: u64,
    pub mismatches: Vec<Mismatch>,
    /// Ferrion's names at a range where javac has no name in B2's part, but those it leaves to
    /// B3.
    pub extra: u64,
    /// Ferrion's names left to B3 at a range where javac has no name in B2's part.
    pub left_to_b3: u64,
}

/// Compares the names in B2's part with Ferrion's.
pub fn compare(
    file: &FileOutput,
    parts: &[Part],
    ferrion: &[ResolvedName],
    offsets: &Offsets,
) -> Compared {
    let mut by_range: HashMap<(i64, i64), &str> = HashMap::new();
    for name in ferrion {
        by_range.insert(offsets.range(name.span), &name.key);
    }
    let mut compared = Compared::default();
    let mut used = HashSet::new();
    for (name, &part) in file.names.iter().zip(parts) {
        if part != Part::B2 {
            continue;
        }
        let ours = by_range.get(&(name.start, name.end)).copied();
        if ours.is_some() {
            used.insert((name.start, name.end));
        }
        let theirs = javac_key(name);
        if ours == Some(theirs) {
            compared.matched += 1;
        } else {
            compared.mismatches.push(Mismatch {
                start: name.start,
                end: name.end,
                context: name.context.clone(),
                javac: theirs.to_owned(),
                ferrion: ours.map(str::to_owned),
                category: categorize(theirs, ours),
            });
        }
    }
    for (range, key) in &by_range {
        if used.contains(range) {
            continue;
        }
        if key.starts_with("b3 ") {
            compared.left_to_b3 += 1;
        } else {
            compared.extra += 1;
        }
    }
    compared
}

/// The known differences: names whose mismatch is explained, each under a category.
///
/// `differences/names-known.txt` has `category <name>: <explanation>` lines, then `<category> <file>
/// <start>-<end>` for each name, where the file is the report's (project, then path) and the
/// range is javac's. `#` starts a comment.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Known {
    pub categories: Vec<(String, String)>,
    pub names: HashMap<(String, i64, i64), String>,
}

const KNOWN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/differences/names-known.txt");

impl Known {
    pub fn load() -> Result<Known> {
        let text = std::fs::read_to_string(KNOWN).with_context(|| format!("reading {KNOWN}"))?;
        Known::parse(&text).with_context(|| KNOWN.to_owned())
    }

    pub fn parse(text: &str) -> Result<Known> {
        let mut known = Known::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("category ") {
                let Some((name, explanation)) = rest.split_once(':') else {
                    bail!("line {}: expected `category <name>: <explanation>`", i + 1);
                };
                known
                    .categories
                    .push((name.trim().to_owned(), explanation.trim().to_owned()));
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [category, file, range] = fields[..] else {
                bail!("line {}: expected `<category> <file> <start>-<end>`", i + 1);
            };
            if !known.categories.iter().any(|(name, _)| name == category) {
                bail!("line {}: unknown category `{category}`", i + 1);
            }
            let (start, end) = range
                .split_once('-')
                .and_then(|(s, e)| Some((s.parse().ok()?, e.parse().ok()?)))
                .with_context(|| format!("line {}: `{range}` isn't `<start>-<end>`", i + 1))?;
            known
                .names
                .insert((file.to_owned(), start, end), category.to_owned());
        }
        Ok(known)
    }

    /// The category a mismatch in `file` is known under.
    pub fn category(&self, file: &str, mismatch: &Mismatch) -> Option<&str> {
        self.names
            .get(&(file.to_owned(), mismatch.start, mismatch.end))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use ferrion_diagnostics::{SourceText, Span};

    use super::*;

    fn name(start: i64, end: i64, context: &str, qualifier: &str, key: &str) -> Name {
        Name {
            start,
            end,
            context: context.into(),
            qualifier: qualifier.into(),
            key: key.into(),
            searched: None,
        }
    }

    #[test]
    fn parts_follow_the_design() {
        // `a.b.c` where `a` is a local of a plain type and `b` a field of one; `x.y.z` where `x`
        // is a method call's result, so `x.y` is B3's and so is `.z`; an implicit constructor
        // call, `this`, and a Lombok type.
        let file = FileOutput {
            names: vec![
                name(0, 5, "expression", "v", "field A.c"),
                name(0, 3, "expression", "v", "field A.b"),
                name(0, 1, "expression", "-", "local f:9"),
                name(10, 15, "expression", "v", "field A.z"),
                name(10, 13, "expression", "e", "field A.y"),
                name(20, -1, "keyword", "-", "method java/lang/Object.<init>()V"),
                name(30, 34, "keyword", "-", "field A.this"),
                name(40, 46, "import", "t", "none"),
                name(50, 51, "type", "-", "type A platform"),
                name(60, 66, "method", "e", "method A.m()V"),
                name(70, 71, "method", "-", "method A.m()V"),
            ],
            lombok: vec![(50, 55)],
            ..FileOutput::default()
        };
        let parts = parts(&file);
        use Part::*;
        assert_eq!(
            parts,
            [
                B2, B2, B2, B3, B3, Implicit, Keyword, NoElement, Lombok, B3, B2
            ]
        );
    }

    #[test]
    fn method_names_compare_the_searched_type() {
        let mut call = name(0, 1, "method", "-", "method p/A.m()V");
        call.searched = Some("type p/A source x".into());
        assert_eq!(javac_key(&call), "type p/A source x");
        assert_eq!(
            javac_key(&name(0, 1, "type", "-", "type p/A platform")),
            "type p/A platform"
        );
    }

    #[test]
    fn categories() {
        assert_eq!(categorize("type p/A platform", None), Category::Missing);
        assert_eq!(
            categorize("type p/A platform", Some("package p.A")),
            Category::Kind
        );
        assert_eq!(
            categorize("type p/A platform", Some("type p/A class jars/a.jar")),
            Category::Origin
        );
        assert_eq!(
            categorize("field p/A.x", Some("field p/A.y")),
            Category::Symbol
        );
        assert_eq!(key_kind("type local f:1"), "local type");
        assert_eq!(key_kind("local f:1"), "local");
    }

    #[test]
    fn compares_by_range() {
        let source = SourceText::decode(b"class A { B b; }".to_vec()).unwrap();
        let offsets = Offsets::new(&source);
        let file = FileOutput {
            names: vec![
                name(10, 11, "type", "-", "type B source s"),
                name(6, 7, "type", "-", "type A source s"),
            ],
            ..FileOutput::default()
        };
        let parts = parts(&file);
        let ferrion = [
            ResolvedName {
                span: Span { start: 10, end: 11 },
                key: "type B source s".into(),
            },
            ResolvedName {
                span: Span { start: 13, end: 14 },
                key: "local f:13".into(),
            },
        ];
        let compared = compare(&file, &parts, &ferrion, &offsets);
        assert_eq!(compared.matched, 1);
        assert_eq!(compared.extra, 1);
        assert_eq!(compared.mismatches.len(), 1);
        assert_eq!(compared.mismatches[0].category, Category::Missing);
    }

    #[test]
    fn known_differences() {
        let known = Known::parse(
            "# comment\ncategory shadowing: javac prefers the source path\n\
             shadowing jedis/src/A.java 10-20 # why\n",
        )
        .unwrap();
        let mismatch = Mismatch {
            start: 10,
            end: 20,
            context: "type".into(),
            javac: "type A platform".into(),
            ferrion: None,
            category: Category::Missing,
        };
        assert_eq!(
            known.category("jedis/src/A.java", &mismatch),
            Some("shadowing")
        );
        assert!(Known::parse("other a/B.java 1-2\n").is_err());
        assert!(Known::load().is_ok());
    }
}
