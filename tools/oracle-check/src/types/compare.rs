//! Ferrion's side of the types comparison, and comparing it with javac's
//! (docs/design/types-oracle-normalization.md, "What is compared").
//!
//! Ferrion gives, per file, the same four kinds of entry the oracle writes, in the oracle's
//! forms: expressions' type strings, constants' values, calls' methods and types as invoked, and
//! variables' finality and types; and, per source set, answers to the oracle's relation queries.
//! Entries are matched by javac's range (an implicit call by its start alone), and compared as
//! strings. A Ferrion value starting with `left-to-` (`left-to-b3c inference`) says a later
//! milestone types it: counted apart, never a mismatch.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use ferrion_diagnostics::Span;

use super::oracle::{Answer, FileOutput, Query, Relations};
use crate::names::ferrion::Offsets;
use crate::names::plan::SourceSet;

/// An expression's type or a constant's value, as Ferrion gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typed {
    /// Bytes in the file's decoded text: javac's range for the tree.
    pub span: Span,
    pub value: String,
}

/// A call's chosen method, as Ferrion gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedCall {
    /// javac's range; for a call javac makes up, `implicit`, and only its start counts.
    pub span: Span,
    pub implicit: bool,
    /// The method's key in the oracle's form.
    pub method: String,
    /// Its type as invoked, where javac gives one.
    pub ty: Option<String>,
}

/// A local variable or parameter, as Ferrion gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedVariable {
    /// The byte offset its `local` key names.
    pub offset: u32,
    pub finality: String,
    pub ty: String,
}

/// Ferrion's results for one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTypes {
    pub expressions: Vec<Typed>,
    pub constants: Vec<Typed>,
    pub calls: Vec<TypedCall>,
    pub variables: Vec<TypedVariable>,
}

/// What Ferrion gave for a source set.
#[derive(Debug, Default)]
pub struct Attributed {
    /// By file: the path relative to the corpus directory, with `/`, as the oracle names files.
    pub files: HashMap<String, FileTypes>,
    /// An answer to each of the oracle's relation queries, in order: a type string for a type
    /// answer, `yes` or `no`, a comma-separated list for supertypes; `None` where Ferrion has
    /// none.
    pub relations: Vec<Option<String>>,
    /// Ferrion's own time for the source set.
    pub seconds: f64,
    /// The most fuel one relation query used, and the query.
    pub fuel: u64,
    pub fuel_query: String,
    /// The types in the relations' arena at the end.
    pub arena: u64,
}

/// Ferrion's type attribution, as the comparison runs it.
pub trait Typer: Sync {
    /// The results for every file of `set` (one of `sets`), and the answers to `relations`, or
    /// `None` when nothing is to be compared. An error is reported for the source set.
    fn attribute(
        &self,
        sets: &[SourceSet],
        set: &SourceSet,
        corpus: &Path,
        relations: &Relations,
    ) -> Result<Option<Attributed>, String>;
}

/// Nothing to compare: until B3's attribution exists, and for `--javac-only`.
pub struct StandIn;

impl Typer for StandIn {
    fn attribute(
        &self,
        _: &[SourceSet],
        _: &SourceSet,
        _: &Path,
        _: &Relations,
    ) -> Result<Option<Attributed>, String> {
        Ok(None)
    }
}

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Aspect {
    Expression,
    Constant,
    Call,
    Variable,
    Relation,
}

impl Aspect {
    pub fn name(self) -> &'static str {
        match self {
            Aspect::Expression => "expression",
            Aspect::Constant => "constant",
            Aspect::Call => "call",
            Aspect::Variable => "variable",
            Aspect::Relation => "relation",
        }
    }

    fn parse(text: &str) -> Option<Aspect> {
        [
            Aspect::Expression,
            Aspect::Constant,
            Aspect::Call,
            Aspect::Variable,
            Aspect::Relation,
        ]
        .into_iter()
        .find(|a| a.name() == text)
    }
}

/// Why an entry didn't match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Ferrion has nothing at javac's range.
    Missing,
    /// Intersections with the same parts in another order.
    Order,
    /// Another type.
    Type,
    /// Another constant value.
    Value,
    /// Another method chosen.
    Method,
    /// The same method, with another type as invoked.
    Instantiation,
    /// Another finality.
    Finality,
    /// Another answer to a relation query.
    Answer,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Missing => "missing",
            Category::Order => "order",
            Category::Type => "type",
            Category::Value => "value",
            Category::Method => "method",
            Category::Instantiation => "instantiation",
            Category::Finality => "finality",
            Category::Answer => "answer",
        }
    }
}

