//! Writes the typed views: a struct per node kind with an accessor per role, and an enum per
//! `enum` item.

use std::collections::HashSet;
use std::fmt::Write as _;

use crate::grammar::{Grammar, NodeDef, Rule, Target, write_doc};
use crate::{HEADER, snake_case};

/// One element of a rule, in source order.
struct Atom<'g> {
    label: Option<&'g str>,
    target: Target<'g>,
    /// Under `*` or `+`.
    repeated: bool,
}

/// An accessor.
struct Field<'g> {
    name: String,
    target: Target<'g>,
    list: bool,
    /// Indices of its atoms.
    positions: Vec<usize>,
    kinds: HashSet<String>,
}

/// Where an accessor looks: after the `nth` direct token in `kinds` (a Rust expression for a
/// slice of kinds), or before it.
struct Anchor {
    kinds: String,
    nth: usize,
}

const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

pub fn generate(grammar: &Grammar) -> Result<String, String> {
    let mut out = String::from(HEADER);
    out.push_str(
        "
use crate::ast::support::{self, Anchor, AstChildren, AstTokens};
use crate::ast::AstNode;
use crate::syntax_kind::token_sets;
use crate::{SyntaxKind, SyntaxNode, SyntaxToken};
",
    );
    for node in &grammar.nodes {
        node_struct(&mut out, grammar, node)?;
    }
    for e in &grammar.enums {
        out.push('\n');
        write_doc(&mut out, "", &e.doc);
        writeln!(
            out,
            "#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]\npub enum {}<'t> {{",
            e.name
        )
        .unwrap();
        for variant in &e.variants {
            writeln!(out, "    {variant}({variant}<'t>),").unwrap();
        }
        out.push_str("}\n");
        let kinds = grammar.kinds(grammar.target(&e.name).unwrap());
        let kinds: Vec<_> = kinds.iter().map(|k| format!("SyntaxKind::{k}")).collect();
        writeln!(
            out,
            "
impl<'t> AstNode<'t> for {name}<'t> {{
    fn can_cast(kind: SyntaxKind) -> bool {{
        matches!(kind, {kinds})
    }}

    fn cast(node: SyntaxNode<'t>) -> Option<Self> {{
        let kind = node.kind();",
            name = e.name,
            kinds = kinds.join(" | ")
        )
        .unwrap();
        for variant in &e.variants {
            writeln!(
                out,
                "if {variant}::can_cast(kind) {{ return {variant}::cast(node).map({}::{variant}); }}",
                e.name
            )
            .unwrap();
        }
        writeln!(
            out,
            "None
    }}

    fn syntax(self) -> SyntaxNode<'t> {{
        match self {{"
        )
        .unwrap();
        for variant in &e.variants {
            writeln!(out, "{}::{variant}(it) => it.syntax(),", e.name).unwrap();
        }
        out.push_str("}\n    }\n}\n");
        for variant in &e.variants {
            writeln!(
                out,
                "
impl<'t> From<{variant}<'t>> for {name}<'t> {{
    fn from(node: {variant}<'t>) -> {name}<'t> {{
        {name}::{variant}(node)
    }}
}}",
                name = e.name
            )
            .unwrap();
        }
    }
    Ok(out)
}

