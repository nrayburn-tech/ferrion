//! The four safety properties (SCOPE.md, "Formatter"; docs/design/formatter.md, "Safety
//! properties"), checked for an input and the formatter's output for it. They hold by
//! construction, since only source tokens are printed, in order, each once; this checks the
//! construction rather than reimplementing the style.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{Parse, ParseOptions, Severity, parse};
use ferrion_syntax::comments::CommentMap;
use ferrion_syntax::{SyntaxKind, SyntaxTree};

use crate::format::adds_space_after_slashes;
use crate::printer::{Lines, trim_end};
use crate::regions;
use crate::{FormatOptions, Formatted, format_tree, modifiers};

pub(crate) mod doc_comments;
mod imports;

/// A safety property.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Property {
    /// Formatting the output gives the output.
    Idempotence,
    /// The output parses to the same tree, apart from the formatter's rewrites (modifier
    /// reordering, and the import section's sorting and removals).
    Equivalence,
    /// The input's and the output's comments are the same, in the same order, apart from what
    /// the formatter may change: trailing whitespace, the indentation of a block comment's
    /// later lines, and the space it adds after a line comment's `//`.
    Comments,
    /// The output has no syntax or version errors.
    NoNewErrors,
}

impl fmt::Display for Property {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Property::Idempotence => "idempotence",
            Property::Equivalence => "equivalence",
            Property::Comments => "comments",
            Property::NoNewErrors => "no new errors",
        })
    }
}

/// A property that doesn't hold, and where.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Violation {
    pub property: Property,
    /// The byte offset in the output, byte-order mark included, where it first differs from
    /// what was expected, when there is one place.
    pub offset: Option<usize>,
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.property, self.message)?;
        if let Some(offset) = self.offset {
            write!(f, " (at byte {offset} of the output)")?;
        }
        Ok(())
    }
}

impl std::error::Error for Violation {}

fn violation(property: Property, offset: Option<usize>, message: impl Into<String>) -> Violation {
    Violation {
        property,
        offset,
        message: message.into(),
    }
}

/// Checks the four safety properties for `input` and `output`, the formatter's output for it at
/// `options.release`. An input with errors is one the formatter skips, so its only valid output
/// is the input itself.
pub fn verify(input: &SourceText, output: &[u8], options: &FormatOptions) -> Result<(), Violation> {
    let before = parse(
        Arc::from(input.text()),
        ParseOptions {
            release: options.release,
        },
    );
    verify_parsed(input, &before, output, options)
}

/// [`verify`], for a caller that has parsed `input` at `options.release` already, as `before`.
pub fn verify_parsed(
    input: &SourceText,
    before: &Parse,
    output: &[u8],
    options: &FormatOptions,
) -> Result<(), Violation> {
    debug_assert_eq!(
        before.tree.text(),
        input.text(),
        "the parse isn't the input's"
    );
    let parse_options = ParseOptions {
        release: options.release,
    };
    if has_errors(before) {
        return if output == input.to_bytes() {
            Ok(())
        } else {
            Err(violation(
                Property::NoNewErrors,
                None,
                "the input has errors, so it isn't formatted, but the output differs from it",
            ))
        };
    }
    // Read as UTF-8, which the formatter writes: the output can have more NUL bytes in its first
    // kilobyte than the input, which `decode` would take for UTF-16.
    let output_source = SourceText::from_utf8(output).map_err(|reason| {
        violation(
            Property::NoNewErrors,
            None,
            format!("the output can't be read: {reason}"),
        )
    })?;
    let bom = if output_source.has_bom() { 3 } else { 0 };
    let after = parse(Arc::from(output_source.text()), parse_options);
    if let Some(error) = after
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error)
    {
        return Err(violation(
            Property::NoNewErrors,
            Some(bom + error.span.start as usize),
            format!("{:?}: {}", error.kind, error.message()),
        ));
    }
    equivalence(&before.tree, &after.tree, bom)?;
    imports::compare(&before.tree, &after.tree, bom)?;
    comments(&before.tree, &after.tree, bom)?;
    match format_tree(&after.tree, &output_source) {
        Formatted::Unchanged => Ok(()),
        Formatted::Changed(again) => {
            let offset = output
                .iter()
                .zip(&again)
                .position(|(a, b)| a != b)
                .unwrap_or(output.len().min(again.len()));
            Err(violation(
                Property::Idempotence,
                Some(offset),
                "formatting the output changes it",
            ))
        }
        Formatted::Skipped(_) => Err(violation(
            Property::Idempotence,
            None,
            "formatting the output skips it",
        )),
    }
}

