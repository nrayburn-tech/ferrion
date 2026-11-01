//! The generated deep-nesting inputs (docs/design/parser-deep-nesting.md, "Testing"), for the
//! parser's stress tests and every consumer of the tree that recurses over it, such as the
//! formatter. Not part of the public API.
//!
//! Each [`Pattern`] repeats one construct: a **nesting** pattern nests each repetition in the
//! last, so it counts toward [`NESTING_LIMIT`](crate::NESTING_LIMIT), and a **chain** pattern
//! puts each one after the last at one level, so it doesn't. Inputs are generated rather than
//! committed, so they can be as large as the tests need.

use crate::NESTING_LIMIT;

/// How a pattern repeats its construct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Each repetition nests in the last, and counts toward the limit.
    Nesting,
    /// Each repetition follows the last at one level, and doesn't count.
    Chain,
}

/// One generated input.
#[derive(Clone, Copy, Debug)]
pub struct Pattern {
    pub name: &'static str,
    pub shape: Shape,
    /// The input with the construct repeated `n` times.
    pub generate: fn(usize) -> String,
    /// For chains, the links to check beyond 100,000.
    pub longest: usize,
}

/// Nesting patterns whose construct takes more than one level of the nesting limit, with how
/// many. Every construct takes one, as the design counts them ("10,000 nested constructs"), unless
/// it's listed here with a reason.
pub const LEVELS: &[(&str, usize)] = &[
    // Each repetition nests an expression (the field's initializer) and a class body.
    ("anonymous_classes", 2),
    // Each repetition nests the rule's expression and the switch block.
    ("switch_expressions", 2),
    // Each repetition nests a block (the lambda's body) and an expression (the local's
    // initializer).
    ("lambda_blocks", 2),
];

/// How many levels below the limit the enclosing class, member, and statement may take.
pub const MARGIN: usize = 32;

macro_rules! patterns {
    ($($name:ident: $shape:ident $(($longest:expr))?, $generate:expr;)*) => {
        /// Every pattern, nesting patterns first.
        pub fn patterns() -> Vec<Pattern> {
            vec![$(Pattern {
                name: stringify!($name),
                shape: Shape::$shape,
                generate: $generate,
                longest: 100_000 $(.max($longest))?,
            },)*]
        }
    };
}

patterns! {
    parentheses: Nesting, |n| field("int", nest("(", "1", ")", n));
    blocks: Nesting, |n| method(nest("{", "", "}", n));
    if_statements: Nesting, |n| method(nest("if (a) ", ";", "", n));
    classes: Nesting, |n| nest("class C { ", "", "}", n);
    anonymous_classes: Nesting, |n| {
        field("Object", nest("new Object() { Object f = ", "null", "; }", n))
    };
    lambdas: Nesting, |n| field("Object", nest("() -> ", "1", "", n));
    lambda_blocks: Nesting, |n| {
        field("Runnable", nest("() -> { Runnable r = ", "null", "; }", n))
    };
    if_blocks: Nesting, |n| method(nest("if (a) { ", ";", "}", n));
    array_initializers: Nesting, |n| field("Object[]", nest("{", "", "}", n));
    generic_types: Nesting, |n| format!("class A {{ {} x; }}\n", nest("List<", "String", ">", n));
    conditionals: Nesting, |n| field("int", nest("a ? ", "1", " : 2", n));
    switch_expressions: Nesting, |n| {
        field("int", nest("switch (a) { default -> ", "1", "; }", n))
    };
    call_arguments: Nesting, |n| field("int", nest("f(", "1", ")", n));
    annotations: Nesting, |n| nest("@A(", "", ")", n) + " package p;\n";

    plus: Chain(250_000), |n| field("int", "a + ".repeat(n) + "a");
    string_concatenation: Chain, |n| field("String", "\"a\" + ".repeat(n) + "\"a\"");
    method_calls: Chain, |n| field("Object", "a".to_owned() + &".b()".repeat(n));
    field_accesses: Chain, |n| field("Object", "a".to_owned() + &".b".repeat(n));
    array_accesses: Chain, |n| field("Object", "a".to_owned() + &"[0]".repeat(n));
    else_if: Chain, |n| method("if (a) {}".to_owned() + &" else if (a) {}".repeat(n));
    conditional_else: Chain, |n| field("int", "a ? 1 : ".repeat(n) + "2");
    prefix_operators: Chain, |n| field("int", "- ".repeat(n) + "1");
    casts: Chain, |n| field("int", "(int) ".repeat(n) + "1");
    statements: Chain, |n| method("a = 1; ".repeat(n));
    members: Chain, |n| format!("class A {{ {}}}\n", "int x; ".repeat(n));
    qualified_import: Chain(500_000), |n| format!("import {}A;\n", "a.".repeat(n));
    qualified_package: Chain(500_000), |n| format!("package {}a;\n", "a.".repeat(n));
    imports: Chain, |n| "import a.B;\n".repeat(n);
}

/// The pattern named `name`. Panics if there is none.
pub fn pattern(name: &str) -> Pattern {
    patterns()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no pattern named {name}"))
}

/// The deepest repetition of a nesting pattern that should parse without a diagnostic: the limit
/// less a margin for the enclosing declarations, divided by the levels each repetition takes.
pub fn below_limit(pattern: &Pattern) -> usize {
    let levels = LEVELS
        .iter()
        .find(|(name, _)| *name == pattern.name)
        .map_or(1, |(_, levels)| *levels);
    (NESTING_LIMIT as usize - MARGIN) / levels
}

/// The most repetitions of `pattern` that parse without errors: just below the limit for a
/// nesting pattern, and [`Pattern::longest`] links for a chain.
pub fn full_depth(pattern: &Pattern) -> usize {
    match pattern.shape {
        Shape::Nesting => below_limit(pattern),
        Shape::Chain => pattern.longest,
    }
}

/// `open` `n` times, then `middle`, then `close` `n` times.
fn nest(open: &str, middle: &str, close: &str, n: usize) -> String {
    open.repeat(n) + middle + &close.repeat(n)
}

/// A class with a field of type `ty` initialized to `init`.
fn field(ty: &str, init: String) -> String {
    format!("class A {{ {ty} x = {init}; }}\n")
}

/// A class with a method whose body holds `body`.
fn method(body: String) -> String {
    format!("class A {{ void m() {{ {body} }} }}\n")
}
