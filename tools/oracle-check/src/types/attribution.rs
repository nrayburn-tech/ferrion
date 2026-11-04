//! Ferrion's side of the comparison of a file's entries (B3b,
//! docs/design/types-oracle-normalization.md#ferrions-side-of-the-files-entries): each file's
//! attribution in the oracle's forms, with what javac's trees have and Ferrion's don't made up,
//! and the other way around.
//!
//! - **Parentheses of statements:** javac's `if`, `while`, `do`, `switch`, and `synchronized` have
//!   a `PARENTHESIZED` tree around their condition, which Ferrion's tree has no node for.
//! - **Folded strings:** javac's parser joins a run of adjacent string literals in a chain of `+`
//!   into one `STRING_LITERAL` ([`folded_operands`]), so the literals and the `+`s inside a run
//!   are no trees of javac's, and the run is one. Only constants javac reports are printed: every
//!   prefix of a run is a constant Ferrion folded, millions of units long in lucene's tables.
//! - **Negative literals:** javac's parser reads `-` before a decimal integer literal as part of
//!   it ([`negative_literal`]), so the literal alone is no tree of javac's.
//! - **An enum constant's creation:** javac's `new` for a constant with arguments or a body is a
//!   `NEW_CLASS` from its arguments, or its body if it has none, through its body, or through its
//!   arguments if it has none; of the anonymous class's type if it has a body, else the enum's.
//! - **An array creation's initializer** is part of its `NEW_ARRAY`, not a tree of its own.
//! - **A resource that is a variable** has a range through its `;`, if one follows.
//! - **Variables:** a record component is a variable only as a compact constructor's parameter;
//!   javac's parameters of implicit constructors, an anonymous class's and a record's, have no
//!   counterpart, and their offsets are listed so that they are counted apart.
//! - **Annotations on record components:** javac keeps one in its trees only if it applies to a
//!   field, by its annotation type's `@Target`, so entries inside others are counted apart when
//!   javac has none.
//! - **Calls:** a creation of an anonymous class is two calls in javac's tree: the anonymous class's
//!   own constructor, and the implicit `super(...)` in it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ferrion_diagnostics::{SourceText, Span};
use ferrion_semantic::attr::{BODY_FUEL, BodyTypes};
use ferrion_semantic::symbols::TypeKey;
use ferrion_semantic::types::{Printed, TypeContext, TypeId};
use ferrion_semantic::{Cancelled, FileId, Snapshot, SourceSetId};
use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeId, SyntaxKind as K, SyntaxNode, WalkEvent};

use super::compare::{FileTypes, Typed, TypedCall, TypedVariable};
use crate::ferrion::tree::{
    first_expr, folded_groups, folded_operands, name, negative_literal, token,
};
use crate::names::ferrion::Offsets;

/// What Ferrion gives for a variable's finality until B3d.
const FINALITY: &str = "left-to-b3d finality";

/// Every file of `view`, attributed and in the oracle's forms, by its path relative to `corpus`.
pub fn set_files(
    snapshot: &Snapshot,
    view: SourceSetId,
    corpus: &Path,
    threads: usize,
) -> Result<HashMap<String, FileTypes>, Cancelled> {
    snapshot.run_symbols(threads)?;
    let files = snapshot.files(view)?;
    let out = Mutex::new(HashMap::with_capacity(files.len()));
    snapshot.run_names(threads, &files, |file| {
        let (path, types) = file_types(snapshot, view, file, corpus)?;
        out.lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path, types);
        Ok(())
    })?;
    Ok(out.into_inner().unwrap_or_else(|e| e.into_inner()))
}

/// What javac's trees have that Ferrion's don't, and the other way around, in one file.
#[derive(Default)]
struct Shape<'t> {
    /// Nodes that are no trees of javac's.
    dropped: HashSet<NodeId>,
    /// Each folded run: its range and its literals.
    runs: Vec<(Span, Vec<SyntaxNode<'t>>)>,
    /// Each statement's parentheses: their range and the expression inside.
    parentheses: Vec<(Span, NodeId)>,
    /// Each enum constant with arguments or a body: the range of its `NEW_CLASS`, and the constant.
    enum_creations: Vec<(Span, NodeId)>,
    /// Where javac's parameters of implicit constructors are.
    implicit_parameters: Vec<u32>,
    /// The annotations on record components.
    component_annotations: Vec<Span>,
}

