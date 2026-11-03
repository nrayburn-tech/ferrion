//! The JSON Schema, `schemas/ferrion.schema.json`: it must be what `cargo codegen` generates, every
//! valid test config must pass it, and every invalid one whose first line is `// schema: rejects`
//! must fail it. An invalid config marked `// schema: accepts` is one the schema can't tell from a
//! valid one (a glob's syntax, a duplicate key); one marked `// schema: n/a` isn't JSONC at all.

use std::fs;
use std::path::{Path, PathBuf};

use ferrion_config::jsonc::{self, Value, ValueKind};
use ferrion_config::{SCHEMA_PATH, json_schema};
use ferrion_diagnostics::SourceText;

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
}

fn inputs(kind: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/configs")
        .join(kind);
    let mut inputs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonc"))
        .collect();
    inputs.sort();
    inputs
}

fn to_serde(value: &Value) -> serde_json::Value {
    match &value.kind {
        ValueKind::Null => serde_json::Value::Null,
        ValueKind::Bool(b) => serde_json::Value::Bool(*b),
        ValueKind::Number(n) => serde_json::from_str(n).unwrap(),
        ValueKind::String(s) => serde_json::Value::String(s.clone()),
        ValueKind::Array(items) => serde_json::Value::Array(items.iter().map(to_serde).collect()),
        ValueKind::Object(members) => serde_json::Value::Object(
            members
                .iter()
                .map(|m| (m.key.clone(), to_serde(&m.value)))
                .collect(),
        ),
    }
}

fn read(path: &Path) -> String {
    SourceText::from_utf8(fs::read(path).unwrap())
        .unwrap()
        .text()
        .to_owned()
}

#[test]
fn the_committed_schema_is_current() {
    let committed = fs::read_to_string(workspace_root().join(SCHEMA_PATH)).unwrap_or_default();
    assert!(
        committed == json_schema(),
        "{SCHEMA_PATH} is out of date: run `cargo codegen`"
    );
}

#[test]
fn test_configs_against_the_schema() {
    let schema: serde_json::Value = serde_json::from_str(&json_schema()).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let mut failures = Vec::new();
    for input in inputs("valid") {
        let instance = to_serde(&jsonc::parse(&read(&input)).unwrap().value);
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|e| e.to_string())
            .collect();
        if !errors.is_empty() {
            failures.push(format!("{}: {}", input.display(), errors.join("; ")));
        }
    }
    let mut rejected = 0;
    for input in inputs("invalid") {
        let text = read(&input);
        let expectation = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// schema: "));
        let instance = || to_serde(&jsonc::parse(&text).unwrap().value);
        match expectation {
            Some("rejects") => {
                rejected += 1;
                if validator.is_valid(&instance()) {
                    failures.push(format!("{}: the schema accepts it", input.display()));
                }
            }
            Some("accepts") => {
                if !validator.is_valid(&instance()) {
                    failures.push(format!(
                        "{}: the schema rejects it, so mark it `// schema: rejects`",
                        input.display()
                    ));
                }
            }
            Some("n/a") => assert!(jsonc::parse(&text).is_err(), "{}", input.display()),
            _ => failures.push(format!(
                "{}: the first line must be `// schema: rejects`, `accepts`, or `n/a`",
                input.display()
            )),
        }
    }
    assert!(rejected > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
