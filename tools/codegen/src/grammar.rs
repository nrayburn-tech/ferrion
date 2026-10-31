//! Reads `java.grammar`: its tokens, token sets, nodes, and enums. The format is described at the
//! top of the grammar file.

use std::collections::HashMap;
use std::fmt::Write as _;

/// A whole grammar file, with every name resolved.
pub struct Grammar {
    pub tokens: Vec<TokenDef>,
    pub token_sets: Vec<TokenSetDef>,
    pub nodes: Vec<NodeDef>,
    pub enums: Vec<EnumDef>,
}

pub struct TokenDef {
    pub name: String,
    pub spelling: Option<String>,
    /// The lexer's `TokenKind` variant, or for a contextual keyword its `ContextualKeyword`
    /// variant.
    pub source: String,
    pub contextual: bool,
    pub trivia: bool,
    pub doc: Vec<String>,
}

pub struct TokenSetDef {
    pub name: String,
    pub tokens: Vec<String>,
    pub doc: Vec<String>,
}

pub struct NodeDef {
    pub name: String,
    pub rule: Option<Rule>,
    /// The rule as written, on one line.
    pub rule_text: Option<String>,
    pub doc: Vec<String>,
}

pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
    pub doc: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Rule {
    Seq(Vec<Rule>),
    Alt(Vec<Rule>),
    Opt(Box<Rule>),
    Star(Box<Rule>),
    Plus(Box<Rule>),
    Labeled(String, Box<Rule>),
    /// A token kind, by name (spellings are resolved to names).
    Token(String),
    /// A token set, node, or enum, by name.
    Ref(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Lexeme {
    Word(String),
    Quoted(String),
    Punct(char),
    DocLine(String),
}

struct Lexed {
    lexeme: Lexeme,
    offset: usize,
}

fn lex(text: &str) -> Result<Vec<Lexed>, String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if text[i..].starts_with("///") {
            let end = text[i..].find('\n').map_or(text.len(), |n| i + n);
            let line = text[i + 3..end]
                .strip_prefix(' ')
                .unwrap_or(&text[i + 3..end]);
            out.push(Lexed {
                lexeme: Lexeme::DocLine(line.to_owned()),
                offset: i,
            });
            i = end;
        } else if text[i..].starts_with("//") {
            i = text[i..].find('\n').map_or(text.len(), |n| i + n);
        } else if c == b'\'' {
            let end = text[i + 1..]
                .find(['\'', '\n'])
                .map(|n| i + 1 + n)
                .filter(|&end| bytes[end] == b'\'')
                .ok_or_else(|| format!("unterminated spelling at {}", line_of(text, i)))?;
            out.push(Lexed {
                lexeme: Lexeme::Quoted(text[i + 1..end].to_owned()),
                offset: i,
            });
            i = end + 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let end = text[i..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .map_or(text.len(), |n| i + n);
            out.push(Lexed {
                lexeme: Lexeme::Word(text[i..end].to_owned()),
                offset: i,
            });
            i = end;
        } else if "=|?*+():".contains(c as char) {
            out.push(Lexed {
                lexeme: Lexeme::Punct(c as char),
                offset: i,
            });
            i += 1;
        } else {
            return Err(format!(
                "unexpected {:?} at {}",
                c as char,
                line_of(text, i)
            ));
        }
    }
    Ok(out)
}

fn line_of(text: &str, offset: usize) -> String {
    format!("line {}", text[..offset].matches('\n').count() + 1)
}

const ITEM_KEYWORDS: [&str; 5] = ["token", "contextual", "tokens", "node", "enum"];

struct Parser<'a> {
    text: &'a str,
    lexemes: Vec<Lexed>,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Lexeme> {
        self.lexemes.get(self.pos).map(|l| &l.lexeme)
    }

    fn at_item_start(&self) -> bool {
        match self.peek() {
            None | Some(Lexeme::DocLine(_)) => true,
            Some(Lexeme::Word(word)) => ITEM_KEYWORDS.contains(&word.as_str()),
            _ => false,
        }
    }

    fn here(&self) -> String {
        let offset = self
            .lexemes
            .get(self.pos)
            .map_or(self.text.len(), |l| l.offset);
        line_of(self.text, offset)
    }

    fn word(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(Lexeme::Word(word)) => {
                let word = word.clone();
                self.pos += 1;
                Ok(word)
            }
            other => Err(format!(
                "expected a name at {}, found {other:?}",
                self.here()
            )),
        }
    }

    fn eat_word(&mut self, expected: &str) -> bool {
        if matches!(self.peek(), Some(Lexeme::Word(word)) if word == expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_punct(&mut self, expected: char) -> bool {
        if self.peek() == Some(&Lexeme::Punct(expected)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, expected: char) -> Result<(), String> {
        if self.eat_punct(expected) {
            Ok(())
        } else {
            Err(format!("expected {expected:?} at {}", self.here()))
        }
    }

    fn quoted(&mut self) -> Option<String> {
        match self.peek() {
            Some(Lexeme::Quoted(text)) => {
                let text = text.clone();
                self.pos += 1;
                Some(text)
            }
            _ => None,
        }
    }

    fn alt(&mut self) -> Result<Rule, String> {
        let mut alternatives = vec![self.seq()?];
        while self.eat_punct('|') {
            alternatives.push(self.seq()?);
        }
        Ok(if alternatives.len() == 1 {
            alternatives.pop().unwrap()
        } else {
            Rule::Alt(alternatives)
        })
    }

    fn seq(&mut self) -> Result<Rule, String> {
        let mut items = Vec::new();
        while !self.at_item_start() && !matches!(self.peek(), Some(Lexeme::Punct('|' | ')'))) {
            items.push(self.postfix()?);
        }
        match items.len() {
            0 => Err(format!("empty alternative at {}", self.here())),
            1 => Ok(items.pop().unwrap()),
            _ => Ok(Rule::Seq(items)),
        }
    }

    fn postfix(&mut self) -> Result<Rule, String> {
        let mut rule = self.atom()?;
        loop {
            rule = if self.eat_punct('?') {
                Rule::Opt(Box::new(rule))
            } else if self.eat_punct('*') {
                Rule::Star(Box::new(rule))
            } else if self.eat_punct('+') {
                Rule::Plus(Box::new(rule))
            } else {
                return Ok(rule);
            };
        }
    }

    fn atom(&mut self) -> Result<Rule, String> {
        if self.eat_punct('(') {
            let rule = self.alt()?;
            self.expect_punct(')')?;
            return Ok(rule);
        }
        if let Some(spelling) = self.quoted() {
            return Ok(Rule::Token(spelling));
        }
        let word = self.word()?;
        if self.eat_punct(':') {
            if !word.starts_with(|c: char| c.is_ascii_lowercase()) {
                return Err(format!("label {word} isn't lower case, at {}", self.here()));
            }
            let target = self.atom()?;
            if !matches!(target, Rule::Token(_) | Rule::Ref(_)) {
                return Err(format!("label {word} isn't on a single name or token"));
            }
            return Ok(Rule::Labeled(word, Box::new(target)));
        }
        if !word.starts_with(|c: char| c.is_ascii_uppercase()) {
            return Err(format!("{word} isn't a name, at {}", self.here()));
        }
        Ok(Rule::Ref(word))
    }

    /// The source text from lexeme `start` up to the current one, on one line.
    fn text_since(&self, start: usize) -> String {
        let from = self.lexemes[start].offset;
        let to = self
            .lexemes
            .get(self.pos)
            .map_or(self.text.len(), |l| l.offset);
        let mut out = String::new();
        for line in self.text[from..to].lines() {
            let line = line.split("//").next().unwrap().trim();
            if !line.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(line);
            }
        }
        out
    }
}

pub fn parse(text: &str) -> Result<Grammar, String> {
    let mut p = Parser {
        text,
        lexemes: lex(text)?,
        pos: 0,
    };
    let mut grammar = Grammar {
        tokens: Vec::new(),
        token_sets: Vec::new(),
        nodes: Vec::new(),
        enums: Vec::new(),
    };
    let mut doc = Vec::new();
    while let Some(lexeme) = p.peek().cloned() {
        if let Lexeme::DocLine(line) = lexeme {
            doc.push(line);
            p.pos += 1;
            continue;
        }
        let keyword = p.word()?;
        let doc = std::mem::take(&mut doc);
        match keyword.as_str() {
            "token" | "contextual" => {
                let contextual = keyword == "contextual";
                let name = p.word()?;
                let spelling = p.quoted();
                let source = if p.eat_word("from") {
                    p.word()?
                } else {
                    name.strip_suffix("Kw").unwrap_or(&name).to_owned()
                };
                let trivia = p.eat_word("trivia");
                if contextual && (spelling.is_none() || trivia) {
                    return Err(format!("contextual keyword {name} needs a spelling only"));
                }
                grammar.tokens.push(TokenDef {
                    name,
                    spelling,
                    source,
                    contextual,
                    trivia,
                    doc,
                });
            }
            "tokens" => {
                let name = p.word()?;
                p.expect_punct('=')?;
                let mut tokens = Vec::new();
                loop {
                    tokens.push(match p.quoted() {
                        Some(spelling) => spelling,
                        None => p.word()?,
                    });
                    if !p.eat_punct('|') {
                        break;
                    }
                }
                grammar.token_sets.push(TokenSetDef { name, tokens, doc });
            }
            "node" => {
                let name = p.word()?;
                let (rule, rule_text) = if p.eat_punct('=') {
                    let start = p.pos;
                    let rule = p.alt()?;
                    (Some(rule), Some(p.text_since(start)))
                } else {
                    (None, None)
                };
                grammar.nodes.push(NodeDef {
                    name,
                    rule,
                    rule_text,
                    doc,
                });
            }
            "enum" => {
                let name = p.word()?;
                p.expect_punct('=')?;
                let mut variants = vec![p.word()?];
                while p.eat_punct('|') {
                    variants.push(p.word()?);
                }
                grammar.enums.push(EnumDef {
                    name,
                    variants,
                    doc,
                });
            }
            _ => return Err(format!("expected an item at {}, found {keyword}", p.here())),
        }
        if !p.at_item_start() {
            return Err(format!("unexpected text at {}", p.here()));
        }
    }
    grammar.resolve()?;
    Ok(grammar)
}

/// What a name in a rule refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target<'g> {
    Token(&'g str),
    TokenSet(&'g str),
    Node(&'g str),
    Enum(&'g str),
}

impl Grammar {
    /// Replaces spellings with token names, everywhere, and checks that every name is declared
    /// once.
    fn resolve(&mut self) -> Result<(), String> {
        let mut names = HashMap::new();
        let all_names = (self.tokens.iter().map(|t| &t.name))
            .chain(self.token_sets.iter().map(|t| &t.name))
            .chain(self.nodes.iter().map(|n| &n.name))
            .chain(self.enums.iter().map(|e| &e.name));
        for name in all_names {
            if names.insert(name.clone(), ()).is_some() {
                return Err(format!("{name} is declared twice"));
            }
        }
        let mut by_spelling = HashMap::new();
        for token in &self.tokens {
            if let Some(spelling) = &token.spelling
                && by_spelling
                    .insert(spelling.clone(), token.name.clone())
                    .is_some()
            {
                return Err(format!("the spelling {spelling:?} is declared twice"));
            }
        }
        let token_name = |text: &str| -> Result<String, String> {
            if let Some(name) = by_spelling.get(text) {
                Ok(name.clone())
            } else if self.tokens.iter().any(|t| t.name == text) {
                Ok(text.to_owned())
            } else {
                Err(format!("no token is spelled or named {text:?}"))
            }
        };
        for set in &mut self.token_sets {
            for token in &mut set.tokens {
                *token = token_name(token)?;
            }
        }
        let mut nodes = std::mem::take(&mut self.nodes);
        for node in &mut nodes {
            if let Some(rule) = &mut node.rule {
                resolve_rule(rule, &token_name, &names)
                    .map_err(|e| format!("in {}: {e}", node.name))?;
            }
        }
        self.nodes = nodes;
        for e in &self.enums {
            for variant in &e.variants {
                match self.target(variant) {
                    Some(Target::Node(_) | Target::Enum(_)) => {}
                    _ => return Err(format!("{}: {variant} isn't a node or enum", e.name)),
                }
            }
        }
        Ok(())
    }

    pub fn target(&self, name: &str) -> Option<Target<'_>> {
        if let Some(t) = self.tokens.iter().find(|t| t.name == name) {
            return Some(Target::Token(&t.name));
        }
        if let Some(t) = self.token_sets.iter().find(|t| t.name == name) {
            return Some(Target::TokenSet(&t.name));
        }
        if let Some(n) = self.nodes.iter().find(|n| n.name == name) {
            return Some(Target::Node(&n.name));
        }
        self.enums
            .iter()
            .find(|e| e.name == name)
            .map(|e| Target::Enum(&e.name))
    }

    /// The token or node kinds a target accepts, enums flattened.
    pub fn kinds(&self, target: Target<'_>) -> Vec<String> {
        match target {
            Target::Token(name) | Target::Node(name) => vec![name.to_owned()],
            Target::TokenSet(name) => {
                let set = self.token_sets.iter().find(|s| s.name == name).unwrap();
                set.tokens.clone()
            }
            Target::Enum(name) => {
                let e = self.enums.iter().find(|e| e.name == name).unwrap();
                let mut kinds = Vec::new();
                for variant in &e.variants {
                    kinds.extend(self.kinds(self.target(variant).unwrap()));
                }
                kinds
            }
        }
    }

    /// Checks that each enum's variants accept disjoint kinds, so that a cast is unambiguous.
    pub fn check_enums(&self) -> Result<(), String> {
        for e in &self.enums {
            let mut seen: HashMap<String, &str> = HashMap::new();
            for variant in &e.variants {
                for kind in self.kinds(self.target(variant).unwrap()) {
                    if let Some(other) = seen.insert(kind.clone(), variant) {
                        return Err(format!(
                            "enum {}: {kind} is in both {other} and {variant}",
                            e.name
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn resolve_rule(
    rule: &mut Rule,
    token_name: &impl Fn(&str) -> Result<String, String>,
    names: &HashMap<String, ()>,
) -> Result<(), String> {
    match rule {
        Rule::Seq(items) | Rule::Alt(items) => {
            for item in items {
                resolve_rule(item, token_name, names)?;
            }
        }
        Rule::Opt(inner) | Rule::Star(inner) | Rule::Plus(inner) | Rule::Labeled(_, inner) => {
            resolve_rule(inner, token_name, names)?;
        }
        Rule::Token(text) => *text = token_name(text)?,
        Rule::Ref(name) => {
            if !names.contains_key(name.as_str()) {
                return Err(format!("{name} isn't declared"));
            }
        }
    }
    Ok(())
}

/// Writes `lines` as `///` doc comment lines, indented by `indent`.
pub fn write_doc(out: &mut String, indent: &str, lines: &[String]) {
    for line in lines {
        if line.is_empty() {
            writeln!(out, "{indent}///").unwrap();
        } else {
            writeln!(out, "{indent}/// {line}").unwrap();
        }
    }
}