impl<'t> Shape<'t> {
    fn of(root: SyntaxNode<'t>) -> Shape<'t> {
        let mut shape = Shape::default();
        for event in root.preorder() {
            let WalkEvent::Enter(node) = event else {
                continue;
            };
            match node.kind() {
                K::BinaryExpr => shape.binary(node),
                K::PrefixExpr => {
                    if let Some(literal) = negative_literal(node) {
                        shape.dropped.insert(literal.id());
                    }
                }
                K::IfStmt
                | K::WhileStmt
                | K::DoStmt
                | K::SwitchStmt
                | K::SwitchExpr
                | K::SynchronizedStmt => shape.parenthesized(node),
                K::EnumConstant => shape.enum_constant(node),
                K::ArrayInitializer => {
                    if node
                        .parent()
                        .is_some_and(|p| p.kind() == K::ArrayCreationExpr)
                    {
                        shape.dropped.insert(node.id());
                    }
                }
                K::Annotation => {
                    let mut around = node.ancestors().skip(1).take(2);
                    if around.any(|a| a.kind() == K::RecordComponent) {
                        shape.component_annotations.push(node.span());
                    }
                }
                K::RecordDecl => {
                    let keyword = node.direct_tokens().find(|t| !t.is_trivia());
                    shape
                        .implicit_parameters
                        .extend(keyword.map(|t| t.span().start));
                }
                _ => {}
            }
        }
        shape
    }

    /// A chain of `+` javac folds in: the literals of each run go, and the `+`s before each run's
    /// last. A `+` that ends the first group is the first group, and goes too.
    fn binary(&mut self, node: SyntaxNode<'t>) {
        let Some(operands) = folded_operands(node) else {
            return;
        };
        let groups = folded_groups(&operands);
        // The `+` whose right operand is each operand: the outermost has the last.
        let mut plus = Vec::with_capacity(operands.len());
        let mut at = node;
        while plus.len() + 1 < operands.len() {
            plus.push(at);
            match first_expr(at) {
                Some(left) => at = left,
                None => break,
            }
        }
        plus.reverse();
        let mut index = 0;
        for (g, group) in groups.into_iter().enumerate() {
            let first = index;
            index += group.len();
            // The operator of operand `i` is `plus[i - 1]`.
            for i in first..index {
                let last_of_group = i + 1 == index;
                if i > 0 && (!last_of_group || g == 0) {
                    self.dropped.insert(plus[i - 1].id());
                }
            }
            if group.len() > 1 {
                for literal in &group {
                    self.dropped.insert(literal.id());
                }
                let span = Span {
                    start: group[0].span().start,
                    end: group[group.len() - 1].span().end,
                };
                self.runs.push((span, group));
            }
        }
    }

    fn parenthesized(&mut self, statement: SyntaxNode<'t>) {
        let (Some(open), Some(close), Some(inside)) = (
            token(statement, K::LParen),
            token(statement, K::RParen),
            first_expr(statement),
        ) else {
            return;
        };
        let span = Span {
            start: open.span().start,
            end: close.span().end,
        };
        self.parentheses.push((span, inside.id()));
    }

    fn enum_constant(&mut self, constant: SyntaxNode<'t>) {
        if let Some(span) = enum_creation(constant) {
            self.enum_creations.push((span, constant.id()));
        }
    }
}

/// The range of javac's `NEW_CLASS` for an enum constant, if it has arguments or a body; one with
/// neither is made up.
fn enum_creation(constant: SyntaxNode<'_>) -> Option<Span> {
    let body = constant.children().find(|c| c.kind() == K::ClassBody);
    let arguments = constant.children().find(|c| c.kind() == K::Arguments);
    Some(Span {
        start: arguments.or(body)?.span().start,
        end: body.or(arguments)?.span().end,
    })
}

/// A type declaration's binary name: its package, then its enclosing types'.
fn binary_name(declaration: SyntaxNode<'_>) -> String {
    let name = |node: SyntaxNode<'_>| name(node).map_or_else(String::new, |t| t.text().to_owned());
    let mut parts = vec![name(declaration)];
    let mut package = String::new();
    for ancestor in declaration.ancestors() {
        match ancestor.kind() {
            K::ClassDecl
            | K::InterfaceDecl
            | K::EnumDecl
            | K::RecordDecl
            | K::AnnotationTypeDecl
                if ancestor.id() != declaration.id() =>
            {
                parts.push(name(ancestor));
            }
            K::CompilationUnit => {
                let decl = ast::CompilationUnit::cast(ancestor).and_then(|u| u.package_decl());
                if let Some(written) = decl.and_then(|d| d.name()) {
                    package = written.syntax().text().split_whitespace().collect();
                    package = package.replace('.', "/") + "/";
                }
            }
            _ => {}
        }
    }
    parts.reverse();
    package + &parts.join("$")
}

/// One file's attribution in the oracle's forms, with its path relative to `corpus`.
fn file_types(
    snapshot: &Snapshot,
    view: SourceSetId,
    file: FileId,
    corpus: &Path,
) -> Result<(String, FileTypes), Cancelled> {
    let info = snapshot.file(file)?;
    let relative = info
        .path
        .strip_prefix(corpus)
        .unwrap_or(&info.path)
        .to_string_lossy()
        .replace('\\', "/");
    let tree = snapshot.tree(file)?;
    let mut types = snapshot.attribute_with_fuel(file, BODY_FUEL)?;
    let body = snapshot.body_names(file)?;
    let mut shape = Shape::of(tree.root());
    let arena = std::mem::take(&mut types.arena);
    let mut writer = Writer {
        cx: snapshot.types_in(view, arena),
        locals: Localizer::new(snapshot, view, corpus),
        strings: HashMap::new(),
    };
    let mut out = FileTypes::default();
    out.expressions.reserve(types.types.len());

    // Each folded run is a string literal, with its literals' value.
    for (span, literals) in std::mem::take(&mut shape.runs) {
        let mut value = String::from("String \"");
        for literal in &literals {
            if let Some(constant) = types.constant(literal.id()) {
                let text = constant.oracle_text();
                let inner = text
                    .strip_prefix("String \"")
                    .and_then(|t| t.strip_suffix('"'));
                value.push_str(inner.unwrap_or(""));
            }
        }
        value.push('"');
        out.constants.push(Typed { span, value });
        if let Some(t) = types.type_of(literals[0].id()) {
            let value = writer.text(t)?;
            out.expressions.push(Typed { span, value });
        }
    }

    for &(node, t) in types.types.iter() {
        if shape.dropped.contains(&node) {
            continue;
        }
        let syntax = tree.node(node);
        let span = expression_range(syntax);
        out.expressions.push(Typed {
            span,
            value: writer.text(t)?,
        });
    }
    for (node, constant) in types.constants.iter() {
        if shape.dropped.contains(node) {
            continue;
        }
        out.constants.push(Typed {
            span: expression_range(tree.node(*node)),
            value: constant.oracle_text(),
        });
    }

    for &(span, inside) in &shape.parentheses {
        if let Some(t) = types.type_of(inside) {
            out.expressions.push(Typed {
                span,
                value: writer.text(t)?,
            });
        }
        if let Some(constant) = types.constant(inside) {
            out.constants.push(Typed {
                span,
                value: constant.oracle_text(),
            });
        }
    }

    for &(span, constant) in &shape.enum_creations {
        let local = |node: NodeId| {
            let index = body.local_types.iter().position(|t| t.node == node)?;
            Some(TypeKey::Local {
                file,
                index: index as u32,
            })
        };
        let constant = tree.node(constant);
        let declaration = constant
            .children()
            .find(|c| c.kind() == K::ClassBody)
            .or_else(|| constant.ancestors().find(|a| a.kind() == K::EnumDecl));
        let Some(declaration) = declaration else {
            continue;
        };
        let value = match local(declaration.id()) {
            Some(key) => {
                let t = writer.cx.class_type(key, None, None);
                writer.text(t)?
            }
            None => binary_name(declaration),
        };
        out.expressions.push(Typed { span, value });
    }

    for local in body.local_types.iter() {
        if tree.node(local.node).kind() == K::ClassBody {
            shape.implicit_parameters.push(local.at);
        }
    }
    out.implicit_parameters = shape.implicit_parameters;
    out.component_annotations = shape.component_annotations;

    calls(snapshot, file, &tree, &types, &mut writer, &mut out)?;
    variables(&tree, &types, &mut writer, &mut out)?;
    Ok((relative, out))
}

/// The range of javac's tree for an expression: a resource that is a variable ends after its
/// `;`, if one follows.
fn expression_range(node: SyntaxNode<'_>) -> Span {
    let span = node.span();
    if node
        .parent()
        .is_some_and(|p| p.kind() == K::ResourceSpecification)
        && let Some(semicolon) = node
            .last_token()
            .next_non_trivia()
            .filter(|t| t.kind() == K::Semi)
    {
        return Span {
            start: span.start,
            end: semicolon.span().end,
        };
    }
    span
}

/// Each call's chosen method, and for those without one what they wait for.
fn calls(
    snapshot: &Snapshot,
    file: FileId,
    tree: &ferrion_syntax::SyntaxTree,
    types: &BodyTypes,
    writer: &mut Writer<'_>,
    out: &mut FileTypes,
) -> Result<(), Cancelled> {
    for (node, call) in types.calls.iter() {
        let syntax = tree.node(*node);
        let text = snapshot.call_text(file, &mut writer.cx, types, *node, call)?;
        let method = writer.locals.oracle_form(&text.method)?;
        let invoked = writer.locals.oracle_form(&text.invoked)?;
        let (span, implicit) = call_position(syntax);
        match text.anonymous {
            Some(own) => {
                out.calls.push(TypedCall {
                    span,
                    implicit,
                    method: writer.locals.oracle_form(&own)?,
                    ty: None,
                });
                out.calls.push(TypedCall {
                    span: anonymous_super(syntax),
                    implicit: true,
                    method,
                    ty: Some(invoked),
                });
            }
            None => out.calls.push(TypedCall {
                span,
                implicit,
                method,
                ty: Some(invoked),
            }),
        }
    }
    for (node, why) in types.unchosen.iter() {
        let syntax = tree.node(*node);
        let label = writer.text(*why)?;
        let (span, implicit) = call_position(syntax);
        out.calls.push(TypedCall {
            span,
            implicit,
            method: label.clone(),
            ty: None,
        });
        if has_class_body(syntax) {
            out.calls.push(TypedCall {
                span: anonymous_super(syntax),
                implicit: true,
                method: label,
                ty: None,
            });
        }
    }
    // A method invocation with no method: what its type waits for says what it waits for.
    for event in tree.root().preorder() {
        let WalkEvent::Enter(node) = event else {
            continue;
        };
        if !matches!(node.kind(), K::MethodCallExpr | K::MethodRefExpr)
            || types.call(node.id()).is_some()
        {
            continue;
        }
        let Some(t) = types.type_of(node.id()) else {
            continue;
        };
        let text = writer.text(t)?;
        if text.starts_with("left-to-") {
            out.calls.push(TypedCall {
                span: node.span(),
                implicit: false,
                method: text,
                ty: None,
            });
        }
    }
    Ok(())
}

fn has_class_body(node: SyntaxNode<'_>) -> bool {
    matches!(node.kind(), K::NewExpr | K::EnumConstant)
        && node.children().any(|c| c.kind() == K::ClassBody)
}

/// Where javac has the implicit `super(...)` of an anonymous class: at its body's `{`, or an enum
/// constant's name.
fn anonymous_super(node: SyntaxNode<'_>) -> Span {
    let start = if node.kind() == K::EnumConstant {
        let name = node
            .direct_tokens()
            .find(|t| matches!(t.kind(), K::Identifier | K::Underscore));
        name.map_or(node.span().start, |t| t.span().start)
    } else {
        let body = node.children().find(|c| c.kind() == K::ClassBody);
        body.map_or(node.span().start, |b| b.span().start)
    };
    Span {
        start,
        end: start + 1,
    }
}

/// Where javac has a call Ferrion records at `node`, and whether javac made it up: a method
/// invocation, an explicit constructor call, or a creation is at its range, and an enum constant's
/// creation at [`enum_creation`]'s, or if it has neither arguments nor a body, made up at its name;
/// javac's implicit `super()` is at a constructor's `{`, or a class's keyword if it has no
/// constructor.
fn call_position(node: SyntaxNode<'_>) -> (Span, bool) {
    let at = |start: u32| {
        (
            Span {
                start,
                end: start + 1,
            },
            true,
        )
    };
    match node.kind() {
        K::Block => at(node.span().start),
        K::ClassDecl | K::EnumDecl | K::RecordDecl => {
            let keyword = node.direct_tokens().find(|t| !t.is_trivia());
            at(keyword.map_or(node.span().start, |t| t.span().start))
        }
        K::EnumConstant => match enum_creation(node) {
            Some(span) => (span, false),
            None => at(anonymous_super(node).start),
        },
        _ => (node.span(), false),
    }
}

/// Each local variable and parameter: by the offset its `local` key names, with its type, and
/// finality left to B3d.
fn variables(
    tree: &ferrion_syntax::SyntaxTree,
    types: &BodyTypes,
    writer: &mut Writer<'_>,
    out: &mut FileTypes,
) -> Result<(), Cancelled> {
    for &(node, t) in types.variables.iter() {
        let syntax = tree.node(node);
        let offset = if syntax.kind() == K::RecordComponent {
            let component = ast::RecordComponent::cast(syntax);
            match component.filter(|_| in_compact_record(syntax)) {
                Some(component) => ferrion_semantic::body::component_type_at(component),
                None => continue,
            }
        } else {
            let name = syntax
                .direct_tokens()
                .filter(|t| matches!(t.kind(), K::Identifier | K::Underscore))
                .last();
            match name {
                Some(name) => name.span().start,
                None => continue,
            }
        };
        out.variables.push(TypedVariable {
            offset,
            finality: FINALITY.to_owned(),
            ty: writer.text(t)?,
        });
    }
    Ok(())
}

/// Whether a record component's record has a compact constructor, whose parameter it is.
fn in_compact_record(component: SyntaxNode<'_>) -> bool {
    let record = component
        .ancestors()
        .find(|node| node.kind() == K::RecordDecl);
    let body = record.and_then(|r| r.children().find(|c| c.kind() == K::ClassBody));
    body.is_some_and(|b| b.children().any(|m| m.kind() == K::CompactConstructorDecl))
}

/// Writes one file's types as the oracle does.
struct Writer<'s> {
    cx: TypeContext<'s>,
    locals: Localizer<'s>,
    /// Types repeat within a file.
    strings: HashMap<TypeId, String>,
}

