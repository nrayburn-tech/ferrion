//! Writes `SyntaxKind`: every token kind, contextual keyword kind, and node kind, with conversions
//! from the lexer's kinds and the grammar's token sets.

use std::fmt::Write as _;

use crate::grammar::{Grammar, write_doc};
use crate::{HEADER, snake_case};

pub fn generate(grammar: &Grammar) -> Result<String, String> {
    let mut out = String::from(HEADER);
    let lexer_tokens: Vec<_> = grammar.tokens.iter().filter(|t| !t.contextual).collect();
    let contextual: Vec<_> = grammar.tokens.iter().filter(|t| t.contextual).collect();
    // Token kinds come first, so that a token set can be a bit set over them.
    if grammar.tokens.iter().position(|t| t.contextual) != Some(lexer_tokens.len()) {
        return Err("contextual keywords must come after every other token".to_owned());
    }
    if grammar.tokens.len() > 256 {
        return Err("more than 256 token kinds".to_owned());
    }

    out.push_str(
        "
use ferrion_lexer::{ContextualKeyword, TokenKind};

/// The kind of a token or a node: the lexer's token kinds, then contextual keyword kinds, then
/// node kinds (see `java.grammar`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[repr(u16)]
pub enum SyntaxKind {
",
    );
    for token in &grammar.tokens {
        let mut doc = token.doc.clone();
        if doc.is_empty()
            && let Some(spelling) = &token.spelling
        {
            doc.push(format!("`{spelling}`"));
        }
        write_doc(&mut out, "    ", &doc);
        writeln!(out, "    {},", token.name).unwrap();
    }
    for node in &grammar.nodes {
        write_doc(&mut out, "    ", &node.doc);
        writeln!(out, "    {},", node.name).unwrap();
    }
    out.push_str("}\n\n");

    let first_contextual = &contextual[0].name;
    let first_node = &grammar.nodes[0].name;
    writeln!(
        out,
        "impl SyntaxKind {{
    /// Every kind, in order.
    pub const ALL: &[SyntaxKind] = &["
    )
    .unwrap();
    for name in grammar
        .tokens
        .iter()
        .map(|t| &t.name)
        .chain(grammar.nodes.iter().map(|n| &n.name))
    {
        writeln!(out, "SyntaxKind::{name},").unwrap();
    }
    writeln!(
        out,
        "];

    /// The first contextual keyword kind; the token kinds the lexer produces come before it.
    pub(crate) const FIRST_CONTEXTUAL: SyntaxKind = SyntaxKind::{first_contextual};

    /// The first node kind; every token kind comes before it.
    pub(crate) const FIRST_NODE: SyntaxKind = SyntaxKind::{first_node};

    /// The kind of a token the lexer produced.
    pub fn from_token_kind(kind: TokenKind) -> SyntaxKind {{
        match kind {{"
    )
    .unwrap();
    for token in &lexer_tokens {
        writeln!(
            out,
            "TokenKind::{} => SyntaxKind::{},",
            token.source, token.name
        )
        .unwrap();
    }
    out.push_str(
        "}
    }

    /// The kind the parser records for a contextual keyword, or `None` for one it never records
    /// (`var`).
    pub fn from_contextual_keyword(keyword: ContextualKeyword) -> Option<SyntaxKind> {
        match keyword {
",
    );
    for token in &contextual {
        writeln!(
            out,
            "ContextualKeyword::{} => Some(SyntaxKind::{}),",
            token.source, token.name
        )
        .unwrap();
    }
    out.push_str(
        "_ => None,
        }
    }

    /// The fixed text of keywords, contextual keywords, separators, and operators, or `None` for
    /// kinds whose text varies and for nodes.
    pub fn fixed_text(self) -> Option<&'static str> {
        Some(match self {
",
    );
    for token in &grammar.tokens {
        if let Some(spelling) = &token.spelling {
            writeln!(out, "SyntaxKind::{} => {spelling:?},", token.name).unwrap();
        }
    }
    out.push_str(
        "_ => return None,
        })
    }

    /// Whitespace, line terminators, and comments.
    pub fn is_trivia(self) -> bool {
        matches!(self,",
    );
    let trivia: Vec<_> = grammar
        .tokens
        .iter()
        .filter(|t| t.trivia)
        .map(|t| format!("SyntaxKind::{}", t.name))
        .collect();
    writeln!(out, "{})\n    }}\n}}\n", trivia.join(" | ")).unwrap();

    out.push_str(
        "/// The grammar's token sets.
pub mod token_sets {
    use super::SyntaxKind;
",
    );
    for set in &grammar.token_sets {
        write_doc(&mut out, "    ", &set.doc);
        let kinds: Vec<_> = set
            .tokens
            .iter()
            .map(|t| format!("SyntaxKind::{t}"))
            .collect();
        writeln!(
            out,
            "pub const {}: &[SyntaxKind] = &[{}];",
            snake_case(&set.name).to_uppercase(),
            kinds.join(", ")
        )
        .unwrap();
    }
    out.push_str("}\n");
    Ok(out)
}
