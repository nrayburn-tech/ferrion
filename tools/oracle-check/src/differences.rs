//! Known differences from javac that the corpus doesn't exercise.
//!
//! The corpus has no lexical errors and no byte-order marks, so how the lexer and javac differ on
//! malformed input, and the allowed differences no corpus file triggers, would otherwise go
//! unchecked. Each case in `differences/` is a small input written for the purpose, with a snapshot
//! next to it (`<id>.expected`): javac's items and errors, the lexer's items and errors, both in
//! javac's terms, the allowed differences counted, and where the two first differ. The cases are
//! described in docs/design/lexer-oracle-normalization.md.
//!
//! `oracle differences` regenerates every snapshot with javac and fails if one changed, which
//! catches a change on either side: the lexer's recovery, or javac's in a new JDK. `--bless`
//! writes the new snapshots instead. The lexer's side is also checked by `cargo test`, without a
//! JDK.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ferrion_diagnostics::{LineIndex, SourceText};

use crate::LATEST_RELEASE;
use crate::allowed::Allowed;
use crate::check::Check;
use crate::oracle::{Oracle, Output};
use crate::tokens::{Prepared, Tokens, first_difference, javac_items};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/differences");

/// Each case's input, in name order.
fn cases() -> Result<Vec<PathBuf>> {
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(DIR).with_context(|| format!("reading {DIR}"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "java") {
            cases.push(path);
        }
    }
    cases.sort();
    Ok(cases)
}

fn expected_path(case: &Path) -> PathBuf {
    case.with_extension("expected")
}

/// Regenerates each case's snapshot, and returns whether all of them are unchanged. With `bless`,
/// changed snapshots are written instead.
pub fn run(oracle_jar: PathBuf, bless: bool) -> Result<bool> {
    let cases = cases()?;
    let oracle = Oracle::new(oracle_jar, Tokens.sections())?;
    if oracle.jdk < LATEST_RELEASE {
        bail!(
            "JAVA_HOME is JDK {}; the cases need JDK {LATEST_RELEASE} or newer",
            oracle.jdk
        );
    }
    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle/differences");
    std::fs::create_dir_all(&scratch)?;
    let paths: Vec<&Path> = cases.iter().map(PathBuf::as_path).collect();
    let mut outputs: Vec<Option<Result<Output, String>>> = cases.iter().map(|_| None).collect();
    oracle.run(LATEST_RELEASE, &paths, &scratch.join("run"), |i, output| {
        outputs[i] = Some(output);
    })?;

    let mut changed = 0;
    for (case, output) in cases.iter().zip(outputs) {
        let output = output
            .context("the oracle produced no output")?
            .map_err(|message| anyhow!(message))
            .with_context(|| format!("running javac on {}", case.display()))?;
        if let Some(failure) = output.failure() {
            bail!("javac failed on {}: {failure}", case.display());
        }
        let bytes = std::fs::read(case)?;
        let actual = snapshot(&bytes, &output).with_context(|| case.display().to_string())?;
        let path = expected_path(case);
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if actual == expected {
            continue;
        }
        changed += 1;
        if bless {
            std::fs::write(&path, &actual)?;
            println!("updated {}", path.display());
        } else {
            println!("{} changed:", path.display());
            print!("{}", line_diff(&expected, &actual));
        }
    }
    println!("{} cases, {changed} changed", cases.len());
    Ok(bless || changed == 0)
}

