//! The attribution walk: the tree's pre-order, a frame per node entered, and a stack of values
//! that each expression pushes as it is left and its parent consumes.

mod calls;

use std::rc::Rc;
use std::sync::Arc;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeId, SyntaxKind, SyntaxNode, WalkEvent};

use super::{BodyTypes, Call};
use crate::body::{BodyNames, Map};
use crate::constants::{self, BinaryOp, Constant, UnaryOp};
use crate::decl_names::{FileNames, ResolvedName, Symbol};
use crate::engine::derived::Reads;
use crate::members::{self, FieldMember, Members};
use crate::resolve::FileScope;
use crate::summary::{Modifiers, Primitive, TypeKind, WildcardBound, modifiers};
use crate::symbols::{Cause, ErrorSymbol, FieldRef, TypeKey};
use crate::types::{Context, Conversion, Steps, Tri, Type, TypeArg, TypeContext, TypeId, Why};
use crate::{Cancelled, FileId, Interner, Name, Snapshot, SourceSetId};

/// What a value on the stack is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    /// An expression's value.
    Expr,
    /// A name of a type (a qualifier, `A` in `A.f`): its class.
    Type(Option<TypeKey>),
    /// A name of a package.
    Package,
}

/// A child's value, as its parent reads it.
#[derive(Clone, Debug)]
struct Value {
    node: NodeId,
    ty: TypeId,
    constant: Option<Constant>,
    what: What,
    /// For a name or field access of a variable, the variable's type before capture: what an
    /// assignment or increment assigns to.
    variable: Option<TypeId>,
    /// What the expression's type is recorded as, where that isn't `ty`: a multi-catch
    /// parameter's union, whose `lub` `ty` is.
    shown: Option<TypeId>,
    /// For a reference conditional in an assignment or invocation context, whose own type is B3c's
    /// (`ty` says so): its operands, which a call's applicability tests (JLS 15.12.2.2).
    operands: Option<Rc<[Value]>>,
    /// How many conditionals deep `operands` go.
    operand_depth: u8,
}

/// How deep a call looks into conditionals among its arguments; a deeper one is B3c's.
const MAX_OPERAND_DEPTH: u8 = 64;

/// A node entered and not yet left.
struct Frame {
    node: NodeId,
    /// Where its children's values start on the stack.
    mark: usize,
    /// The type an array initializer's elements are converted to: its own type.
    expected: Option<TypeId>,
}

/// A local variable or parameter: its type, its value if it is a constant variable, and what
/// its type is recorded as, where that isn't `ty`: a multi-catch parameter's union.
#[derive(Clone)]
struct Local {
    ty: TypeId,
    constant: Option<Constant>,
    shown: Option<TypeId>,
}

/// A class whose body the walk is in.
struct Class {
    /// The node whose leaving closes it.
    node: NodeId,
    key: TypeKey,
}

/// The walk over one file, or over one field initializer for its constant.
pub(crate) struct Walker<'s, 'r> {
    cx: TypeContext<'s>,
    reads: &'r mut Reads<'s>,
    snapshot: &'s Snapshot,
    view: SourceSetId,
    names: Arc<Interner>,
    /// The names by node: the body walk's, then the declarations'; for one initializer, its
    /// own.
    body_names: Option<Arc<BodyNames>>,
    decl_names: Option<Arc<FileNames>>,
    initializer_names: Box<[ResolvedName]>,
    fs: Option<Arc<FileScope>>,
    local_of_node: Map<NodeId, TypeKey>,
    release: u32,
    fuel: u64,
    out: BodyTypes,
    types: Vec<(NodeId, TypeId)>,
    constants: Vec<(NodeId, Constant)>,
    conversions: Vec<(NodeId, Steps)>,
    compound: Vec<(NodeId, Steps)>,
    resolved: Vec<(NodeId, Symbol)>,
    calls: Vec<(NodeId, Call)>,
    unchosen: Vec<(NodeId, TypeId)>,
    body_fuel: Vec<(NodeId, u64)>,
    frames: Vec<Frame>,
    values: Vec<Value>,
    locals: Map<NodeId, Local>,
    classes: Vec<Class>,
    /// The summary's types the walk is in, with their indexes, and the next index.
    summary_types: Vec<(NodeId, usize)>,
    next_index: usize,
    /// The body being walked, at the top level.
    body: Option<NodeId>,
    /// Each open switch block's selector type.
    selectors: Vec<(NodeId, TypeId)>,
    /// Each method's result type while its body is walked; `None` in a lambda.
    results: Vec<(NodeId, Option<TypeId>)>,
    /// A `var` declared by an enhanced `for`, typed when its expression is left.
    for_each: Map<NodeId, NodeId>,
    local_tables: Map<TypeKey, Option<Arc<Members>>>,
    /// Anonymous classes created with `<>`, whose supertype inference gives (B3c).
    inferred_supertypes: crate::body::Set<TypeKey>,
    /// Anonymous classes made through an outer instance, with their superclasses.
    outer_superclasses: Map<TypeKey, TypeKey>,
    /// Constant fields of local classes, as their initializers are left.
    local_constants: Map<FieldRef, Constant>,
    nodes: usize,
    string: Option<TypeId>,
}

fn id_of<'t, N: AstNode<'t>>(node: Option<N>) -> Option<NodeId> {
    node.map(|n| n.syntax().id())
}

fn child(children: &[Value], node: Option<NodeId>) -> Option<&Value> {
    let node = node?;
    children.iter().find(|v| v.node == node)
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

fn is_method_like(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::MethodDecl | SyntaxKind::ConstructorDecl | SyntaxKind::CompactConstructorDecl
    )
}

/// Whether a node starts a body, as B2's walk starts its regions.
fn starts_body(node: SyntaxNode<'_>, parent: SyntaxNode<'_>) -> bool {
    match node.kind() {
        SyntaxKind::Block => {
            is_method_like(parent.kind()) || parent.kind() == SyntaxKind::Initializer
        }
        SyntaxKind::DefaultValue | SyntaxKind::AnnotationArguments => true,
        SyntaxKind::Arguments | SyntaxKind::ClassBody => parent.kind() == SyntaxKind::EnumConstant,
        kind => parent.kind() == SyntaxKind::VariableDeclarator && kind != SyntaxKind::Dim,
    }
}

/// A binary operator by its tokens: `>>` and `>>>` are two and three `>` tokens.
fn binary_op(node: SyntaxNode<'_>) -> Option<BinaryOp> {
    let tokens: Vec<SyntaxKind> = node
        .direct_tokens()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind())
        .collect();
    Some(match tokens.as_slice() {
        [SyntaxKind::Gt, SyntaxKind::Gt, SyntaxKind::Gt] => BinaryOp::UShr,
        [SyntaxKind::Gt, SyntaxKind::Gt] => BinaryOp::Shr,
        [kind] => simple_op(*kind)?,
        _ => return None,
    })
}

fn simple_op(kind: SyntaxKind) -> Option<BinaryOp> {
    Some(match kind {
        SyntaxKind::Star | SyntaxKind::StarEq => BinaryOp::Mul,
        SyntaxKind::Slash | SyntaxKind::SlashEq => BinaryOp::Div,
        SyntaxKind::Percent | SyntaxKind::PercentEq => BinaryOp::Rem,
        SyntaxKind::Plus | SyntaxKind::PlusEq => BinaryOp::Add,
        SyntaxKind::Minus | SyntaxKind::MinusEq => BinaryOp::Sub,
        SyntaxKind::LtLt | SyntaxKind::LtLtEq => BinaryOp::Shl,
        SyntaxKind::Lt => BinaryOp::Lt,
        SyntaxKind::Gt => BinaryOp::Gt,
        SyntaxKind::LtEq => BinaryOp::Le,
        SyntaxKind::GtEq => BinaryOp::Ge,
        SyntaxKind::EqEq => BinaryOp::Eq,
        SyntaxKind::BangEq => BinaryOp::Ne,
        SyntaxKind::Amp | SyntaxKind::AmpEq => BinaryOp::And,
        SyntaxKind::Caret | SyntaxKind::CaretEq => BinaryOp::Xor,
        SyntaxKind::Pipe | SyntaxKind::PipeEq => BinaryOp::Or,
        SyntaxKind::AmpAmp => BinaryOp::AndAnd,
        SyntaxKind::PipePipe => BinaryOp::OrOr,
        _ => return None,
    })
}

/// A compound assignment's operator, `None` for `=`: `>>=` is `>` `>=`, `>>>=` `>` `>` `>=`.
fn assign_op(node: SyntaxNode<'_>) -> Result<Option<BinaryOp>, ()> {
    let tokens: Vec<SyntaxKind> = node
        .direct_tokens()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind())
        .collect();
    Ok(match tokens.as_slice() {
        [SyntaxKind::Eq] => None,
        [SyntaxKind::Gt, SyntaxKind::Gt, SyntaxKind::GtEq] => Some(BinaryOp::UShr),
        [SyntaxKind::Gt, SyntaxKind::GtEq] => Some(BinaryOp::Shr),
        [kind] => Some(simple_op(*kind).ok_or(())?),
        _ => return Err(()),
    })
}

fn primitive_of(keyword: SyntaxKind) -> Option<Primitive> {
    Some(match keyword {
        SyntaxKind::BooleanKw => Primitive::Boolean,
        SyntaxKind::ByteKw => Primitive::Byte,
        SyntaxKind::CharKw => Primitive::Char,
        SyntaxKind::ShortKw => Primitive::Short,
        SyntaxKind::IntKw => Primitive::Int,
        SyntaxKind::LongKw => Primitive::Long,
        SyntaxKind::FloatKw => Primitive::Float,
        SyntaxKind::DoubleKw => Primitive::Double,
        _ => return None,
    })
}

fn names_at(names: &[ResolvedName], node: NodeId) -> Option<&Symbol> {
    let i = names.binary_search_by_key(&node, |n| n.node).ok()?;
    Some(&names[i].symbol)
}

impl<'s, 'r> Walker<'s, 'r> {
    /// Attributes `file`, each body with `fuel`.
    pub(crate) fn attribute(
        reads: &'r mut Reads<'s>,
        file: FileId,
        fuel: u64,
    ) -> Result<BodyTypes, Cancelled> {
        let snapshot = reads.snapshot;
        let (tree, _) = reads.tree(file)?;
        let view = snapshot.file(file)?.set;
        let decl_memo = snapshot.decl_names_memo(file)?;
        reads.record(
            crate::engine::derived::Dep::DeclNames(file),
            decl_memo.fingerprint,
        );
        let body_memo = snapshot.body_memo(file)?;
        reads.record(
            crate::engine::derived::Dep::BodyNames(file),
            body_memo.fingerprint,
        );
        let fs = reads.file_scope(file)?;
        let body: &BodyNames = &body_memo.value;
        let local_of_node = body
            .local_types
            .iter()
            .enumerate()
            .map(|(i, t)| {
                (
                    t.node,
                    TypeKey::Local {
                        file,
                        index: i as u32,
                    },
                )
            })
            .collect();
        let mut w = Walker::new(reads, view, tree.release(), fuel);
        w.body_names = Some(body_memo.value.clone());
        w.decl_names = Some(decl_memo.value.clone());
        w.fs = Some(fs);
        w.local_of_node = local_of_node;
        w.walk(tree.root())?;
        Ok(w.finish())
    }

