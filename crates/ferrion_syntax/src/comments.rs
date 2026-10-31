//! Which node or token each comment belongs to, and which comment documents a declaration
//! (docs/design/parser-comments.md).
//!
//! Comments are trivia tokens, and the tree records no attachment. [`CommentMap::new`] computes
//! one in a pass over the tree: each comment is leading or trailing a node or token, or dangling
//! in a node with nothing else in it. The formatter and the linter build their own rules on it.
//!
//! [`doc_comment`] finds a declaration's documentation comment by javac's rule, without a map;
//! the typed views offer it as `doc_comment()` (for example
//! [`ast::ClassDecl::doc_comment`](crate::ast::ClassDecl::doc_comment)). `ferrion_doc_comments`
//! parses the comment's text.

mod doc;

use crate::SyntaxKind as K;
use crate::tree::{NodeId, TokenId};
use crate::{NodeOrToken, SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree};

pub use doc::{DocComment, DocCommentKind, can_have_doc_comment, doc_comment};

/// The first release whose `///` comments are documentation comments (JEP 467). Below it they are
/// ordinary line comments.
pub const MARKDOWN_DOC_COMMENT_RELEASE: u32 = 23;

/// A node or a token of a tree, by id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Element {
    Node(NodeId),
    Token(TokenId),
}

impl Element {
    /// The node or token in `tree`, which must be the tree the id comes from.
    pub fn get(self, tree: &SyntaxTree) -> NodeOrToken<'_> {
        match self {
            Element::Node(id) => NodeOrToken::Node(tree.node(id)),
            Element::Token(id) => NodeOrToken::Token(tree.token(id)),
        }
    }
}

impl From<NodeId> for Element {
    fn from(id: NodeId) -> Element {
        Element::Node(id)
    }
}

impl From<TokenId> for Element {
    fn from(id: TokenId) -> Element {
        Element::Token(id)
    }
}

impl From<SyntaxNode<'_>> for Element {
    fn from(node: SyntaxNode<'_>) -> Element {
        Element::Node(node.id())
    }
}

impl From<SyntaxToken<'_>> for Element {
    fn from(token: SyntaxToken<'_>) -> Element {
        Element::Token(token.id())
    }
}

impl From<NodeOrToken<'_>> for Element {
    fn from(element: NodeOrToken<'_>) -> Element {
        match element {
            NodeOrToken::Node(node) => node.into(),
            NodeOrToken::Token(token) => token.into(),
        }
    }
}

/// Where a comment goes relative to its element.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Placement {
    /// Before the element: `// why` on the line above `foo();`.
    Leading,
    /// After the element: `foo(); // why`.
    Trailing,
    /// Inside the element, a node with nothing else between its delimiters: `{ /* nothing */ }`,
    /// `f(/* none */)`, or a file of comments.
    Dangling,
}

/// A comment token and where it belongs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Comment {
    /// A `LineComment`, `DocLineComment`, `BlockComment`, or `DocComment`.
    pub token: TokenId,
    pub placement: Placement,
    /// What the comment is leading or trailing: a child node or direct token of
    /// [`enclosing`](Comment::enclosing). For a dangling comment, `enclosing` itself.
    pub element: Element,
    /// The innermost node containing the comment, which has it as a direct token.
    pub enclosing: NodeId,
    /// Line terminators between the comment and the previous token that isn't whitespace or a
    /// line terminator (a comment, or code), or the start of the text. 0 when the comment
    /// continues a line, 2 or more after a blank line.
    pub line_breaks_before: u32,
    /// Line terminators between the comment and the next token that isn't whitespace or a line
    /// terminator (a comment, code, or `Eof`).
    pub line_breaks_after: u32,
}

