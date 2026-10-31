//! The comment map and doc comment lookup on files parsed by `ferrion_parser`.
//!
//! Only package declarations and imports parse so far; everything after the imports is one
//! `Unparsed` node. Declarations, statements, and expressions get their tests as they parse.

use std::sync::Arc;

use ferrion_doc_comments::{Inline, markdown_comment_runs, parse_javadoc, parse_markdown};
use ferrion_parser::{ParseOptions, parse};
use ferrion_syntax::ast::{AstNode, CompilationUnit};
use ferrion_syntax::comments::{CommentMap, DocCommentKind, Placement};
use ferrion_syntax::{NodeOrToken, SyntaxTree};

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