fn has_errors(parsed: &Parse) -> bool {
    parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
}

/// A token that isn't trivia, as the equivalence check compares it.
#[derive(PartialEq, Eq, Debug)]
struct TokenSignature<'t> {
    kind: SyntaxKind,
    text: Cow<'t, str>,
    /// The innermost node containing it, by its pre-order index.
    parent: usize,
}

/// Compares the two trees outside their imports, which [`imports::compare`] compares: their
/// nodes' kinds and subtree sizes in pre-order, then their tokens other than trivia, by kind,
/// text, and the node they're in, with each run of keyword modifiers sorted. Comparing nodes
/// catches what tokens alone can't, such as `>` `>` no longer being a shift.
fn equivalence(before: &SyntaxTree, after: &SyntaxTree, bom: usize) -> Result<(), Violation> {
    let (kept_before, kept_after) = (imports::Kept::new(before), imports::Kept::new(after));
    let shape = |tree: &SyntaxTree, kept: &imports::Kept| -> Vec<(SyntaxKind, usize)> {
        tree.nodes()
            .filter(|n| kept.index(n.id()).is_some())
            .map(|n| (n.kind(), kept.count(n.subtree())))
            .collect()
    };
    let (a, b) = (shape(before, &kept_before), shape(after, &kept_after));
    if let Some(i) = first_difference(&a, &b) {
        let offset = (i < b.len()).then(|| {
            let node = after
                .nodes()
                .filter(|n| kept_after.index(n.id()).is_some())
                .nth(i)
                .unwrap();
            bom + node.span().start as usize
        });
        let describe = |nodes: &[(SyntaxKind, usize)]| {
            nodes
                .get(i)
                .map_or("nothing".to_owned(), |(kind, _)| format!("{kind:?}"))
        };
        return Err(violation(
            Property::Equivalence,
            offset,
            format!(
                "node {i} is {} in the input and {} in the output",
                describe(&a),
                describe(&b)
            ),
        ));
    }
    let (a, b) = (tokens(before, &kept_before), tokens(after, &kept_after));
    if let Some(i) = first_difference(&a, &b) {
        let output_tokens: Vec<_> = after
            .tokens()
            .filter(|t| !t.is_trivia() && kept_after.index(t.parent().id()).is_some())
            .collect();
        let offset = output_tokens.get(i).map(|t| bom + t.span().start as usize);
        let describe = |tokens: &[TokenSignature<'_>]| {
            tokens.get(i).map_or("nothing".to_owned(), |t| {
                format!("{:?} {:?}", t.kind, t.text)
            })
        };
        return Err(violation(
            Property::Equivalence,
            offset,
            format!(
                "token {i} is {} in the input and {} in the output",
                describe(&a),
                describe(&b)
            ),
        ));
    }
    Ok(())
}

/// The tokens of `tree` that aren't trivia, outside its imports, each run of keyword modifiers
/// sorted. A text block is compared apart from its line terminators, which the formatter
/// converts.
fn tokens<'t>(tree: &'t SyntaxTree, kept: &imports::Kept) -> Vec<TokenSignature<'t>> {
    let mut position = vec![usize::MAX; tree.token_count()];
    let mut signatures = Vec::new();
    for token in tree.tokens().filter(|t| !t.is_trivia()) {
        let Some(parent) = kept.index(token.parent().id()) else {
            continue;
        };
        position[token.id().index()] = signatures.len();
        let text = match token.kind() {
            SyntaxKind::TextBlock => {
                Cow::Owned(Lines::new(token.text()).collect::<Vec<_>>().join("\n"))
            }
            _ => Cow::Borrowed(token.text()),
        };
        signatures.push(TokenSignature {
            kind: token.kind(),
            text,
            parent,
        });
    }
    for node in tree.nodes().filter(|n| n.kind() == SyntaxKind::Modifiers) {
        for item in modifiers::items(node) {
            let modifiers::Item::Run(mut run) = item else {
                continue;
            };
            let start = position[run[0][0].id().index()];
            run.sort_by(|a, b| {
                (modifiers::rank(a), a[0].text()).cmp(&(modifiers::rank(b), b[0].text()))
            });
            let sorted: Vec<_> = run
                .iter()
                .flatten()
                .map(|t| TokenSignature {
                    kind: t.kind(),
                    text: Cow::Borrowed(t.text()),
                    parent: kept
                        .index(t.parent().id())
                        .expect("a modifier in an import"),
                })
                .collect();
            for (i, signature) in sorted.into_iter().enumerate() {
                signatures[start + i] = signature;
            }
        }
    }
    signatures
}

