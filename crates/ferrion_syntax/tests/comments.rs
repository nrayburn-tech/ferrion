//! The comment map and doc comment lookup on files parsed by `ferrion_parser`.
//!
//! Doc comment expectations were checked against javac 27 (`DocTrees.getDocComment`), as
//! docs/design/parser-comments.md describes. Placements follow the rules `CommentMap` documents.

use std::sync::Arc;

use ferrion_doc_comments::{Inline, markdown_comment_runs, parse_javadoc, parse_markdown};
use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::ast::{AstNode, CompilationUnit};
use ferrion_syntax::ast::{Member, TypeDecl};
use ferrion_syntax::comments::{self, CommentMap, DocCommentKind, Placement};
use ferrion_syntax::{NodeOrToken, SyntaxKind, SyntaxNode, SyntaxTree};

fn parse_at(text: &str, release: u32) -> SyntaxTree {
    let text: Arc<str> = Arc::from(text);
    let parsed = parse(text.clone(), ParseOptions { release });
    // The tree's invariants include the comment map's and the doc comment lookup's.
    ferrion_parser::invariants::check(&text, release, &parsed).unwrap();
    parsed.tree
}

fn unit(tree: &SyntaxTree) -> CompilationUnit<'_> {
    CompilationUnit::cast(tree.root()).unwrap()
}

/// Each comment, one per line: its text, placement, and element.
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

#[test]
fn package_info() {
    let text = "// File header\n\n/** Package docs. */\n@Deprecated\npackage a.b;\n";
    let tree = parse_at(text, 21);
    let package = unit(&tree).package_decl().unwrap();
    let doc = package.doc_comment().unwrap();
    assert_eq!(doc.kind(), DocCommentKind::Javadoc);
    assert_eq!(&text[doc.span().range()], "/** Package docs. */");

    // The doc comment hands off to the doc comment parser.
    let javadoc = parse_javadoc(tree.text(), doc.span());
    assert_eq!(javadoc.span, doc.span());
    let [Inline::Text(description)] = javadoc.description.as_slice() else {
        panic!("{:?}", javadoc.description);
    };
    assert_eq!(description.text, "Package docs.");

    assert_eq!(
        placements(&tree),
        "// File header Leading PackageDecl \"@Deprecated\\npackage a.b;\"
/** Package docs. */ Leading PackageDecl \"@Deprecated\\npackage a.b;\"
"
    );
}

#[test]
fn package_doc_comments() {
    let doc = |text: &str, release| {
        let tree = parse_at(text, release);
        let package = unit(&tree).package_decl().unwrap();
        package
            .doc_comment()
            .map(|doc| text[doc.span().range()].to_owned())
    };
    assert_eq!(
        doc("/** a */ /** b */ package p;", 21).as_deref(),
        Some("/** b */")
    );
    assert_eq!(
        doc("/** a */ // x\npackage p;", 21).as_deref(),
        Some("/** a */")
    );
    assert_eq!(
        doc("/** a */ @A package p;", 21).as_deref(),
        Some("/** a */")
    );
    assert_eq!(doc("@A /** a */ package p;", 21), None);
    assert_eq!(doc("/* a */ package p;", 21), None);
    assert_eq!(doc("package p;", 21), None);
    // `///` documents from release 23.
    assert_eq!(
        doc("/** a */\n/// b\npackage p;", 22).as_deref(),
        Some("/** a */")
    );
    assert_eq!(
        doc("/** a */\n/// b\npackage p;", 23).as_deref(),
        Some("/// b")
    );
}

#[test]
fn markdown_package_docs() {
    let text = "/// First line.\n///\n/// Second paragraph.\npackage a;\n";
    let tree = parse_at(text, 23);
    let doc = unit(&tree).package_decl().unwrap().doc_comment().unwrap();
    assert_eq!(doc.kind(), DocCommentKind::Markdown);
    assert_eq!(doc.tokens().count(), 3);

    // The lines are a run as the doc comment crate groups them, and parse as one comment.
    let lines: Vec<_> = doc.tokens().map(|t| t.span()).collect();
    assert_eq!(
        markdown_comment_runs(tree.lexer_tokens()),
        std::slice::from_ref(&lines)
    );
    let markdown = parse_markdown(tree.text(), &lines);
    assert_eq!(markdown.span, doc.span());
    assert_eq!(markdown.description.len(), 2);

    let tree = parse_at(text, 22);
    assert_eq!(unit(&tree).package_decl().unwrap().doc_comment(), None);
}

