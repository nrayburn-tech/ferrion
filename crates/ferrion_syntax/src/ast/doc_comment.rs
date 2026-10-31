//! `doc_comment()` on the views of declarations that javac documents, written by hand next to
//! the generated accessors (docs/design/parser-comments.md#doc-comments).

use super::*;
use crate::comments::{self, DocComment};

macro_rules! doc_comment {
    ($($view:ident),* $(,)?) => {$(
        impl<'t> $view<'t> {
            /// The declaration's documentation comment, by javac's rule: the last one before
            /// its first token, annotations and modifiers included, with only whitespace and
            /// other comments between. See [`comments::doc_comment`].
            pub fn doc_comment(self) -> Option<DocComment<'t>> {
                comments::doc_comment(self.syntax())
            }
        }
    )*};
}

doc_comment!(
    PackageDecl,
    ModuleDecl,
    ClassDecl,
    InterfaceDecl,
    EnumDecl,
    RecordDecl,
    AnnotationTypeDecl,
    FieldDecl,
    MethodDecl,
    ConstructorDecl,
    CompactConstructorDecl,
    EnumConstant,
    TypeDecl,
);

impl<'t> Member<'t> {
    /// The member's documentation comment, by javac's rule (see [`comments::doc_comment`]).
    /// Always `None` for an initializer, which javac doesn't document.
    pub fn doc_comment(self) -> Option<DocComment<'t>> {
        comments::doc_comment(self.syntax())
    }
}