/// The placement of every comment in a tree. Built in one pass over the tree with
/// [`CommentMap::new`]; nothing in the tree depends on it.
///
/// For each comment, in source order:
///
/// 1. Its **enclosing node** is the innermost node containing it. Its **elements** are that
///    node's child nodes and direct tokens other than trivia.
/// 2. **Preceding** is the last element before the comment, and **following** the first after.
///    An opening `(`, `[`, or `{` doesn't count as preceding below, and a closing `)`, `]`, `}`,
///    or the `Eof` doesn't count as following, except where rule 4 says.
/// 3. With neither, the comment is **dangling** in the enclosing node.
/// 4. It is **trailing** the preceding element, delimiters included, if it starts on the line
///    where that element ends, isn't a documentation comment, and is a line comment, is followed
///    by a line break before the next code, or has no following element.
/// 5. Otherwise it is **leading** the following element.
/// 6. Otherwise, at the end of a list or block, it is trailing the preceding element.
///
/// A line break is a line terminator, or a block comment that spans lines. A documentation
/// comment (`/** */`, or `///` from release 23) never trails, so a declaration's doc comment is
/// leading the declaration (or the outermost node that starts where it does).
#[derive(Clone, Debug, Default)]
pub struct CommentMap {
    /// Every comment, in source order.
    comments: Vec<Comment>,
    /// The same comments, sorted by element, then placement, then source order.
    by_element: Vec<Comment>,
}

impl CommentMap {
    /// Places every comment in `tree`. Linear in the tree's size, apart from sorting the
    /// comments by element.
    pub fn new(tree: &SyntaxTree) -> CommentMap {
        let comments = Builder::new(tree).run();
        let mut by_element = comments.clone();
        by_element.sort_by_key(|c| (c.element, c.placement, c.token));
        CommentMap {
            comments,
            by_element,
        }
    }

    /// Every comment, in source order.
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The comment `token` is, or `None` if it isn't a comment.
    pub fn get(&self, token: TokenId) -> Option<&Comment> {
        let index = self
            .comments
            .binary_search_by_key(&token, |c| c.token)
            .ok()?;
        Some(&self.comments[index])
    }

    /// The comments before `element`, in source order.
    pub fn leading(&self, element: impl Into<Element>) -> &[Comment] {
        self.find(element.into(), Some(Placement::Leading))
    }

    /// The comments after `element`, in source order.
    pub fn trailing(&self, element: impl Into<Element>) -> &[Comment] {
        self.find(element.into(), Some(Placement::Trailing))
    }

    /// The comments inside `node` with nothing else between its delimiters, in source order.
    pub fn dangling(&self, node: impl Into<Element>) -> &[Comment] {
        self.find(node.into(), Some(Placement::Dangling))
    }

    /// Every comment placed on `element`: its leading, then trailing, then dangling comments.
    pub fn of(&self, element: impl Into<Element>) -> &[Comment] {
        self.find(element.into(), None)
    }

    fn find(&self, element: Element, placement: Option<Placement>) -> &[Comment] {
        let first = (element, placement.unwrap_or(Placement::Leading));
        let start = self
            .by_element
            .partition_point(|c| (c.element, c.placement) < first);
        let rest = &self.by_element[start..];
        let len = rest.partition_point(|c| {
            c.element == element && placement.is_none_or(|p| c.placement == p)
        });
        &rest[..len]
    }
}

/// Whether `kind` is a comment token's.
pub fn is_comment(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LineComment
            | SyntaxKind::DocLineComment
            | SyntaxKind::BlockComment
            | SyntaxKind::DocComment
    )
}

/// Whether a comment of `kind` is a documentation comment at `release`.
pub fn is_doc_comment(kind: SyntaxKind, release: u32) -> bool {
    kind == SyntaxKind::DocComment
        || kind == SyntaxKind::DocLineComment && release >= MARKDOWN_DOC_COMMENT_RELEASE
}

/// A node open at the current token, with its last element so far.
struct Open {
    id: NodeId,
    token_end: TokenId,
    last: Option<Element>,
}

/// A comment waiting for the next code token, which decides its following element.
struct Pending {
    /// Its index in the comments.
    index: usize,
    /// Whether a line break comes between the preceding element and the comment.
    break_before: bool,
}

struct Builder<'t> {
    tree: &'t SyntaxTree,
    comments: Vec<Comment>,
    open: Vec<Open>,
    next_node: usize,
    /// The comments since the last code token.
    pending: Vec<Pending>,
    /// The last line break since the last code token: a line terminator, or a comment spanning
    /// lines.
    last_break: Option<TokenId>,
    /// Line terminators since the last token that wasn't whitespace or a line terminator.
    breaks: u32,
}