#[test]
fn imports() {
    let text = "\
package a;

import b.C; // trailing
// group
import d.E;

/** not documentation */
import static f.G.h;
";
    let tree = parse_at(text, 21);
    assert_eq!(
        placements(&tree),
        "// trailing Trailing ImportDecl \"import b.C;\"
// group Leading ImportDecl \"import d.E;\"
/** not documentation */ Leading ImportDecl \"import static f.G.h;\"
"
    );
    let map = CommentMap::new(&tree);
    let comments = map.comments();
    assert_eq!(
        (
            comments[1].line_breaks_before,
            comments[1].line_breaks_after
        ),
        (1, 1)
    );
    assert_eq!(comments[2].line_breaks_before, 2);
    let imports: Vec<_> = unit(&tree).import_decls().collect();
    assert_eq!(map.trailing(imports[0].syntax()).len(), 1);
    assert_eq!(map.leading(imports[1].syntax()).len(), 1);
}

#[test]
fn comments_inside_declarations() {
    let text = "package /* a */ b . /* c */ d;\nimport e.f /* g */ ;";
    let tree = parse_at(text, 21);
    assert_eq!(
        placements(&tree),
        "/* a */ Leading Name \"b . /* c */ d\"
/* c */ Leading \"d\"
/* g */ Leading \";\"
"
    );
}

#[test]
fn a_file_of_comments() {
    let tree = parse_at("// a\n\n/* b */\n", 21);
    let map = CommentMap::new(&tree);
    assert!(
        map.comments()
            .iter()
            .all(|c| c.placement == Placement::Dangling && c.enclosing == tree.root().id())
    );
    assert_eq!(map.dangling(tree.root()).len(), 2);
}

/// Every node that can have a doc comment, one per line: its kind, its name, and its doc
/// comment's lines (`-` for none).
fn docs(text: &str, release: u32) -> String {
    let tree = parse_at(text, release);
    let mut out = String::new();
    for node in tree.nodes() {
        if !comments::can_have_doc_comment(node.kind()) {
            continue;
        }
        let doc = comments::doc_comment(node).map_or("-".to_owned(), |doc| {
            let lines: Vec<_> = doc.tokens().map(|t| t.text()).collect();
            lines.join("|")
        });
        out.push_str(&format!("{:?} {} {doc}\n", node.kind(), name(node)));
    }
    out
}

/// A declaration's name: its identifier, its `Name`, or its first declarator's.
fn name(node: SyntaxNode<'_>) -> String {
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                return token.text().to_owned();
            }
            NodeOrToken::Node(child)
                if matches!(
                    child.kind(),
                    SyntaxKind::Name | SyntaxKind::VariableDeclarator
                ) =>
            {
                return child.text().split([' ', '=']).next().unwrap().to_owned();
            }
            _ => {}
        }
    }
    "?".to_owned()
}

/// The table in docs/design/parser-comments.md, in one class.
const DOC_TABLE: &str = "\
class T {
    /** a */ /** b */ void m1() {}
    /** c */ /* x */ void m2() {}
    /** d */ // y
    void m3() {}
    /** e */ @Deprecated void m4() {}
    @Deprecated /** f */ void m5() {}
    public /** g */ void m6() {}
    /** h */ <T> void m7() {}
    <T> /** i */ void m8() {}
    /// j
    /** k */
    void m9() {}
    /** l */
    /// m
    void m10() {}
    /// n

    /// o
    void m11() {}
    /** p */ ; void m12() {}
}
";