fn node_struct(out: &mut String, grammar: &Grammar, node: &NodeDef) -> Result<(), String> {
    let name = &node.name;
    out.push('\n');
    let mut doc = node.doc.clone();
    if let Some(rule) = &node.rule_text {
        if !doc.is_empty() {
            doc.push(String::new());
        }
        doc.push("```text".to_owned());
        doc.push(format!("{name} = {rule}"));
        doc.push("```".to_owned());
    }
    write_doc(out, "", &doc);
    writeln!(
        out,
        "#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct {name}<'t>(SyntaxNode<'t>);

impl<'t> AstNode<'t> for {name}<'t> {{
    fn can_cast(kind: SyntaxKind) -> bool {{
        kind == SyntaxKind::{name}
    }}

    fn cast(node: SyntaxNode<'t>) -> Option<Self> {{
        Self::can_cast(node.kind()).then_some({name}(node))
    }}

    fn syntax(self) -> SyntaxNode<'t> {{
        self.0
    }}
}}"
    )
    .unwrap();
    let Some(rule) = &node.rule else {
        return Ok(());
    };
    let mut atoms = Vec::new();
    flatten(grammar, rule, None, false, &mut atoms);
    let fields = fields(grammar, &atoms).map_err(|e| format!("{name}: {e}"))?;
    if fields.is_empty() {
        return Ok(());
    }
    writeln!(out, "\nimpl<'t> {name}<'t> {{").unwrap();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (after, before) =
            anchors(&atoms, &fields, field).map_err(|e| format!("{name}: {e}"))?;
        let after = anchor_expr(after);
        let before = anchor_expr(before);
        let accessor = &field.name;
        match field.target {
            Target::Token(_) | Target::TokenSet(_) => {
                let kinds = kinds_expr(field.target);
                if field.list {
                    writeln!(
                        out,
                        "pub fn {accessor}(self) -> AstTokens<'t> {{ support::tokens(self.0, {kinds}, {after}, {before}) }}"
                    )
                    .unwrap();
                } else {
                    writeln!(
                        out,
                        "pub fn {accessor}(self) -> Option<SyntaxToken<'t>> {{ support::token(self.0, {kinds}, {after}, {before}) }}"
                    )
                    .unwrap();
                }
            }
            Target::Node(ty) | Target::Enum(ty) => {
                if field.list {
                    writeln!(
                        out,
                        "pub fn {accessor}(self) -> AstChildren<'t, {ty}<'t>> {{ support::children(self.0, {after}, {before}) }}"
                    )
                    .unwrap();
                } else {
                    writeln!(
                        out,
                        "pub fn {accessor}(self) -> Option<{ty}<'t>> {{ support::child(self.0, {after}, {before}) }}"
                    )
                    .unwrap();
                }
            }
        }
    }
    out.push_str("}\n");
    Ok(())
}

/// A Rust expression for the token kinds a token or token set accepts.
fn kinds_expr(target: Target<'_>) -> String {
    match target {
        Target::TokenSet(set) => format!("token_sets::{}", snake_case(set).to_uppercase()),
        Target::Token(token) => format!("&[SyntaxKind::{token}]"),
        Target::Node(_) | Target::Enum(_) => unreachable!(),
    }
}

fn anchor_expr(anchor: Option<Anchor>) -> String {
    match anchor {
        None => "None".to_owned(),
        Some(Anchor { kinds, nth }) => format!("Some(Anchor {{ kinds: {kinds}, nth: {nth} }})"),
    }
}

fn flatten<'g>(
    grammar: &'g Grammar,
    rule: &'g Rule,
    label: Option<&'g str>,
    repeated: bool,
    atoms: &mut Vec<Atom<'g>>,
) {
    match rule {
        Rule::Seq(items) | Rule::Alt(items) => {
            for item in items {
                flatten(grammar, item, label, repeated, atoms);
            }
        }
        Rule::Opt(inner) => flatten(grammar, inner, label, repeated, atoms),
        Rule::Star(inner) | Rule::Plus(inner) => flatten(grammar, inner, label, true, atoms),
        Rule::Labeled(l, inner) => flatten(grammar, inner, Some(l), repeated, atoms),
        Rule::Token(name) | Rule::Ref(name) => atoms.push(Atom {
            label,
            target: grammar.target(name).unwrap(),
            repeated,
        }),
    }
}