fn snapshot(bytes: &[u8], output: &Output) -> Result<String> {
    let (prepared, ferrion) = ferrion_lines(bytes)?;
    let ours = prepared.items().map_err(|panic| anyhow!("{panic}"))?;
    let mut allowed = prepared.allowed();
    let javac = javac_items(output, prepared.has_bom(), &mut allowed).map_err(|e| anyhow!(e))?;

    let mut lines = vec![format!(
        "# javac --release {LATEST_RELEASE}, then Ferrion, both in javac's terms. Regenerate with \
         `cargo oracle differences --bless`."
    )];
    for item in &javac.items {
        lines.push(format!("javac   {}", prepared.show(item)));
    }
    for diagnostic in output.scan_diagnostics.iter().flatten() {
        lines.push(format!(
            "javac   {} {} at {}",
            diagnostic.kind.to_lowercase(),
            diagnostic.code,
            diagnostic.start
        ));
    }
    lines.extend(ferrion);
    for difference in Allowed::ALL {
        let count = allowed.get(difference);
        if count > 0 {
            lines.push(format!("allowed {} {count}", difference.id()));
        }
    }
    lines.push(match first_difference(ours, &javac.items, None) {
        None => "same after normalization".to_owned(),
        Some(at) => format!("first difference at item {}", at + 1),
    });
    Ok(lines.join("\n") + "\n")
}

/// The lexer's lines of a snapshot: its items, then its errors.
fn ferrion_lines(bytes: &[u8]) -> Result<(Prepared, Vec<String>)> {
    let source = SourceText::decode(bytes).map_err(|reason| anyhow!("{reason}"))?;
    let prepared = Tokens.prepare(&source, bytes);
    let items = prepared
        .items()
        .map_err(|panic| anyhow!("the lexer panicked: {panic}"))?;
    let mut lines: Vec<String> = items
        .iter()
        .map(|item| format!("ferrion {}", prepared.show(item)))
        .collect();
    let text = source.text();
    let index = LineIndex::new(text);
    let utf16 = |byte: u32| index.utf16_offset(byte) + u32::from(source.has_bom());
    for error in ferrion_lexer::lex(text).errors {
        lines.push(format!(
            "ferrion error {:?} {}..{} {:?}",
            error.kind,
            utf16(error.span.start),
            utf16(error.span.end),
            &text[error.span.range()]
        ));
    }
    Ok((prepared, lines))
}

/// The lines only in `old` (`-`) and only in `new` (`+`), in order. Snapshots are short, so a
/// longest-common-subsequence table is fine.
fn line_diff(old: &str, new: &str) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            (i, j) = (i + 1, j + 1);
        } else if j < new.len() && (i == old.len() || common[i][j + 1] > common[i + 1][j]) {
            out += &format!("  + {}\n", new[j]);
            j += 1;
        } else {
            out += &format!("  - {}\n", old[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document that describes every case.
    const DOC: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/lexer-oracle-normalization.md"
    );

    /// The lexer's side of every snapshot, which needs no JDK.
    #[test]
    fn the_lexer_matches_every_snapshot() {
        let cases = cases().unwrap();
        assert!(!cases.is_empty());
        let mut failures = Vec::new();
        for case in &cases {
            let expected = std::fs::read_to_string(expected_path(case))
                .unwrap_or_else(|_| panic!("{} has no snapshot", case.display()));
            let expected: Vec<&str> = expected
                .lines()
                .filter(|line| line.starts_with("ferrion "))
                .collect();
            let (_, actual) = ferrion_lines(&std::fs::read(case).unwrap()).unwrap();
            if actual != expected {
                failures.push(format!(
                    "{}:\n{}",
                    case.display(),
                    line_diff(&expected.join("\n"), &actual.join("\n"))
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "the lexer no longer matches these snapshots; if the change is intended, run \
             `cargo oracle differences --bless` and update the normalization document:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn the_document_describes_every_case() {
        let doc = std::fs::read_to_string(DOC).unwrap();
        for case in cases().unwrap() {
            let name = case.file_name().unwrap().to_str().unwrap();
            assert!(
                doc.contains(&format!("differences/{name}")),
                "{name} isn't described in {DOC}"
            );
        }
    }

    #[test]
    fn line_diffs() {
        assert_eq!(line_diff("a\nb\nc", "a\nc\nd"), "  - b\n  + d\n");
        assert_eq!(line_diff("", "a"), "  + a\n");
        assert_eq!(line_diff("a", "a"), "");
        assert_eq!(line_diff("a\nb", "c\nd"), "  - a\n  - b\n  + c\n  + d\n");
    }
}
