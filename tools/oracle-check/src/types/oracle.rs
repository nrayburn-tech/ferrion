//! The output of `javac-oracle types` for a source set (docs/design/types-oracle-normalization.md,
//! "The oracle's output"), cached as `names`' is, under `target/oracle/types`.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::names::oracle::{CommandOutput, PROCESSOR_ERROR, SetOutput, parse_trailer};
use crate::oracle::Diagnostic;

/// Bumped when the oracle's `types` output or the input hash changes meaning.
pub const FORMAT: &str = "types-1";

/// One file's output.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOutput {
    /// Relative to the corpus directory.
    pub file: String,
    /// 1 for a file an annotation processor wrote.
    #[serde(default)]
    pub generated: u8,
    /// 1 for a `module-info.java`, which `types` skips.
    #[serde(default)]
    pub module: u8,
    /// 1 when javac stopped before analyzing the file.
    #[serde(default)]
    pub unanalyzed: u8,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// `[start, end]` of each class that is, or has a member, annotated from `lombok`.
    #[serde(default)]
    pub lombok: Vec<(i64, i64)>,
    /// The time javac spent analyzing the file's classes, in microseconds.
    #[serde(default)]
    pub analyze_micros: u64,
    /// The file's type strings, each once; the entries name them by index.
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub expressions: Vec<Expression>,
    #[serde(default)]
    pub constants: Vec<Constant>,
    #[serde(default)]
    pub calls: Vec<Call>,
    #[serde(default)]
    pub variables: Vec<Variable>,
    /// javac threw on the file, even on its own.
    #[serde(default)]
    pub crash: Option<String>,
    /// The oracle left the file out: a module declaration.
    #[serde(default)]
    pub skipped: Option<String>,
}

impl FileOutput {
    /// javac's language errors in the file: every error but those processors report.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.is_error() && d.code != PROCESSOR_ERROR)
    }

    /// The type string at `index`, or `None` for -1.
    pub fn type_at(&self, index: i64) -> Option<&str> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.types.get(i))
            .map(String::as_str)
    }

    /// Whether `[start, end]` is inside a Lombok-annotated class.
    pub fn in_lombok(&self, start: i64) -> bool {
        self.lombok.iter().any(|&(s, e)| s <= start && start < e)
    }
}

/// An expression's type: `[start, end, kind, type]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "(i64, i64, String, i64)")]
pub struct Expression {
    /// UTF-16 offsets.
    pub start: i64,
    pub end: i64,
    /// javac's `Tree.Kind`.
    pub kind: String,
    /// The index of its type string.
    pub ty: i64,
}

impl From<(i64, i64, String, i64)> for Expression {
    fn from((start, end, kind, ty): (i64, i64, String, i64)) -> Expression {
        Expression {
            start,
            end,
            kind,
            ty,
        }
    }
}

/// A constant expression's value: `[start, end, value]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "(i64, i64, String)")]
pub struct Constant {
    pub start: i64,
    pub end: i64,
    /// `<type> <value>`.
    pub value: String,
}

impl From<(i64, i64, String)> for Constant {
    fn from((start, end, value): (i64, i64, String)) -> Constant {
        Constant { start, end, value }
    }
}

/// The method a call chose: `[start, end, kind, method, type]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "(i64, i64, String, String, i64)")]
pub struct Call {
    pub start: i64,
    /// -1 for a call javac made up: an implicit `super()`, an enum constant's `new`.
    pub end: i64,
    pub kind: String,
    /// The method's key, as `names` keys it.
    pub method: String,
    /// The index of the method's type as invoked, or -1 where javac doesn't give it.
    pub ty: i64,
}

impl From<(i64, i64, String, String, i64)> for Call {
    fn from((start, end, kind, method, ty): (i64, i64, String, String, i64)) -> Call {
        Call {
            start,
            end,
            kind,
            method,
            ty,
        }
    }
}

/// A local variable or parameter: `[offset, kind, finality, type]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "(i64, String, String, i64)")]
pub struct Variable {
    /// The offset its `local` key has.
    pub offset: i64,
    /// javac's `ElementKind`.
    pub kind: String,
    /// `final`, `effectively-final`, or `mutable`.
    pub finality: String,
    pub ty: i64,
}

impl From<(i64, String, String, i64)> for Variable {
    fn from((offset, kind, finality, ty): (i64, String, String, i64)) -> Variable {
        Variable {
            offset,
            kind,
            finality,
            ty,
        }
    }
}

/// javac's answers about a sample of the source set's declared types.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Relations {
    /// Where each sampled type is declared (`field p/A.f type`, `method p/A.m(I)V parameter
    /// 0`, ...), with the index of its type string.
    pub sites: Vec<(String, usize)>,
    pub queries: Vec<Query>,
    pub types: Vec<String>,
    /// What javac threw while answering, which lost the relations.
    pub error: Option<String>,
}

