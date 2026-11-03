use std::sync::Arc;

use super::*;
use crate::SyntaxKind as K;
use crate::ast::{self, AstNode};
use crate::invariants;
use crate::test_tree::{Events, build};

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
fn dump() {
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

    let error = build(
        "a",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::Error)
            .error()
            .token(K::Identifier)
            .finish()
            .token(K::Eof)
            .finish(),
    );
    assert_eq!(
        error.debug_dump(),
        "CompilationUnit 0..1 [error]\n  Error 0..1 [error]\n    Identifier 0..1 \"a\"\n  Eof 1..1 \"\"\n"
    );
}

#[test]
#[should_panic(expected = "an invalid syntax tree")]
#[cfg(debug_assertions)]
fn new_checks_the_arrays() {
    let tokens = ferrion_lexer::lex("a").tokens;
    let kinds = vec![K::Identifier, K::Eof];
    // A root that doesn't cover the `Eof`.
    let root = Node {
        kind: K::CompilationUnit,
        flags: 0,
        parent: NodeId::ROOT,
        first_token: TokenId::from_index(0),
        token_end: TokenId::from_index(1),
        subtree_end: NodeId::from_index(1),
    };
    SyntaxTree::new(Arc::from("a"), tokens, kinds, vec![root], 21);
}

#[test]
fn cursors() {
    let tree = package_tree();
    let root = tree.root();
    assert_eq!(root.parent(), None);
    let package = root.first_child().unwrap();
    assert_eq!(package.kind(), K::PackageDecl);
    assert_eq!(package.text(), "package a /* x */ . b;");
    assert_eq!(package.parent(), Some(root));
    assert_eq!(package.next_sibling(), None);
    let outer = package.children().next().unwrap();
    let inner = outer.first_child().unwrap();
    assert_eq!(inner.text(), "a");
    assert!(root.contains(inner) && outer.contains(inner) && !inner.contains(outer));
    let ancestors: Vec<_> = inner.ancestors().map(|n| n.kind()).collect();
    assert_eq!(
        ancestors,
        [K::Name, K::Name, K::PackageDecl, K::CompilationUnit]
    );

    let children: Vec<_> = outer
        .children_with_tokens()
        .map(|child| (child.kind(), child.span()))
        .collect();
    assert_eq!(
        children,
        [
            (K::Name, Span::new(13, 14)),
            (K::Whitespace, Span::new(14, 15)),
            (K::BlockComment, Span::new(15, 22)),
            (K::Whitespace, Span::new(22, 23)),
            (K::Dot, Span::new(23, 24)),
            (K::Whitespace, Span::new(24, 25)),
            (K::Identifier, Span::new(25, 26)),
        ]
    );
    let direct: Vec<_> = package.direct_tokens().map(|t| t.text()).collect();
    assert_eq!(direct, ["package", " ", ";"]);

    assert_eq!(package.first_token().text(), "package");
    assert_eq!(package.last_token().text(), ";");
    assert_eq!(root.first_token().kind(), K::LineComment);
    assert_eq!(root.last_token().kind(), K::Eof);
    let semi = package.last_token();
    assert_eq!(semi.prev_token().unwrap().text(), "b");
    assert_eq!(semi.next_token().unwrap().kind(), K::LineTerminator);
    assert_eq!(semi.next_non_trivia().unwrap().kind(), K::Eof);
    let dot = tree.token_at_offset(23).unwrap();
    assert_eq!(dot.kind(), K::Dot);
    assert_eq!(dot.prev_non_trivia().unwrap().text(), "a");
    assert_eq!(dot.parent(), outer);
    assert_eq!(tree.token_at_offset(15).unwrap().parent(), outer);
    assert_eq!(tree.token_at_offset(0).unwrap().parent(), root);
    assert_eq!(tree.token_at_offset(28).unwrap().kind(), K::Eof);
    assert_eq!(tree.token_at_offset(29), None);
    assert_eq!(tree.node_at_offset(13), Some(inner));
    assert_eq!(tree.node_at_offset(27), Some(root));

    let all: Vec<_> = tree.tokens().map(|t| t.text()).collect();
    assert_eq!(all.concat(), tree.text());
    let backwards: Vec<_> = package.tokens().rev().map(|t| t.text()).collect();
    assert_eq!(backwards.first(), Some(&";"));
}

#[test]
fn walks() {
    let tree = package_tree();
    let descendants: Vec<_> = tree.root().descendants().map(|n| n.kind()).collect();
    assert_eq!(
        descendants,
        [K::CompilationUnit, K::PackageDecl, K::Name, K::Name]
    );
    let events: Vec<_> = tree
        .root()
        .preorder()
        .map(|event| match event {
            WalkEvent::Enter(node) => format!("+{:?}", node.kind()),
            WalkEvent::Leave(node) => format!("-{:?}", node.kind()),
        })
        .collect();
    assert_eq!(
        events,
        [
            "+CompilationUnit",
            "+PackageDecl",
            "+Name",
            "+Name",
            "-Name",
            "-Name",
            "-PackageDecl",
            "-CompilationUnit"
        ]
    );
    let mut walk = tree.root().preorder();
    let mut entered = Vec::new();
    while let Some(event) = walk.next() {
        if let WalkEvent::Enter(node) = event {
            entered.push(node.kind());
            if node.kind() == K::PackageDecl {
                walk.skip_subtree();
            }
        }
    }
    assert_eq!(entered, [K::CompilationUnit, K::PackageDecl]);
}

