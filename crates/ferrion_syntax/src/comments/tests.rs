use ferrion_diagnostics::Span;

use super::*;
use crate::SyntaxKind as K;
use crate::ast::AstNode;
use crate::test_tree::{Events, build, build_for};

/// Each comment in `tree`, one per line: its text, placement, and element (a node's kind and
/// text, or a token's text).
fn placements(tree: &SyntaxTree) -> String {
    let map = CommentMap::new(tree);
    let mut out = String::new();
    for comment in map.comments() {
        let element = match comment.element.get(tree) {
            NodeOrToken::Node(node) => format!("{:?} {:?}", node.kind(), node.text()),
            NodeOrToken::Token(token) => format!("{:?}", token.text()),
        };
        let text = tree.token(comment.token).text();
        out.push_str(&format!("{text} {:?} {element}\n", comment.placement));
    }
    out
}

/// A call statement, `name(args);`, with `args` code tokens between the parentheses.
const CALL: &str = "ExprStmt(MethodCallExpr(* Arguments(* *)) *)";

fn block(text: &str, statements: usize) -> SyntaxTree {
    let spec = format!(
        "CompilationUnit(Block(* {} *) *)",
        [CALL].repeat(statements).join(" ")
    );
    build(text, Events::parse(&spec))
}

#[test]
fn statements_in_a_block() {
    let tree = block(
        "{\n  // lead\n  a(); // trail\n  /* x */ b();\n  c(); /* y */\n  // end\n}\n",
        3,
    );
    assert_eq!(
        placements(&tree),
        "// lead Leading ExprStmt \"a();\"
// trail Trailing ExprStmt \"a();\"
/* x */ Leading ExprStmt \"b();\"
/* y */ Trailing ExprStmt \"c();\"
// end Trailing ExprStmt \"c();\"
"
    );
}

#[test]
fn dangling() {
    let tree = block("{ /* nothing */ }", 0);
    assert_eq!(
        placements(&tree),
        "/* nothing */ Dangling Block \"{ /* nothing */ }\"\n"
    );

    let tree = build(
        "f(/* none */);",
        Events::parse("CompilationUnit(ExprStmt(MethodCallExpr(* Arguments(* *)) *) *)"),
    );
    assert_eq!(
        placements(&tree),
        "/* none */ Dangling Arguments \"(/* none */)\"\n"
    );

    let tree = build(
        "// only\n/* comments */",
        Events::parse("CompilationUnit(*)"),
    );
    assert_eq!(
        placements(&tree),
        "// only Dangling CompilationUnit \"// only\\n/* comments */\"
/* comments */ Dangling CompilationUnit \"// only\\n/* comments */\"
"
    );
}

#[test]
fn tokens_are_elements() {
    // `if /* c */ (x) {}`: between two tokens of the `if`.
    let tree = build(
        "if /* c */ (x) {}",
        Events::parse("CompilationUnit(IfStmt(* * NameExpr(*) * Block(* *)) *)"),
    );
    assert_eq!(placements(&tree), "/* c */ Leading \"(\"\n");

    // A line comment after an opening brace trails it.
    let tree = block("{ // c\n  a();\n}", 1);
    assert_eq!(placements(&tree), "// c Trailing \"{\"\n");

    let call = |text: &str, args: &str| {
        build(
            text,
            Events::parse(&format!(
                "CompilationUnit(ExprStmt(MethodCallExpr(* Arguments(* {args} *)) *) *)"
            )),
        )
    };
    let tree = call("f(a /* c */);", "NameExpr(*)");
    assert_eq!(placements(&tree), "/* c */ Trailing NameExpr \"a\"\n");
    let tree = call("f(a, /* c */ b);", "NameExpr(*) * NameExpr(*)");
    assert_eq!(placements(&tree), "/* c */ Leading NameExpr \"b\"\n");
    let tree = call("f(a, // c\n  b);", "NameExpr(*) * NameExpr(*)");
    assert_eq!(placements(&tree), "// c Trailing \",\"\n");
    let tree = call("f(/* c */ a);", "NameExpr(*)");
    assert_eq!(placements(&tree), "/* c */ Leading NameExpr \"a\"\n");
}