/// A yes-or-no answer, or javac threw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Threw,
}

/// One question to `javax.lang.model.util.Types` (or `Elements.overrides`), with javac's answer.
/// Types are indices into [`Relations::types`]; a type answer of `None` is javac throwing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    Erasure(usize, Option<usize>),
    Capture(usize, Option<usize>),
    Supertypes(usize, Vec<usize>),
    Subtype(usize, usize, Answer),
    Assignable(usize, usize, Answer),
    Same(usize, usize, Answer),
    /// `asMemberOf(type, member)`.
    Member(usize, String, Option<usize>),
    /// `isSubsignature` of two methods as members of the type.
    Subsignature(usize, String, String, Answer),
    /// `overrides(method, overridden, class)`, by keys.
    Overrides(String, String, String, Answer),
    /// javac threw while asking about the type (a class file it can't read), so its block of
    /// queries is lost: not compared.
    Threw(usize, String),
}

impl Query {
    pub fn name(&self) -> &'static str {
        match self {
            Query::Erasure(..) => "erasure",
            Query::Capture(..) => "capture",
            Query::Supertypes(..) => "supertypes",
            Query::Subtype(..) => "subtype",
            Query::Assignable(..) => "assignable",
            Query::Same(..) => "same",
            Query::Member(..) => "member",
            Query::Subsignature(..) => "subsignature",
            Query::Overrides(..) => "overrides",
            Query::Threw(..) => "threw",
        }
    }

    fn parse(value: &Value) -> Result<Query> {
        let fields = value.as_array().context("a query isn't an array")?;
        let name = fields.first().and_then(Value::as_str).unwrap_or("");
        let index = |i: usize| -> Result<usize> {
            fields
                .get(i)
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .with_context(|| format!("{name}: field {i} isn't a type"))
        };
        let result = |i: usize| -> Option<usize> {
            fields
                .get(i)
                .and_then(Value::as_i64)
                .and_then(|n| usize::try_from(n).ok())
        };
        let text = |i: usize| -> Result<String> {
            fields
                .get(i)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .with_context(|| format!("{name}: field {i} isn't a string"))
        };
        let answer = |i: usize| match fields.get(i) {
            Some(Value::Bool(true)) => Answer::Yes,
            Some(Value::Bool(false)) => Answer::No,
            _ => Answer::Threw,
        };
        Ok(match name {
            "erasure" => Query::Erasure(index(1)?, result(2)),
            "capture" => Query::Capture(index(1)?, result(2)),
            "supertypes" => Query::Supertypes(
                index(1)?,
                fields
                    .get(2)
                    .and_then(Value::as_array)
                    .context("supertypes: no list")?
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|n| n as usize)
                    .collect(),
            ),
            "subtype" => Query::Subtype(index(1)?, index(2)?, answer(3)),
            "assignable" => Query::Assignable(index(1)?, index(2)?, answer(3)),
            "same" => Query::Same(index(1)?, index(2)?, answer(3)),
            "member" => Query::Member(index(1)?, text(2)?, result(3)),
            "subsignature" => Query::Subsignature(index(1)?, text(2)?, text(3)?, answer(4)),
            "overrides" => Query::Overrides(text(1)?, text(2)?, text(3)?, answer(4)),
            "threw" => Query::Threw(index(1)?, text(2)?),
            other => bail!("unknown query `{other}`"),
        })
    }
}

#[derive(Deserialize)]
struct RelationsLine {
    relations: RawRelations,
}

#[derive(Deserialize)]
struct RawRelations {
    #[serde(default)]
    sites: Vec<(String, usize)>,
    #[serde(default)]
    queries: Vec<Value>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

/// The oracle's `types` output for a source set.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub files: Vec<FileOutput>,
    pub relations: Relations,
    pub set: SetOutput,
}

impl CommandOutput for Output {
    fn parse(text: &str) -> Result<Output> {
        let mut output = Output::default();
        let mut trailer = None;
        for (i, line) in text.lines().enumerate() {
            if let Some(parsed) = parse_trailer(line) {
                trailer = Some(parsed.with_context(|| format!("line {}", i + 1))?);
            } else if line.starts_with("{\"relations\"") {
                let parsed: RelationsLine = serde_json::from_str(line)
                    .with_context(|| format!("line {}: the relations", i + 1))?;
                output.relations = Relations {
                    sites: parsed.relations.sites,
                    queries: parsed
                        .relations
                        .queries
                        .iter()
                        .map(Query::parse)
                        .collect::<Result<_>>()
                        .with_context(|| format!("line {}: the relations", i + 1))?,
                    types: parsed.relations.types,
                    error: parsed.relations.error,
                };
            } else {
                let file: FileOutput = serde_json::from_str(line)
                    .with_context(|| format!("line {} of the oracle's output", i + 1))?;
                output.files.push(file);
            }
        }
        output.set =
            trailer.context("the oracle's output has no summary line; it didn't finish")?;
        Ok(output)
    }

