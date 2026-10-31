//! Files of the corpus and javac's tests that show a known difference.
//!
//! `differences/known-files.txt` names, for each check, the files whose mismatch is one of the
//! known differences in `differences/` (see [`crate::differences`]), with the case that shows it.
//! A listed file's mismatch is reported as skipped, with the case, instead of failing the run. A
//! listed file that matches is left as it is.

use anyhow::{Context, Result, bail};

use crate::check::Status;
use crate::inventory::SourceFile;
use crate::run::FileResult;

const LIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/differences/known-files.txt");

/// One line of the list.
#[derive(Debug, PartialEq, Eq)]
pub struct Known {
    pub check: String,
    /// The file as reports name it: the project, then the path in it.
    pub file: String,
    /// The case in `differences/` that shows the difference.
    pub case: String,
}

/// Reads the list: `<check> <file> <case>` per line, with `#` comments.
pub fn parse(text: &str) -> Result<Vec<Known>> {
    let mut known = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [check, file, case] = fields[..] else {
            bail!("line {}: expected `<check> <file> <case>`", i + 1);
        };
        known.push(Known {
            check: check.to_owned(),
            file: file.to_owned(),
            case: case.to_owned(),
        });
    }
    Ok(known)
}

pub fn load() -> Result<Vec<Known>> {
    let text = std::fs::read_to_string(LIST).with_context(|| format!("reading {LIST}"))?;
    parse(&text).with_context(|| LIST.to_owned())
}

/// Reports the mismatches of listed files as skipped.
pub fn apply(check: &str, files: &[SourceFile], results: &mut [FileResult]) -> Result<()> {
    let known = load()?;
    for result in results {
        if !matches!(result.outcome.status, Status::Mismatched(_)) {
            continue;
        }
        let file = &files[result.file].display;
        if let Some(entry) = known.iter().find(|k| k.check == check && &k.file == file) {
            result.outcome.status = Status::Skipped(format!(
                "a known difference, shown by differences/{}.java",
                entry.case
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_case_exists() {
        let known = load().unwrap();
        for entry in &known {
            let case = format!(
                "{}/differences/{}.java",
                env!("CARGO_MANIFEST_DIR"),
                entry.case
            );
            assert!(
                std::path::Path::new(&case).exists(),
                "{} names {}, which doesn't exist",
                entry.file,
                case
            );
            assert!(
                ["tokens", "syntax", "tree"].contains(&entry.check.as_str()),
                "{}: unknown check {}",
                entry.file,
                entry.check
            );
        }
    }

    #[test]
    fn lines() {
        let known = parse("# comment\n\ntokens jdk-tests/a/B.java dot-dot # why\n").unwrap();
        assert_eq!(
            known,
            [Known {
                check: "tokens".into(),
                file: "jdk-tests/a/B.java".into(),
                case: "dot-dot".into(),
            }]
        );
        assert!(parse("tokens a/B.java").is_err());
    }
}
