//! Snapshots of javac's parser output on small inputs written for the purpose.
//!
//! Each case in `parser-cases/` shows how javac positions some construct, or what it reports for
//! some error, and docs/design/parser-oracle-normalization.md describes it. Next to each input,
//! `<case>.expected` is a snapshot of javac's parse diagnostics at each release the case names, and
//! its tree at the last of them. A case names its releases in a first line such as
//! `// releases: 8 15 16`; without one, it's run at the latest release.
//!
//! `oracle parser-cases` regenerates every snapshot with javac and fails if one changed, so a new
//! JDK that moves a position or changes an error is noticed. `--bless` writes the new snapshots.
//! The parser's side will be added to the snapshots when it exists, as the lexer's is in
//! `differences`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ferrion_java_version::LATEST_RELEASE;

use crate::oracle::{Oracle, Output, Section};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/parser-cases");

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

/// The releases a case names in its first line, or the latest.
fn releases(text: &str) -> Result<Vec<u32>> {
    let Some(list) = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("// releases:"))
    else {
        return Ok(vec![LATEST_RELEASE]);
    };
    let releases = list
        .split_whitespace()
        .map(|release| match release.parse() {
            Ok(release) if ferrion_java_version::is_supported(release) => Ok(release),
            _ => Err(anyhow!("`{release}` isn't a supported release")),
        })
        .collect::<Result<Vec<u32>>>()?;
    if releases.is_empty() {
        bail!("`// releases:` names no release");
    }
    Ok(releases)
}

/// Regenerates each case's snapshot, and returns whether all of them are unchanged. With `bless`,
/// changed snapshots are written instead.
pub fn run(oracle_jar: PathBuf, bless: bool) -> Result<bool> {
    let cases = cases()?;
    let oracle = Oracle::new(oracle_jar, &[Section::Diagnostics, Section::Tree])?;
    if oracle.jdk < LATEST_RELEASE {
        bail!(
            "JAVA_HOME is JDK {}; the cases need JDK {LATEST_RELEASE} or newer",
            oracle.jdk
        );
    }
    let texts: Vec<String> = cases
        .iter()
        .map(|case| std::fs::read_to_string(case).with_context(|| case.display().to_string()))
        .collect::<Result<_>>()?;
    let mut by_release: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, text) in texts.iter().enumerate() {
        for release in releases(text).with_context(|| cases[i].display().to_string())? {
            by_release.entry(release).or_default().push(i);
        }
    }

    let scratch =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oracle/parser-cases");
    std::fs::create_dir_all(&scratch)?;
    let mut outputs: Vec<BTreeMap<u32, Output>> = cases.iter().map(|_| BTreeMap::new()).collect();
    for (&release, indices) in &by_release {
        let paths: Vec<&Path> = indices.iter().map(|&i| cases[i].as_path()).collect();
        let mut failure = None;
        oracle.run(release, &paths, &scratch.join("run"), |n, output| {
            let i = indices[n];
            match output {
                Ok(output) => match output.failure() {
                    Some(message) => {
                        failure.get_or_insert(anyhow!(
                            "javac failed on {}: {message}",
                            paths[n].display()
                        ));
                    }
                    None => {
                        outputs[i].insert(release, output);
                    }
                },
                Err(message) => {
                    failure.get_or_insert(anyhow!("{}: {message}", paths[n].display()));
                }
            }
        })?;
        if let Some(failure) = failure {
            return Err(failure);
        }
    }

    let mut changed = 0;
    for ((case, text), outputs) in cases.iter().zip(&texts).zip(&outputs) {
        let actual = snapshot(text, outputs);
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
            print!("{}", crate::differences::line_diff(&expected, &actual));
        }
    }
    println!("{} cases, {changed} changed", cases.len());
    Ok(bless || changed == 0)
}

