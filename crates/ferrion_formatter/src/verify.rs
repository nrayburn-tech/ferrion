//! The four safety properties (SCOPE.md, "Formatter"; docs/design/formatter.md, "Safety
//! properties"), checked for an input and the formatter's output for it. They hold by
//! construction, since only source tokens are printed, in order, each once; this checks the
//! construction rather than reimplementing the style.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use ferrion_diagnostics::SourceText;
use ferrion_parser::{Parse, ParseOptions, Severity, parse};
use ferrion_syntax::{SyntaxKind, SyntaxTree};

use crate::printer::{Lines, trim_end};
use crate::{FormatOptions, Formatted, format_tree, modifiers};

/// A safety property.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Property {
    /// Formatting the output gives the output.
    Idempotence,
    /// The output parses to the same tree, apart from the formatter's rewrites (modifier
    /// reordering).
    Equivalence,
    /// The input's and the output's comments are the same, in the same order, apart from what
    /// the formatter may change: trailing whitespace and the indentation of a block comment's
    /// later lines.
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
    let parse_options = ParseOptions {
        release: options.release,
    };
    let before = parse(Arc::from(input.text()), parse_options);
    if has_errors(&before) {
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
    let output_source = SourceText::decode(output).map_err(|reason| {
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

/// Compares the two trees: their nodes' kinds and subtree sizes in pre-order, then their tokens
/// other than trivia, by kind, text, and the node they're in, with each run of keyword modifiers
/// sorted. Comparing nodes catches what tokens alone can't, such as `>` `>` no longer being a
/// shift.
fn equivalence(before: &SyntaxTree, after: &SyntaxTree, bom: usize) -> Result<(), Violation> {
    let shape = |tree: &SyntaxTree| -> Vec<(SyntaxKind, usize)> {
        tree.nodes()
            .map(|n| {
                (
                    n.kind(),
                    n.subtree().end.index() - n.subtree().start.index(),
                )
            })
            .collect()
    };
    let (a, b) = (shape(before), shape(after));
    if let Some(i) = first_difference(&a, &b) {
        let offset =
            (i < b.len()).then(|| bom + after.nodes().nth(i).unwrap().span().start as usize);
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
    let (a, b) = (tokens(before), tokens(after));
    if let Some(i) = first_difference(&a, &b) {
        let output_tokens: Vec<_> = after.tokens().filter(|t| !t.is_trivia()).collect();
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

/// The tokens of `tree` that aren't trivia, each run of keyword modifiers sorted. A text block
/// is compared apart from its line terminators, which the formatter converts.
fn tokens(tree: &SyntaxTree) -> Vec<TokenSignature<'_>> {
    let mut position = vec![usize::MAX; tree.token_count()];
    let mut signatures = Vec::new();
    for token in tree.tokens().filter(|t| !t.is_trivia()) {
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
            parent: token.parent().id().index(),
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
                    parent: t.parent().id().index(),
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
/// whitespace and the indentation of later lines.
fn comments(before: &SyntaxTree, after: &SyntaxTree, bom: usize) -> Result<(), Violation> {
    let list = |tree: &SyntaxTree| -> Vec<(SyntaxKind, String, usize)> {
        tree.tokens()
            .filter(|t| ferrion_syntax::comments::is_comment(t.kind()))
            .map(|t| {
                (
                    t.kind(),
                    normalize_comment(t.text()),
                    t.span().start as usize,
                )
            })
            .collect()
    };
    let (a, b) = (list(before), list(after));
    let strip = |list: &[(SyntaxKind, String, usize)]| -> Vec<(SyntaxKind, String)> {
        list.iter().map(|(k, t, _)| (*k, t.clone())).collect()
    };
    if let Some(i) = first_difference(&strip(&a), &strip(&b)) {
        let describe = |list: &[(SyntaxKind, String, usize)]| {
            list.get(i)
                .map_or("nothing".to_owned(), |(_, text, _)| format!("{text:?}"))
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
    Ok(())
}

/// A comment's text as the comparison sees it: each line without trailing whitespace, later
/// lines also without leading whitespace, joined with LF.
fn normalize_comment(text: &str) -> String {
    let mut lines = Lines::new(text);
    let mut normalized = trim_end(lines.next().unwrap_or_default()).to_owned();
    for line in lines {
        normalized.push('\n');
        normalized.push_str(trim_end(line).trim_start_matches([' ', '\t', '\x0c']));
    }
    normalized
}

fn first_difference<T: PartialEq>(a: &[T], b: &[T]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or((a.len() != b.len()).then(|| a.len().min(b.len())))
}

#[cfg(test)]
mod tests;
