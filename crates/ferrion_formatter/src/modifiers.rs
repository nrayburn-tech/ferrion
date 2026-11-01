//! Modifier reordering (docs/design/formatter.md, "Modifier reordering"). Keyword modifiers are
//! put in [`ORDER`], merged from the JLS productions for class, field, method, constructor, and
//! interface member modifiers (JLS 8.1.1, 8.3.1, 8.4.3, 8.8.3, 9.1.1, 9.4).
//!
//! Only a **run** is reordered: adjacent keyword modifiers with nothing but whitespace and line
//! breaks between them. An annotation or a comment ends a run and stays where it is, so a comment never moves
//! across a modifier. A list with a formatter marker in it, or in a disable region, isn't
//! reordered at all.

use ferrion_syntax::{NodeOrToken, SyntaxKind, SyntaxNode, SyntaxToken};

use crate::format::Formatter;

/// The order keyword modifiers are put in. Pairs the JLS never allows together (`final` and
/// `sealed`, `default` and `static`) get an order anyway, so the result is deterministic.
pub const ORDER: &[SyntaxKind] = &[
    SyntaxKind::PublicKw,
    SyntaxKind::ProtectedKw,
    SyntaxKind::PrivateKw,
    SyntaxKind::AbstractKw,
    SyntaxKind::DefaultKw,
    SyntaxKind::StaticKw,
    SyntaxKind::FinalKw,
    SyntaxKind::SealedKw,
    SyntaxKind::NonSealedKw,
    SyntaxKind::TransientKw,
    SyntaxKind::VolatileKw,
    SyntaxKind::SynchronizedKw,
    SyntaxKind::NativeKw,
    SyntaxKind::StrictfpKw,
];

/// One item of a `Modifiers` node: an annotation, or a run of keyword modifiers.
pub enum Item<'t> {
    Annotation(SyntaxNode<'t>),
    /// The run's modifiers in source order, each as its tokens: one, or three for `non-sealed`.
    Run(Vec<Vec<SyntaxToken<'t>>>),
}

/// The items of `modifiers`, a `Modifiers` node, in source order.
pub fn items(modifiers: SyntaxNode<'_>) -> Vec<Item<'_>> {
    let mut items = Vec::new();
    // A comment since the last modifier, which ends its run.
    let mut comment = false;
    for element in modifiers.children_with_tokens() {
        match element {
            NodeOrToken::Node(node) => items.push(Item::Annotation(node)),
            NodeOrToken::Token(token) if ferrion_syntax::comments::is_comment(token.kind()) => {
                comment = true;
            }
            NodeOrToken::Token(token) if token.is_trivia() => {}
            NodeOrToken::Token(token) => {
                let continues_unit = token.kind() == SyntaxKind::NonSealedKw
                    && token
                        .prev_token()
                        .is_some_and(|t| t.kind() == SyntaxKind::NonSealedKw);
                match items.last_mut() {
                    Some(Item::Run(run)) if continues_unit => {
                        run.last_mut().expect("a run is never empty").push(token);
                    }
                    Some(Item::Run(run)) if !comment => run.push(vec![token]),
                    _ => items.push(Item::Run(vec![vec![token]])),
                }
                comment = false;
            }
        }
    }
    items
}

/// Where a modifier goes in [`ORDER`].
pub fn rank(unit: &[SyntaxToken<'_>]) -> usize {
    let kind = unit[0].kind();
    ORDER.iter().position(|&k| k == kind).unwrap_or(ORDER.len())
}

/// Emits one run of keyword modifiers, with a space between them, sorted into [`ORDER`] if
/// `reorder`. The rule for a `Modifiers` node, in `rules::declarations`, lays out the runs and
/// annotations, and allows reordering unless the list is in a disable region or holds a
/// formatter marker.
pub fn run<'t>(f: &mut Formatter<'t>, run: &[Vec<SyntaxToken<'t>>], reorder: bool) {
    let mut sorted: Vec<&[SyntaxToken<'t>]> = run.iter().map(Vec::as_slice).collect();
    sorted.sort_by_key(|unit| rank(unit));
    let in_order = sorted.iter().zip(run).all(|(a, b)| a[0].id() == b[0].id());
    if reorder && !in_order {
        let first = run[0][0].id();
        let last = run.last().and_then(|unit| unit.last()).unwrap().id();
        f.reordered_run(first, last, &sorted);
    } else {
        for (j, unit) in run.iter().enumerate() {
            if j > 0 {
                f.space();
            }
            for &token in unit {
                f.token(token);
            }
        }
    }
}
