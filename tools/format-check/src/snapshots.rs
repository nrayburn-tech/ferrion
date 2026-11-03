//! `format-check snapshots`: the formatter's snapshots, compared with GJF (see "Testing" in
//! docs/design/formatter.md).
//!
//! `cargo test` checks that Ferrion gives each snapshot's expected output. This checks that the
//! expected output is GJF's fixed point for the same input, so the snapshots record GJF's style
//! and can't drift from it. A snapshot whose expected output isn't GJF's on purpose, as for a
//! departure, or because its area's rules aren't written yet, is in [`NOT_GJF`] with the reason;
//! one that comes to match GJF fails until it's taken out of the list.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::Options;
use crate::gjf::{Gjf, Input, Outcome};

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/ferrion_formatter/tests/snapshots"
);

const EXPECTED: &str = ".formatted.java";

/// The snapshots whose expected output isn't GJF's, by path under the snapshot directory without
/// `.java`, with the reason.
const NOT_GJF: &[(&str, &str)] = &[
    (
        "core/copied-text",
        "a text block's content isn't re-indented (docs/style.md)",
    ),
    (
        "core/modifiers",
        "a comment between modifiers pins them (docs/style.md)",
    ),
    ("core/regions", "disable regions are copied (docs/style.md)"),
    (
        "core/comments",
        "an unused import with a comment is kept (docs/style.md, \"Imports\")",
    ),
    (
        "declarations/compilation-unit",
        "an unused import with a comment is kept (docs/style.md, \"Imports\")",
    ),
    (
        "imports/comment-after-last",
        "a comment on the line after the last import stays with what follows (docs/style.md)",
    ),
    (
        "imports/comment-keeps-duplicate",
        "a duplicate import with a comment is kept, so the comment isn't lost (docs/style.md)",
    ),
    (
        "imports/comment-keeps-unused",
        "an unused import with a comment is kept (docs/style.md)",
    ),
    (
        "imports/comments-above",
        "a comment belongs to the import below it, and GJF's to the one above (docs/style.md)",
    ),
    (
        "imports/escapes",
        "GJF rejects a Unicode escape in an import (docs/style.md)",
    ),
    (
        "imports/floating-comments",
        "GJF rejects a header after a blank line, a block comment between imports, and a comment \
         inside one (docs/style.md)",
    ),
    (
        "imports/header-above-first",
        "an unused first import right below a comment is kept (docs/style.md)",
    ),
    (
        "imports/java-lang-and-wildcards",
        "java.lang and wildcard imports are never removed (docs/style.md)",
    ),
    (
        "imports/markdown-before-23",
        "below release 23, a `///` comment isn't a doc comment and uses no import (docs/style.md)",
    ),
    (
        "imports/markers",
        "formatter markers pin the import section (docs/style.md)",
    ),
    (
        "imports/suppression",
        "a ferrion-disable comment pins the import section (docs/style.md)",
    ),
    (
        "javadoc/departure-at-after-break",
        "a word starting with `@` that a line break would put at the start of a line re-indents \
         the comment only: GJF puts it there, where javadoc reads a block tag, and a second run \
         adds a blank line before it (docs/style.md)",
    ),
    (
        "javadoc/departure-blank-in-inline-tag",
        "a blank line in an inline tag is a space: GJF adds a `<p>` in the tag, and another on \
         each run (docs/style.md)",
    ),
    (
        "javadoc/departure-escaped-at",
        "a doc comment with a Unicode escape for a character other than a letter or digit is \
         re-indented only (docs/style.md)",
    ),
    (
        "javadoc/departure-hyphen-at-a-break",
        "no line breaks after a word ending with a hyphen: GJF breaks there, then joins the two \
         words on its next run (docs/style.md)",
    ),
    (
        "javadoc/departure-stars",
        "a line's leading stars all go, as javadoc reads them: GJF keeps all but one, adding \
         them to the text (docs/style.md)",
    ),
    (
        "javadoc/departure-uppercase-tag",
        "a block tag whose name starts with an uppercase letter stays a block tag: GJF joins it \
         into the text before it (docs/style.md)",
    ),
    (
        "javadoc/fallback-starless-line",
        "a block comment's text doesn't change: GJF adds a star to a doc comment's lines without \
         one (docs/style.md)",
    ),
    (
        "javadoc/placement",
        "an unused import with a comment, here a doc comment, is kept (docs/style.md, \"Imports\")",
    ),
    ("javadoc/pre-at", PRE_INDENTATION),
    ("javadoc/pre-at-code", PRE_INDENTATION),
    ("javadoc/pre-at-tags", PRE_INDENTATION),
    ("javadoc/pre-blank", PRE_INDENTATION),
    ("javadoc/pre-indented", PRE_INDENTATION),
    ("javadoc/pre-margin", PRE_INDENTATION),
    ("javadoc/pre-starless", PRE_INDENTATION),
    ("javadoc/pre-tags", PRE_INDENTATION),
    (
        "javadoc/regions",
        "doc comments in disable regions are copied (docs/style.md)",
    ),
    (
        "markdown/before-23",
        "below release 23, a `///` comment is a line comment, and isn't reflowed (docs/style.md)",
    ),
    (
        "markdown/departure-block-start",
        "no line starts with a word that would start a block: GJF breaks before it, making a \
         list, heading, quote, or fence, and `#` and fences change again on its next run \
         (docs/style.md)",
    ),
    (
        "markdown/departure-code-span",
        "a code span is copied as written: GJF collapses its spaces (docs/style.md)",
    ),
    (
        "markdown/departure-escapes",
        "a `///` comment with a Unicode escape for other than a letter or digit is re-indented \
         only (docs/style.md)",
    ),
    (
        "markdown/departure-fallback-every-line",
        "a `///` comment that isn't reflowed has every line moved: GJF moves only the first \
         (docs/style.md)",
    ),
    (
        "markdown/departure-hard-break",
        "a two-space hard break is written with a backslash: GJF drops it and joins the lines \
         (docs/style.md)",
    ),
    (
        "markdown/departure-heading",
        "a heading stays on one line: GJF wraps it, and the rest becomes a paragraph on its next \
         run (docs/style.md)",
    ),
    (
        "markdown/departure-html-blocks",
        "an HTML block is copied as written, to the blank line that ends it: GJF reflows some \
         with Javadoc's rules, and drops a block quote's `>` from a copied one (docs/style.md)",
    ),
    (
        "markdown/departure-hyphen-at-a-break",
        "no line breaks after a word ending with a hyphen: GJF breaks there, then joins the two \
         words on its next run (docs/style.md)",
    ),
    (
        "markdown/departure-item-first-block",
        "a list item's first block starts on its marker's line: GJF puts the marker alone \
         before a blank line, which ends the item on its next run (docs/style.md)",
    ),
    (
        "markdown/departure-list-marker-spaces",
        "one space follows a list marker: GJF keeps two, but continues the item at two, and its \
         later paragraphs leave it on its next run (docs/style.md)",
    ),
    (
        "markdown/departure-not-doc-comment",
        "a long `///` comment that isn't a doc comment is a line comment, and isn't wrapped: GJF \
         wraps it with `//` (docs/style.md)",
    ),
    (
        "markdown/departure-snippet",
        "a `///` comment with a `{@snippet}` is re-indented only: GJF makes the snippet a \
         paragraph of its own (docs/style.md)",
    ),
    (
        "markdown/departure-tags",
        "a block tag whose body starts with fenced code, or holds an HTML block, is re-indented \
         only: GJF indents the block's first line and not its others, changing what it holds \
         (docs/style.md)",
    ),
    (
        "markdown/regions",
        "doc comments in disable regions are copied (docs/style.md)",
    ),
];