#[test]
fn doc_comments_follow_javac() {
    assert_eq!(
        docs(DOC_TABLE, 25),
        "ClassDecl T -
MethodDecl m1 /** b */
MethodDecl m2 /** c */
MethodDecl m3 /** d */
MethodDecl m4 /** e */
MethodDecl m5 -
MethodDecl m6 -
MethodDecl m7 /** h */
MethodDecl m8 -
MethodDecl m9 /** k */
MethodDecl m10 /// m
MethodDecl m11 /// o
MethodDecl m12 -
"
    );
    // Below 23, `///` lines are ordinary line comments.
    assert_eq!(
        docs(DOC_TABLE, 22),
        "ClassDecl T -
MethodDecl m1 /** b */
MethodDecl m2 /** c */
MethodDecl m3 /** d */
MethodDecl m4 /** e */
MethodDecl m5 -
MethodDecl m6 -
MethodDecl m7 /** h */
MethodDecl m8 -
MethodDecl m9 /** k */
MethodDecl m10 /** l */
MethodDecl m11 -
MethodDecl m12 -
"
    );
}

/// Every kind of declaration javac documents, and some it doesn't.
const DECLARATIONS: &str = "\
/** pkg */
package p;

/** cls */
public class T<X> {
    /** ctor */ T() {}
    /** fields */ int a, b = 1;
    /** inner */ static class N {}
    /** e */ enum E { /** c1 */ C1, /** c2 */ @Deprecated C2, @Deprecated /** c3 */ C3; }
    /** r */ record R(/** comp */ int x) { /** compact */ R {} }
    /** i */ interface I { /** im */ default void im() {} }
    /** at */ @interface A { /** v */ int v() default 1; }
    /** init */ static {}
    /** init2 */ {}
    void m(/** param */ int p) {
        /** local class */ class L {}
        /** local record */ record LR() {}
        /** local var */ int x = 1;
        /** local final */ final int y = 2;
        Runnable r = () -> { /** in lambda */ class LL {} };
    }
}
";

#[test]
fn doc_comments_by_declaration_kind() {
    assert_eq!(
        docs(DECLARATIONS, 25),
        "PackageDecl p /** pkg */
ClassDecl T /** cls */
ConstructorDecl T /** ctor */
FieldDecl a /** fields */
ClassDecl N /** inner */
EnumDecl E /** e */
EnumConstant C1 /** c1 */
EnumConstant C2 /** c2 */
EnumConstant C3 -
RecordDecl R /** r */
CompactConstructorDecl R /** compact */
InterfaceDecl I /** i */
MethodDecl im /** im */
AnnotationTypeDecl A /** at */
MethodDecl v /** v */
MethodDecl m -
ClassDecl L /** local class */
RecordDecl LR /** local record */
ClassDecl LL /** in lambda */
"
    );

    // javac documents none of these, so they have none, though each comment leads its node.
    let tree = parse_at(DECLARATIONS, 25);
    let map = CommentMap::new(&tree);
    let mut undocumented = Vec::new();
    for node in tree.nodes() {
        if matches!(
            node.kind(),
            SyntaxKind::RecordComponent
                | SyntaxKind::Initializer
                | SyntaxKind::FormalParameter
                | SyntaxKind::LocalVarDecl
        ) {
            assert_eq!(comments::doc_comment(node), None);
            let leading = map.leading(node).first();
            let leading = leading.map_or("-", |c| tree.token(c.token).text());
            undocumented.push(format!("{:?} {leading}", node.kind()));
        }
    }
    assert_eq!(
        undocumented,
        [
            "RecordComponent /** comp */",
            "Initializer /** init */",
            "Initializer /** init2 */",
            "FormalParameter /** param */",
            "LocalVarDecl /** local var */",
            "LocalVarDecl /** local final */",
            "LocalVarDecl -",
        ]
    );
}

#[test]
fn typed_doc_comment_accessors() {
    let tree = parse_at(DECLARATIONS, 25);
    let member = unit(&tree).members().next().unwrap();
    fn text(doc: Option<comments::DocComment<'_>>) -> Option<&str> {
        doc.map(|d| d.first_token().text())
    }
    assert_eq!(text(member.doc_comment()), Some("/** cls */"));
    let Member::TypeDecl(TypeDecl::ClassDecl(class)) = member else {
        panic!("{member:?}");
    };
    assert_eq!(text(class.doc_comment()), Some("/** cls */"));
    let members: Vec<_> = class
        .body()
        .unwrap()
        .members()
        .map(|m| text(m.doc_comment()))
        .collect();
    assert_eq!(
        members,
        [
            Some("/** ctor */"),
            Some("/** fields */"),
            Some("/** inner */"),
            Some("/** e */"),
            Some("/** r */"),
            Some("/** i */"),
            Some("/** at */"),
            None,
            None,
            None,
        ]
    );
}