/// A case's snapshot: the diagnostics at each release, then the tree at the last one.
fn snapshot(text: &str, outputs: &BTreeMap<u32, Output>) -> String {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let excerpt = |start: i64, end: i64| -> String {
        match (usize::try_from(start), usize::try_from(end)) {
            (Ok(start), Ok(end)) if start <= end && end <= utf16.len() => {
                let text = String::from_utf16_lossy(&utf16[start..end]);
                if text.chars().count() > 40 {
                    format!(" {:?}…", text.chars().take(40).collect::<String>())
                } else {
                    format!(" {text:?}")
                }
            }
            _ => String::new(),
        }
    };
    let mut out = String::from(
        "# javac's parse diagnostics at each release, then its tree at the last. Regenerate with \
         `cargo oracle parser-cases --bless`.\n",
    );
    for (release, output) in outputs {
        writeln!(out, "diagnostics at {release}:").unwrap();
        let diagnostics = output.diagnostics.as_deref().unwrap_or_default();
        if diagnostics.is_empty() {
            writeln!(out, "  none").unwrap();
        }
        for d in diagnostics {
            writeln!(
                out,
                "  {} {} {}..{} @{} [{}] {}",
                d.kind,
                d.code,
                d.start,
                d.end,
                d.position,
                d.args.join(", "),
                d.message.lines().next().unwrap_or_default()
            )
            .unwrap();
        }
    }
    if let Some((release, output)) = outputs.last_key_value() {
        writeln!(out, "tree at {release}:").unwrap();
        for (kind, start, end, depth, preferred, shared) in output.tree.iter().flatten() {
            writeln!(
                out,
                "  {}{kind} {start}..{end} @{preferred}{}{}",
                "  ".repeat(*depth as usize),
                if *shared == 1 { " shared" } else { "" },
                excerpt(*start, *end)
            )
            .unwrap();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{Diagnostic, Node};

    /// The document that describes every case.
    const DOC: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/parser-oracle-normalization.md"
    );

    #[test]
    fn every_case_has_a_snapshot_and_is_described() {
        let doc = std::fs::read_to_string(DOC).unwrap();
        let cases = cases().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let name = case.file_name().unwrap().to_str().unwrap();
            assert!(
                expected_path(&case).exists(),
                "{name} has no snapshot; run `cargo oracle parser-cases --bless`"
            );
            assert!(
                doc.contains(&format!("parser-cases/{name}")),
                "{name} isn't described in {DOC}"
            );
            releases(&std::fs::read_to_string(&case).unwrap()).unwrap();
        }
    }

    #[test]
    fn release_lines() {
        assert_eq!(releases("class A {}").unwrap(), [LATEST_RELEASE]);
        assert_eq!(
            releases("// releases: 8 16\nrecord R() {}").unwrap(),
            [8, 16]
        );
        assert!(releases("// releases: 7").is_err());
        assert!(releases("// releases:").is_err());
    }

    #[test]
    fn snapshots() {
        let text = "record R() {}";
        let diagnostic = Diagnostic {
            kind: "ERROR".into(),
            code: "compiler.err.feature.not.supported.in.source.plural".into(),
            start: 0,
            position: 0,
            end: 0,
            args: vec![
                "compiler.misc.feature.records".into(),
                "8".into(),
                "16".into(),
            ],
            message: "records are not supported in -source 8\n  (use -source 16)".into(),
        };
        let tree: Vec<Node> = vec![
            ("COMPILATION_UNIT".into(), 0, 13, 0, 0, 0),
            ("RECORD".into(), 0, 13, 1, 0, 0),
            ("MODIFIERS".into(), -1, -1, 2, -1, 1),
        ];
        let outputs = BTreeMap::from([
            (
                8,
                Output {
                    diagnostics: Some(vec![diagnostic]),
                    tree: Some(Vec::new()),
                    ..Output::default()
                },
            ),
            (
                16,
                Output {
                    diagnostics: Some(Vec::new()),
                    tree: Some(tree),
                    ..Output::default()
                },
            ),
        ]);
        let snapshot = snapshot(text, &outputs);
        let lines: Vec<&str> = snapshot.lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                "diagnostics at 8:",
                "  ERROR compiler.err.feature.not.supported.in.source.plural 0..0 @0 \
                 [compiler.misc.feature.records, 8, 16] records are not supported in -source 8",
                "diagnostics at 16:",
                "  none",
                "tree at 16:",
                "  COMPILATION_UNIT 0..13 @0 \"record R() {}\"",
                "    RECORD 0..13 @0 \"record R() {}\"",
                "      MODIFIERS -1..-1 @-1 shared",
            ]
        );
    }
}