#[test]
fn node_maps() {
    let tree = package_tree();
    let mut depths: NodeMap<usize> = NodeMap::new(&tree);
    assert_eq!(depths.len(), tree.node_count());
    for node in tree.nodes() {
        if let Some(parent) = node.parent() {
            depths[node.id()] = depths[parent.id()] + 1;
        }
    }
    let values: Vec<_> = depths.iter().map(|(_, &depth)| depth).collect();
    assert_eq!(values, [0, 1, 2, 3]);
    let kinds = NodeMap::from_fn(&tree, |node| node.kind());
    assert_eq!(kinds[NodeId::ROOT], K::CompilationUnit);
    assert_eq!(kinds.get(NodeId::from_index(4)), None);
    let flags = NodeMap::filled(&tree, false);
    assert!(!flags[NodeId::from_index(3)]);
}

#[test]
fn pointers_find_nodes_in_another_tree() {
    let first = package_tree();
    let second = package_tree();
    for node in first.nodes() {
        let ptr = SyntaxPtr::new(node);
        let found = ptr.resolve(&second).unwrap();
        assert_eq!(found.id(), node.id());
        assert!(std::ptr::eq(found.tree(), &second));
    }
    let missing = SyntaxPtr {
        kind: K::IfStmt,
        span: Span::new(5, 27),
    };
    assert_eq!(missing.resolve(&second), None);
}

#[test]
fn typed_views() {
    let tree = package_tree();
    let package = ast::PackageDecl::cast(tree.root().first_child().unwrap()).unwrap();
    let name = package.name().unwrap();
    assert_eq!(name.name().unwrap().text(), "b");
    assert_eq!(name.qualifier().unwrap().name().unwrap().text(), "a");
    assert_eq!(name.qualifier().unwrap().qualifier(), None);
    assert_eq!(package.annotations().count(), 0);
    assert!(ast::ImportDecl::cast(package.syntax()).is_none());
    let unit = ast::CompilationUnit::cast(tree.root()).unwrap();
    assert_eq!(unit.package_decl(), Some(package));
    assert_eq!(unit.members().count(), 0);
    assert!(ast::Expr::cast(name.syntax()).is_none());
    assert!(ast::Expr::can_cast(K::NameExpr) && ast::Expr::can_cast(K::SwitchExpr));
    assert!(!ast::Expr::can_cast(K::Name));
    assert!(ast::Stmt::can_cast(K::ClassDecl));
}

#[test]
fn accessors_look_between_their_tokens() {
    // `c ? : e`: the then branch is missing, and the else branch mustn't be taken for it.
    let tree = build(
        "c ? : e",
        Events::new()
            .start(K::CompilationUnit)
            .start(K::ConditionalExpr)
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .tokens(&[K::Question, K::Colon])
            .start(K::NameExpr)
            .token(K::Identifier)
            .finish()
            .finish()
            .token(K::Eof)
            .finish(),
    );
    let conditional = ast::ConditionalExpr::cast(tree.root().first_child().unwrap()).unwrap();
    assert_eq!(conditional.condition().unwrap().syntax().text(), "c");
    assert_eq!(conditional.then_expr(), None);
    assert_eq!(conditional.else_expr().unwrap().syntax().text(), "e");
}

#[test]
fn invariants_catch_broken_trees() {
    let good = package_tree();
    let broken = |change: fn(&mut SyntaxTree)| {
        let mut tree = good.clone();
        change(&mut tree);
        invariants::check_tree(&tree).unwrap_err()
    };
    let error = broken(|tree| tree.nodes[2].parent = NodeId::ROOT);
    assert!(error.contains("its parent is 0"), "{error}");
    let error = broken(|tree| tree.nodes[3].token_end = tree.nodes[3].first_token);
    assert!(error.contains("is empty"), "{error}");
    let error = broken(|tree| tree.nodes[1].first_token = TokenId::from_index(1));
    assert!(error.contains("trivia"), "{error}");
    let error = broken(|tree| tree.nodes[2].subtree_end = NodeId::from_index(3));
    assert!(error.contains("node 3"), "{error}");
    let error = broken(|tree| tree.token_kinds[2] = K::Identifier);
    assert!(error.contains("parsed as Identifier"), "{error}");
    let error = broken(|tree| tree.nodes[3].flags = flags::CONTAINS_ERROR);
    assert!(error.contains("CONTAINS_ERROR"), "{error}");
    let error = broken(|tree| tree.nodes[0].flags = flags::CONTAINS_ERROR);
    assert!(error.contains("without an error of its own"), "{error}");
    let error = broken(|tree| tree.nodes[0].flags = flags::OWN_ERROR);
    assert!(
        error.contains("OWN_ERROR without CONTAINS_ERROR"),
        "{error}"
    );
}

/// Runs `f` on a thread with a small stack, so that recursion shows up as a stack overflow.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_trees_build_walk_and_drop_without_recursion() {
    on_small_stack(|| {
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
        let deepest = tree.node(NodeId::from_index(DEPTH + 1));
        assert_eq!(deepest.ancestors().count(), DEPTH + 2);
        assert_eq!(tree.root().preorder().count(), 2 * (DEPTH + 2));
        assert_eq!(
            tree.token_at_offset(DEPTH as u32).unwrap().parent(),
            deepest
        );
        drop(tree);
    });
}