#[test]
fn implicitly_declared_classes() {
    assert_eq!(
        docs("/** main */ void main() {}\n/** field */ int x;\n", 25),
        "MethodDecl main /** main */\nFieldDecl x /** field */\n"
    );
}

#[test]
fn module_info() {
    let text = "/** mod */ @Deprecated open module m { /** dir */ requires java.base; }";
    assert_eq!(docs(text, 25), "ModuleDecl m /** mod */\n");
    assert_eq!(
        placements(&parse_at(text, 25)),
        "/** mod */ Leading ModuleDecl \"@Deprecated open module m { /** dir */ requires java.base; }\"
/** dir */ Leading RequiresDirective \"requires java.base;\"
"
    );
}

#[test]
fn doc_comment_after_code_on_the_same_line() {
    let text = "class T {\n    int x; /** a */\n    void m() {}\n    int y; /** b */ int z;\n}\n";
    assert_eq!(
        docs(text, 25),
        "ClassDecl T -
FieldDecl x -
MethodDecl m /** a */
FieldDecl y -
FieldDecl z /** b */
"
    );
    // A doc comment never trails, so it leads what it documents.
    assert_eq!(
        placements(&parse_at(text, 25)),
        "/** a */ Leading MethodDecl \"void m() {}\"
/** b */ Leading FieldDecl \"int z;\"
"
    );
}

/// `body` as the statements of a method.
fn in_method(body: &str) -> SyntaxTree {
    parse_at(&format!("class T {{\n void m() {{\n{body}\n }}\n}}\n"), 25)
}

#[test]
fn statements_in_blocks() {
    let tree = in_method(
        "    // lead
    a(); // trail
    /* x */ b();
    c(); /* y */
    // end",
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
    let tree = in_method("    { /* nothing */ }");
    assert_eq!(
        placements(&tree),
        "/* nothing */ Dangling Block \"{ /* nothing */ }\"\n"
    );
}

#[test]
fn if_and_else() {
    let tree = in_method(
        "    if /* c */ (x) { // then
        a();
    } /* before */ else /* after */ {
        b();
    } // end
    if (x) {
    } // why
    else {
    }",
    );
    assert_eq!(
        placements(&tree),
        r#"/* c */ Leading "("
// then Trailing "{"
/* before */ Leading "else"
/* after */ Leading Block "{\n        b();\n    }"
// end Trailing IfStmt "if /* c */ (x) { // then\n        a();\n    } /* before */ else /* after */ {\n        b();\n    }"
// why Trailing Block "{\n    }"
"#
    );
}

#[test]
fn switch_rules_and_groups() {
    let tree = in_method(
        "    switch (x) {
        // rule
        case 1 -> a(); // after
        /* d */ default -> {}
    }
    switch (x) {
        case 1: // one
            a();
            // fall
        case 2:
            b();
        // last
    }",
    );
    assert_eq!(
        placements(&tree),
        r#"// rule Leading SwitchRule "case 1 -> a();"
// after Trailing SwitchRule "case 1 -> a();"
/* d */ Leading SwitchRule "default -> {}"
// one Trailing ":"
// fall Leading SwitchGroup "case 2:\n            b();"
// last Trailing SwitchGroup "case 2:\n            b();"
"#
    );
}

#[test]
fn arguments_and_commas() {
    let tree = in_method(
        "    f(/* none */);
    g(a /* c1 */, /* c2 */ b, // c3
      c /* c4 */);",
    );
    assert_eq!(
        placements(&tree),
        r#"/* none */ Dangling Arguments "(/* none */)"
/* c1 */ Leading ","
/* c2 */ Leading NameExpr "b"
// c3 Trailing ","
/* c4 */ Trailing NameExpr "c"
"#
    );
}

#[test]
fn binary_chains() {
    let tree = in_method(
        "    int v = a // first
        + b /* second */
        + /* third */ c;",
    );
    assert_eq!(
        placements(&tree),
        r#"// first Trailing NameExpr "a"
/* second */ Trailing BinaryExpr "a // first\n        + b"
/* third */ Leading NameExpr "c"
"#
    );
}
