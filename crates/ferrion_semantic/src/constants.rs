//! Constant values (JLS 15.29, docs/design/semantic-attribution.md#constants): the arithmetic
//! of constant expressions, literals' values, and each constant field's value as a derived entry,
//! since a constant crosses files.
//!
//! A source field's value is computed from its initializer alone: the entry reads its file's
//! summary, whose fingerprint covers the initializer's tokens, and not the file's text, so an
//! edit elsewhere in the file, a body's included, leaves it valid. The initializer's names are
//! resolved as the file's body walk resolves them ([`crate::body`]) and its value folded by the
//! attribution walk ([`crate::attr`]), other constants read through [`Reads`]. A cycle of
//! constants (an error javac reports) reads as not constant. A class file's constant is its
//! field's `ConstantValue` attribute.

mod literal;
mod string;
#[cfg(test)]
mod tests;
mod value;

use std::sync::Arc;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{SyntaxKind, SyntaxNode, SyntaxTree};

pub use self::literal::literal_value;
pub use self::string::JavaString;
pub use self::value::{
    BinaryOp, Constant, UnaryOp, binary, double_hex, double_to_string, float_hex, float_to_string,
    unary,
};

use crate::attr::Walker;
use crate::body::initializer_names;
use crate::engine::derived::{Dep, Memo, Reads};
use crate::summary::{Header, Modifiers, Summary, TypeKind, UnitKind, name_text};
use crate::symbols::{FieldKind, FieldRef, TypeKey};
use crate::types::{Context, Tri, Type};
use crate::{Cancelled, Fingerprint, Interner, Name, Snapshot};

impl Snapshot {
    /// A field's constant value, or `None` if it isn't a constant variable (JLS 4.12.4). A local
    /// class's constants are its file's attribution's, not this.
    pub fn constant(&self, field: FieldRef) -> Result<Option<Constant>, Cancelled> {
        self.check()?;
        match field.owner {
            TypeKey::Class { .. } => Ok(self.class_constant(field)),
            TypeKey::Local { .. } => Ok(None),
            _ => Ok(self
                .constant_memo(field)?
                .and_then(|m| m.value.as_ref().clone())),
        }
    }

    fn class_constant(&self, field: FieldRef) -> Option<Constant> {
        self.class_entry(field.owner)?
            .fields
            .get(field.index as usize)?
            .constant
            .clone()
    }

    pub(crate) fn constant_memo(
        &self,
        field: FieldRef,
    ) -> Result<Option<Arc<Memo<Option<Constant>>>>, Cancelled> {
        self.shared().constants.get_or_cycle(self, &field, |reads| {
            let value = compute(reads, field)?;
            Ok(if reads.in_cycle() { None } else { value })
        })
    }
}

impl Reads<'_> {
    /// A field's constant value. Records it, for a source field.
    pub(crate) fn constant(&mut self, field: FieldRef) -> Result<Option<Constant>, Cancelled> {
        match field.owner {
            TypeKey::Class { .. } => Ok(self.snapshot.class_constant(field)),
            TypeKey::Local { .. } => Ok(None),
            _ => {
                let memo = self.snapshot.constant_memo(field)?;
                self.record(
                    Dep::Constant(field),
                    memo.as_ref().map_or(Fingerprint::CYCLE, |m| m.fingerprint),
                );
                Ok(memo.and_then(|m| m.value.as_ref().clone()))
            }
        }
    }
}

/// A source field's value: a `final` field (an interface's are) with an initializer whose
/// tokens the summary fingerprints, of a primitive type or `String`, whose initializer is a
/// constant expression, converted to the field's type.
fn compute(reads: &mut Reads<'_>, field: FieldRef) -> Result<Option<Constant>, Cancelled> {
    let entry = reads.type_entry(field.owner)?;
    let Some(f) = entry.fields.get(field.index as usize) else {
        return Ok(None);
    };
    let interface = matches!(entry.kind, TypeKind::Interface | TypeKind::Annotation);
    if f.kind != FieldKind::Field || !(f.modifiers.contains(Modifiers::FINAL) || interface) {
        return Ok(None);
    }
    let Some(located) = reads.locate(field.owner)? else {
        return Ok(None);
    };
    let has_initializer = located.summary.types[located.index]
        .fields
        .get(field.index as usize)
        .is_some_and(|f| f.constant.is_some());
    if !has_initializer {
        return Ok(None);
    }
    // The text isn't recorded: the summary is, and it has the initializer's fingerprint.
    let tree = reads.snapshot.tree(located.file)?;
    let names = reads.snapshot.names().clone();
    let found = initializer_node(&tree, &located.summary, located.index, field.index, &names);
    let Some(initializer) = found else {
        return Ok(None);
    };
    if !constant_shaped(initializer) {
        return Ok(None);
    }
    let names = initializer_names(reads, located.file, &tree, initializer.id(), located.index)?;
    let view = reads.snapshot.file(located.file)?.set;
    let mut walker = Walker::for_initializer(reads, view, names, tree.release());
    let Some((ty, Some(value))) = walker.initializer(field.owner, initializer)? else {
        return Ok(None);
    };
    let cx = walker.context();
    let target = cx.field_type(field.owner, field.index)?;
    let constant = match cx.get(target) {
        Type::Primitive(p) => match cx.convert(Context::Assignment, ty, target, Some(&value))? {
            Tri::Yes(_) => value.convert(p),
            _ => None,
        },
        Type::Class { key, .. } if cx.lang("java/lang/String")? == Some(key) => {
            matches!(value, Constant::String(_)).then_some(value)
        }
        _ => None,
    };
    let more = cx.take_reads();
    drop(walker);
    reads.extend(more);
    Ok(constant)
}