impl Writer<'_> {
    /// A type as the oracle writes it, or the label of what it waits for.
    fn text(&mut self, t: TypeId) -> Result<String, Cancelled> {
        if let Some(text) = self.strings.get(&t) {
            return Ok(text.clone());
        }
        let text = self.cx.type_string(t, Printed::Captures)?;
        let text = if text.contains("left-to-b3c") {
            "left-to-b3c inference".to_owned()
        } else if text.contains("left-to-b3d") {
            "left-to-b3d".to_owned()
        } else if text.contains("left-to-b4") {
            "left-to-b4 lombok".to_owned()
        } else {
            self.locals.oracle_form(&text)?
        };
        self.strings.insert(t, text.clone());
        Ok(text)
    }
}

/// Converts Ferrion's text to javac's where it names a local class: each `local <path>:<byte
/// offset>` with the path relative to the corpus, and the offset in UTF-16 code units.
pub struct Localizer<'s> {
    snapshot: &'s Snapshot,
    view: SourceSetId,
    corpus: &'s Path,
    prefix: String,
    /// The view's files by path, made when a file's offsets are first asked for.
    files: Option<HashMap<PathBuf, FileId>>,
    offsets: HashMap<PathBuf, Offsets>,
}

impl<'s> Localizer<'s> {
    pub fn new(snapshot: &'s Snapshot, view: SourceSetId, corpus: &'s Path) -> Localizer<'s> {
        Localizer {
            snapshot,
            view,
            corpus,
            prefix: format!("local {}", corpus.join("").display()),
            files: None,
            offsets: HashMap::new(),
        }
    }

    pub fn oracle_form(&mut self, text: &str) -> Result<String, Cancelled> {
        if !text.contains(&self.prefix) {
            return Ok(text.to_owned());
        }
        let prefix = self.prefix.clone();
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find(&prefix) {
            out.push_str(&rest[..at]);
            out.push_str("local ");
            rest = &rest[at + prefix.len()..];
            let colon = rest.find(':').unwrap_or(rest.len());
            let relative = &rest[..colon];
            out.push_str(relative);
            rest = &rest[colon..];
            if let Some(after) = rest.strip_prefix(':') {
                let digits = after.bytes().take_while(u8::is_ascii_digit).count();
                let byte: u32 = after[..digits].parse().unwrap_or(0);
                let path = self.corpus.join(relative);
                let offsets = self.offsets_of(&path)?;
                out.push(':');
                out.push_str(&offsets.utf16(byte).to_string());
                rest = &after[digits..];
            }
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The offsets of a file of the view, or for a file it hasn't, on disk.
    pub fn offsets_of(&mut self, path: &Path) -> Result<&Offsets, Cancelled> {
        if !self.offsets.contains_key(path) {
            let file = self.file(path)?;
            let offsets = match file {
                Some(file) => {
                    let text = self.snapshot.text(file)?;
                    let bom = std::fs::read(path)
                        .is_ok_and(|bytes| bytes.starts_with(&[0xef, 0xbb, 0xbf]));
                    Offsets::from_text(&text, bom)
                }
                None => match std::fs::read(path)
                    .ok()
                    .and_then(|b| SourceText::decode(b).ok())
                {
                    Some(source) => Offsets::new(&source),
                    None => Offsets::from_text("", false),
                },
            };
            self.offsets.insert(path.to_path_buf(), offsets);
        }
        Ok(&self.offsets[path])
    }

    /// A file of the view by its path.
    pub fn file(&mut self, path: &Path) -> Result<Option<FileId>, Cancelled> {
        if self.files.is_none() {
            let mut files = HashMap::new();
            for &file in self.snapshot.files(self.view)?.iter() {
                files.insert(self.snapshot.file(file)?.path.to_path_buf(), file);
            }
            self.files = Some(files);
        }
        Ok(self.files.as_ref().and_then(|f| f.get(path).copied()))
    }
}

#[cfg(test)]
mod tests {
    use ferrion_semantic::Engine;
    use ferrion_semantic::testing::relations_input;

    use super::*;
    use crate::types::compare::{Compared, compare};
    use crate::types::oracle::FileOutput;

    /// A case of `types-cases`: its source as `Q.java`, with the classes the JDK's stand-ins lack,
    /// compared with javac's output on it.
    fn compared(source: &str, oracle: &str) -> Compared {
        let files =
            format!("//- class interface java/lang/AutoCloseable\n//- file main Q.java\n{source}");
        let engine = Engine::new(relations_input(&files));
        let snapshot = engine.snapshot();
        let view = SourceSetId::from_index(0);
        let file = snapshot.files(view).unwrap()[0];
        let path = snapshot.file(file).unwrap().path;
        let corpus = path.parent().unwrap();
        snapshot.run_symbols(1).unwrap();
        let (relative, ours) = file_types(&snapshot, view, file, corpus).unwrap();
        assert_eq!(relative, "Q.java");
        let javac: FileOutput = serde_json::from_str(oracle).unwrap();
        compare(&javac, &ours, &Offsets::from_text(source, false))
    }

    #[test]
    fn javacs_trees_and_ferrions() {
        let compared = compared(
            include_str!("../../types-cases/shapes.java"),
            include_str!("../../types-cases/shapes.jsonl"),
        );
        let shown: Vec<_> = compared
            .mismatches
            .iter()
            .map(|m| {
                format!(
                    "{} {}-{} {}: javac {}, Ferrion {:?}",
                    m.aspect.name(),
                    m.start,
                    m.end,
                    m.kind,
                    m.javac,
                    m.ferrion
                )
            })
            .collect();
        assert!(shown.is_empty(), "{}", shown.join("\n"));
        assert_eq!(compared.extras, []);
        // The two parameters of `Implicit`'s constructor, and the two of the `new Base`'s.
        assert_eq!(compared.implicit_parameters, 5);
        assert!(compared.matched > 0 && compared.left > 0);
    }
}
