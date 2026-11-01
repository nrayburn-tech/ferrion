//! Questions the rules of every area ask about the comments around a token, from the input's
//! trivia. Each looks at the trivia next to one token, so it costs no more than that trivia.

use ferrion_syntax::SyntaxKind as K;
use ferrion_syntax::SyntaxToken;
use ferrion_syntax::comments::is_comment;

/// Whether there is a comment between `token` and the token before it that isn't trivia.
pub(crate) fn has_comments_before(token: SyntaxToken<'_>) -> bool {
    let mut previous = token.prev_token();
    while let Some(t) = previous
        && t.is_trivia()
    {
        if is_comment(t.kind()) {
            return true;
        }
        previous = t.prev_token();
    }
    false
}

/// Whether the first thing after `token` other than whitespace is a block comment, which is then
/// on `token`'s line.
pub(crate) fn block_comment_after(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    while let Some(t) = next
        && t.kind() == K::Whitespace
    {
        next = t.next_token();
    }
    next.is_some_and(|t| matches!(t.kind(), K::BlockComment | K::DocComment))
}

/// Whether a block comment that leads `token` sits on the line of the code before it, as one
/// after a separator does (`a, /* c */` then `b`). A rule that keeps it there places it before
/// its line break.
pub(crate) fn leads_on_code_line(token: SyntaxToken<'_>) -> bool {
    token.prev_non_trivia().is_some_and(block_comment_after)
}

/// Whether a line comment follows `token` on its line, with only spaces between.
pub(crate) fn line_comment_after(token: SyntaxToken<'_>) -> bool {
    let mut next = token.next_token();
    while let Some(t) = next {
        match t.kind() {
            K::Whitespace => next = t.next_token(),
            K::LineComment | K::DocLineComment => return true,
            _ => return false,
        }
    }
    false
}