/// A type string's intersection parts, sorted, if it is an intersection at the top level.
fn sorted_parts(ty: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in ty.char_indices() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth = depth.saturating_sub(1),
            '&' if depth == 0 => {
                parts.push(&ty[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(&ty[start..]);
    parts.sort_unstable();
    Some(parts)
}

/// The category of a type mismatch.
pub fn type_category(javac: &str, ferrion: &str) -> Category {
    match (sorted_parts(javac), sorted_parts(ferrion)) {
        (Some(a), Some(b)) if a == b => Category::Order,
        _ => Category::Type,
    }
}

/// An entry whose values differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub aspect: Aspect,
    pub start: i64,
    pub end: i64,
    /// javac's tree kind, or variable kind, or query.
    pub kind: String,
    pub javac: String,
    pub ferrion: Option<String>,
    pub category: Category,
}

/// The comparison of one file, or of a source set's relations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compared {
    pub matched: u64,
    pub mismatches: Vec<Mismatch>,
    /// Ferrion's entries where javac has none.
    pub extra: u64,
    /// Ferrion's entries that a later milestone types (`left-to-...`).
    pub left: u64,
    /// Relation queries about a type javac couldn't resolve: not compared.
    pub unresolved: u64,
}

impl Compared {
    fn check(&mut self, entry: Mismatch, category: impl Fn(&str, &str) -> Category) {
        match &entry.ferrion {
            Some(ours) if ours.starts_with("left-to-") => self.left += 1,
            Some(ours) if *ours == entry.javac => self.matched += 1,
            Some(ours) => {
                let category = category(&entry.javac, ours);
                self.mismatches.push(Mismatch { category, ..entry });
            }
            None => self.mismatches.push(Mismatch {
                category: Category::Missing,
                ..entry
            }),
        }
    }
}

/// Compares a file's entries, but those in Lombok-annotated types, with Ferrion's.
pub fn compare(file: &FileOutput, ours: &FileTypes, offsets: &Offsets) -> Compared {
    let mut compared = Compared::default();
    let ty = |index: i64| file.type_at(index).unwrap_or("").to_owned();

    let expressions: HashMap<(i64, i64), &str> = ours
        .expressions
        .iter()
        .map(|t| (offsets.range(t.span), t.value.as_str()))
        .collect();
    let constants: HashMap<(i64, i64), &str> = ours
        .constants
        .iter()
        .map(|t| (offsets.range(t.span), t.value.as_str()))
        .collect();
    let calls: HashMap<(i64, i64), &TypedCall> = ours
        .calls
        .iter()
        .map(|c| {
            let (start, end) = offsets.range(c.span);
            ((start, if c.implicit { -1 } else { end }), c)
        })
        .collect();
    let variables: HashMap<i64, &TypedVariable> = ours
        .variables
        .iter()
        .map(|v| (offsets.utf16(v.offset), v))
        .collect();
    let mut used = HashSet::new();

    for e in file.expressions.iter().filter(|e| !file.in_lombok(e.start)) {
        let found = expressions.get(&(e.start, e.end));
        used.insert((Aspect::Expression, e.start, e.end));
        compared.check(
            mismatch(Aspect::Expression, e.start, e.end, &e.kind, ty(e.ty), found),
            type_category,
        );
    }
    for c in file.constants.iter().filter(|c| !file.in_lombok(c.start)) {
        let found = constants.get(&(c.start, c.end));
        used.insert((Aspect::Constant, c.start, c.end));
        compared.check(
            mismatch(Aspect::Constant, c.start, c.end, "", c.value.clone(), found),
            |_, _| Category::Value,
        );
    }
    for c in file.calls.iter().filter(|c| !file.in_lombok(c.start)) {
        used.insert((Aspect::Call, c.start, c.end));
        let found = calls.get(&(c.start, c.end));
        let method = found.map(|f| f.method.as_str());
        compared.check(
            mismatch(
                Aspect::Call,
                c.start,
                c.end,
                &c.kind,
                c.method.clone(),
                method.as_ref(),
            ),
            |_, _| Category::Method,
        );
        // The type as invoked, once the method matches.
        if let (Some(javac), Some(found)) = (file.type_at(c.ty), found)
            && found.method == c.method
        {
            compared.check(
                mismatch(
                    Aspect::Call,
                    c.start,
                    c.end,
                    &c.kind,
                    javac.to_owned(),
                    found.ty.as_deref().as_ref(),
                ),
                |_, _| Category::Instantiation,
            );
        }
    }
    for v in file.variables.iter().filter(|v| !file.in_lombok(v.offset)) {
        used.insert((Aspect::Variable, v.offset, v.offset));
        let found = variables.get(&v.offset);
        let finality = found.map(|f| f.finality.as_str());
        compared.check(
            mismatch(
                Aspect::Variable,
                v.offset,
                v.offset,
                &v.kind,
                v.finality.clone(),
                finality.as_ref(),
            ),
            |_, _| Category::Finality,
        );
        let ty_found = found.map(|f| f.ty.as_str());
        compared.check(
            mismatch(
                Aspect::Variable,
                v.offset,
                v.offset,
                &v.kind,
                ty(v.ty),
                ty_found.as_ref(),
            ),
            type_category,
        );
    }

    let extra = |aspect: Aspect, range: (i64, i64), value: &str, compared: &mut Compared| {
        if used.contains(&(aspect, range.0, range.1)) {
            return;
        }
        if value.starts_with("left-to-") {
            compared.left += 1;
        } else {
            compared.extra += 1;
        }
    };
    for (&range, value) in &expressions {
        extra(Aspect::Expression, range, value, &mut compared);
    }
    for (&range, value) in &constants {
        extra(Aspect::Constant, range, value, &mut compared);
    }
    for (&range, call) in &calls {
        extra(Aspect::Call, range, &call.method, &mut compared);
    }
    for (&offset, variable) in &variables {
        extra(
            Aspect::Variable,
            (offset, offset),
            &variable.ty,
            &mut compared,
        );
    }
    compared
}

fn mismatch(
    aspect: Aspect,
    start: i64,
    end: i64,
    kind: &str,
    javac: String,
    ferrion: Option<&&str>,
) -> Mismatch {
    Mismatch {
        aspect,
        start,
        end,
        kind: kind.to_owned(),
        javac,
        ferrion: ferrion.map(|s| (*s).to_owned()),
        category: Category::Missing,
    }
}

/// javac's answer to a relation query, in the form [`Attributed::relations`] gives Ferrion's.
pub fn javac_answer(relations: &Relations, query: &Query) -> String {
    let ty = |i: &usize| relations.types.get(*i).cloned().unwrap_or_default();
    let maybe = |r: &Option<usize>| r.as_ref().map_or("threw".to_owned(), ty);
    let answer = |a: &Answer| {
        match a {
            Answer::Yes => "yes",
            Answer::No => "no",
            Answer::Threw => "threw",
        }
        .to_owned()
    };
    match query {
        Query::Erasure(_, r) | Query::Capture(_, r) | Query::Member(_, _, r) => maybe(r),
        Query::Supertypes(_, list) => list.iter().map(ty).collect::<Vec<_>>().join(","),
        Query::Subtype(_, _, a)
        | Query::Assignable(_, _, a)
        | Query::Same(_, _, a)
        | Query::Subsignature(_, _, _, a)
        | Query::Overrides(_, _, _, a) => answer(a),
        Query::Threw(_, exception) => format!("threw {exception}"),
    }
}

/// Whether a query is about a type javac couldn't resolve, or javac's answer has one: a class
/// a jar names that javac's classpath lacks, which javac reports nothing about where nothing
/// needs it (docs/design/types-oracle-normalization.md, "What is compared").
fn unresolved(relations: &Relations, query: &Query, answer: &str) -> bool {
    let error = |i: &usize| {
        relations
            .types
            .get(*i)
            .is_some_and(|t| t.contains("error "))
    };
    answer.contains("error ")
        || dotted_descriptor(answer)
        || match query {
            Query::Subtype(s, t, _) | Query::Assignable(s, t, _) | Query::Same(s, t, _) => {
                error(s) || error(t)
            }
            Query::Erasure(t, _) | Query::Capture(t, _) | Query::Supertypes(t, _) => error(t),
            Query::Member(t, m, _) => error(t) || dotted_descriptor(m),
            Query::Subsignature(t, m, n, _) => {
                error(t) || dotted_descriptor(m) || dotted_descriptor(n)
            }
            Query::Overrides(m, n, _, _) => dotted_descriptor(m) || dotted_descriptor(n),
            Query::Threw(..) => false,
        }
}

/// Whether a method key's descriptor names a class with `.`s, which is how javac writes the
/// erasure of a class it couldn't resolve (`()Lcom.google.Foo;`); a local class's
/// `Llocal <file>:<offset>;` has a `.` in its file's name and isn't one.
fn dotted_descriptor(text: &str) -> bool {
    let Some(open) = text.find('(') else {
        return false;
    };
    let mut rest = &text[open..];
    while let Some(at) = rest.find('L') {
        let after = &rest[at + 1..];
        let Some(end) = after.find(';') else {
            return false;
        };
        let name = &after[..end];
        if !name.starts_with("local ") && name.contains('.') && !name.contains(['(', ')', ' ']) {
            return true;
        }
        rest = &after[end + 1..];
    }
    false
}

/// Compares the relation queries' answers with Ferrion's: query `i` is at `[i, i]`; those
/// about a type javac couldn't resolve are counted apart.
pub fn compare_relations(relations: &Relations, ours: &[Option<String>]) -> Compared {
    let mut compared = Compared::default();
    for (i, query) in relations.queries.iter().enumerate() {
        if let Query::Threw(..) = query {
            continue;
        }
        let theirs = javac_answer(relations, query);
        if unresolved(relations, query, &theirs) {
            compared.unresolved += 1;
            continue;
        }
        let found = ours.get(i).and_then(Option::as_deref);
        let i = i as i64;
        compared.check(
            mismatch(Aspect::Relation, i, i, query.name(), theirs, found.as_ref()),
            |_, _| Category::Answer,
        );
    }
    compared
}

/// The known differences: entries whose mismatch is explained, each under a category.
///
/// `differences/types-known.txt` has `category <name>: <explanation>` lines, then `<category>
/// <file> <aspect> <start>-<end>` for each entry, where the file is the report's (project, then
/// path), the aspect `expression`, `constant`, `call`, or `variable`, and the range javac's (a
/// variable's is its offset twice). `#` starts a comment.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Known {
    pub categories: Vec<(String, String)>,
    pub entries: HashMap<(String, Aspect, i64, i64), String>,
}