fn fields<'g>(grammar: &'g Grammar, atoms: &[Atom<'g>]) -> Result<Vec<Field<'g>>, String> {
    let mut fields: Vec<Field<'g>> = Vec::new();
    for (i, atom) in atoms.iter().enumerate() {
        let name = match (atom.label, atom.target) {
            (Some(label), _) => label.to_owned(),
            (None, Target::Node(ty) | Target::Enum(ty)) => {
                let name = snake_case(ty);
                if atom.repeated {
                    format!("{name}s")
                } else {
                    name
                }
            }
            // Unlabeled tokens are anchors, not fields.
            (None, Target::Token(_) | Target::TokenSet(_)) => continue,
        };
        if RUST_KEYWORDS.contains(&name.as_str()) {
            return Err(format!("{name} is a Rust keyword"));
        }
        if let Some(field) = fields.iter_mut().find(|f| f.name == name) {
            if field.target != atom.target {
                return Err(format!("{name} has two different types"));
            }
            field.list = true;
            field.positions.push(i);
        } else {
            fields.push(Field {
                name,
                target: atom.target,
                list: atom.repeated,
                positions: vec![i],
                kinds: grammar.kinds(atom.target).into_iter().collect(),
            });
        }
    }
    Ok(fields)
}

/// The tokens around `field` that tell it apart from fields that accept the same kinds: the
/// nearest token between an earlier such field and this one, and between this one and a later
/// such field.
fn anchors(
    atoms: &[Atom<'_>],
    fields: &[Field<'_>],
    field: &Field<'_>,
) -> Result<(Option<Anchor>, Option<Anchor>), String> {
    let first = *field.positions.first().unwrap();
    let last = *field.positions.last().unwrap();
    let mut previous = None;
    let mut next = None;
    for other in fields {
        if other.name == field.name || other.kinds.is_disjoint(&field.kinds) {
            continue;
        }
        for &position in &other.positions {
            if position < first {
                previous = previous.max(Some(position));
            } else if position > last {
                next = Some(next.map_or(position, |n: usize| n.min(position)));
            } else {
                return Err(format!(
                    "{} and {} accept the same kinds and are interleaved",
                    field.name, other.name
                ));
            }
        }
    }
    let token_anchor = |range: std::ops::Range<usize>, from_end: bool| -> Result<Anchor, String> {
        let mut candidates =
            range.filter(|&i| matches!(atoms[i].target, Target::Token(_) | Target::TokenSet(_)));
        let i = if from_end {
            candidates.next_back()
        } else {
            candidates.next()
        }
        .ok_or_else(|| {
            format!(
                "{} needs a token between it and another role of the same kinds",
                field.name
            )
        })?;
        let kinds = kinds_expr(atoms[i].target);
        let nth = atoms[..i]
            .iter()
            .filter(|a| a.target == atoms[i].target)
            .count();
        Ok(Anchor { kinds, nth })
    };
    let after = previous
        .map(|p| token_anchor(p + 1..first, true))
        .transpose()?;
    let before = next.map(|n| token_anchor(last + 1..n, false)).transpose()?;
    Ok((after, before))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar;

    const TOKENS: &str = "
token Identifier
token If 'if'
token Else 'else'
token Plus '+'
contextual WhenKw 'when'
";

    fn generate_from(rules: &str) -> Result<String, String> {
        let grammar = grammar::parse(&format!("{TOKENS}{rules}"))?;
        grammar.check_enums()?;
        generate(&grammar)
    }

    #[test]
    fn roles_of_the_same_kinds_look_between_tokens() {
        let out = generate_from(
            "node Name = Identifier
node IfStmt = 'if' cond:Identifier then:Name ('else' other:Name)?",
        )
        .unwrap();
        assert!(out.contains(
            "pub fn then(self) -> Option<Name<'t>> { support::child(self.0, None, \
             Some(Anchor { kinds: &[SyntaxKind::Else], nth: 0 })) }"
        ));
        assert!(out.contains(
            "pub fn other(self) -> Option<Name<'t>> { support::child(self.0, \
             Some(Anchor { kinds: &[SyntaxKind::Else], nth: 0 }), None) }"
        ));
    }

    #[test]
    fn errors() {
        let error = |rules| generate_from(rules).unwrap_err();
        assert!(error("node A = B").contains("B isn't declared"));
        assert!(error("node A = x:(Identifier '+')").contains("isn't on a single name"));
        assert!(error("node A = type:Identifier").contains("Rust keyword"));
        assert!(error("node A = Identifier\nnode B = x:A y:A").contains("needs a token"));
        assert!(error("node A = Identifier\nenum E = A | A").contains("in both A and A"));
        assert!(error("node A = '-'").contains("no token is spelled"));
    }
}