    /// A walker with no names yet, for [`Walker::attribute`] or [`Walker::for_initializer`].
    fn new(reads: &'r mut Reads<'s>, view: SourceSetId, release: u32, fuel: u64) -> Walker<'s, 'r> {
        let snapshot = reads.snapshot;
        let mut cx = snapshot.recording_types(view);
        cx.refuel(fuel);
        Walker {
            cx,
            snapshot,
            view,
            names: snapshot.names().clone(),
            body_names: None,
            decl_names: None,
            initializer_names: Box::default(),
            fs: None,
            local_of_node: Map::default(),
            release,
            fuel,
            out: BodyTypes::default(),
            types: Vec::new(),
            constants: Vec::new(),
            conversions: Vec::new(),
            compound: Vec::new(),
            resolved: Vec::new(),
            calls: Vec::new(),
            unchosen: Vec::new(),
            body_fuel: Vec::new(),
            frames: Vec::new(),
            values: Vec::new(),
            locals: Map::default(),
            classes: Vec::new(),
            summary_types: Vec::new(),
            next_index: 0,
            body: None,
            selectors: Vec::new(),
            results: Vec::new(),
            for_each: Map::default(),
            local_tables: Map::default(),
            inferred_supertypes: Default::default(),
            outer_superclasses: Map::default(),
            local_constants: Map::default(),
            nodes: 0,
            string: None,
            reads,
        }
    }

    /// A walker for one field initializer, whose names are `names`.
    pub(crate) fn for_initializer(
        reads: &'r mut Reads<'s>,
        view: SourceSetId,
        names: Box<[ResolvedName]>,
        release: u32,
    ) -> Walker<'s, 'r> {
        let mut w = Walker::new(reads, view, release, crate::types::DEFAULT_FUEL);
        w.initializer_names = names;
        w
    }

    /// Attributes one initializer of a field of `owner`, alone, for its constant value: the
    /// initializer's value and type.
    pub(crate) fn initializer(
        &mut self,
        owner: TypeKey,
        node: SyntaxNode<'_>,
    ) -> Result<Option<(TypeId, Option<Constant>)>, Cancelled> {
        self.classes.push(Class {
            node: NodeId::from_index(usize::MAX >> 1),
            key: owner,
        });
        self.body = Some(node.id());
        for event in node.preorder() {
            match event {
                WalkEvent::Enter(n) => self.enter(n)?,
                WalkEvent::Leave(n) => self.leave(n)?,
            }
        }
        let value = self.values.pop();
        self.reads.extend(self.cx.take_reads());
        Ok(value.map(|v| (v.ty, v.constant)))
    }