/// Compares the two trees' comments in order, by kind and by text apart from trailing
/// whitespace and the indentation of later lines. The input's line comments outside disable
/// regions compare with the space the formatter adds after their `//`, so the output's must have
/// it. In a file with imports, the comments around the import section move with its imports, so
/// [`imports::compare_comments`] compares those apart.
fn comments(before: &SyntaxTree, after: &SyntaxTree, bom: usize) -> Result<(), Violation> {
    let sections = imports::has_imports(before);
    let list = |tree: &SyntaxTree, input: bool| {
        // Both sides' regions: a doc comment in one is copied, so it isn't compared as reflowed.
        let regions = if tree.text().contains("@formatter:") {
            regions::find(tree, &CommentMap::new(tree))
        } else {
            Vec::new()
        };
        let section = if sections {
            imports::comment_region(tree)
        } else {
            0..0
        };
        let mut regions = regions.into_iter().peekable();
        let mut in_section = Vec::new();
        let mut rest = Vec::new();
        for t in tree
            .tokens()
            .filter(|t| ferrion_syntax::comments::is_comment(t.kind()))
        {
            // The comments come in order, so the regions they're past are dropped.
            while regions
                .next_if(|r| r.end.is_some_and(|end| end < t.id()))
                .is_some()
            {}
            let in_region = regions.peek().is_some_and(|r| r.start <= t.id());
            let text = if t.kind() == SyntaxKind::DocComment && !in_region {
                doc_comments::normalize(t.text())
            } else {
                doc_comments::Compared::text(normalize_comment(
                    t.text(),
                    input && !in_region && t.kind() == SyntaxKind::LineComment,
                ))
            };
            if section.contains(&t.id().index()) {
                in_section.push(imports::SectionComment {
                    token: t.id(),
                    kind: t.kind(),
                    text: text.key(),
                    start: t.span().start as usize,
                });
            } else {
                rest.push((t.kind(), text, t.span().start as usize));
            }
        }
        (rest, in_section)
    };
    let ((a, a_section), (b, b_section)) = (list(before, true), list(after, false));
    if let Some(i) = first_difference(&strip(&a), &strip(&b)) {
        let describe = |list: &[(SyntaxKind, doc_comments::Compared, usize)]| {
            list.get(i)
                .map_or("nothing".to_owned(), |(_, compared, _)| {
                    format!("{:?}", compared.text)
                })
        };
        return Err(violation(
            Property::Comments,
            b.get(i).map(|(_, _, start)| bom + start),
            format!(
                "comment {i} is {} in the input and {} in the output",
                describe(&a),
                describe(&b)
            ),
        ));
    }
    imports::compare_comments(before, after, &a_section, &b_section, bom)
}

/// A comment's text as the comparison sees it: each line without trailing whitespace, later
/// lines also without leading whitespace, joined with LF. With `add_space`, a line comment has
/// the space the formatter adds after its `//`.
fn normalize_comment(text: &str, add_space: bool) -> String {
    let trimmed = trim_end(text);
    if add_space && adds_space_after_slashes(trimmed) {
        return format!("// {}", &trimmed[2..]);
    }
    let mut lines = Lines::new(text);
    let mut normalized = trim_end(lines.next().unwrap_or_default()).to_owned();
    for line in lines {
        normalized.push('\n');
        normalized.push_str(trim_end(line).trim_start_matches([' ', '\t', '\x0c']));
    }
    normalized
}

/// The comments of a list, as the check compares them: by kind and normalized text.
fn strip(
    list: &[(SyntaxKind, doc_comments::Compared, usize)],
) -> Vec<(SyntaxKind, &doc_comments::Compared)> {
    list.iter().map(|(k, t, _)| (*k, t)).collect()
}

fn first_difference<T: PartialEq>(a: &[T], b: &[T]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or((a.len() != b.len()).then(|| a.len().min(b.len())))
}

#[cfg(test)]
mod tests;
