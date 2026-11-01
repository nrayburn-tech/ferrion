//! `@formatter:off` and `@formatter:on` regions (docs/design/formatter.md, "Formatter disable
//! regions"): text left exactly as written. A region runs from the start of an `off` comment to
//! the end of the next `on` comment, or to the end of the file. Markers are found by their text,
//! in comments of any kind; an `on` outside a region, or an `off` inside one, does nothing.

use ferrion_syntax::comments::CommentMap;
use ferrion_syntax::{SyntaxToken, SyntaxTree, TokenId};

pub const OFF: &str = "@formatter:off";
pub const ON: &str = "@formatter:on";

/// One disable region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Region {
    /// The `off` comment.
    pub start: TokenId,
    /// The `on` comment, or `None` if the region runs to the end of the file.
    pub end: Option<TokenId>,
}

/// Every region of a file, in source order, found in one scan of its comments.
pub fn find(tree: &SyntaxTree, comments: &CommentMap) -> Vec<Region> {
    let mut regions = Vec::new();
    let mut open = None;
    for comment in comments.comments() {
        let text = tree.token(comment.token).text();
        match open {
            None if text.contains(OFF) => open = Some(comment.token),
            Some(start) if text.contains(ON) => {
                regions.push(Region {
                    start,
                    end: Some(comment.token),
                });
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        regions.push(Region { start, end: None });
    }
    regions
}

/// Whether `token`, a comment, is a formatter marker.
pub fn is_marker(token: SyntaxToken<'_>) -> bool {
    let text = token.text();
    text.contains(OFF) || text.contains(ON)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrion_parser::{ParseOptions, parse};

    use super::*;

    fn regions(text: &str) -> Vec<(String, Option<String>)> {
        let tree = parse(Arc::from(text), ParseOptions { release: 27 }).tree;
        let comments = CommentMap::new(&tree);
        find(&tree, &comments)
            .into_iter()
            .map(|r| {
                (
                    tree.token(r.start).text().to_owned(),
                    r.end.map(|e| tree.token(e).text().to_owned()),
                )
            })
            .collect()
    }

    #[test]
    fn pairs_each_off_with_the_next_on() {
        let found = regions(
            "// @formatter:on\nclass A {\n// @formatter:off\nint a;\n/* @formatter:off */\n\
             /* @formatter:on */\nint b;\n// @formatter:off x\n}\n",
        );
        assert_eq!(
            found,
            [
                (
                    "// @formatter:off".to_owned(),
                    Some("/* @formatter:on */".to_owned())
                ),
                ("// @formatter:off x".to_owned(), None),
            ]
        );
    }

    #[test]
    fn no_markers_no_regions() {
        assert!(regions("class A { /* formatter:off */ }").is_empty());
    }
}
