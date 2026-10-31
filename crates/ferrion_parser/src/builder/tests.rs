use std::sync::Arc;

use ferrion_syntax::SyntaxKind as K;
use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::invariants;

use super::*;

/// Events written by hand, as the parser would emit them.
struct Events(Vec<Event>);

impl Events {
    fn new() -> Events {
        Events(Vec::new())
    }

    fn start(mut self, kind: SyntaxKind) -> Events {
        self.0.push(Event::Start {
            kind,
            forward_parent: 0,
        });
        self
    }

    fn token(mut self, kind: SyntaxKind) -> Events {
        self.0.push(Event::Token { kind });
        self
    }

    fn tokens(mut self, kinds: &[SyntaxKind]) -> Events {
        for &kind in kinds {
            self = self.token(kind);
        }
        self
    }

    fn finish(mut self) -> Events {
        self.0.push(Event::Finish);
        self
    }

    fn event(mut self, event: Event) -> Events {
        self.0.push(event);
        self
    }
}

fn build(text: &str, events: Events) -> SyntaxTree {
    let lexed = ferrion_lexer::lex(text);
    let errors: Vec<_> = lexed.errors.iter().map(|e| e.span).collect();
    let tree = Builder::new(Arc::from(text), lexed.tokens, &errors, 21).build(events.0);
    invariants::check_tree(&tree).unwrap();
    tree
}

/// `package a.b;` with comments around it.
fn package_tree() -> SyntaxTree {
    build(
        "// c\npackage a /* x */ . b;\n",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::PackageDecl)
            .token(K::PackageKw)
            .start(K::Name)
            .start(K::Name)
            .token(K::Identifier)
            .finish()
            .tokens(&[K::Dot, K::Identifier])
            .finish()
            .token(K::Semi)
            .finish()
            .token(K::Eof)
            .finish(),
    )
}

#[test]
fn builder_places_trivia_around_nodes() {
    let tree = package_tree();
    assert_eq!(
        tree.debug_dump(),
        r#"CompilationUnit 0..28
  LineComment 0..4 "// c"
  LineTerminator 4..5 "\n"
  PackageDecl 5..27
    PackageKw 5..12 "package"
    Whitespace 12..13 " "
    Name 13..26
      Name 13..14
        Identifier 13..14 "a"
      Whitespace 14..15 " "
      BlockComment 15..22 "/* x */"
      Whitespace 22..23 " "
      Dot 23..24 "."
      Whitespace 24..25 " "
      Identifier 25..26 "b"
    Semi 26..27 ";"
  LineTerminator 27..28 "\n"
  Eof 28..28 ""
"#
    );
    assert_eq!(tree.release(), 21);
    assert_eq!(tree.node_count(), 4);
    assert_eq!(tree.text(), "// c\npackage a /* x */ . b;\n");
}

#[test]
fn builder_moves_preceded_nodes_outward() {
    // `a + b`, parsed as the parser does it: `a` first, then a node started before it.
    let mut events = Events::new()
        .start(K::CompilationUnit)
        .start(K::NameExpr)
        .token(K::Identifier)
        .finish();
    // `precede` leaves the NameExpr's Start pointing at the BinaryExpr's Start.
    let name_start = 1;
    events.0.push(Event::Start {
        kind: K::BinaryExpr,
        forward_parent: 0,
    });
    let binary_start = events.0.len() - 1;
    events.0[name_start] = Event::Start {
        kind: K::NameExpr,
        forward_parent: (binary_start - name_start) as u32,
    };
    let events = events
        .token(K::Plus)
        .start(K::NameExpr)
        .token(K::Identifier)
        .finish()
        .finish()
        .token(K::Eof)
        .finish();
    let tree = build("a + b", events);
    let kinds: Vec<_> = tree.nodes().map(|n| n.kind()).collect();
    assert_eq!(
        kinds,
        [K::CompilationUnit, K::BinaryExpr, K::NameExpr, K::NameExpr]
    );
    let binary = ast::BinaryExpr::cast(tree.root().first_child().unwrap()).unwrap();
    assert_eq!(binary.lhs().unwrap().syntax().text(), "a");
    assert_eq!(binary.op().unwrap().text(), "+");
    assert_eq!(binary.rhs().unwrap().syntax().text(), "b");
}

