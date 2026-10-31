//! Documentation comments, found by javac's rule (docs/design/parser-comments.md#doc-comments).

use ferrion_diagnostics::Span;

use super::is_doc_comment;
use crate::SyntaxKind as K;
use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// A declaration's documentation comment: a `/** */` comment, or from release 23 a run of `///`
/// lines.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DocComment<'t> {
    first: SyntaxToken<'t>,
    last: SyntaxToken<'t>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DocCommentKind {
    /// `/** ... */`, for `ferrion_doc_comments::parse_javadoc` with the comment's
    /// [`span`](DocComment::span).
    Javadoc,
    /// A run of `///` lines, for `ferrion_doc_comments::parse_markdown` with the spans of its
    /// [`tokens`](DocComment::tokens).
    Markdown,
}

impl<'t> DocComment<'t> {
    pub fn kind(self) -> DocCommentKind {
        match self.first.kind() {
            K::DocComment => DocCommentKind::Javadoc,
            _ => DocCommentKind::Markdown,
        }
    }

    /// From the start of its first token to the end of its last.
    pub fn span(self) -> Span {
        Span::new(self.first.span().start, self.last.span().end)
    }

    /// The comment's tokens: one `DocComment`, or the run's `DocLineComment`s in order, without
    /// the whitespace and line terminators between them.
    pub fn tokens(self) -> impl Iterator<Item = SyntaxToken<'t>> + 't {
        let (kind, end) = (self.first.kind(), self.last.id());
        std::iter::successors(Some(self.first), move |token| {
            token.next_token().filter(|t| t.id() <= end)
        })
        .filter(move |t| t.kind() == kind)
    }

    pub fn first_token(self) -> SyntaxToken<'t> {
        self.first
    }

    pub fn last_token(self) -> SyntaxToken<'t> {
        self.last
    }
}

/// Whether javac attaches documentation comments to nodes of `kind`: packages, modules, classes,
/// interfaces, enums, records, annotation types (local ones included), fields, methods (and
/// annotation type elements), constructors, compact constructors, and enum constants. Observed
/// with javac 27: not to record components, initializers, parameters, local variables, or
/// statements.
pub fn can_have_doc_comment(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        K::PackageDecl
            | K::ModuleDecl
            | K::ClassDecl
            | K::InterfaceDecl
            | K::EnumDecl
            | K::RecordDecl
            | K::AnnotationTypeDecl
            | K::FieldDecl
            | K::MethodDecl
            | K::ConstructorDecl
            | K::CompactConstructorDecl
            | K::EnumConstant
    )
}

/// `node`'s documentation comment, as javac finds it: the last documentation comment before the
/// node's first token (its modifiers and annotations included), with only whitespace and other
/// comments between them. `None` if there isn't one, or if nodes of its kind don't have one
/// ([`can_have_doc_comment`]).
///
/// A run of `///` lines is a documentation comment only at release 23 and later; below that, its
/// lines are ordinary line comments, skipped like any other. The lines form one run while each is
/// on the line after the previous one, with only whitespace between, as
/// `ferrion_doc_comments::markdown_comment_runs` groups them.
///
/// Costs a scan of the trivia before the node.
pub fn doc_comment(node: SyntaxNode<'_>) -> Option<DocComment<'_>> {
    if !can_have_doc_comment(node.kind()) {
        return None;
    }
    let release = node.tree().release();
    let mut token = node.first_token().prev_token();
    while let Some(t) = token {
        match t.kind() {
            K::Whitespace | K::LineTerminator | K::LineComment | K::BlockComment => {}
            K::DocLineComment if !is_doc_comment(K::DocLineComment, release) => {}
            K::DocComment => return Some(DocComment { first: t, last: t }),
            K::DocLineComment => {
                return Some(DocComment {
                    first: markdown_run_start(t),
                    last: t,
                });
            }
            _ => return None,
        }
        token = t.prev_token();
    }
    None
}

/// The first line of the run of `///` lines that ends with `last`.
fn markdown_run_start(last: SyntaxToken<'_>) -> SyntaxToken<'_> {
    fn skip_whitespace(token: Option<SyntaxToken<'_>>) -> Option<SyntaxToken<'_>> {
        let mut token = token?;
        while token.kind() == K::Whitespace {
            token = token.prev_token()?;
        }
        Some(token)
    }
    let mut first = last;
    loop {
        let Some(line_break) = skip_whitespace(first.prev_token()) else {
            return first;
        };
        if line_break.kind() != K::LineTerminator {
            return first;
        }
        match skip_whitespace(line_break.prev_token()) {
            Some(previous) if previous.kind() == K::DocLineComment => first = previous,
            _ => return first,
        }
    }
}