#[test]
fn same_line_and_line_breaks() {
    // Block comments trail only when the rest of the line has no code.
    let tree = block("{ a(); /* x */ /* y */\n b(); /* z */ c(); }", 3);
    assert_eq!(
        placements(&tree),
        "/* x */ Trailing ExprStmt \"a();\"
/* y */ Trailing ExprStmt \"a();\"
/* z */ Leading ExprStmt \"c();\"
"
    );
    // A block comment spanning lines puts what follows it on another line.
    let tree = block("{ a(); /* x\n */ /* y */ // z\n b(); }", 2);
    assert_eq!(
        placements(&tree),
        "/* x\n */ Trailing ExprStmt \"a();\"
/* y */ Leading ExprStmt \"b();\"
// z Leading ExprStmt \"b();\"
"
    );

    let tree = block("{\n  a();\n\n\n  // c\n  /* d */ b();\n}", 2);
    let map = CommentMap::new(&tree);
    let breaks: Vec<_> = map
        .comments()
        .iter()
        .map(|c| (c.line_breaks_before, c.line_breaks_after))
        .collect();
    assert_eq!(breaks, [(3, 1), (1, 0)]);
}

#[test]
fn comments_at_the_start_and_end_of_the_file() {
    let tree = build(
        "// header\n\npackage a; // trail\n// end\n",
        Events::parse("CompilationUnit(PackageDecl(* Name(*) *) *)"),
    );
    assert_eq!(
        placements(&tree),
        "// header Leading PackageDecl \"package a;\"
// trail Trailing PackageDecl \"package a;\"
// end Trailing PackageDecl \"package a;\"
"
    );
    let map = CommentMap::new(&tree);
    assert_eq!(map.comments()[0].line_breaks_before, 0);
    assert_eq!(map.comments()[0].line_breaks_after, 2);
    assert_eq!(map.comments()[2].line_breaks_after, 1);
}

#[test]
fn leading_the_outermost_node_starting_there() {
    // The comment leads the statement, not the call or the name inside it.
    let tree = block("{ /* c */ a(); }", 1);
    assert_eq!(placements(&tree), "/* c */ Leading ExprStmt \"a();\"\n");
}

#[test]
fn doc_comments_never_trail() {
    let field = "FieldDecl(PrimitiveType(*) VariableDeclarator(*) *)";
    let text = "class T { int x; /** d */\n  int y; /// e\n  int z; }";
    let spec = format!("CompilationUnit(ClassDecl(* * ClassBody(* {field} {field} {field} *)) *)");
    // `///` is a line comment below 23, and a doc comment from 23.
    assert_eq!(
        placements(&build_for(text, Events::parse(&spec), 22)),
        "/** d */ Leading FieldDecl \"int y;\"
/// e Trailing FieldDecl \"int y;\"
"
    );
    assert_eq!(
        placements(&build_for(text, Events::parse(&spec), 23)),
        "/** d */ Leading FieldDecl \"int y;\"
/// e Leading FieldDecl \"int z;\"
"
    );
}

#[test]
fn lookups() {
    let tree = block("{\n  // a\n  /* b */\n  x(); // c\n}", 1);
    let map = CommentMap::new(&tree);
    let statement = tree.nodes().find(|n| n.kind() == K::ExprStmt).unwrap();
    let texts = |comments: &[Comment]| -> Vec<&str> {
        comments
            .iter()
            .map(|c| tree.token(c.token).text())
            .collect()
    };
    assert_eq!(texts(map.leading(statement)), ["// a", "/* b */"]);
    assert_eq!(texts(map.trailing(statement)), ["// c"]);
    assert_eq!(texts(map.dangling(statement)), [] as [&str; 0]);
    assert_eq!(texts(map.of(statement)), ["// a", "/* b */", "// c"]);
    assert_eq!(texts(map.of(tree.root())), [] as [&str; 0]);

    let first = map.comments()[0];
    assert_eq!(map.get(first.token), Some(&first));
    assert_eq!(map.get(statement.first_token().id()), None);
    assert_eq!(first.enclosing, statement.parent().unwrap().id());
}

