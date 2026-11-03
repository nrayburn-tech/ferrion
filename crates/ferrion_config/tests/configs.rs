//! The config test inputs: every file under `tests/configs/valid/` must load, and every file under
//! `tests/configs/invalid/` must fail to. The errors of the invalid ones, rendered as `pretty`
//! output shows them, are the snapshot `tests/configs/errors.txt`: the catalog of every config
//! error message, which is reviewed as a whole. Each error code must appear in it.
//!
//! To write the snapshot after an intended change, run with `FERRION_BLESS=1`, then review the
//! diff:
//!
//! ```text
//! FERRION_BLESS=1 cargo test --package ferrion_config --test configs
//! ```

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ferrion_config::{Config, ConfigErrors, ErrorCode, LoadError, LoadErrorKind, MAX_ERRORS};
use ferrion_diagnostics::SourceText;

fn dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/configs")
        .join(kind)
}

fn inputs(kind: &str) -> Vec<PathBuf> {
    let mut inputs: Vec<PathBuf> = fs::read_dir(dir(kind))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonc"))
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no inputs in {}", dir(kind).display());
    inputs
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn valid_configs_load() {
    for input in inputs("valid") {
        if let Err(error) = Config::from_path(&input) {
            panic!("{}: {error}", name(&input));
        }
    }
}

#[test]
fn error_catalog() {
    let mut catalog = String::new();
    let mut codes = BTreeSet::new();
    let mut failures = Vec::new();
    for input in inputs("invalid") {
        let name = name(&input);
        match Config::from_path(&input) {
            Ok(_) => failures.push(format!("{name}: loaded without errors")),
            Err(LoadError {
                kind: LoadErrorKind::Invalid { text, errors },
                ..
            }) => {
                codes.extend(errors.errors().iter().map(|e| e.code));
                catalog.push_str(&format!("=== {name}\n"));
                catalog.push_str(&errors.render("ferrion.jsonc", &text));
                catalog.push('\n');
            }
            Err(other) => failures.push(format!("{name}: {other}")),
        }
    }
    // A config file that isn't UTF-8 has no excerpt.
    let encoding = SourceText::from_utf8(b"{ \"name\": \"caf\xE9\" }".to_vec()).unwrap_err();
    let error = LoadError {
        path: "ferrion.jsonc".into(),
        kind: LoadErrorKind::Encoding(encoding),
    };
    catalog.push_str(&format!("=== not UTF-8\nerror: {error}\n"));

    let missing: Vec<_> = ErrorCode::ALL
        .iter()
        .filter(|c| !codes.contains(c))
        .collect();
    if !missing.is_empty() {
        failures.push(format!("no input has an error with the codes {missing:?}"));
    }
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs/errors.txt");
    let expected = fs::read_to_string(&snapshot).unwrap_or_default();
    if catalog != expected {
        if std::env::var_os("FERRION_BLESS").is_some() {
            fs::write(&snapshot, &catalog).unwrap();
        } else {
            failures.push(format!(
                "the errors differ from {} (run with FERRION_BLESS=1 to update it):\n{catalog}",
                snapshot.display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn errors(text: &str) -> ConfigErrors {
    Config::from_text(text).unwrap_err()
}

#[test]
fn errors_are_in_file_order() {
    // The duplicate key is found while parsing, before validation, but is reported in its place.
    let text = r#"{ "configs": [{ "fles": [] }], "plugins": 1, "configs": [] }"#;
    let errors = errors(text);
    let starts: Vec<u32> = errors.errors().iter().map(|e| e.span.start).collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(starts, sorted);
    let codes: Vec<ErrorCode> = errors.errors().iter().map(|e| e.code).collect();
    assert_eq!(
        codes,
        [
            ErrorCode::UnknownKey,
            ErrorCode::WrongType,
            ErrorCode::DuplicateKey
        ]
    );
}

#[test]
fn errors_are_capped() {
    let entries: Vec<String> = (0..MAX_ERRORS + 7)
        .map(|_| r#"{ "x": 1 }"#.to_owned())
        .collect();
    let text = format!(r#"{{ "configs": [{}] }}"#, entries.join(", "));
    let errors = errors(&text);
    assert_eq!(errors.errors().len(), MAX_ERRORS);
    assert_eq!(errors.omitted(), 7);
    assert!(
        errors
            .render("ferrion.jsonc", &text)
            .ends_with("and 7 more errors\n")
    );
    assert_eq!(
        errors.to_string(),
        "unknown key `x` in configs[0] (56 more)"
    );
}

#[test]
fn a_syntax_error_stops_at_itself() {
    let errors = errors(r#"{ "fles": 1, "configs": [} "#);
    assert_eq!(errors.errors().len(), 1);
    assert_eq!(errors.errors()[0].code, ErrorCode::Syntax);
}

#[test]
fn suggestions_are_kept_for_quick_fixes() {
    let text = r#"{ "configs": [{ "ignore": [], "rules": { "x": "wan" } }] }"#;
    let errors = errors(text);
    let suggestions: Vec<_> = errors
        .errors()
        .iter()
        .map(|e| (&text[e.span.range()], e.suggestion.as_deref()))
        .collect();
    assert_eq!(
        suggestions,
        [("\"ignore\"", Some("ignores")), ("\"wan\"", Some("warn"))]
    );
}

#[test]
fn the_model() {
    let config = Config::from_path(&dir("valid").join("every-key.jsonc")).unwrap();
    assert_eq!(
        config.schema.as_deref(),
        Some("./schemas/ferrion.schema.json")
    );
    assert_eq!(config.plugins, ["example"]);
    assert_eq!(
        config.linter_options.report_unused_disable_directives,
        Some(ferrion_config::Level::Off)
    );
    assert_eq!(config.baseline.as_deref(), Some("ferrion-baseline.json"));
    assert_eq!(config.entries.len(), 4);
    let ignore = &config.entries[0];
    assert!(ignore.is_global_ignore());
    assert_eq!(ignore.label(), r#"configs[0] "generated""#);
    let globs: Vec<_> = ignore
        .ignores
        .iter()
        .flatten()
        .map(|g| (g.raw.as_str(), g.pattern.as_str()))
        .collect();
    assert_eq!(
        globs,
        [("build/", "build/**"), ("./target/**", "target/**")]
    );
    let everything = &config.entries[1];
    assert!(!everything.is_global_ignore());
    let rules = everything.rules.as_ref().unwrap();
    assert_eq!(rules.len(), 6);
    assert_eq!(rules[5].name, "f");
    assert_eq!(rules[5].level, ferrion_config::Level::Error);
    assert_eq!(
        rules[5].options.as_ref().unwrap().to_json(),
        r#"{ "nested": [1, -2.5e3, true, null, { "x": "y" }] }"#
    );
    assert_eq!(rules[0].value.to_json(), r#""off""#);
    assert!(everything.format.unwrap().enabled);
    assert_eq!(config.entries[3].label(), "configs[3]");
}

#[test]
fn a_missing_file() {
    let error = Config::from_path(&dir("valid").join("not-there.jsonc")).unwrap_err();
    assert!(matches!(error.kind, LoadErrorKind::Read(_)), "{error}");
    assert!(error.to_string().starts_with("can't read the config file "));
}

#[test]
fn byte_order_mark_spans_are_after_the_mark() {
    let error = Config::from_text("\u{FEFF}{ \"x\": 1 }").unwrap_err();
    assert_eq!(error.errors()[0].span.start, 5);
    let path =
        std::env::temp_dir().join(format!("ferrion-config-bom-{}.jsonc", std::process::id()));
    fs::write(&path, b"\xEF\xBB\xBF{ \"x\": 1 }").unwrap();
    let error = Config::from_path(&path).unwrap_err();
    let _ = fs::remove_file(&path);
    let LoadErrorKind::Invalid { text, errors } = error.kind else {
        panic!("{error}");
    };
    assert_eq!(text, "{ \"x\": 1 }");
    assert_eq!(&text[errors.errors()[0].span.range()], "\"x\"");
}