const KNOWN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/differences/types-known.txt");

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
            let [category, file, aspect, range] = fields[..] else {
                bail!(
                    "line {}: expected `<category> <file> <aspect> <start>-<end>`",
                    i + 1
                );
            };
            if !known.categories.iter().any(|(name, _)| name == category) {
                bail!("line {}: unknown category `{category}`", i + 1);
            }
            let aspect = Aspect::parse(aspect)
                .with_context(|| format!("line {}: unknown aspect `{aspect}`", i + 1))?;
            let (start, end) = range
                .split_once('-')
                .and_then(|(s, e)| Some((s.parse().ok()?, e.parse().ok()?)))
                .with_context(|| format!("line {}: `{range}` isn't `<start>-<end>`", i + 1))?;
            known
                .entries
                .insert((file.to_owned(), aspect, start, end), category.to_owned());
        }
        Ok(known)
    }

    /// The category a mismatch in `file` is known under.
    pub fn category(&self, file: &str, mismatch: &Mismatch) -> Option<&str> {
        self.entries
            .get(&(
                file.to_owned(),
                mismatch.aspect,
                mismatch.start,
                mismatch.end,
            ))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use ferrion_diagnostics::SourceText;

    use super::*;
    use crate::types::oracle::{Call, Constant, Expression, Variable};

    fn file() -> FileOutput {
        FileOutput {
            types: vec![
                "int".into(),
                "java/lang/String".into(),
                "java/lang/Comparable<?>&java/io/Serializable".into(),
            ],
            expressions: vec![
                Expression {
                    start: 0,
                    end: 1,
                    kind: "IDENTIFIER".into(),
                    ty: 0,
                },
                Expression {
                    start: 2,
                    end: 7,
                    kind: "CONDITIONAL_EXPRESSION".into(),
                    ty: 2,
                },
                Expression {
                    start: 8,
                    end: 9,
                    kind: "IDENTIFIER".into(),
                    ty: 1,
                },
                Expression {
                    start: 20,
                    end: 21,
                    kind: "IDENTIFIER".into(),
                    ty: 1,
                },
            ],
            constants: vec![Constant {
                start: 0,
                end: 1,
                value: "int 1".into(),
            }],
            calls: vec![
                Call {
                    start: 3,
                    end: -1,
                    kind: "METHOD_INVOCATION".into(),
                    method: "method java/lang/Object.<init>()V".into(),
                    ty: -1,
                },
                Call {
                    start: 2,
                    end: 7,
                    kind: "METHOD_INVOCATION".into(),
                    method: "method p/A.m()I".into(),
                    ty: 0,
                },
            ],
            variables: vec![Variable {
                offset: 8,
                kind: "LOCAL_VARIABLE".into(),
                finality: "effectively-final".into(),
                ty: 1,
            }],
            lombok: vec![(15, 25)],
            ..FileOutput::default()
        }
    }

    fn typed(start: u32, end: u32, value: &str) -> Typed {
        Typed {
            span: Span { start, end },
            value: value.into(),
        }
    }

    #[test]
    fn compares_each_aspect_by_range() {
        let source = SourceText::decode(b"x ? a : b y;".to_vec()).unwrap();
        let offsets = Offsets::new(&source);
        let ours = FileTypes {
            expressions: vec![
                typed(0, 1, "int"),
                typed(2, 7, "java/io/Serializable&java/lang/Comparable<?>"),
                typed(8, 9, "left-to-b3c inference"),
                typed(10, 11, "int"),
            ],
            constants: vec![typed(0, 1, "int 2")],
            calls: vec![
                TypedCall {
                    span: Span { start: 3, end: 3 },
                    implicit: true,
                    method: "method java/lang/Object.<init>()V".into(),
                    ty: None,
                },
                TypedCall {
                    span: Span { start: 2, end: 7 },
                    implicit: false,
                    method: "method p/A.m()I".into(),
                    ty: Some("long".into()),
                },
            ],
            variables: vec![TypedVariable {
                offset: 8,
                finality: "mutable".into(),
                ty: "java/lang/String".into(),
            }],
        };
        let compared = compare(&file(), &ours, &offsets);
        let categories: Vec<_> = compared
            .mismatches
            .iter()
            .map(|m| (m.aspect, m.category))
            .collect();
        assert_eq!(
            categories,
            [
                (Aspect::Expression, Category::Order),
                (Aspect::Constant, Category::Value),
                (Aspect::Call, Category::Instantiation),
                (Aspect::Variable, Category::Finality),
            ]
        );
        // The expression at `0-1`, both calls' methods, and the variable's type.
        assert_eq!(compared.matched, 4);
        assert_eq!(compared.left, 1);
        // The expression at 10-11.
        assert_eq!(compared.extra, 1);
    }

    #[test]
    fn missing_entries() {
        let offsets = Offsets::from_text("x ? a : b y;", false);
        let compared = compare(&file(), &FileTypes::default(), &offsets);
        assert!(
            compared
                .mismatches
                .iter()
                .all(|m| m.category == Category::Missing)
        );
        // Three expressions, a constant, two calls, and a variable's finality and type: the
        // expression in the Lombok type isn't compared.
        assert_eq!(compared.mismatches.len(), 8);
    }

    #[test]
    fn intersection_order() {
        assert_eq!(type_category("A&B<C&D>", "B<C&D>&A"), Category::Order);
        assert_eq!(type_category("A&B", "A&C"), Category::Type);
        assert_eq!(type_category("A", "B"), Category::Type);
    }

    #[test]
    fn relations() {
        let relations = Relations {
            sites: Vec::new(),
            queries: vec![
                Query::Subtype(0, 1, Answer::Yes),
                Query::Supertypes(0, vec![1, 0]),
                Query::Erasure(1, None),
            ],
            types: vec!["int".into(), "long".into()],
            error: None,
        };
        assert_eq!(javac_answer(&relations, &relations.queries[1]), "long,int");
        let compared =
            compare_relations(&relations, &[Some("yes".into()), Some("long".into()), None]);
        assert_eq!(compared.matched, 1);
        assert_eq!(compared.mismatches[0].category, Category::Answer);
        assert_eq!(compared.mismatches[1].category, Category::Missing);
    }

    #[test]
    fn known_differences() {
        let known = Known::parse(
            "# comment\ncategory lub-depth: javac cuts lub off\n\
             lub-depth jedis/src/A.java expression 10-20 # why\n",
        )
        .unwrap();
        let mismatch = Mismatch {
            aspect: Aspect::Expression,
            start: 10,
            end: 20,
            kind: "CONDITIONAL_EXPRESSION".into(),
            javac: "A".into(),
            ferrion: None,
            category: Category::Missing,
        };
        assert_eq!(
            known.category("jedis/src/A.java", &mismatch),
            Some("lub-depth")
        );
        assert!(Known::parse("category a: b\na x/B.java call 1-2\n").is_ok());
        assert!(Known::parse("category a: b\na x/B.java tree 1-2\n").is_err());
        assert!(Known::parse("other a/B.java expression 1-2\n").is_err());
        assert!(Known::load().is_ok());
    }
}