/// The doc comment of a `kind` node holding `decl`'s code tokens, after `before`, at `release`:
/// its tokens' text, joined with `|`.
fn doc_at(before: &str, decl: &str, kind: SyntaxKind, release: u32) -> Option<String> {
    let code = |text: &str| {
        ferrion_lexer::lex(text)
            .tokens
            .iter()
            .filter(|t| !t.kind.is_trivia() && t.kind != ferrion_lexer::TokenKind::Eof)
            .count()
    };
    let (n, m) = (code(before), code(decl));
    let before_spec = if n == 0 {
        String::new()
    } else {
        format!("*{n}")
    };
    let spec = format!("CompilationUnit({before_spec} {kind:?}(*{m}) *)");
    let tree = build_for(&format!("{before}{decl}"), Events::parse(&spec), release);
    let node = tree.nodes().find(|node| node.kind() == kind).unwrap();
    let doc = doc_comment(node)?;
    assert_eq!(
        doc.kind(),
        match doc.first_token().kind() {
            K::DocComment => DocCommentKind::Javadoc,
            _ => DocCommentKind::Markdown,
        }
    );
    let tokens: Vec<_> = doc.tokens().collect();
    assert_eq!(
        doc.span(),
        Span::new(tokens[0].span().start, tokens.last().unwrap().span().end)
    );
    Some(doc.tokens().map(|t| t.text()).collect::<Vec<_>>().join("|"))
}

fn doc(before: &str, decl: &str) -> Option<String> {
    doc_at(before, decl, K::MethodDecl, 27)
}

#[test]
fn doc_comments_follow_javac() {
    // Observed with javac 27 (docs/design/parser-comments.md#doc-comments).
    let some = |s: &str| Some(s.to_owned());
    let method = "void m() {}";
    assert_eq!(doc("/** a */ /** b */ ", method), some("/** b */"));
    assert_eq!(doc("/** a */ /* x */ ", method), some("/** a */"));
    assert_eq!(doc("/** a */ // y\n", method), some("/** a */"));
    assert_eq!(
        doc("/** a */ ", "@Deprecated void m() {}"),
        some("/** a */")
    );
    assert_eq!(doc("", "@Deprecated /** a */ void m() {}"), None);
    assert_eq!(doc("", "public /** a */ void m() {}"), None);
    assert_eq!(doc("/** a */ ", "<T> void m() {}"), some("/** a */"));
    assert_eq!(doc("", "<T> /** a */ void m() {}"), None);
    assert_eq!(doc("/// a\n/** b */\n", method), some("/** b */"));
    assert_eq!(doc("/** a */\n/// b\n", method), some("/// b"));
    assert_eq!(doc("/** a */ /// b\n", method), some("/// b"));
    assert_eq!(doc("/// a\n/* x */ ", method), some("/// a"));
    assert_eq!(doc("/// a\n\n/// b\n", method), some("/// b"));
    assert_eq!(doc("/// a\n// x\n/// b\n", method), some("/// b"));
    assert_eq!(
        doc("int x; /// a\n /// b\n  ///\n", method),
        some("/// a|/// b|///")
    );
    assert_eq!(doc("//// a\n", method), some("//// a"));
    // Code between the comment and the declaration.
    assert_eq!(doc("/** a */ ; ", method), None);
    assert_eq!(doc("/** a */ int x; ", method), None);
    assert_eq!(doc("", method), None);
    // `/**/` is an empty block comment, not a doc comment (lexer-oracle-normalization.md).
    assert_eq!(doc("/**/ ", method), None);
    assert_eq!(doc("/** a */ /**/ ", method), some("/** a */"));
    assert_eq!(doc("/***/ ", method), some("/***/"));
}