/// The reason for the snapshots of the `<pre>` departure.
const PRE_INDENTATION: &str = "`<pre>` content keeps its indentation after the star, where GJF \
                               removes a margin (docs/style.md)";

/// Each snapshot's input under [`DIR`], in path order, as its path and its name in [`NOT_GJF`].
fn inputs(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut paths = Vec::new();
    collect(dir, &mut paths)?;
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|path| {
            let name = path.strip_prefix(dir).unwrap().with_extension("");
            let name = name
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            (path, name)
        })
        .collect())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let file_name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            collect(&path, out)?;
        } else if file_name.ends_with(".java") && !file_name.ends_with(EXPECTED) {
            out.push(path);
        }
    }
    Ok(())
}

/// Compares the snapshots named by `targets` (an area, such as `statements`, or a snapshot,
/// such as `core/comments`; all of them if none) with GJF, and returns whether they all hold.
pub fn run(targets: &[String], options: &Options) -> Result<bool> {
    let dir = Path::new(DIR)
        .canonicalize()
        .with_context(|| format!("finding {DIR}"))?;
    for (name, _) in NOT_GJF {
        if !dir.join(format!("{name}.java")).exists() {
            bail!("NOT_GJF lists {name}, which isn't a snapshot");
        }
    }
    let selected: Vec<(PathBuf, String)> = inputs(&dir)?
        .into_iter()
        .filter(|(_, name)| {
            targets.is_empty()
                || targets.iter().any(|target| {
                    name == target
                        || name.starts_with(&format!("{}/", target.trim_end_matches('/')))
                })
        })
        .collect();
    if selected.is_empty() {
        bail!("no snapshots match {}", targets.join(", "));
    }

    let dirs = corpus::Dirs::new()?;
    let gjf = Gjf::new(&dirs, options.jobs, options.batch)?;
    let inputs: Vec<Vec<u8>> = selected
        .iter()
        .map(|(path, _)| std::fs::read(path).with_context(|| format!("reading {}", path.display())))
        .collect::<Result<_>>()?;
    let file_names: Vec<String> = selected
        .iter()
        .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let outcomes = gjf.fixed_points(
        &file_names
            .iter()
            .zip(&inputs)
            .map(|(name, bytes)| Input { name, bytes })
            .collect::<Vec<_>>(),
    )?;

    let mut failures = 0;
    let mut listed = 0;
    for ((path, name), outcome) in selected.iter().zip(outcomes) {
        let not_gjf = NOT_GJF
            .iter()
            .find(|(case, _)| case == name)
            .map(|(_, why)| *why);
        let expected_path = path.with_file_name(format!(
            "{}{EXPECTED}",
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches(".java")
        ));
        let expected = std::fs::read(&expected_path).unwrap_or_default();
        let gjf_output = match outcome {
            Outcome::Settled { output, .. } => output,
            Outcome::Rejected(error) | Outcome::Unsettled(error) => {
                if let Some(why) = not_gjf {
                    println!("{name}: not GJF's: {why}; GJF gives no fixed point: {error}");
                    listed += 1;
                } else {
                    println!("{name}: GJF gives no fixed point: {error}");
                    failures += 1;
                }
                continue;
            }
        };
        match (gjf_output == expected, not_gjf) {
            (true, None) => {}
            (true, Some(_)) => {
                println!("{name}: the expected output is GJF's now; remove it from NOT_GJF");
                failures += 1;
            }
            (false, Some(why)) => {
                println!("{name}: not GJF's: {why}");
                listed += 1;
            }
            (false, None) => {
                println!(
                    "{name}: the expected output isn't GJF's; from {} to GJF's:",
                    expected_path.file_name().unwrap().to_string_lossy()
                );
                print!("{}", crate::differences::show_diff(&expected, &gjf_output));
                failures += 1;
            }
        }
    }
    println!(
        "{} snapshots, {} GJF's, {listed} listed as not GJF's, {failures} failed",
        selected.len(),
        selected.len() - listed - failures,
    );
    Ok(failures == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_snapshot_exists() {
        let dir = Path::new(DIR);
        for (name, _) in NOT_GJF {
            let path = dir.join(format!("{name}.java"));
            assert!(
                path.exists(),
                "NOT_GJF lists {}, which is missing",
                path.display()
            );
        }
    }
}