#[test]
fn builder_skips_abandoned_nodes() {
    let tree = build(
        "a",
        Events::new()
            .start(K::CompilationUnit)
            .event(Event::Tombstone)
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .token(K::Eof)
            .finish(),
    );
    let kinds: Vec<_> = tree.nodes().map(|n| n.kind()).collect();
    assert_eq!(kinds, [K::CompilationUnit, K::NameExpr]);
}

#[test]
fn builder_records_parsed_token_kinds() {
    let tree = build(
        "record",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::Name)
            .token(K::RecordKw)
            .finish()
            .token(K::Eof)
            .finish(),
    );
    let token = tree.token(TokenId::from_index(0));
    assert_eq!(token.kind(), K::RecordKw);
    assert_eq!(token.lexer_kind(), ferrion_lexer::TokenKind::Identifier);
}

#[test]
fn builder_covers_tokens_the_parser_left() {
    // The root takes every token, even ones the events never consumed.
    let tree = build(
        "a b",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .finish(),
    );
    assert_eq!(tree.root().token_range().end.index(), tree.token_count());
    assert_eq!(tree.root().span(), Span::new(0, 3));
}

#[test]
fn builder_makes_a_root_from_no_events() {
    let tree = build("", Events::new());
    assert_eq!(tree.node_count(), 1);
    assert_eq!(tree.root().kind(), K::CompilationUnit);
    assert_eq!(tree.root().span(), Span::new(0, 0));
}

#[test]
fn errors_mark_ancestors() {
    let tree = build(
        "a b;",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::ExprStmt)
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .start(K::Error)
            .event(Event::Error)
            .token(K::Identifier)
            .finish()
            .token(K::Semi)
            .finish()
            .token(K::Eof)
            .finish(),
    );
    let errors: Vec<_> = tree
        .nodes()
        .filter(|n| n.contains_error())
        .map(|n| n.kind())
        .collect();
    assert_eq!(errors, [K::CompilationUnit, K::ExprStmt, K::Error]);
}

#[test]
fn lexical_errors_mark_the_node_holding_the_token() {
    // An illegal character is placed like trivia, in the innermost node around it.
    let tree = build(
        "a # b",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::BinaryExpr)
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .finish()
            .token(K::Eof)
            .finish(),
    );
    let errors: Vec<_> = tree
        .nodes()
        .filter(|n| n.contains_error())
        .map(|n| n.kind())
        .collect();
    assert_eq!(errors, [K::CompilationUnit, K::BinaryExpr]);
    assert_eq!(
        tree.token_at_offset(2).unwrap().parent().kind(),
        K::BinaryExpr
    );

    // An unterminated comment is trivia, so it marks the node holding it.
    let tree = build(
        "a /* b",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .token(K::Eof)
            .finish(),
    );
    assert!(tree.root().contains_error());
    assert!(!tree.root().first_child().unwrap().contains_error());
}

#[test]
fn deep_trees_build_without_recursion() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            const DEPTH: usize = 1_000_000;
            let text = format!("{}x{}", "(".repeat(DEPTH), ")".repeat(DEPTH));
            let mut events = Events::new().start(K::CompilationUnit);
            for _ in 0..DEPTH {
                events = events.start(K::ParenExpr).token(K::LParen);
            }
            events = events.start(K::NameExpr).token(K::Identifier).finish();
            for _ in 0..DEPTH {
                events = events.token(K::RParen).finish();
            }
            let tree = build(&text, events.token(K::Eof).finish());
            assert_eq!(tree.node_count(), DEPTH + 2);
            drop(tree);
        })
        .unwrap()
        .join()
        .unwrap();
}