#[test]
fn markdown_doc_comments_from_23() {
    let method = "void m() {}";
    let at = |before: &str, release| doc_at(before, method, K::MethodDecl, release);
    assert_eq!(at("/// a\n", 22), None);
    assert_eq!(at("/// a\n", 23).as_deref(), Some("/// a"));
    assert_eq!(at("/** b */\n/// a\n", 22).as_deref(), Some("/** b */"));
    assert_eq!(at("/** b */\n/// a\n", 23).as_deref(), Some("/// a"));
}

#[test]
fn doc_comments_by_kind() {
    let at = |decl: &str, kind| doc_at("/** a */ ", decl, kind, 27);
    for (decl, kind) in [
        ("package p;", K::PackageDecl),
        ("module m {}", K::ModuleDecl),
        ("class T {}", K::ClassDecl),
        ("interface T {}", K::InterfaceDecl),
        ("enum T {}", K::EnumDecl),
        ("record T() {}", K::RecordDecl),
        ("@interface T {}", K::AnnotationTypeDecl),
        ("int x, y;", K::FieldDecl),
        ("T() {}", K::ConstructorDecl),
        ("T {}", K::CompactConstructorDecl),
        ("A", K::EnumConstant),
    ] {
        assert_eq!(at(decl, kind).as_deref(), Some("/** a */"), "{decl}");
    }
    for (decl, kind) in [
        ("static {}", K::Initializer),
        ("int x", K::RecordComponent),
        ("int x", K::FormalParameter),
        ("int x = 1;", K::LocalVarDecl),
        ("return;", K::ReturnStmt),
        ("import a.B;", K::ImportDecl),
        ("requires a;", K::RequiresDirective),
    ] {
        assert_eq!(at(decl, kind), None, "{decl}");
    }
}

#[test]
fn typed_accessors() {
    let tree = build(
        "/** p */\n@A package a;",
        Events::parse("CompilationUnit(PackageDecl(Annotation(* Name(*)) * Name(*) *) *)"),
    );
    let package = tree
        .nodes()
        .find_map(crate::ast::PackageDecl::cast)
        .unwrap();
    assert_eq!(
        package.doc_comment().unwrap().first_token().text(),
        "/** p */"
    );

    let tree = build(
        "class T { /** d */ static {} /** m */ void m() {} }",
        Events::parse(
            "CompilationUnit(ClassDecl(* * ClassBody(* Initializer(* Block(* *)) \
             MethodDecl(VoidType(*) * FormalParameters(* *) Block(* *)) *)) *)",
        ),
    );
    let class = tree.nodes().find_map(crate::ast::TypeDecl::cast).unwrap();
    assert_eq!(class.doc_comment(), None);
    let crate::ast::TypeDecl::ClassDecl(class) = class else {
        panic!("not a class");
    };
    let members: Vec<_> = class.body().unwrap().members().collect();
    assert_eq!(members[0].doc_comment(), None);
    let doc = members[1].doc_comment().unwrap();
    assert_eq!(doc.kind(), DocCommentKind::Javadoc);
    assert_eq!(&tree.text()[doc.span().range()], "/** m */");
}

/// Nesting as deep as the parser allows, with a comment after each closing parenthesis: placing
/// them takes no recursion, and time linear in the depth.
#[test]
fn deep_nesting() {
    const DEPTH: usize = 20_000;
    let text = format!("{}x{}", "(".repeat(DEPTH), ")/* c */".repeat(DEPTH));
    let spec = format!(
        "CompilationUnit({} NameExpr(*) {} *)",
        "ParenExpr(*".repeat(DEPTH),
        "*)".repeat(DEPTH)
    );
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let tree = build(&text, Events::parse(&spec));
            let map = CommentMap::new(&tree);
            assert_eq!(map.comments().len(), DEPTH);
            // Each comment but the last trails the parenthesized expression it follows.
            for comment in &map.comments()[..DEPTH - 1] {
                assert_eq!(comment.placement, Placement::Trailing);
                assert_eq!(tree.node(comment.enclosing).kind(), K::ParenExpr);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
