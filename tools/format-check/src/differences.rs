//! `format-check differences`: the departures from GJF, checked (see "Departures, checked" in
//! docs/design/formatter.md).
//!
//! Each case in `differences/` is an input, `<name>.java`, written for the purpose, GJF's output
//! for it, `<name>.gjf`, and Ferrion's, `<name>.ferrion`. The command runs GJF on every input to a
//! fixed point and Ferrion's formatter on it at the latest release, and fails if either no longer
//! gives its expected output, or if the two expected outputs are the same (the case no longer
//! shows a departure). When the GJF pin moves, a departure GJF no longer makes shows up here.
//!
//! GJF's side is generated: `--bless` writes the `.gjf` files that changed. Ferrion's side is the
//! departure as docs/style.md decides it, so it's written by hand and never blessed. While the
//! formatter is the stand-in (see [`crate::formatter`]), Ferrion's side is reported as pending
//! and not checked; `cargo test` checks it, without a JDK, once the formatter is wired in.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use ferrion_diagnostics::SourceText;
use ferrion_java_version::LATEST_RELEASE;

use crate::diff::Diff;
use crate::formatter::STAND_IN;
use crate::gjf::{Gjf, Input, Outcome};
use crate::{Options, ferrion_output};

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

/// Checks every case, and returns whether all of them hold. With `--bless`, GJF's outputs that
/// changed are written instead.
pub fn run(options: &Options) -> Result<bool> {
    let cases = cases()?;
    let dirs = corpus::Dirs::new()?;
    let gjf = Gjf::new(&dirs, options.jobs, options.batch)?;
    let inputs: Vec<Vec<u8>> = cases
        .iter()
        .map(|case| std::fs::read(case).with_context(|| format!("reading {}", case.display())))
        .collect::<Result<_>>()?;
    let names: Vec<String> = cases
        .iter()
        .map(|case| case.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let outcomes = gjf.fixed_points(
        &names
            .iter()
            .zip(&inputs)
            .map(|(name, bytes)| Input { name, bytes })
            .collect::<Vec<_>>(),
    )?;

    let mut failures = 0;
    let mut changed = 0;
    for ((case, input), outcome) in cases.iter().zip(&inputs).zip(outcomes) {
        let name = case.file_stem().unwrap().to_string_lossy();
        let gjf_output = match outcome {
            Outcome::Settled { output, .. } => output,
            Outcome::Rejected(error) | Outcome::Unsettled(error) => {
                println!("{name}: GJF gives no fixed point: {error}");
                failures += 1;
                continue;
            }
        };
        let gjf_path = case.with_extension("gjf");
        let expected_gjf = std::fs::read(&gjf_path).unwrap_or_default();
        if gjf_output != expected_gjf {
            changed += 1;
            if options.bless {
                std::fs::write(&gjf_path, &gjf_output)?;
                println!("updated {}", gjf_path.display());
            } else {
                println!("{name}: GJF's output changed:");
                print!("{}", show_diff(&expected_gjf, &gjf_output));
                failures += 1;
            }
        }
        let expected_ferrion = std::fs::read(case.with_extension("ferrion"))
            .with_context(|| format!("{name} has no .ferrion file"))?;
        if gjf_output == expected_ferrion {
            println!(
                "{name}: GJF now gives Ferrion's output, so this is no longer a departure; \
                 remove the case and its entry in docs/style.md"
            );
            failures += 1;
        }
        if !STAND_IN && let Some(problem) = check_ferrion(input, &expected_ferrion)? {
            println!("{name}: {problem}");
            failures += 1;
        }
    }
    println!(
        "{} cases, {changed} with GJF's output changed, {failures} failed{}",
        cases.len(),
        if STAND_IN {
            "; Ferrion's side pending: the formatter isn't wired in yet (see \
             tools/format-check/src/formatter.rs)"
        } else {
            ""
        }
    );
    Ok(failures == 0)
}

/// What's wrong with Ferrion's output for `input`, if anything.
fn check_ferrion(input: &[u8], expected: &[u8]) -> Result<Option<String>> {
    let source = SourceText::decode(input).map_err(|reason| anyhow!("{reason}"))?;
    Ok(match ferrion_output(&source, LATEST_RELEASE) {
        None => Some("Ferrion skipped it".to_owned()),
        Some(output) if output == expected => None,
        Some(output) => Some(format!(
            "Ferrion's output changed:\n{}",
            show_diff(expected, &output)
        )),
    })
}

/// Every hunk from `old` to `new`, indented.
fn show_diff(old: &[u8], new: &[u8]) -> String {
    let diff = Diff::new(old, new);
    let mut out = String::new();
    for hunk in diff.hunks(2) {
        for line in diff.show(&hunk).lines() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_has_both_outputs_and_departs() {
        let cases = cases().unwrap();
        assert!(!cases.is_empty());
        for case in &cases {
            let [gjf, ferrion] = ["gjf", "ferrion"].map(|extension| {
                let path = case.with_extension(extension);
                std::fs::read(&path).unwrap_or_else(|_| panic!("{} is missing", path.display()))
            });
            assert_ne!(gjf, ferrion, "{} shows no departure", case.display());
        }
    }

    /// Ferrion's side of every case, which needs no JDK. It passes trivially until the formatter
    /// is wired in.
    #[test]
    fn the_formatter_gives_every_expected_output() {
        if STAND_IN {
            return;
        }
        let mut failures = Vec::new();
        for case in cases().unwrap() {
            let input = std::fs::read(&case).unwrap();
            let expected = std::fs::read(case.with_extension("ferrion")).unwrap();
            if let Some(problem) = check_ferrion(&input, &expected).unwrap() {
                failures.push(format!("{}: {problem}", case.display()));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