    pub(crate) fn context(&mut self) -> &mut TypeContext<'s> {
        &mut self.cx
    }

    fn walk(&mut self, root: SyntaxNode<'_>) -> Result<(), Cancelled> {
        for event in root.preorder() {
            match event {
                WalkEvent::Enter(n) => self.enter(n)?,
                WalkEvent::Leave(n) => self.leave(n)?,
            }
        }
        Ok(())
    }

    fn finish(mut self) -> BodyTypes {
        self.reads.extend(self.cx.take_reads());
        let mut out = std::mem::take(&mut self.out);
        let sort = |v: &mut Vec<(NodeId, _)>| v.sort_by_key(|(n, _)| *n);
        sort(&mut self.types);
        self.types.dedup_by_key(|(n, _)| *n);
        self.constants.sort_by_key(|(n, _)| *n);
        self.constants.dedup_by_key(|(n, _)| *n);
        self.conversions.sort_by_key(|(n, _)| *n);
        self.conversions.dedup_by_key(|(n, _)| *n);
        self.compound.sort_by_key(|(n, _)| *n);
        self.resolved.sort_by_key(|(n, _)| *n);
        self.resolved.dedup_by_key(|(n, _)| *n);
        self.calls.sort_by_key(|(n, _)| *n);
        self.unchosen.sort_by_key(|(n, _)| *n);
        self.body_fuel.sort_by_key(|(n, _)| *n);
        let mut tables: Vec<(TypeKey, Arc<Members>)> = self
            .local_tables
            .iter()
            .filter_map(|(k, t)| t.clone().map(|t| (*k, t)))
            .collect();
        tables.sort_by_key(|(k, _)| *k);
        out.types = std::mem::take(&mut self.types).into_boxed_slice();
        out.constants = std::mem::take(&mut self.constants).into_boxed_slice();
        out.conversions = std::mem::take(&mut self.conversions).into_boxed_slice();
        out.compound = std::mem::take(&mut self.compound).into_boxed_slice();
        out.resolved = std::mem::take(&mut self.resolved).into_boxed_slice();
        out.calls = std::mem::take(&mut self.calls).into_boxed_slice();
        out.unchosen = std::mem::take(&mut self.unchosen).into_boxed_slice();
        out.fuel = std::mem::take(&mut self.body_fuel).into_boxed_slice();
        let mut variables: Vec<(NodeId, TypeId)> = self
            .locals
            .iter()
            .map(|(n, l)| (*n, l.shown.unwrap_or(l.ty)))
            .collect();
        variables.sort_by_key(|(n, _)| *n);
        out.variables = variables.into_boxed_slice();
        out.local_members = tables.into_boxed_slice();
        out.arena = std::mem::replace(self.cx.arena_mut(), crate::types::Arena::new());
        out
    }

    fn symbol(&self, node: NodeId) -> Option<&Symbol> {
        names_at(&self.initializer_names, node)
            .or_else(|| names_at(&self.body_names.as_ref()?.names, node))
            .or_else(|| names_at(&self.decl_names.as_ref()?.names, node))
    }

    fn unknown(&mut self, why: Why) -> TypeId {
        self.cx.arena_mut().unknown(why)
    }

    fn error_type(&mut self, error: ErrorSymbol) -> TypeId {
        // A member not found in a type with a Lombok annotation may be one Lombok generates.
        if let Cause::Incomplete { ty } = error.cause
            && self.lombok_above(ty)
        {
            return self.unknown(Why::LeftToB4);
        }
        let e = self.cx.arena_mut().error(error);
        self.cx.arena_mut().intern(Type::Error(e))
    }

    /// A type's members can't be known: for a Lombok annotation on it or a supertype, until B4.
    fn incomplete(&mut self, key: TypeKey) -> TypeId {
        let why = if self.lombok_above(key) {
            Why::LeftToB4
        } else {
            Why::Incomplete(key)
        };
        self.unknown(why)
    }

    fn lombok_above(&mut self, key: TypeKey) -> bool {
        let fuel = self.cx.save_fuel();
        let found = self.cx.lombok_above(key).unwrap_or(false);
        self.cx.restore_fuel(fuel);
        found
    }

    fn primitive(&mut self, p: Primitive) -> TypeId {
        self.cx.primitive(p)
    }

    fn lang(&mut self, binary: &str) -> Result<TypeId, Cancelled> {
        Ok(match self.cx.lang(binary)? {
            Some(key) => self.cx.class_type(key, None, None),
            None => self.error_type(ErrorSymbol {
                name: self.names.intern(binary),
                cause: Cause::NotFound,
            }),
        })
    }

    fn string_type(&mut self) -> Result<TypeId, Cancelled> {
        if let Some(t) = self.string {
            return Ok(t);
        }
        let t = self.lang("java/lang/String")?;
        self.string = Some(t);
        Ok(t)
    }

    fn is_string(&mut self, t: TypeId) -> Result<bool, Cancelled> {
        Ok(self.string_type()? == t)
    }

    fn non_answer(&self, t: TypeId) -> Option<Why> {
        self.cx.arena().non_answer(t)
    }

    // -------------------------------------------------------------------------------------------
    // Entering.

    fn enter(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        self.nodes += 1;
        if self.nodes.is_multiple_of(1024) {
            self.reads.check()?;
        }
        let kind = node.kind();
        let mut expected = None;
        if let Some(parent) = node.parent() {
            if self.body.is_none() && starts_body(node, parent) {
                self.body = Some(node.id());
                self.cx.refuel(self.fuel);
            }
            if kind == SyntaxKind::ArrayInitializer {
                expected = self.initializer_type(node, parent)?;
            }
        }
        if is_type_decl(kind) {
            self.enter_type(node);
        } else if kind == SyntaxKind::ClassBody
            && let Some(&key) = self.local_of_node.get(&node.id())
        {
            // An anonymous class's body. Created with `<>`, its supertype's arguments are
            // inference's (B3c), and its entry has none.
            let diamond = node
                .parent()
                .and_then(ast::NewExpr::cast)
                .and_then(|n| n.ty())
                .is_some_and(|t| match t {
                    ast::Type::ClassType(c) => c
                        .type_arguments()
                        .is_some_and(|a| a.arguments().next().is_none()),
                    _ => false,
                });
            if diamond {
                self.inferred_supertypes.insert(key);
            } else if let Some(new) = node.parent().and_then(ast::NewExpr::cast) {
                self.enter_anonymous_of_outer(key, new)?;
            }
            self.classes.push(Class {
                node: node.id(),
                key,
            });
        } else if kind == SyntaxKind::CompilationUnit
            && let Some(fs) = &self.fs
            && fs.is_compact()
        {
            self.summary_types.push((node.id(), 0));
            self.next_index = 1;
            let key = fs.keys[0];
            self.classes.push(Class {
                node: node.id(),
                key,
            });
        } else if kind == SyntaxKind::MethodDecl {
            let result = ast::MethodDecl::cast(node).unwrap().result();
            let t = match result {
                Some(t) => Some(self.syntax_type(t)?),
                None => None,
            };
            self.results.push((node.id(), t));
        } else if kind == SyntaxKind::LambdaExpr {
            self.results.push((node.id(), None));
        }
        self.frames.push(Frame {
            node: node.id(),
            mark: self.values.len(),
            expected,
        });
        Ok(())
    }

    /// A type declaration: the summary's next type if it is a member of one, else a local
    /// class of the file's body result.
    fn enter_type(&mut self, node: SyntaxNode<'_>) {
        if let Some(&key) = self.local_of_node.get(&node.id()) {
            self.classes.push(Class {
                node: node.id(),
                key,
            });
            return;
        }
        let Some(fs) = self.fs.clone() else {
            return;
        };
        let Some(parent) = node.parent() else {
            return;
        };
        let top = self.summary_types.last().map(|t| t.0);
        let member = match parent.kind() {
            SyntaxKind::CompilationUnit => true,
            SyntaxKind::ClassBody | SyntaxKind::EnumBody => {
                parent.parent().is_some_and(|p| Some(p.id()) == top)
            }
            _ => false,
        };
        if !member {
            return;
        }
        let header = crate::summary::Header::of(ast::TypeDecl::cast(node).unwrap());
        let name = header
            .name
            .map_or(Name::EMPTY, |n| crate::summary::name_text(n, &self.names));
        let outer = self.summary_types.last().map(|t| t.1);
        let index = self.next_index;
        let matches = fs
            .summary
            .types
            .get(index)
            .is_some_and(|t| t.name == name && t.outer.map(|o| o as usize) == outer);
        if !matches {
            return;
        }
        self.next_index += 1;
        self.summary_types.push((node.id(), index));
        self.classes.push(Class {
            node: node.id(),
            key: fs.keys[index],
        });
    }

    /// The type an array initializer is, from its parent: a creation's type, a variable's
    /// declared type, or an enclosing initializer's component type.
    fn initializer_type(
        &mut self,
        node: SyntaxNode<'_>,
        parent: SyntaxNode<'_>,
    ) -> Result<Option<TypeId>, Cancelled> {
        Ok(match parent.kind() {
            SyntaxKind::ArrayCreationExpr => {
                let creation = ast::ArrayCreationExpr::cast(parent).unwrap();
                match creation.element_type() {
                    Some(t) => {
                        let mut t = self.syntax_type(t)?;
                        for _ in creation.dims() {
                            t = self.cx.arena_mut().array(t);
                        }
                        Some(t)
                    }
                    None => None,
                }
            }
            SyntaxKind::VariableDeclarator => self.declarator_type(parent)?,
            SyntaxKind::ArrayInitializer => {
                let outer = self
                    .frames
                    .iter()
                    .rev()
                    .find(|f| f.node == parent.id())
                    .and_then(|f| f.expected);
                outer.and_then(|t| match self.cx.get(t) {
                    Type::Array(c) => Some(c),
                    _ => None,
                })
            }
            _ => {
                let _ = node;
                None
            }
        })
    }

    /// A declarator's declared type, from its declaration's type and its own dimensions; `None`
    /// for `var`.
    fn declarator_type(&mut self, declarator: SyntaxNode<'_>) -> Result<Option<TypeId>, Cancelled> {
        let d = ast::VariableDeclarator::cast(declarator).unwrap();
        let ty = match declarator.parent() {
            Some(p) if p.kind() == SyntaxKind::LocalVarDecl => {
                ast::LocalVarDecl::cast(p).unwrap().ty()
            }
            Some(p) if p.kind() == SyntaxKind::FieldDecl => ast::FieldDecl::cast(p).unwrap().ty(),
            _ => None,
        };
        let Some(ty) = ty else {
            return Ok(None);
        };
        if self.is_var(ty) {
            return Ok(None);
        }
        let mut t = self.syntax_type(ty)?;
        for _ in d.dims() {
            t = self.cx.arena_mut().array(t);
        }
        Ok(Some(t))
    }

    /// Whether a type is `var` (JLS 14.4): a name `var` that B2 didn't resolve as a type.
    fn is_var(&self, ty: ast::Type<'_>) -> bool {
        let ast::Type::ClassType(c) = ty else {
            return false;
        };
        self.release >= 10
            && c.qualifier().is_none()
            && c.type_arguments().is_none()
            && c.name().is_some_and(|n| n.text() == "var")
            && self.symbol(c.syntax().id()).is_none()
    }

    // -------------------------------------------------------------------------------------------
    // Types as written.

    /// The type a type as written in a body or signature denotes, its names from the file's
    /// names (docs/design/semantic-attribution.md#the-walk).
    fn syntax_type(&mut self, ty: ast::Type<'_>) -> Result<TypeId, Cancelled> {
        ferrion_stack::ensure_stack(|| self.syntax_type_inner(ty))
    }

    fn syntax_type_inner(&mut self, ty: ast::Type<'_>) -> Result<TypeId, Cancelled> {
        Ok(match ty {
            ast::Type::PrimitiveType(p) => match p.keyword().and_then(|k| primitive_of(k.kind())) {
                Some(p) => self.primitive(p),
                None => self.unknown(Why::Later),
            },
            ast::Type::VoidType(_) => self.cx.arena_mut().intern(Type::Void),
            ast::Type::ArrayType(a) => {
                let mut t = match a.element_type() {
                    Some(e) => self.syntax_type(e)?,
                    None => self.unknown(Why::Later),
                };
                for _ in a.dims() {
                    t = self.cx.arena_mut().array(t);
                }
                t
            }
            ast::Type::ClassType(c) => self.class_type(c)?,
            ast::Type::IntersectionType(i) => {
                let mut parts = Vec::new();
                for t in i.types() {
                    parts.push(self.syntax_type(t)?);
                }
                self.cx.arena_mut().intersection(&parts)
            }
            ast::Type::UnionType(u) => {
                let mut parts = Vec::new();
                for t in u.types() {
                    parts.push(self.syntax_type(t)?);
                }
                self.cx.lub(&parts)?
            }
        })
    }

    fn class_type(&mut self, c: ast::ClassType<'_>) -> Result<TypeId, Cancelled> {
        let symbol = self.symbol(c.syntax().id()).cloned();
        let key = match symbol {
            Some(Symbol::Type(key)) => key,
            Some(Symbol::TypeVar(owner, index)) => {
                return Ok(self.cx.arena_mut().var(owner, index));
            }
            Some(Symbol::Error(e)) => return Ok(self.error_type(e)),
            _ => return Ok(self.unknown(Why::Later)),
        };
        let args = self.written_arguments(c)?;
        // The type it is a member of: written with arguments, or implied where an inner class of
        // a generic class is used without a qualifier inside it; a qualifier written without
        // arguments makes it raw, as javac has it (W1's observation 11).
        let outer = match c.qualifier() {
            Some(q) => {
                let q = ferrion_stack::ensure_stack(|| self.class_type(q))?;
                if self.cx.has_type_args(q) {
                    Some(self.cx.outer_of_member(key, q)?)
                } else {
                    None
                }
            }
            None => self.implicit_outer(key)?,
        };
        Ok(self.cx.class_type(key, outer, args.as_deref()))
    }

    /// A class type's type arguments as written; `None` if it has none.
    fn written_arguments(
        &mut self,
        c: ast::ClassType<'_>,
    ) -> Result<Option<Vec<TypeArg>>, Cancelled> {
        let mut args = Vec::new();
        if let Some(arguments) = c.type_arguments() {
            for a in arguments.arguments() {
                args.push(match a {
                    ast::TypeArgument::Type(t) => TypeArg::Type(self.syntax_type(t)?),
                    ast::TypeArgument::Wildcard(w) => {
                        let bound = match w.bound() {
                            Some(b) => {
                                let kind = if w.super_kw().is_some() {
                                    WildcardBound::Super
                                } else {
                                    WildcardBound::Extends
                                };
                                Some((kind, self.syntax_type(b)?))
                            }
                            None => None,
                        };
                        TypeArg::Wildcard(bound)
                    }
                });
            }
        }
        Ok((!args.is_empty()).then_some(args))
    }

    /// The outer type an inner class of a generic class has where it is used unqualified: as
    /// the innermost class around the use sees it.
    fn implicit_outer(&mut self, key: TypeKey) -> Result<Option<TypeId>, Cancelled> {
        match self.classes.last().map(|c| c.key) {
            Some(site) => self.cx.implicit_outer_in(key, site),
            None => Ok(None),
        }
    }

    // -------------------------------------------------------------------------------------------
    // Leaving.

    fn leave(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let Some(frame) = self.frames.pop() else {
            return Ok(());
        };
        debug_assert_eq!(frame.node, node.id());
        let mark = frame.mark.min(self.values.len());
        let children = self.values.split_off(mark);
        // A call's arguments stay on the stack for the call to read.
        let value = if node.kind() == SyntaxKind::Arguments {
            self.values.extend(children);
            None
        } else {
            self.value(node, &children, frame.expected)?
        };
        if let Some(value) = value {
            if value.what == What::Expr {
                self.types
                    .push((value.node, value.shown.unwrap_or(value.ty)));
                if let Some(c) = &value.constant {
                    self.constants.push((value.node, c.clone()));
                }
            }
            self.after(node, &value)?;
            self.values.push(value);
        }
        let id = node.id();
        if self.classes.last().is_some_and(|c| c.node == id) {
            self.classes.pop();
        }
        if self.summary_types.last().is_some_and(|t| t.0 == id) {
            self.summary_types.pop();
        }
        if self.results.last().is_some_and(|r| r.0 == id) {
            self.results.pop();
        }
        if self.selectors.last().is_some_and(|s| s.0 == id) {
            self.selectors.pop();
        }
        if self.body == Some(id) {
            self.body = None;
            self.body_fuel.push((id, self.cx.fuel_used()));
            // What is between bodies, a method's parameters' types, doesn't pay for a body's
            // running out.
            self.cx.refuel(self.fuel);
        }
        Ok(())
    }

    fn expr(&self, node: NodeId, ty: TypeId) -> Value {
        Value {
            node,
            ty,
            constant: None,
            what: What::Expr,
            variable: None,
            shown: None,
            operands: None,
            operand_depth: 0,
        }
    }

    fn convert(
        &mut self,
        context: Context,
        from: TypeId,
        to: TypeId,
        constant: Option<&Constant>,
    ) -> Result<Tri<Steps>, Cancelled> {
        self.cx.convert(context, from, to, constant)
    }

    /// Records the conversions `context` applies to a child, unless they are identity.
    fn record_conversion(
        &mut self,
        value: &Value,
        context: Context,
        to: TypeId,
    ) -> Result<Tri<Steps>, Cancelled> {
        let answer = self.convert(context, value.ty, to, value.constant.as_ref())?;
        if let Tri::Yes(steps) = answer
            && steps.as_slice() != [Conversion::Identity]
        {
            self.conversions.push((value.node, steps));
        }
        Ok(answer)
    }

    /// What a node pushes as it is left, from its children's values.
    fn value(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
        expected: Option<TypeId>,
    ) -> Result<Option<Value>, Cancelled> {
        let id = node.id();
        Ok(Some(match node.kind() {
            SyntaxKind::LiteralExpr => self.literal(node)?,
            SyntaxKind::NameExpr => self.name(node)?,
            SyntaxKind::FieldAccessExpr => self.field_access(node, children)?,
            SyntaxKind::ThisExpr => self.this(node, children)?,
            SyntaxKind::SuperExpr => self.super_(node, children)?,
            SyntaxKind::ParenExpr => {
                let inner = id_of(ast::ParenExpr::cast(node).unwrap().expr());
                match child(children, inner) {
                    Some(v) if v.what == What::Expr => Value {
                        node: id,
                        variable: None,
                        ..v.clone()
                    },
                    _ => {
                        let t = self.unknown(Why::Later);
                        self.expr(id, t)
                    }
                }
            }
            SyntaxKind::ClassLiteralExpr => self.class_literal(node, children)?,
            SyntaxKind::ArrayAccessExpr => self.array_access(node, children)?,
            SyntaxKind::MethodCallExpr | SyntaxKind::NewExpr | SyntaxKind::ConstructorCallExpr => {
                self.call(node, children)?
            }
            SyntaxKind::ArrayCreationExpr => self.array_creation(node)?,
            SyntaxKind::ArrayInitializer => self.array_initializer(node, children, expected)?,
            SyntaxKind::PrefixExpr => self.prefix(node, children)?,
            SyntaxKind::PostfixExpr => {
                let operand = id_of(ast::PostfixExpr::cast(node).unwrap().operand());
                let t = match child(children, operand) {
                    Some(v) => v.variable.unwrap_or(v.ty),
                    None => self.unknown(Why::Later),
                };
                self.expr(id, t)
            }
            SyntaxKind::BinaryExpr => self.binary(node, children)?,
            SyntaxKind::InstanceofExpr => {
                let t = self.primitive(Primitive::Boolean);
                self.expr(id, t)
            }
            SyntaxKind::ConditionalExpr => self.conditional(node, children)?,
            SyntaxKind::AssignExpr => self.assign(node, children)?,
            SyntaxKind::CastExpr => self.cast(node, children)?,
            SyntaxKind::LambdaExpr | SyntaxKind::MethodRefExpr => {
                let t = self.unknown(Why::LeftToB3c);
                self.expr(id, t)
            }
            SyntaxKind::SwitchExpr => {
                let t = self.unknown(Why::LeftToB3d);
                self.expr(id, t)
            }
            SyntaxKind::ElementValueArray => self.element_array(node, children)?,
            _ => {
                self.statement(node, children)?;
                return Ok(None);
            }
        }))
    }

    /// What a value's parent does with it as soon as it is pushed: a `switch` keeps its
    /// selector's type, an enhanced `for` types its `var`.
    fn after(&mut self, node: SyntaxNode<'_>, value: &Value) -> Result<(), Cancelled> {
        let Some(parent) = node.parent() else {
            return Ok(());
        };
        match parent.kind() {
            SyntaxKind::SwitchStmt | SyntaxKind::SwitchExpr => {
                let selector = match parent.kind() {
                    SyntaxKind::SwitchStmt => {
                        id_of(ast::SwitchStmt::cast(parent).unwrap().selector())
                    }
                    _ => id_of(ast::SwitchExpr::cast(parent).unwrap().selector()),
                };
                if selector == Some(node.id()) {
                    let body = match parent.kind() {
                        SyntaxKind::SwitchStmt => {
                            id_of(ast::SwitchStmt::cast(parent).unwrap().body())
                        }
                        _ => id_of(ast::SwitchExpr::cast(parent).unwrap().body()),
                    };
                    if let Some(body) = body {
                        self.selectors.push((body, value.ty));
                    }
                }
            }
            SyntaxKind::ForEachStmt => {
                let s = ast::ForEachStmt::cast(parent).unwrap();
                if id_of(s.iterable()) == Some(node.id())
                    && let Some(declarator) = self.for_each.remove(&parent.id())
                {
                    let element = self.element_type(value.ty)?;
                    let t = self.cx.upward_projection(element)?;
                    self.locals.insert(
                        declarator,
                        Local {
                            ty: t,
                            constant: None,
                            shown: None,
                        },
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The type of an enhanced `for`'s variable (JLS 14.14.2): an array's component, or
    /// `Iterable`'s argument through the expression's supertype, `Object` for a raw one.
    fn element_type(&mut self, t: TypeId) -> Result<TypeId, Cancelled> {
        if let Some(why) = self.non_answer(t) {
            return Ok(self.unknown(why));
        }
        if let Type::Array(c) = self.cx.get(t) {
            return Ok(c);
        }
        let Some(iterable) = self.cx.lang("java/lang/Iterable")? else {
            return Ok(self.unknown(Why::Later));
        };
        let captured = self.cx.fresh_capture(t)?;
        Ok(match self.cx.as_super(captured, iterable)? {
            Tri::Yes(s) => match self.cx.arena().class_args(s).first().copied() {
                Some(TypeArg::Type(e)) => e,
                Some(TypeArg::Wildcard(_)) => self.unknown(Why::Later),
                None => self.lang("java/lang/Object")?,
            },
            Tri::No => self.unknown(Why::Later),
            Tri::Unknown(why) => self.unknown(why),
        })
    }

    // -------------------------------------------------------------------------------------------
    // Expressions.

    fn literal(&mut self, node: SyntaxNode<'_>) -> Result<Value, Cancelled> {
        let token = ast::LiteralExpr::cast(node).unwrap().value();
        let Some(token) = token else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(node.id(), t));
        };
        let t = match token.kind() {
            SyntaxKind::IntLiteral => self.primitive(Primitive::Int),
            SyntaxKind::LongLiteral => self.primitive(Primitive::Long),
            SyntaxKind::FloatLiteral => self.primitive(Primitive::Float),
            SyntaxKind::DoubleLiteral => self.primitive(Primitive::Double),
            SyntaxKind::CharLiteral => self.primitive(Primitive::Char),
            SyntaxKind::TrueKw | SyntaxKind::FalseKw => self.primitive(Primitive::Boolean),
            SyntaxKind::StringLiteral | SyntaxKind::TextBlock => self.string_type()?,
            SyntaxKind::NullKw => self.cx.arena_mut().intern(Type::Null),
            _ => self.unknown(Why::Later),
        };
        Ok(Value {
            constant: constants::literal_value(token),
            ..self.expr(node.id(), t)
        })
    }

    /// Whether a variable named here is read for its value, and so captured (JLS 6.5.6.1):
    /// not as an assignment's or increment's target, nor as a qualifier, where javac keeps the
    /// declared type, through parentheses too (observed: `xs` in `xs.size()`, `(xs).size()`, and
    /// `xs = null`).
    fn captured_here(node: SyntaxNode<'_>) -> bool {
        let mut node = node;
        while let Some(parent) = node.parent().filter(|p| p.kind() == SyntaxKind::ParenExpr) {
            node = parent;
        }
        let Some(parent) = node.parent() else {
            return true;
        };
        match parent.kind() {
            SyntaxKind::AssignExpr => {
                id_of(ast::AssignExpr::cast(parent).unwrap().target()) != Some(node.id())
            }
            SyntaxKind::FieldAccessExpr => {
                id_of(ast::FieldAccessExpr::cast(parent).unwrap().target()) != Some(node.id())
            }
            SyntaxKind::MethodCallExpr => {
                id_of(ast::MethodCallExpr::cast(parent).unwrap().receiver()) != Some(node.id())
            }
            SyntaxKind::MethodRefExpr | SyntaxKind::PostfixExpr => false,
            SyntaxKind::PrefixExpr => !ast::PrefixExpr::cast(parent)
                .unwrap()
                .op()
                .is_some_and(|t| matches!(t.kind(), SyntaxKind::PlusPlus | SyntaxKind::MinusMinus)),
            _ => true,
        }
    }

    /// A variable's value: its type captured where it is read.
    fn variable(
        &mut self,
        node: SyntaxNode<'_>,
        ty: TypeId,
        constant: Option<Constant>,
    ) -> Result<Value, Cancelled> {
        let t = if Self::captured_here(node) {
            let captured = self.cx.fresh_capture(ty)?;
            self.variable_as_argument(node, captured)?
        } else {
            ty
        };
        Ok(Value {
            node: node.id(),
            ty: t,
            constant,
            what: What::Expr,
            variable: Some(ty),
            shown: None,
            operands: None,
            operand_depth: 0,
        })
    }

    fn name(&mut self, node: SyntaxNode<'_>) -> Result<Value, Cancelled> {
        let id = node.id();
        let symbol = self.symbol(id).cloned();
        if node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::SwitchLabel)
            && let Some(value) = self.case_constant(node, symbol.as_ref())?
        {
            return Ok(value);
        }
        Ok(match symbol {
            Some(Symbol::Local { node: decl, .. }) => match self.locals.get(&decl).cloned() {
                Some(local) => Value {
                    shown: local.shown,
                    ..self.variable(node, local.ty, local.constant)?
                },
                None => {
                    let t = self.unknown(Why::Later);
                    self.expr(id, t)
                }
            },
            Some(Symbol::Field { owner, name }) => self.simple_field(node, owner, name)?,
            Some(Symbol::Type(key)) => self.type_name(id, Some(key)),
            Some(Symbol::TypeVar(..)) => self.type_name(id, None),
            Some(Symbol::Package(_)) => {
                let t = self.unknown(Why::Later);
                Value {
                    what: What::Package,
                    ..self.expr(id, t)
                }
            }
            Some(Symbol::Error(e)) => {
                let t = self.error_type(e);
                self.expr(id, t)
            }
            _ => {
                let t = self.unknown(Why::Later);
                self.expr(id, t)
            }
        })
    }

    fn type_name(&mut self, node: NodeId, key: Option<TypeKey>) -> Value {
        let t = match key {
            Some(key) => self.cx.class_type(key, None, None),
            None => self.unknown(Why::Later),
        };
        Value {
            node,
            ty: t,
            constant: None,
            what: What::Type(key),
            variable: None,
            shown: None,
            operands: None,
            operand_depth: 0,
        }
    }

    /// A `case` label's simple name, decided by the selector's type now that it has one
    /// (JLS 14.11.1): a constant of the enum, if the selector is an enum, whatever variable B2
    /// found in scope when it couldn't tell (its `case-label` difference); waiting with a
    /// selector that has no type yet; else `None`, and it is a constant variable, as B2 has it.
    fn case_constant(
        &mut self,
        node: SyntaxNode<'_>,
        symbol: Option<&Symbol>,
    ) -> Result<Option<Value>, Cancelled> {
        let Some(selector) = self.selectors.last().map(|s| s.1) else {
            return Ok(None);
        };
        if let Some(why) = self.non_answer(selector) {
            let t = self.unknown(why);
            return Ok(Some(self.expr(node.id(), t)));
        }
        let name = ast::NameExpr::cast(node)
            .unwrap()
            .name()
            .map(|t| crate::summary::name_text(t, &self.names));
        if let Type::Class { key, .. } = self.cx.get(selector)
            && let Some(name) = name
            && self.cx.kind(key)? == Some(TypeKind::Enum)
            && let Some(index) = self.cx.field_named(key, name)?
        {
            let field = Symbol::Field { owner: key, name };
            if symbol != Some(&field) {
                self.resolved.push((node.id(), field));
            }
            let t = self.cx.field_type(key, index)?;
            return Ok(Some(self.expr(node.id(), t)));
        }
        Ok(match symbol {
            Some(Symbol::LeftToB3(_)) => {
                let t = self.unknown(Why::Later);
                Some(self.expr(node.id(), t))
            }
            _ => None,
        })
    }

    /// A field's index in its owner.
    fn field_ref(&mut self, owner: TypeKey, name: Name) -> Result<Option<FieldRef>, Cancelled> {
        Ok(self
            .cx
            .field_named(owner, name)?
            .map(|index| FieldRef { owner, index }))
    }

    /// A constant field's value: a source or class file field's from its entry, a local class's
    /// from the walk.
    fn field_constant(&mut self, field: FieldRef) -> Result<Option<Constant>, Cancelled> {
        if let TypeKey::Local { .. } = field.owner {
            return Ok(self.local_constants.get(&field).cloned());
        }
        // Most fields aren't candidates, and need no entry to say so.
        if !self.cx.may_be_constant(field)? {
            return Ok(None);
        }
        self.reads.constant(field)
    }

    /// Whether `key` is an anonymous class created with `<>` and `owner` one of its supertypes,
    /// so that a member of `owner` seen from it needs the arguments inference gives.
    fn inherited_through_inference(
        &mut self,
        key: TypeKey,
        owner: TypeKey,
    ) -> Result<bool, Cancelled> {
        if !self.inferred_supertypes.contains(&key) {
            return Ok(false);
        }
        let Some(supers) = self.snapshot.supertypes(key)? else {
            return Ok(false);
        };
        let direct: Vec<TypeKey> = supers
            .superclass
            .iter()
            .chain(supers.interfaces.iter())
            .filter_map(|s| s.as_ref().ok().copied())
            .collect();
        for class in direct {
            if self.cx.is_subclass(class, owner)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A field named by its simple name: as a member of the innermost class around it that has
    /// it, or as declared for a static one.
    fn simple_field(
        &mut self,
        node: SyntaxNode<'_>,
        owner: TypeKey,
        name: Name,
    ) -> Result<Value, Cancelled> {
        let Some(field) = self.field_ref(owner, name)? else {
            let t = self.unknown(Why::Incomplete(owner));
            return Ok(self.expr(node.id(), t));
        };
        let is_static = self.cx.field_info(field)?.is_some_and(|f| f.0);
        let mut ty = None;
        if !is_static {
            let keys: Vec<TypeKey> = self.classes.iter().rev().map(|c| c.key).collect();
            for key in keys {
                if self.inherited_through_inference(key, owner)? {
                    let t = self.unknown(Why::LeftToB3c);
                    return Ok(self.expr(node.id(), t));
                }
                if self.cx.is_subclass(key, owner)? {
                    let this = self.cx.declared_type(key)?;
                    if let Tri::Yes(t) = self.cx.field_type_in(this, owner, field.index)? {
                        ty = Some(t);
                    }
                    break;
                }
            }
        }
        let ty = match ty {
            Some(t) => t,
            None => self.cx.field_type(owner, field.index)?,
        };
        let constant = self.field_constant(field)?;
        self.variable(node, ty, constant)
    }

    fn field_access(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let access = ast::FieldAccessExpr::cast(node).unwrap();
        let qualifier = child(children, id_of(access.target())).cloned();
        let id = node.id();
        let symbol = self.symbol(id).cloned();
        let name = access
            .name()
            .map_or(Name::EMPTY, |t| crate::summary::name_text(t, &self.names));
        match symbol {
            Some(Symbol::Type(key)) => return Ok(self.type_name(id, Some(key))),
            Some(Symbol::TypeVar(..)) => return Ok(self.type_name(id, None)),
            Some(Symbol::Package(_)) => {
                let t = self.unknown(Why::Later);
                return Ok(Value {
                    what: What::Package,
                    ..self.expr(id, t)
                });
            }
            Some(Symbol::Error(e)) => {
                let t = self.error_type(e);
                return Ok(self.expr(id, t));
            }
            _ => {}
        }
        let Some(qualifier) = qualifier else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(id, t));
        };
        match symbol {
            Some(Symbol::Field { owner, .. }) => {
                let Some(field) = self.field_ref(owner, name)? else {
                    let t = self.unknown(Why::Incomplete(owner));
                    return Ok(self.expr(id, t));
                };
                self.member_field(node, &qualifier, field)
            }
            Some(Symbol::LeftToB3(_)) => {
                if let Some(why) = self.non_answer(qualifier.ty) {
                    let t = self.unknown(why);
                    return Ok(self.expr(id, t));
                }
                if let Type::Array(_) = self.cx.get(qualifier.ty) {
                    let length = self.names.intern("length");
                    if name == length {
                        let t = self.primitive(Primitive::Int);
                        return Ok(Value {
                            variable: Some(t),
                            ..self.expr(id, t)
                        });
                    }
                    let error = ErrorSymbol {
                        name,
                        cause: Cause::NotFound,
                    };
                    let t = self.error_type(error);
                    return Ok(self.expr(id, t));
                }
                match self.find_field(qualifier.ty, name)? {
                    Ok(field) => {
                        self.resolved.push((
                            id,
                            Symbol::Field {
                                owner: field.owner,
                                name,
                            },
                        ));
                        self.member_field(node, &qualifier, field)
                    }
                    Err(t) => Ok(self.expr(id, t)),
                }
            }
            _ => {
                let t = self.unknown(Why::Later);
                Ok(self.expr(id, t))
            }
        }
    }

    /// A field selected from a qualifier: as a member of the qualifier's type, or as declared
    /// for a static one or after a type name; a constant only after a type name (JLS 15.29:
    /// `this.K` isn't one, observed).
    fn member_field(
        &mut self,
        node: SyntaxNode<'_>,
        qualifier: &Value,
        field: FieldRef,
    ) -> Result<Value, Cancelled> {
        if let Some(why) = self.non_answer(qualifier.ty)
            && qualifier.what == What::Expr
        {
            let t = self.unknown(why);
            return Ok(self.expr(node.id(), t));
        }
        let is_static = self.cx.field_info(field)?.is_some_and(|f| f.0);
        let ty = if is_static || qualifier.what != What::Expr {
            self.cx.field_type(field.owner, field.index)?
        } else {
            let of = self.cx.fresh_capture(qualifier.ty)?;
            match self.cx.field_type_in(of, field.owner, field.index)? {
                Tri::Yes(t) => t,
                Tri::No => self.unknown(Why::Later),
                Tri::Unknown(why) => self.unknown(why),
            }
        };
        let constant = match qualifier.what {
            What::Type(_) => self.field_constant(field)?,
            _ => None,
        };
        self.variable(node, ty, constant)
    }

    /// A field by name in a type's members: a class's, or through a type variable's or
    /// intersection's bounds. `Err` with the non-answer type when there is none.
    fn find_field(&mut self, t: TypeId, name: Name) -> Result<Result<FieldRef, TypeId>, Cancelled> {
        let parts: Vec<TypeId> = match self.cx.get(t) {
            Type::Var(..) | Type::Capture(_) => {
                let upper = self.cx.upper_bound(t)?;
                match self.cx.get(upper) {
                    Type::Intersection(list) => self.cx.arena().list(list).to_vec(),
                    _ => vec![upper],
                }
            }
            Type::Intersection(list) => self.cx.arena().list(list).to_vec(),
            _ => vec![t],
        };
        let mut incomplete = None;
        for part in parts {
            match self.cx.get(part) {
                Type::Class { key, .. } => {
                    let Some(table) = self.members(key)? else {
                        incomplete.get_or_insert(key);
                        continue;
                    };
                    match table.field(name) {
                        Some(FieldMember::One(f)) => return Ok(Ok(*f)),
                        Some(FieldMember::Ambiguous(fs)) => {
                            let owners = fs.iter().map(|f| f.owner).collect();
                            let t = self.error_type(ErrorSymbol {
                                name,
                                cause: Cause::Ambiguous(owners),
                            });
                            return Ok(Err(t));
                        }
                        None if table.incomplete => {
                            incomplete.get_or_insert(key);
                        }
                        None => {}
                    }
                }
                Type::Var(..) | Type::Capture(_) | Type::Intersection(_) => {
                    if let Ok(found) = ferrion_stack::ensure_stack(|| self.find_field(part, name))?
                    {
                        return Ok(Ok(found));
                    }
                }
                _ => {
                    if let Some(why) = self.non_answer(part) {
                        return Ok(Err(self.unknown(why)));
                    }
                }
            }
        }
        Ok(Err(match incomplete {
            Some(key) => self.incomplete(key),
            None => self.error_type(ErrorSymbol {
                name,
                cause: Cause::NotFound,
            }),
        }))
    }

    /// A type's member table: the shared entry, or a local class's, made here.
    fn members(&mut self, key: TypeKey) -> Result<Option<Arc<Members>>, Cancelled> {
        if let TypeKey::Local { .. } = key {
            let saved = self.cx.save_fuel();
            let table = local_table(
                self.snapshot,
                self.view,
                self.reads,
                &mut self.local_tables,
                &self.outer_superclasses,
                key,
            )?;
            self.cx.restore_fuel(saved);
            return Ok(table);
        }
        self.reads.members(self.view, key)
    }

    fn this(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let qualifier = id_of(ast::ThisExpr::cast(node).unwrap().qualifier());
        let key = match qualifier {
            Some(q) => match child(children, Some(q)) {
                Some(Value {
                    what: What::Type(Some(key)),
                    ..
                }) => Some(*key),
                Some(v) if v.what == What::Expr => {
                    let t = v.ty;
                    return Ok(self.expr(node.id(), t));
                }
                _ => None,
            },
            None => self.classes.last().map(|c| c.key),
        };
        let t = match key {
            Some(key) => self.cx.declared_type(key)?,
            None => self.unknown(Why::Later),
        };
        Ok(self.expr(node.id(), t))
    }

    /// `super` in `super.f` or `super.m()`: the direct superclass's type, as declared (observed:
    /// `Base<java/lang/String>`); `I.super` an interface's, as a supertype of the class.
    fn super_(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let qualifier = id_of(ast::SuperExpr::cast(node).unwrap().qualifier());
        let current = self.classes.last().map(|c| c.key);
        let t = match qualifier.and_then(|q| child(children, Some(q))) {
            Some(Value {
                what: What::Type(Some(key)),
                ..
            }) => {
                let key = *key;
                if self.cx.is_interface(key)? {
                    match current {
                        Some(c) => {
                            let this = self.cx.declared_type(c)?;
                            match self.cx.as_super(this, key)? {
                                Tri::Yes(t) => t,
                                Tri::No => self.unknown(Why::Later),
                                Tri::Unknown(why) => self.unknown(why),
                            }
                        }
                        None => self.unknown(Why::Later),
                    }
                } else {
                    self.superclass_of(key)?
                }
            }
            Some(v) => {
                let why = self.non_answer(v.ty).unwrap_or(Why::Later);
                self.unknown(why)
            }
            None => match current {
                Some(key) if self.inferred_supertypes.contains(&key) => {
                    self.unknown(Why::LeftToB3c)
                }
                Some(key) => self.superclass_of(key)?,
                None => self.unknown(Why::Later),
            },
        };
        Ok(self.expr(node.id(), t))
    }

    /// A class's superclass as `super` has it: an error if it didn't resolve, where the class's
    /// hierarchy goes on from `Object`.
    fn superclass_of(&mut self, key: TypeKey) -> Result<TypeId, Cancelled> {
        let supers = self.snapshot.supertypes(key)?;
        if let Some(Err(error)) = supers.as_ref().and_then(|s| s.superclass.as_ref()) {
            return Ok(self.error_type(error.clone()));
        }
        Ok(match self.cx.superclass(key)? {
            Some(t) => t,
            None => self.unknown(Why::Incomplete(key)),
        })
    }

    fn class_literal(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let target = ast::ClassLiteralExpr::cast(node).unwrap().target();
        let named = match target {
            Some(ast::ExprOrType::Type(t)) => self.syntax_type(t)?,
            Some(ast::ExprOrType::Expr(e)) => match child(children, Some(e.syntax().id())) {
                Some(Value {
                    what: What::Type(Some(key)),
                    ..
                }) => {
                    let key = *key;
                    self.cx.class_type(key, None, None)
                }
                Some(v) => {
                    let why = self.non_answer(v.ty).unwrap_or(Why::Later);
                    self.unknown(why)
                }
                None => self.unknown(Why::Later),
            },
            None => self.unknown(Why::Later),
        };
        // `Class<C>` for the erasure of the type named, its box for a primitive type, `Void`
        // for `void` (JLS 15.8.2).
        let argument = match self.cx.get(named) {
            Type::Primitive(p) => self.cx.boxed(p)?,
            Type::Void => self.lang("java/lang/Void")?,
            _ => self.cx.erasure(named)?,
        };
        let t = match self.non_answer(argument) {
            Some(why) => self.unknown(why),
            None => match self.cx.lang("java/lang/Class")? {
                Some(class) => self
                    .cx
                    .class_type(class, None, Some(&[TypeArg::Type(argument)])),
                None => self.lang("java/lang/Class")?,
            },
        };
        Ok(self.expr(node.id(), t))
    }

    fn array_access(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let access = ast::ArrayAccessExpr::cast(node).unwrap();
        let array = child(children, id_of(access.array())).cloned();
        if let Some(index) = child(children, id_of(access.index())).cloned() {
            let int = self.primitive(Primitive::Int);
            self.record_conversion(&index, Context::Numeric, int)?;
        }
        let t = match array {
            Some(v) => match self.cx.get(v.ty) {
                Type::Array(c) => c,
                _ => match self.non_answer(v.ty) {
                    Some(why) => self.unknown(why),
                    None => {
                        let upper = self.cx.upper_bound(v.ty)?;
                        match self.cx.get(upper) {
                            Type::Array(c) => c,
                            _ => self.unknown(Why::Later),
                        }
                    }
                },
            },
            None => self.unknown(Why::Later),
        };
        let shown = self.variable_as_argument(node, t)?;
        Ok(Value {
            variable: Some(t),
            ..self.expr(node.id(), shown)
        })
    }

    fn array_creation(&mut self, node: SyntaxNode<'_>) -> Result<Value, Cancelled> {
        let creation = ast::ArrayCreationExpr::cast(node).unwrap();
        let mut t = match creation.element_type() {
            Some(e) => self.syntax_type(e)?,
            None => self.unknown(Why::Later),
        };
        let dims = creation.dim_exprs().count() + creation.dims().count();
        for _ in 0..dims {
            t = self.cx.arena_mut().array(t);
        }
        Ok(self.expr(node.id(), t))
    }

    fn array_initializer(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
        expected: Option<TypeId>,
    ) -> Result<Value, Cancelled> {
        let Some(t) = expected else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(node.id(), t));
        };
        if let Type::Array(component) = self.cx.get(t) {
            for element in children.iter().cloned() {
                if element.what == What::Expr {
                    self.record_conversion(&element, Context::Assignment, component)?;
                }
            }
        }
        Ok(self.expr(node.id(), t))
    }

    /// An annotation's `{...}`: an array of its element's type (observed: `NEW_ARRAY` of
    /// `java/lang/String[]` for `names = {"a", "b"}`).
    fn element_array(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let t = match self.element_type_of(node)? {
            Some(t) => t,
            None => self.unknown(Why::Later),
        };
        if let Type::Array(component) = self.cx.get(t) {
            for element in children.iter().cloned() {
                if element.what == What::Expr {
                    self.record_conversion(&element, Context::Assignment, component)?;
                }
            }
        }
        Ok(self.expr(node.id(), t))
    }

    /// The type of the annotation element an element value is for: its method's result, from
    /// the element-value pair's name (`value` for a single value), or for a default value its
    /// method's.
    fn element_type_of(&mut self, node: SyntaxNode<'_>) -> Result<Option<TypeId>, Cancelled> {
        let Some(parent) = node.parent() else {
            return Ok(None);
        };
        match parent.kind() {
            SyntaxKind::ElementValuePair => {
                let Some(Symbol::Element { owner, name }) = self.symbol(parent.id()).cloned()
                else {
                    return Ok(None);
                };
                self.element_result(owner, name)
            }
            SyntaxKind::AnnotationArguments => {
                let Some(annotation) = parent.parent() else {
                    return Ok(None);
                };
                let owner = ast::Annotation::cast(annotation)
                    .and_then(|a| a.name())
                    .and_then(|n| self.symbol(n.syntax().id()).cloned());
                let Some(Symbol::Type(owner)) = owner else {
                    return Ok(None);
                };
                let value = self.names.intern("value");
                self.element_result(owner, value)
            }
            SyntaxKind::DefaultValue => {
                let method = parent.parent().and_then(ast::MethodDecl::cast);
                match method.and_then(|m| m.result()) {
                    Some(t) => Ok(Some(self.syntax_type(t)?)),
                    None => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    fn element_result(&mut self, owner: TypeKey, name: Name) -> Result<Option<TypeId>, Cancelled> {
        let count = self.cx.method_count(owner)?;
        for index in 0..count as u32 {
            let key = crate::symbols::MethodRef { owner, index };
            if self.cx.method_name(key)? == Some(name) {
                let m = self.cx.method_type(owner, index)?;
                return Ok(Some(m.result));
            }
        }
        Ok(None)
    }

    fn prefix(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let e = ast::PrefixExpr::cast(node).unwrap();
        let operand = child(children, id_of(e.operand())).cloned();
        let op = e.op().map(|t| t.kind());
        let Some(operand) = operand else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(node.id(), t));
        };
        let id = node.id();
        let unary = match op {
            Some(SyntaxKind::PlusPlus | SyntaxKind::MinusMinus) => {
                let t = operand.variable.unwrap_or(operand.ty);
                return Ok(self.expr(id, t));
            }
            Some(SyntaxKind::Bang) => UnaryOp::Not,
            Some(SyntaxKind::Plus) => UnaryOp::Plus,
            Some(SyntaxKind::Minus) => UnaryOp::Minus,
            Some(SyntaxKind::Tilde) => UnaryOp::Complement,
            _ => {
                let t = self.unknown(Why::Later);
                return Ok(self.expr(id, t));
            }
        };
        if let Some(why) = self.non_answer(operand.ty) {
            let t = self.unknown(why);
            return Ok(self.expr(id, t));
        }
        let t = if unary == UnaryOp::Not {
            let boolean = self.primitive(Primitive::Boolean);
            self.record_conversion(&operand, Context::Numeric, boolean)?;
            boolean
        } else {
            match self.cx.unary_promotion(operand.ty)? {
                Tri::Yes(t) => {
                    self.record_conversion(&operand, Context::Numeric, t)?;
                    t
                }
                Tri::No => self.unknown(Why::Later),
                Tri::Unknown(why) => self.unknown(why),
            }
        };
        let constant = operand
            .constant
            .as_ref()
            .and_then(|c| constants::unary(unary, c));
        Ok(Value {
            constant,
            ..self.expr(id, t)
        })
    }

    /// The type of `a op b` (JLS 15.17 to 15.24), recording each operand's promotion.
    fn binary_type(
        &mut self,
        op: BinaryOp,
        a: &Value,
        b: &Value,
        record: bool,
    ) -> Result<TypeId, Cancelled> {
        use BinaryOp::*;
        let boolean = self.primitive(Primitive::Boolean);
        if let Some(why) = self.non_answer(a.ty).or(self.non_answer(b.ty)) {
            return Ok(self.unknown(why));
        }
        // String concatenation.
        if op == Add && (self.is_string(a.ty)? || self.is_string(b.ty)?) {
            let string = self.string_type()?;
            if record {
                for v in [a, b] {
                    if v.ty != string {
                        self.record_conversion(v, Context::String, string)?;
                    }
                }
            }
            return Ok(string);
        }
        let numeric_or_boolean =
            |w: &mut Self, v: &Value| -> Result<Option<Primitive>, Cancelled> {
                Ok(match w.cx.get(v.ty) {
                    Type::Primitive(p) => Some(p),
                    _ => w.cx.unboxed_primitive(v.ty)?,
                })
            };
        let (pa, pb) = (numeric_or_boolean(self, a)?, numeric_or_boolean(self, b)?);
        match op {
            AndAnd | OrOr => {
                if record {
                    self.record_conversion(a, Context::Numeric, boolean)?;
                    self.record_conversion(b, Context::Numeric, boolean)?;
                }
                return Ok(boolean);
            }
            Eq | Ne => {
                let numeric = |p: Option<Primitive>| p.is_some_and(|p| p != Primitive::Boolean);
                let primitive_side = matches!(self.cx.get(a.ty), Type::Primitive(_))
                    || matches!(self.cx.get(b.ty), Type::Primitive(_));
                if primitive_side && numeric(pa) && numeric(pb) {
                    if let Tri::Yes(t) = self.cx.binary_promotion(a.ty, b.ty)?
                        && record
                    {
                        self.record_conversion(a, Context::Numeric, t)?;
                        self.record_conversion(b, Context::Numeric, t)?;
                    }
                } else if primitive_side
                    && pa == Some(Primitive::Boolean)
                    && pb == Some(Primitive::Boolean)
                    && record
                {
                    self.record_conversion(a, Context::Numeric, boolean)?;
                    self.record_conversion(b, Context::Numeric, boolean)?;
                }
                return Ok(boolean);
            }
            And | Or | Xor if pa == Some(Primitive::Boolean) && pb == Some(Primitive::Boolean) => {
                if record {
                    self.record_conversion(a, Context::Numeric, boolean)?;
                    self.record_conversion(b, Context::Numeric, boolean)?;
                }
                return Ok(boolean);
            }
            Shl | Shr | UShr => {
                let left = self.cx.unary_promotion(a.ty)?;
                let right = self.cx.unary_promotion(b.ty)?;
                return Ok(match (left, right) {
                    (Tri::Yes(l), Tri::Yes(r)) => {
                        if record {
                            self.record_conversion(a, Context::Numeric, l)?;
                            self.record_conversion(b, Context::Numeric, r)?;
                        }
                        l
                    }
                    (Tri::Unknown(why), _) | (_, Tri::Unknown(why)) => self.unknown(why),
                    _ => self.unknown(Why::Later),
                });
            }
            _ => {}
        }
        let promoted = self.cx.binary_promotion(a.ty, b.ty)?;
        let t = match promoted {
            Tri::Yes(t) => t,
            Tri::No => return Ok(self.unknown(Why::Later)),
            Tri::Unknown(why) => return Ok(self.unknown(why)),
        };
        if record {
            self.record_conversion(a, Context::Numeric, t)?;
            self.record_conversion(b, Context::Numeric, t)?;
        }
        Ok(match op {
            Lt | Gt | Le | Ge => boolean,
            _ => t,
        })
    }

    fn binary(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let e = ast::BinaryExpr::cast(node).unwrap();
        let (a, b) = (
            child(children, id_of(e.lhs())).cloned(),
            child(children, id_of(e.rhs())).cloned(),
        );
        let (Some(a), Some(b), Some(op)) = (a, b, binary_op(node)) else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(node.id(), t));
        };
        let t = self.binary_type(op, &a, &b, true)?;
        let constant = match (&a.constant, &b.constant) {
            (Some(x), Some(y)) if self.non_answer(t).is_none() => constants::binary(op, x, y),
            _ => None,
        };
        Ok(Value {
            constant,
            ..self.expr(node.id(), t)
        })
    }

    /// Whether a conditional is in an assignment or invocation context, where a reference
    /// conditional is a poly expression (JLS 15.25): B3c's.
    fn poly_context(node: SyntaxNode<'_>) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            match parent.kind() {
                SyntaxKind::ParenExpr => {}
                SyntaxKind::ConditionalExpr => {
                    let c = ast::ConditionalExpr::cast(parent).unwrap();
                    if id_of(c.condition()) == Some(current.id()) {
                        return false;
                    }
                }
                SyntaxKind::VariableDeclarator => {
                    // `var` has no target: its initializer is standalone.
                    let ty = parent.parent().and_then(|d| match d.kind() {
                        SyntaxKind::LocalVarDecl => ast::LocalVarDecl::cast(d).unwrap().ty(),
                        SyntaxKind::FieldDecl => ast::FieldDecl::cast(d).unwrap().ty(),
                        _ => None,
                    });
                    let var = matches!(ty, Some(ast::Type::ClassType(c))
                        if c.qualifier().is_none() && c.type_arguments().is_none()
                            && c.name().is_some_and(|n| n.text() == "var"));
                    return !var;
                }
                SyntaxKind::AssignExpr => {
                    let a = ast::AssignExpr::cast(parent).unwrap();
                    return id_of(a.value()) == Some(current.id())
                        && a.op().is_some_and(|t| t.kind() == SyntaxKind::Eq);
                }
                SyntaxKind::ReturnStmt
                | SyntaxKind::ArrayInitializer
                | SyntaxKind::Arguments
                | SyntaxKind::LambdaExpr
                | SyntaxKind::YieldStmt => return true,
                SyntaxKind::SwitchRule => {
                    return parent
                        .parent()
                        .and_then(|b| b.parent())
                        .is_some_and(|s| s.kind() == SyntaxKind::SwitchExpr);
                }
                _ => return false,
            }
            current = parent;
        }
        false
    }

    /// A conditional (JLS 15.25): boolean, numeric by 15.25.2's table, or a reference
    /// conditional's `lub` where it is standalone.
    fn conditional(
        &mut self,
        node: SyntaxNode<'_>,
        children: &[Value],
    ) -> Result<Value, Cancelled> {
        let c = ast::ConditionalExpr::cast(node).unwrap();
        let condition = child(children, id_of(c.condition())).cloned();
        let a = child(children, id_of(c.then_expr())).cloned();
        let b = child(children, id_of(c.else_expr())).cloned();
        let id = node.id();
        let (Some(condition), Some(a), Some(b)) = (condition, a, b) else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(id, t));
        };
        let boolean = self.primitive(Primitive::Boolean);
        self.record_conversion(&condition, Context::Numeric, boolean)?;
        if let Some(why) = self.non_answer(a.ty).or(self.non_answer(b.ty)) {
            let t = self.unknown(why);
            let (operands, operand_depth) = Self::operands_of(node, &a, &b);
            return Ok(Value {
                operands,
                operand_depth,
                ..self.expr(id, t)
            });
        }
        let pa = self.primitive_or_unboxed(a.ty)?;
        let pb = self.primitive_or_unboxed(b.ty)?;
        let mut operands = None;
        let mut operand_depth = 0;
        let t = match (pa, pb) {
            (Some(Primitive::Boolean), Some(Primitive::Boolean)) => {
                let both_boxed = !matches!(self.cx.get(a.ty), Type::Primitive(_))
                    && !matches!(self.cx.get(b.ty), Type::Primitive(_));
                if both_boxed && a.ty == b.ty {
                    a.ty
                } else {
                    boolean
                }
            }
            (Some(x), Some(y)) if x != Primitive::Boolean && y != Primitive::Boolean => {
                self.numeric_conditional(&a, &b, x, y)?
            }
            _ => {
                if Self::poly_context(node) {
                    (operands, operand_depth) = Self::operands_of(node, &a, &b);
                    self.unknown(Why::LeftToB3c)
                } else {
                    self.reference_conditional(&a, &b)?
                }
            }
        };
        if !matches!(self.cx.get(t), Type::Unknown(_) | Type::Error(_)) {
            for v in [&a, &b] {
                self.record_conversion(v, Context::Assignment, t)?;
            }
        }
        let constant = match (&condition.constant, &a.constant, &b.constant) {
            (Some(Constant::Boolean(choice)), Some(x), Some(y)) => {
                let chosen = if *choice { x } else { y };
                match self.cx.get(t) {
                    Type::Primitive(p) => chosen.convert(p),
                    _ if self.is_string(t)? => Some(chosen.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        Ok(Value {
            constant,
            operands,
            operand_depth,
            ..self.expr(id, t)
        })
    }

    /// What a conditional in an assignment or invocation context keeps of its operands, for a
    /// call's applicability; not without bound, since dropping them recurses.
    fn operands_of(node: SyntaxNode<'_>, a: &Value, b: &Value) -> (Option<Rc<[Value]>>, u8) {
        let depth = a.operand_depth.max(b.operand_depth).saturating_add(1);
        if depth > MAX_OPERAND_DEPTH || !Self::poly_context(node) {
            return (None, depth);
        }
        (Some(Rc::from([a.clone(), b.clone()])), depth)
    }

    fn primitive_or_unboxed(&mut self, t: TypeId) -> Result<Option<Primitive>, Cancelled> {
        Ok(match self.cx.get(t) {
            Type::Primitive(p) => Some(p),
            _ => self.cx.unboxed_primitive(t)?,
        })
    }

    /// A numeric conditional's type (JLS 15.25.2).
    fn numeric_conditional(
        &mut self,
        a: &Value,
        b: &Value,
        x: Primitive,
        y: Primitive,
    ) -> Result<TypeId, Cancelled> {
        use Primitive::*;
        if a.ty == b.ty {
            return Ok(a.ty);
        }
        let (a_primitive, b_primitive) = (
            matches!(self.cx.get(a.ty), Type::Primitive(_)),
            matches!(self.cx.get(b.ty), Type::Primitive(_)),
        );
        if x == y {
            // One primitive and its box: the primitive.
            return Ok(self.primitive(x));
        }
        if matches!((x, y), (Byte, Short) | (Short, Byte)) {
            return Ok(self.primitive(Short));
        }
        // A `byte`, `short`, or `char` and an `int` constant that fits.
        let fits = |c: &Option<Constant>, ty: TypeId, w: &Self, p: Primitive| -> bool {
            matches!(w.cx.get(ty), Type::Primitive(Int))
                && c.as_ref()
                    .and_then(Constant::int_value)
                    .is_some_and(|v| match p {
                        Byte => i8::try_from(v).is_ok(),
                        Short => i16::try_from(v).is_ok(),
                        Char => u16::try_from(v).is_ok(),
                        _ => false,
                    })
        };
        if matches!(x, Byte | Short | Char) && fits(&b.constant, b.ty, self, x) {
            let _ = a_primitive;
            return Ok(self.primitive(x));
        }
        if matches!(y, Byte | Short | Char) && fits(&a.constant, a.ty, self, y) {
            let _ = b_primitive;
            return Ok(self.primitive(y));
        }
        Ok(match self.cx.binary_promotion(a.ty, b.ty)? {
            Tri::Yes(t) => t,
            Tri::No => self.unknown(Why::Later),
            Tri::Unknown(why) => self.unknown(why),
        })
    }

    /// A standalone reference conditional: the `lub` of its operands' types, boxed, with `null`
    /// below every other (JLS 15.25.3).
    fn reference_conditional(&mut self, a: &Value, b: &Value) -> Result<TypeId, Cancelled> {
        let mut operands = Vec::new();
        for v in [a, b] {
            let t = match self.cx.get(v.ty) {
                Type::Primitive(p) => self.cx.boxed(p)?,
                Type::Null => continue,
                _ => v.ty,
            };
            operands.push(t);
        }
        Ok(match operands.as_slice() {
            [] => self.cx.arena_mut().intern(Type::Null),
            [one] => *one,
            [x, y] if x == y => *x,
            _ => self.cx.lub(&operands)?,
        })
    }

    fn assign(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let e = ast::AssignExpr::cast(node).unwrap();
        let target = child(children, id_of(e.target())).cloned();
        let value = child(children, id_of(e.value())).cloned();
        let id = node.id();
        let (Some(target), Some(value), Ok(op)) = (target, value, assign_op(node)) else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(id, t));
        };
        let variable = target.variable.unwrap_or(target.ty);
        if let Some(why) = self.non_answer(variable) {
            let t = self.unknown(why);
            return Ok(self.expr(id, t));
        }
        // The assignment's type is the variable's, captured (JLS 15.26; observed `xs = null`).
        let t = self.cx.fresh_capture(variable)?;
        match op {
            None => {
                self.record_conversion(&value, Context::Assignment, variable)?;
            }
            Some(op) => {
                // `a op= b` is `a = (T) (a op b)` (JLS 15.26.2).
                let read = Value {
                    ty: variable,
                    ..target.clone()
                };
                let result = self.binary_type(op, &read, &value, true)?;
                if self.non_answer(result).is_none()
                    && let Tri::Yes(steps) =
                        self.convert(Context::Casting, result, variable, None)?
                    && steps.as_slice() != [Conversion::Identity]
                {
                    self.compound.push((id, steps));
                }
            }
        }
        Ok(self.expr(id, t))
    }

    fn cast(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<Value, Cancelled> {
        let e = ast::CastExpr::cast(node).unwrap();
        let operand = child(children, id_of(e.expr())).cloned();
        let target = match e.ty() {
            Some(t) => self.syntax_type(t)?,
            None => self.unknown(Why::Later),
        };
        let id = node.id();
        let Some(operand) = operand else {
            let t = self.unknown(Why::Later);
            return Ok(self.expr(id, t));
        };
        if self.non_answer(operand.ty).is_some() && operand.what == What::Expr {
            // A lambda or method reference cast to its target is B3c's; its cast still has the
            // target type.
        } else {
            self.record_conversion(&operand, Context::Casting, target)?;
        }
        // The cast's type is the target, captured (JLS 15.16; observed `(List<?>) o`).
        let t = self.cx.fresh_capture(target)?;
        let constant = match (&operand.constant, self.cx.get(target)) {
            (Some(c), Type::Primitive(p)) => c.convert(p),
            (Some(c @ Constant::String(_)), _) if self.is_string(target)? => Some(c.clone()),
            _ => None,
        };
        Ok(Value {
            constant,
            ..self.expr(id, t)
        })
    }

    // -------------------------------------------------------------------------------------------
    // Declarations and statements.

    fn statement(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<(), Cancelled> {
        match node.kind() {
            SyntaxKind::EnumConstant => self.enum_constant(node, children)?,
            SyntaxKind::Block => self.implicit_super_in_body(node)?,
            kind if is_type_decl(kind) => self.default_constructor(node)?,
            SyntaxKind::VariableDeclarator => self.declarator(node, children)?,
            SyntaxKind::FormalParameter => {
                let p = ast::FormalParameter::cast(node).unwrap();
                let mut t = match p.ty() {
                    Some(t) => self.syntax_type(t)?,
                    None => self.unknown(Why::Later),
                };
                for _ in 0..p.dims().count() + usize::from(p.varargs().is_some()) {
                    t = self.cx.arena_mut().array(t);
                }
                self.declare(node.id(), t, None);
            }
            SyntaxKind::LambdaParameter => {
                let p = ast::LambdaParameter::cast(node).unwrap();
                let t = match p.ty() {
                    Some(ty) if !self.is_var(ty) => {
                        let mut t = self.syntax_type(ty)?;
                        for _ in 0..p.dims().count() + usize::from(p.varargs().is_some()) {
                            t = self.cx.arena_mut().array(t);
                        }
                        t
                    }
                    // An implicitly typed parameter, or `var`: inference's.
                    _ => self.unknown(Why::LeftToB3c),
                };
                self.declare(node.id(), t, None);
            }
            SyntaxKind::CatchParameter => {
                let p = ast::CatchParameter::cast(node).unwrap();
                let mut t = match p.ty() {
                    Some(t) => self.syntax_type(t)?,
                    None => self.unknown(Why::Later),
                };
                for _ in p.dims() {
                    t = self.cx.arena_mut().array(t);
                }
                // A multi-catch parameter's type is the `lub` of its alternatives, and javac
                // records the union (observed:
                // `java/lang/InterruptedException|java/lang/IllegalStateException`).
                let shown = match p.ty() {
                    Some(ast::Type::UnionType(u)) => {
                        let mut parts = Vec::new();
                        for alternative in u.types() {
                            parts.push(self.syntax_type(alternative)?);
                        }
                        Some(self.cx.arena_mut().union(&parts))
                    }
                    _ => None,
                };
                self.locals.insert(
                    node.id(),
                    Local {
                        ty: t,
                        constant: None,
                        shown,
                    },
                );
            }
            SyntaxKind::TypePattern => {
                let p = ast::TypePattern::cast(node).unwrap();
                let t = match p.ty() {
                    Some(ty) if !self.is_var(ty) => self.syntax_type(ty)?,
                    _ => self.unknown(Why::LeftToB3d),
                };
                self.declare(node.id(), t, None);
            }
            SyntaxKind::RecordComponent => {
                let c = ast::RecordComponent::cast(node).unwrap();
                let mut t = match c.ty() {
                    Some(t) => self.syntax_type(t)?,
                    None => self.unknown(Why::Later),
                };
                if c.varargs().is_some() {
                    t = self.cx.arena_mut().array(t);
                }
                self.declare(node.id(), t, None);
            }
            // A dimension's size is promoted to `int` (JLS 15.10.1).
            SyntaxKind::DimExpr => {
                let size = id_of(ast::DimExpr::cast(node).unwrap().size());
                if let Some(v) = child(children, size).cloned() {
                    let int = self.primitive(Primitive::Int);
                    self.record_conversion(&v, Context::Numeric, int)?;
                }
            }
            // A condition converts to `boolean`: an unboxing, for a `Boolean`.
            SyntaxKind::IfStmt
            | SyntaxKind::WhileStmt
            | SyntaxKind::DoStmt
            | SyntaxKind::ForStmt
            | SyntaxKind::AssertStmt => {
                let condition = match node.kind() {
                    SyntaxKind::IfStmt => ast::IfStmt::cast(node).unwrap().condition(),
                    SyntaxKind::WhileStmt => ast::WhileStmt::cast(node).unwrap().condition(),
                    SyntaxKind::DoStmt => ast::DoStmt::cast(node).unwrap().condition(),
                    SyntaxKind::ForStmt => ast::ForStmt::cast(node).unwrap().condition(),
                    _ => ast::AssertStmt::cast(node).unwrap().condition(),
                };
                if let Some(v) = child(children, id_of(condition)).cloned() {
                    let boolean = self.primitive(Primitive::Boolean);
                    self.record_conversion(&v, Context::Numeric, boolean)?;
                }
            }
            SyntaxKind::ReturnStmt => {
                let value = id_of(ast::ReturnStmt::cast(node).unwrap().value());
                let result = self.results.last().and_then(|r| r.1);
                if let (Some(v), Some(result)) = (child(children, value).cloned(), result) {
                    self.record_conversion(&v, Context::Assignment, result)?;
                }
            }
            SyntaxKind::ElementValuePair
            | SyntaxKind::AnnotationArguments
            | SyntaxKind::DefaultValue => {
                // A single element value's conversion to its element's type.
                let value = match node.kind() {
                    SyntaxKind::ElementValuePair => {
                        ast::ElementValuePair::cast(node).unwrap().value()
                    }
                    SyntaxKind::AnnotationArguments => {
                        ast::AnnotationArguments::cast(node).unwrap().value()
                    }
                    _ => node.children().find_map(ast::ElementValue::cast),
                };
                if let Some(ast::ElementValue::Expr(e)) = value
                    && let Some(v) = child(children, Some(e.syntax().id())).cloned()
                    && let Some(element) = self.element_type_of(e.syntax())?
                {
                    let target = match self.cx.get(element) {
                        // A single value for an array element is its one element.
                        Type::Array(c) => c,
                        _ => element,
                    };
                    self.record_conversion(&v, Context::Assignment, target)?;
                }
            }
            SyntaxKind::SwitchLabel => {
                let label = ast::SwitchLabel::cast(node).unwrap();
                let selector = self.selectors.last().map(|s| s.1);
                if let Some(selector) = selector
                    && matches!(self.cx.get(selector), Type::Primitive(_))
                {
                    for element in label.elements() {
                        if let ast::CaseElement::Expr(e) = element
                            && let Some(v) = child(children, Some(e.syntax().id())).cloned()
                        {
                            self.record_conversion(&v, Context::Assignment, selector)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn declare(&mut self, node: NodeId, ty: TypeId, constant: Option<Constant>) {
        self.locals.insert(
            node,
            Local {
                ty,
                constant,
                shown: None,
            },
        );
    }

    /// A variable declarator: a local's type (`var` from its initializer's upward projection,
    /// or for an enhanced `for` its expression's element), and its value if it is a constant
    /// variable (JLS 4.12.4); the initializer's conversion; a local class's constant field.
    fn declarator(&mut self, node: SyntaxNode<'_>, children: &[Value]) -> Result<(), Cancelled> {
        let d = ast::VariableDeclarator::cast(node).unwrap();
        let init = match d.initializer() {
            Some(ast::VariableInitializer::Expr(e)) => {
                child(children, Some(e.syntax().id())).cloned()
            }
            Some(ast::VariableInitializer::ArrayInitializer(a)) => {
                child(children, Some(a.syntax().id())).cloned()
            }
            None => None,
        };
        let declared = self.declarator_type(node)?;
        let Some(parent) = node.parent() else {
            return Ok(());
        };
        let modifiers_of = |p: SyntaxNode<'_>, names: &Interner| -> Modifiers {
            let m = match p.kind() {
                SyntaxKind::LocalVarDecl => ast::LocalVarDecl::cast(p).unwrap().modifiers(),
                SyntaxKind::FieldDecl => ast::FieldDecl::cast(p).unwrap().modifiers(),
                _ => None,
            };
            modifiers(m, names).0
        };
        let is_final = modifiers_of(parent, &self.names).contains(Modifiers::FINAL);
        let ty = match (declared, &init) {
            (Some(t), Some(v)) => {
                self.record_conversion(v, Context::Assignment, t)?;
                t
            }
            (Some(t), None) => t,
            (None, Some(v)) => {
                // `var` (JLS 14.4.1).
                if let Some(why) = self.non_answer(v.ty) {
                    self.unknown(why)
                } else {
                    self.cx.upward_projection(v.ty)?
                }
            }
            (None, None) => {
                // An enhanced `for`'s `var`, typed when its expression is left.
                if let Some(statement) = parent
                    .parent()
                    .filter(|s| s.kind() == SyntaxKind::ForEachStmt)
                {
                    self.for_each.insert(statement.id(), node.id());
                }
                self.unknown(Why::Later)
            }
        };
        // A constant variable: final, of a primitive type or `String`, initialized with a
        // constant expression, converted by assignment to its type.
        let constant = match &init {
            Some(v) if is_final => match (&v.constant, self.cx.get(ty)) {
                (Some(c), Type::Primitive(p)) => {
                    let fits = matches!(
                        self.cx.convert(Context::Assignment, v.ty, ty, Some(c))?,
                        Tri::Yes(_)
                    );
                    if fits { c.convert(p) } else { None }
                }
                (Some(c @ Constant::String(_)), _) if self.is_string(ty)? => Some(c.clone()),
                _ => None,
            },
            _ => None,
        };
        match parent.kind() {
            SyntaxKind::LocalVarDecl => self.declare(node.id(), ty, constant),
            SyntaxKind::FieldDecl => {
                // A local class's field, whose constant the walk keeps for its uses.
                if let Some(c) = constant
                    && let Some(class) = self.classes.last().map(|c| c.key)
                    && let TypeKey::Local { .. } = class
                    && let Some(name) = d.name()
                {
                    let name = crate::summary::name_text(name, &self.names);
                    if let Some(field) = self.field_ref(class, name)? {
                        self.local_constants.insert(field, c);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// A local or anonymous class's member table, made with a context of its own, its supertypes'
/// from the shared entries or, for local ones, from `tables` (where `None` marks one being
/// made, so that a cycle of local classes ends).
fn local_table<'s>(
    snapshot: &'s Snapshot,
    view: SourceSetId,
    reads: &mut Reads<'s>,
    tables: &mut Map<TypeKey, Option<Arc<Members>>>,
    outer_superclasses: &Map<TypeKey, TypeKey>,
    key: TypeKey,
) -> Result<Option<Arc<Members>>, Cancelled> {
    if let Some(table) = tables.get(&key) {
        return Ok(table.clone());
    }
    tables.insert(key, None);
    let mut cx = snapshot.recording_types(view);
    // Only the class matters here: the walk sees the methods through the superclass's arguments.
    if let Some(&superclass) = outer_superclasses.get(&key) {
        let t = cx.class_type(superclass, None, None);
        cx.set_supertypes(key, &[t]);
    }
    let mut supers =
        |reads: &mut Reads<'s>, k: TypeKey| -> Result<Option<Arc<Members>>, Cancelled> {
            match k {
                TypeKey::Local { .. } => ferrion_stack::ensure_stack(|| {
                    local_table(snapshot, view, reads, tables, outer_superclasses, k)
                }),
                _ => reads.members(view, k),
            }
        };
    let table = Arc::new(members::compute(&mut cx, reads, key, &mut supers)?);
    reads.extend(cx.take_reads());
    tables.insert(key, Some(table.clone()));
    Ok(Some(table))
}