impl<'t> Builder<'t> {
    fn new(tree: &'t SyntaxTree) -> Builder<'t> {
        Builder {
            tree,
            comments: Vec::new(),
            open: Vec::new(),
            next_node: 0,
            pending: Vec::new(),
            last_break: None,
            breaks: 0,
        }
    }

    fn run(mut self) -> Vec<Comment> {
        let nodes = self.tree.raw_nodes();
        for index in 0..self.tree.token_count() {
            let id = TokenId::from_index(index);
            while let Some(open) = self.open.pop_if(|open| open.token_end <= id) {
                if let Some(parent) = self.open.last_mut() {
                    parent.last = Some(Element::Node(open.id));
                }
            }
            // Only the root starts at trivia, at the first token.
            let enclosing = self.open.len();
            let opened = self.next_node;
            while let Some(node) = nodes.get(self.next_node)
                && node.first_token == id
            {
                self.open.push(Open {
                    id: NodeId::from_index(self.next_node),
                    token_end: node.token_end,
                    last: None,
                });
                self.next_node += 1;
            }
            let kind = self.tree.token_kind(id);
            match kind {
                SyntaxKind::Whitespace => continue,
                SyntaxKind::LineTerminator => {
                    self.breaks += 1;
                    self.last_break = Some(id);
                    continue;
                }
                _ => {}
            }
            if let Some(previous) = self.comments.last_mut()
                && previous.line_breaks_after == u32::MAX
            {
                previous.line_breaks_after = self.breaks;
            }
            let line_breaks_before = std::mem::take(&mut self.breaks);
            if is_comment(kind) {
                let enclosing = self.open.last().expect("the root is open").id;
                self.pending.push(Pending {
                    index: self.comments.len(),
                    break_before: self.last_break.is_some(),
                });
                self.comments.push(Comment {
                    token: id,
                    // Placed when the next code token is reached.
                    placement: Placement::Dangling,
                    element: Element::Node(enclosing),
                    enclosing,
                    line_breaks_before,
                    line_breaks_after: u32::MAX,
                });
                if spans_lines(self.tree.token(id)) {
                    self.last_break = Some(id);
                }
                continue;
            }
            if !self.pending.is_empty() {
                // The pending comments are in the node that was innermost before any node opened
                // here, and the outermost node that opened, if any, follows them.
                let preceding = self.open[enclosing - 1].last;
                let following = if opened < self.next_node {
                    Element::Node(NodeId::from_index(opened))
                } else {
                    Element::Token(id)
                };
                self.place_pending(preceding, following);
            }
            self.open.last_mut().expect("the root is open").last = Some(Element::Token(id));
            self.last_break = None;
        }
        self.comments
    }

    /// Places the comments since the last code token, between `preceding` (if any) and
    /// `following`.
    fn place_pending(&mut self, preceding: Option<Element>, following: Element) {
        let tree = self.tree;
        let is_token = |element: Option<Element>, kinds: &[SyntaxKind]| match element {
            Some(Element::Token(id)) => kinds.contains(&tree.token_kind(id)),
            _ => false,
        };
        let opens = is_token(preceding, &[K::LParen, K::LBracket, K::LBrace]);
        let closes = is_token(
            Some(following),
            &[K::RParen, K::RBracket, K::RBrace, K::Eof],
        );
        let real_preceding = preceding.filter(|_| !opens);
        let real_following = (!closes).then_some(following);
        for pending in self.pending.drain(..) {
            let comment = &mut self.comments[pending.index];
            let kind = tree.token_kind(comment.token);
            let break_after = self.last_break.is_some_and(|b| b > comment.token);
            let trails = !pending.break_before
                && !is_doc_comment(kind, tree.release())
                && (matches!(kind, K::LineComment | K::DocLineComment)
                    || break_after
                    || real_following.is_none());
            (comment.placement, comment.element) = match (preceding, real_following) {
                _ if real_preceding.is_none() && real_following.is_none() => {
                    (Placement::Dangling, Element::Node(comment.enclosing))
                }
                (Some(preceding), _) if trails => (Placement::Trailing, preceding),
                (_, Some(following)) => (Placement::Leading, following),
                (_, None) => (
                    Placement::Trailing,
                    real_preceding.expect("a preceding element without a following one"),
                ),
            };
        }
    }
}

/// Whether a comment token spans lines: a block comment with a line terminator in it.
fn spans_lines(token: SyntaxToken<'_>) -> bool {
    matches!(
        token.kind(),
        SyntaxKind::BlockComment | SyntaxKind::DocComment
    ) && token.text().contains(['\n', '\r'])
}

#[cfg(test)]
mod tests;