    fn generated(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| f.generated == 1)
            .map(|f| f.file.as_str())
            .collect()
    }
}

/// The codes of javac's errors about a switch's labels: exhaustiveness, dominance, duplicate
/// labels, and fall-through to patterns (observed on JDK 21 and 27; `.details` is 27's).
pub const SWITCH_ERRORS: &[&str] = &[
    "compiler.err.not.exhaustive",
    "compiler.err.not.exhaustive.details",
    "compiler.err.not.exhaustive.statement",
    "compiler.err.not.exhaustive.statement.details",
    "compiler.err.pattern.dominated",
    "compiler.err.duplicate.case.label",
    "compiler.err.duplicate.default.label",
    "compiler.err.duplicate.unconditional.pattern",
    "compiler.err.unconditional.pattern.and.default",
    "compiler.err.flows.through.to.pattern",
    "compiler.err.flows.through.from.pattern",
];

/// Whether an error's code is one of [`SWITCH_ERRORS`].
pub fn is_switch_error(code: &str) -> bool {
    SWITCH_ERRORS.contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_parses() {
        let text = concat!(
            r#"{"file":"projects/p/src/A.java","diagnostics":[],"lombok":[[40,90]],"analyzeMicros":812,"#,
            r#""types":["int","java/lang/String","(int)java/lang/String"],"#,
            r#""expressions":[[10,15,"METHOD_INVOCATION",1],[13,14,"INT_LITERAL",0]],"#,
            r#""constants":[[13,14,"int 1"]],"#,
            r#""calls":[[3,-1,"METHOD_INVOCATION","method java/lang/Object.<init>()V",-1],[10,15,"METHOD_INVOCATION","method p/A.m(I)Ljava/lang/String;",2]],"#,
            r#""variables":[[20,"LOCAL_VARIABLE","effectively-final",1]]}"#,
            "\n",
            r#"{"file":"projects/p/src/module-info.java","module":1,"skipped":"a module declaration"}"#,
            "\n",
            r#"{"relations":{"sites":[["field p/A.f type",0]],"queries":[["erasure",0,0],["supertypes",0,[]],["subtype",0,1,false],["same",0,0,true],["assignable",1,0,"error"],["member",1,"method p/A.m()V",-1],["subsignature",1,"m1","m2",true],["overrides","m1","m2","type p/A platform",false],["threw",1,"BadClassFile"]],"types":["int","java/lang/String"]}}"#,
            "\n",
            r#"{"sourceSet":{"processors":[],"diagnostics":[],"processorRuns":[],"moduleDiagnostics":[],"millis":12}}"#,
            "\n"
        );
        let output = <Output as CommandOutput>::parse(text).unwrap();
        assert_eq!(output.files.len(), 2);
        let a = &output.files[0];
        assert_eq!(a.analyze_micros, 812);
        assert_eq!(a.type_at(a.expressions[0].ty), Some("java/lang/String"));
        assert_eq!(a.type_at(-1), None);
        assert_eq!(a.calls[0].end, -1);
        assert_eq!(a.variables[0].finality, "effectively-final");
        assert_eq!(a.constants[0].value, "int 1");
        assert!(a.in_lombok(40) && !a.in_lombok(90));
        assert_eq!(
            output.files[1].skipped.as_deref(),
            Some("a module declaration")
        );
        let relations = &output.relations;
        assert_eq!(relations.sites, [("field p/A.f type".to_owned(), 0)]);
        assert_eq!(relations.queries[0], Query::Erasure(0, Some(0)));
        assert_eq!(relations.queries[2], Query::Subtype(0, 1, Answer::No));
        assert_eq!(relations.queries[4], Query::Assignable(1, 0, Answer::Threw));
        assert_eq!(
            relations.queries[5],
            Query::Member(1, "method p/A.m()V".into(), None)
        );
        assert_eq!(relations.queries[7].name(), "overrides");
        assert_eq!(relations.queries[8], Query::Threw(1, "BadClassFile".into()));
        assert_eq!(output.set.millis, 12);
        // Without the summary line, the run didn't finish.
        assert!(<Output as CommandOutput>::parse(text.lines().next().unwrap()).is_err());
    }

    #[test]
    fn switch_errors() {
        assert!(is_switch_error("compiler.err.not.exhaustive.statement"));
        assert!(!is_switch_error("compiler.err.cant.resolve"));
    }
}