/// Whether an initializer is made only of what a constant expression can be (JLS 15.29), so
/// that one that can't be isn't resolved or attributed at all.
fn constant_shaped(node: SyntaxNode<'_>) -> bool {
    node.descendants().all(|n| {
        matches!(
            n.kind(),
            SyntaxKind::LiteralExpr
                | SyntaxKind::NameExpr
                | SyntaxKind::FieldAccessExpr
                | SyntaxKind::ParenExpr
                | SyntaxKind::BinaryExpr
                | SyntaxKind::ConditionalExpr
                | SyntaxKind::CastExpr
                | SyntaxKind::PrimitiveType
                | SyntaxKind::ClassType
                | SyntaxKind::Annotation
                | SyntaxKind::Name
        ) || (n.kind() == SyntaxKind::PrefixExpr
            && !ast::PrefixExpr::cast(n)
                .unwrap()
                .op()
                .is_some_and(|t| matches!(t.kind(), SyntaxKind::PlusPlus | SyntaxKind::MinusMinus)))
    })
}

fn is_type_decl(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ClassDecl
            | SyntaxKind::InterfaceDecl
            | SyntaxKind::EnumDecl
            | SyntaxKind::RecordDecl
            | SyntaxKind::AnnotationTypeDecl
    )
}

/// The initializer of the `field`th field of the summary's type at `index`: the type's node
/// found by its path of member types from the top level, each the how-manieth member type of
/// its outer type the summary says, then its declarators in order.
fn initializer_node<'t>(
    tree: &'t SyntaxTree,
    summary: &Summary,
    index: usize,
    field: u32,
    names: &Interner,
) -> Option<SyntaxNode<'t>> {
    let root = tree.root();
    let compact = summary.kind == UnitKind::Compact;
    let mut path = Vec::new();
    let mut next = Some(index);
    while let Some(i) = next {
        path.push(i);
        next = summary.types[i].outer.map(|o| o as usize);
    }
    path.reverse();
    let mut node = root;
    let mut outer: Option<usize> = None;
    for &i in &path {
        if compact && i == 0 {
            outer = Some(0);
            continue;
        }
        // The member types declared before it beside it, and so the position among them.
        let position = (0..i)
            .filter(|&j| {
                summary.types[j].outer.map(|o| o as usize) == outer
                    && summary.types[j].kind != TypeKind::Implicit
            })
            .count();
        let body = if node == root {
            root
        } else {
            node.children()
                .find(|c| matches!(c.kind(), SyntaxKind::ClassBody | SyntaxKind::EnumBody))?
        };
        let found = body
            .children()
            .filter(|c| is_type_decl(c.kind()))
            .nth(position)?;
        let header = Header::of(ast::TypeDecl::cast(found).unwrap());
        let name = header.name.map_or(Name::EMPTY, |n| name_text(n, names));
        if name != summary.types[i].name {
            return None;
        }
        node = found;
        outer = Some(i);
    }
    let body = if node == root {
        root
    } else {
        node.children()
            .find(|c| matches!(c.kind(), SyntaxKind::ClassBody | SyntaxKind::EnumBody))?
    };
    let declarator = body
        .children()
        .filter(|c| c.kind() == SyntaxKind::FieldDecl)
        .flat_map(|d| ast::FieldDecl::cast(d).unwrap().declarators())
        .nth(field as usize)?;
    match declarator.initializer()? {
        ast::VariableInitializer::Expr(e) => Some(e.syntax()),
        ast::VariableInitializer::ArrayInitializer(_) => None,
    }
}
