//! Names inside bodies: every name in method, constructor, initializer, lambda, and field
//! initializer bodies, enum constants' arguments and bodies, annotation arguments, and the
//! local and anonymous classes they declare, resolved and keyed by `NodeId` of the file's
//! current tree (docs/design/semantic-names.md#names-in-bodies).
//!
//! A file's body result is a derived entry, like its names in declarations: it reads the file's
//! tree and scope, and project symbols only through [`Reads`], so a change inside a body reaches
//! only its own file's result, and a new type only the results that look up its name.
//!
//! The walk is the tree's flat pre-order walk, with an explicit stack of scopes, so that deep
//! nesting never recurses on the native stack. Each scope's declarations are bindings in maps by
//! name, innermost last, so a lookup costs one map read however many scopes enclose it: locals,
//! parameters, and pattern bindings; local classes and type parameters; and the fields, member
//! types, and methods of each local or anonymous class around the name, bound when its body is
//! entered. What the bindings don't have, the summary's class around the body and its outer
//! classes answer, then the imports, as in declarations ([`crate::resolve`]).
//!
//! Expression names are classified as each node is left (JLS 6.5.2), so a qualifier's meaning
//! is known when the name it qualifies is; a name's context (a type, a package or type, an
//! ambiguous name, an expression) is set as each node is entered. Pattern bindings follow JLS
//! 6.3.1 and 6.3.2: what an expression introduces when true and when false is worked out as it
//! is left, and bound in the operands, branches, and statements the rules name.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use ferrion_syntax::ast::{self, AstNode};
use ferrion_syntax::{NodeId, SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree, WalkEvent};

use crate::decl_names::{FileNames, ResolvedName, Symbol};
use crate::engine::derived::{Memo, Reads};
use crate::resolve::{
    BodyTypes, FileScope, MAX_SEGMENTS, MemberLookup, Resolver, Scope, members_of, package_of,
    type_params,
};
use crate::summary::{
    Header, ImportKind, MethodKind, Modifiers, QualifiedName, TypeKind, TypeRef, TypeSummary,
    anonymous_type, local_type, modifiers, name_text,
};
use crate::symbols::{
    Cause, ErrorSymbol, LeftToB3, MemberAccess, MemberType, MemberTypes, MethodRef, Supertypes, Ty,
    TyPart, TypeEntry, TypeKey, TypeResolution, TypeVarOwner,
};
use crate::view::Found;
use crate::{Cancelled, FileId, Interner, Name, Snapshot, SourceSetId};

/// A hasher for the walk's own maps, whose keys are ids it made (nodes, names, type keys): a
/// rotate, xor, and multiply per word. SipHash, the default, was a fifth of the walk's time, and
/// its resistance to chosen keys isn't needed for maps that last one computation.
#[derive(Clone, Copy, Default)]
pub(crate) struct IdHasher(u64);

impl IdHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(u64::from(n));
    }

    fn write_u16(&mut self, n: u16) {
        self.add(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.add(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

pub(crate) type Map<K, V> = HashMap<K, V, BuildHasherDefault<IdHasher>>;
pub(crate) type Set<K> = HashSet<K, BuildHasherDefault<IdHasher>>;

/// A file's names inside bodies, and the local and anonymous classes they declare.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct BodyNames {
    /// In the order of their nodes.
    pub names: Box<[ResolvedName]>,
    /// By [`TypeKey::Local`]'s index, in the order the walk found them.
    pub local_types: Box<[LocalType]>,
}

/// A local or anonymous class, or a member type of one, with what a summary's type has in its
/// entries, kept in its file's body result.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LocalType {
    /// The type declaration, or for an anonymous class its body.
    pub node: NodeId,
    /// The offset of its `class` (or `interface`, `enum`, `record`) keyword, or for an
    /// anonymous class of its body's `{` or its enum constant's name: where its key says it is.
    pub at: u32,
    pub entry: Arc<TypeEntry>,
    pub supertypes: Arc<Supertypes>,
    pub member_types: Arc<MemberTypes>,
    /// The class whose body declares it: what an unqualified inner class of a generic class is
    /// a member of, inside it.
    pub lexical: Option<TypeKey>,
}

impl Snapshot {
    /// The names inside `file`'s bodies, by node of its current tree ([`Snapshot::tree`]).
    pub fn body_names(&self, file: FileId) -> Result<Arc<BodyNames>, Cancelled> {
        self.check()?;
        Ok(self.body_memo(file)?.value.clone())
    }

    pub(crate) fn body_memo(&self, file: FileId) -> Result<Arc<Memo<BodyNames>>, Cancelled> {
        self.shared()
            .body_names
            .get(self, &file, |reads| compute(reads, file))
    }

    /// Every name in `file`, in its declarations ([`Snapshot::decl_names`]) and in its bodies
    /// ([`Snapshot::body_names`]), by node of its current tree, with ranges.
    pub fn file_names(&self, file: FileId) -> Result<Arc<FileNames>, Cancelled> {
        let decl = self.decl_names(file)?;
        let body = self.body_names(file)?;
        let mut names = Vec::with_capacity(decl.names.len() + body.names.len());
        let (mut a, mut b) = (decl.names.iter().peekable(), body.names.iter().peekable());
        loop {
            let next = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) if x.node <= y.node => {
                    // A node both have is the declaration's.
                    if x.node == y.node {
                        b.next();
                    }
                    a.next()
                }
                (Some(_), None) => a.next(),
                (_, Some(_)) => b.next(),
                (None, None) => break,
            };
            names.extend(next.cloned());
        }
        Ok(Arc::new(FileNames {
            names: names.into_boxed_slice(),
        }))
    }

    /// A local or anonymous class, from its file's body result.
    pub(crate) fn local_type(&self, key: TypeKey) -> Result<Option<LocalType>, Cancelled> {
        let TypeKey::Local { file, index } = key else {
            return Ok(None);
        };
        if !self.file_exists(file)? {
            return Ok(None);
        }
        Ok(self
            .body_names(file)?
            .local_types
            .get(index as usize)
            .cloned())
    }
}

/// Where a local class, or a member type of one, is for its key: its `class`, `interface`,
/// `enum`, or `record` keyword (or the `@` of `@interface`), after its modifiers, which is where
/// javac's tree for it is.
fn declaration_at(decl: SyntaxNode<'_>) -> u32 {
    decl.direct_tokens()
        .find(|t| !t.is_trivia())
        .map_or(decl.span().start, |t| t.span().start)
}

/// Where a compact constructor's parameter for `component` is for its key: where javac's tree
/// for the component's type is, since javac makes the parameter from the component (observed:
/// `String` in `@A String a`, the last `.` of `java.lang.String`, the `<` of `List<String>`, the
/// first `[` or its annotation in an array type, and the `...` of `String... f`).
pub fn component_type_at(component: ast::RecordComponent<'_>) -> u32 {
    if let Some(varargs) = component.varargs() {
        return varargs.syntax().span().start;
    }
    let Some(ty) = component.ty() else {
        return component.syntax().span().start;
    };
    let ty = ty.syntax();
    let child = |kind: SyntaxKind| ty.children().find(|c| c.kind() == kind);
    match ty.kind() {
        SyntaxKind::ClassType => {
            if let Some(arguments) = child(SyntaxKind::TypeArguments) {
                return arguments.span().start;
            }
            if let Some(annotation) = child(SyntaxKind::Annotation) {
                return annotation.span().start;
            }
            let tokens = || ty.direct_tokens().filter(|t| !t.is_trivia());
            if child(SyntaxKind::ClassType).is_some()
                && let Some(dot) = tokens().find(|t| t.kind() == SyntaxKind::Dot)
            {
                return dot.span().start;
            }
            tokens()
                .filter(|t| matches!(t.kind(), SyntaxKind::Identifier | SyntaxKind::Underscore))
                .last()
                .map_or(ty.span().start, |t| t.span().start)
        }
        SyntaxKind::ArrayType => child(SyntaxKind::Dim).map_or(ty.span().start, |d| d.span().start),
        _ => ty.span().start,
    }
}

/// What a variable's declared type says about its members.
#[derive(Clone, PartialEq, Eq, Debug)]
enum VarTy {
    Class(TypeKey),
    TypeVar,
    Array,
    Primitive,
    /// Needs attribution: `var`, an implicitly typed lambda parameter, a union.
    Unknown,
    Error(ErrorSymbol),
}

/// A local variable, parameter, exception parameter, resource, or pattern binding.
struct LocalVar {
    node: NodeId,
    name: Name,
    at: u32,
    ty: VarTy,
}

/// A field found by name in a type: declared there or inherited.
#[derive(Clone, PartialEq, Eq, Debug)]
enum FieldFound {
    /// The `index`th field of `owner`'s entry.
    One(TypeKey, u32),
    /// Inherited from two or more supertypes as different fields.
    Ambiguous(Box<[TypeKey]>),
}

/// A type's fields and methods by name, declared and inherited (JLS 8.3, 8.4.8, 9.3, 9.4.1): a
/// declared field hides every inherited one of its name, and private members, package-access
/// ones in another package, and an interface's static methods aren't inherited. Methods aren't
/// chosen among, so every method of the name is kept, overridden ones too.
#[derive(Default)]
struct ClassMaps {
    fields: Map<Name, (FieldFound, MemberAccess)>,
    methods: Map<Name, Vec<(MethodRef, MemberAccess)>>,
    /// A supertype didn't resolve, or is cyclic, or is incomplete itself.
    incomplete: bool,
}

/// A local or anonymous class as the walk builds it.
struct LocalState {
    key: TypeKey,
    node: NodeId,
    at: u32,
    kind: TypeKind,
    access: MemberAccess,
    /// The member types it declares, by name.
    declared: Map<Name, u32>,
    summary: Option<TypeSummary>,
    /// Set when its body is entered.
    supertypes: Option<Arc<Supertypes>>,
    member_types: Option<Arc<MemberTypes>>,
    entry: Option<Arc<TypeEntry>>,
    maps: Option<Arc<ClassMaps>>,
    /// Methods and constructors entered so far, for their type parameters' owners.
    methods: u32,
    /// The class whose body declares it.
    lexical: Option<TypeKey>,
}

/// What a name binds in a scope.
#[derive(Clone)]
enum Var {
    Local(u32),
    Field(FieldFound),
}

/// One scope: the node that opened it, which closes it when left, and the names it bound.
struct Frame {
    owner: NodeId,
    vars: Vec<Name>,
    types: Vec<Name>,
    methods: Vec<Name>,
}

/// A type to search for a method name, and its methods of the name.
type Candidates = (TypeKey, Arc<[MethodRef]>);

/// The scopes around the walk's position, and what the body has declared so far.
struct Env {
    frames: Vec<Frame>,
    vars: Map<Name, Vec<Var>>,
    types: Map<Name, Vec<TypeResolution>>,
    /// The methods of a name of each local or anonymous class around, with it as the type to
    /// search.
    methods: Map<Name, Vec<Candidates>>,
    locals: Vec<LocalVar>,
    local_types: Vec<LocalState>,
    /// Each local type's declaration, and its body once entered, by node.
    local_of_node: Map<NodeId, u32>,
    /// The file's package, which its local classes are in.
    package: Name,
}

impl Env {
    fn push(&mut self, owner: NodeId) {
        self.frames.push(Frame {
            owner,
            vars: Vec::new(),
            types: Vec::new(),
            methods: Vec::new(),
        });
    }

    /// Closes every scope `owner` opened.
    fn pop(&mut self, owner: NodeId) {
        while self.frames.last().is_some_and(|f| f.owner == owner) {
            let frame = self.frames.pop().unwrap();
            self.unbind(&frame);
        }
    }

    fn unbind(&mut self, frame: &Frame) {
        for name in &frame.vars {
            if let Some(v) = self.vars.get_mut(name) {
                v.pop();
            }
        }
        for name in &frame.types {
            if let Some(v) = self.types.get_mut(name) {
                v.pop();
            }
        }
        for name in &frame.methods {
            if let Some(v) = self.methods.get_mut(name) {
                v.pop();
            }
        }
    }

    fn bind_var(&mut self, name: Name, var: Var) {
        let Some(frame) = self.frames.last_mut().filter(|_| !name.is_empty()) else {
            return;
        };
        frame.vars.push(name);
        self.vars.entry(name).or_default().push(var);
    }

    fn bind_type(&mut self, name: Name, ty: TypeResolution) {
        let Some(frame) = self.frames.last_mut().filter(|_| !name.is_empty()) else {
            return;
        };
        frame.types.push(name);
        self.types.entry(name).or_default().push(ty);
    }

    fn bind_methods(&mut self, name: Name, search: TypeKey, methods: Arc<[MethodRef]>) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        frame.methods.push(name);
        self.methods
            .entry(name)
            .or_default()
            .push((search, methods));
    }

    fn state(&self, key: TypeKey) -> Option<&LocalState> {
        match key {
            TypeKey::Local { index, .. } => self.local_types.get(index as usize),
            _ => None,
        }
    }
}

impl BodyTypes for Env {
    fn simple(
        &self,
        _r: &mut Resolver<'_, '_>,
        name: Name,
    ) -> Result<Option<TypeResolution>, Cancelled> {
        Ok(self.types.get(&name).and_then(|v| v.last()).cloned())
    }

    fn member_type(
        &self,
        _r: &mut Resolver<'_, '_>,
        owner: TypeKey,
        name: Name,
    ) -> Result<MemberLookup, Cancelled> {
        let Some(state) = self.state(owner) else {
            return Ok(MemberLookup::None { incomplete: true });
        };
        if let Some(members) = &state.member_types {
            return Ok(match members.get(name) {
                Some((MemberType::One(key), _)) => MemberLookup::Found(*key),
                Some((MemberType::Ambiguous(keys), _)) => MemberLookup::Ambiguous(keys.clone()),
                None => MemberLookup::None {
                    incomplete: members.incomplete,
                },
            });
        }
        // Named before its body is entered: what it declares is known, not what it inherits.
        Ok(match state.declared.get(&name) {
            Some(&index) => MemberLookup::Found(self.local_types[index as usize].key),
            None => MemberLookup::None { incomplete: true },
        })
    }
}

/// The JLS 6.5.1 context of a simple or qualified name in an expression: what it can be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Context {
    /// A variable.
    Expr,
    /// A variable, else a type, else a package (JLS 6.5.2).
    Ambiguous,
    /// A type, else a package.
    PackageOrType,
    /// A type.
    Type,
    /// A `case` label's constant, where the selector's type is this enum.
    Case(TypeKey),
    /// A `case` label's constant, where the selector's type needs attribution: a variable in
    /// scope that could be a constant of the label, or B3's.
    CaseOfUnknown,
}

/// What a switch's selector says about its `case` labels' simple names (JLS 14.11.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Selector {
    /// An enum, whose constants they are.
    Enum(TypeKey),
    /// Another type: they are constant variables in scope.
    Other,
    /// A type that needs attribution.
    Unknown,
}

/// What a qualifier turned out to be.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Meaning {
    /// A package, with how many segments it was written with.
    Package(Name, u32),
    Type(TypeKey),
    TypeVar,
    /// A variable: a value of a declared type.
    Value(VarTy),
    /// `this` or `Q.this`, whose members are its class's.
    This(TypeKey),
    /// `super` or `Q.super`: the type whose members are searched.
    Super(TypeKey),
    Error(ErrorSymbol),
    /// Needs attribution.
    Unknown,
}

/// A body the walk is in, and the summary's class around it.
struct Region {
    node: NodeId,
    /// The summary's index of the innermost type around the body, if any.
    summary: Option<usize>,
    /// Annotation arguments of a declaration, whose annotation names are the declaration's.
    expressions_only: bool,
}

/// A type of the summary the walk is in, outside bodies.
struct SummaryType {
    node: NodeId,
    index: usize,
    /// Methods and constructors entered so far, for their type parameters' owners.
    methods: u32,
}

/// A statement `break` can leave, with its label if it is a labeled statement.
struct Breakable {
    node: NodeId,
    label: Option<Name>,
}

struct Walker<'f, 'r, 's> {
    r: Resolver<'r, 's>,
    env: Env,
    fs: &'f FileScope,
    file: FileId,
    set: SourceSetId,
    names: Arc<Interner>,
    release: u32,
    out: Vec<ResolvedName>,
    /// Each qualifier's meaning, until the name it qualifies is left.
    meanings: Map<NodeId, Meaning>,
    /// The context of each `FieldAccessExpr` being walked.
    contexts: Map<NodeId, Context>,
    /// What each class type in a body resolved to: its name's last segment.
    type_of: Map<NodeId, TypeResolution>,
    /// `var` where a type would be, which isn't a type name (JLS 14.4).
    var_types: Set<NodeId>,
    /// The pattern bindings an expression introduces when true and when false.
    bindings: Map<NodeId, (Vec<u32>, Vec<u32>)>,
    /// Pattern bindings declared and not yet claimed by their `instanceof` or `case` label.
    patterns: Vec<u32>,
    /// Where each open `instanceof`'s and `case` label's bindings start in `patterns`.
    pattern_marks: Map<NodeId, usize>,
    /// Each `case` label's bindings, with its guard's when true.
    label_bindings: Map<NodeId, Vec<u32>>,
    /// The switch groups whose labels' bindings are bound.
    open_groups: Set<NodeId>,
    /// The local an enhanced `for` declares, bound when its body is entered.
    for_each: Map<NodeId, u32>,
    /// Statements that can't complete normally (JLS 14.22), as far as bindings need.
    abrupt: Set<NodeId>,
    /// Statements a `break` leaves.
    broken: Set<NodeId>,
    breakable: Vec<Breakable>,
    /// The pattern bindings a statement introduces after it (JLS 6.3.2).
    introduced: Map<NodeId, Vec<u32>>,
    /// Each open switch block, with what its selector is.
    switch_enums: Vec<(NodeId, Selector)>,
    regions: Vec<Region>,
    summary_types: Vec<SummaryType>,
    /// The summary's next type index; the walk meets types in the summary's order.
    next_index: usize,
    /// The summary's method or constructor being walked, as its type parameters' owner.
    member_method: Option<(NodeId, TypeVarOwner)>,
    /// Each type's fields and methods, of every name or of one, made so far.
    class_maps: Map<(TypeKey, Option<Name>), Arc<ClassMaps>>,
    /// Each summary type's or class file's direct supertypes, as read.
    supertypes: Map<TypeKey, (Vec<TypeKey>, bool)>,
    /// Each type's methods of a name, as a call's candidates.
    candidates: Map<(TypeKey, Name), Arc<[MethodRef]>>,
    /// The local and anonymous classes whose bodies the walk is in, innermost last.
    classes: Vec<(NodeId, TypeKey)>,
    next_check: usize,
}

/// A scope for a name in a body: the body's declarations, then the summary's class around it.
macro_rules! scope {
    ($w:expr) => {
        Scope {
            file: $w.fs,
            innermost: $w.regions.last().and_then(|r| r.summary),
            header: false,
            method: None,
            body: Some(&$w.env),
        }
    };
}

fn compute(reads: &mut Reads<'_>, file: FileId) -> Result<BodyNames, Cancelled> {
    let (tree, _) = reads.tree(file)?;
    walk_names(reads, file, &tree, None)
}

/// The names in one field initializer of `file`'s `tree`, `target`, alone, as the whole file's
/// walk resolves them: for a constant's value, which reads the file's summary and not its text
/// (docs/design/semantic-attribution.md#constants). Its local classes, which no constant has,
/// aren't numbered as the file's are.
pub(crate) fn initializer_names(
    reads: &mut Reads<'_>,
    file: FileId,
    tree: &SyntaxTree,
    target: NodeId,
    summary: usize,
) -> Result<Box<[ResolvedName]>, Cancelled> {
    Ok(walk_names(reads, file, tree, Some((target, summary)))?.names)
}

/// The walk over the whole file, or with `target` over one field initializer of the summary's
/// type at an index alone.
fn walk_names(
    reads: &mut Reads<'_>,
    file: FileId,
    tree: &SyntaxTree,
    target: Option<(NodeId, usize)>,
) -> Result<BodyNames, Cancelled> {
    let set = reads.snapshot.file(file)?.set;
    let fs = reads.file_scope(file)?;
    let names = reads.snapshot.names().clone();
    let mut w = Walker {
        r: Resolver::new(reads, set),
        env: Env {
            frames: Vec::new(),
            vars: Map::default(),
            types: Map::default(),
            methods: Map::default(),
            locals: Vec::new(),
            local_types: Vec::new(),
            local_of_node: Map::default(),
            package: fs.package(),
        },
        fs: &fs,
        file,
        set,
        release: tree.release(),
        names,
        out: Vec::new(),
        meanings: Map::default(),
        contexts: Map::default(),
        type_of: Map::default(),
        var_types: Set::default(),
        bindings: Map::default(),
        patterns: Vec::new(),
        pattern_marks: Map::default(),
        label_bindings: Map::default(),
        open_groups: Set::default(),
        for_each: Map::default(),
        abrupt: Set::default(),
        broken: Set::default(),
        breakable: Vec::new(),
        introduced: Map::default(),
        switch_enums: Vec::new(),
        regions: Vec::new(),
        summary_types: Vec::new(),
        next_index: 0,
        member_method: None,
        class_maps: Map::default(),
        supertypes: Map::default(),
        candidates: Map::default(),
        classes: Vec::new(),
        next_check: 0,
    };
    match target {
        None => w.walk(tree)?,
        Some((node, summary)) => w.walk_initializer(tree.node(node), summary)?,
    }
    let mut out = std::mem::take(&mut w.out);
    out.sort_by_key(|n| n.node);
    out.dedup_by_key(|n| n.node);
    let local_types = w
        .env
        .local_types
        .into_iter()
        .map(|state| LocalType {
            node: state.node,
            at: state.at,
            entry: state
                .entry
                .unwrap_or_else(|| Arc::new(empty_entry(state.key, state.kind))),
            supertypes: state.supertypes.unwrap_or_else(|| {
                Arc::new(Supertypes {
                    superclass: None,
                    interfaces: Box::default(),
                    cycle: Arc::from([]),
                })
            }),
            member_types: state.member_types.unwrap_or_default(),
            lexical: state.lexical,
        })
        .collect();
    Ok(BodyNames {
        names: out.into_boxed_slice(),
        local_types,
    })
}

/// The entry of a local class whose body the walk never entered, for a syntax error.
fn empty_entry(key: TypeKey, kind: TypeKind) -> TypeEntry {
    TypeEntry {
        key,
        kind,
        modifiers: Modifiers::default(),
        annotations: Box::default(),
        type_parameters: Box::default(),
        superclass: None,
        interfaces: Box::default(),
        permits: Box::default(),
        fields: Box::default(),
        methods: Box::default(),
        incomplete: true,
        lombok: false,
        generated: false,
        errors: Box::default(),
    }
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

/// Whether `child` is the node `role` gives of its parent.
fn is<'t, N: AstNode<'t>>(child: SyntaxNode<'t>, role: Option<N>) -> bool {
    role.is_some_and(|n| n.syntax() == child)
}

fn id_of<'t, N: AstNode<'t>>(node: Option<N>) -> Option<NodeId> {
    node.map(|n| n.syntax().id())
}

/// Whether an expression is the literal `true`.
fn is_true(expr: Option<ast::Expr<'_>>) -> bool {
    match expr {
        Some(ast::Expr::LiteralExpr(e)) => e.syntax().first_token().kind() == SyntaxKind::TrueKw,
        Some(ast::Expr::ParenExpr(e)) => is_true(e.expr()),
        _ => false,
    }
}

/// Who can inherit a member with `modifiers`, declared in `package`.
fn access_of(modifiers: Modifiers, package: Name) -> MemberAccess {
    if modifiers.contains(Modifiers::PRIVATE) {
        MemberAccess::Private
    } else if modifiers.contains(Modifiers::PUBLIC) || modifiers.contains(Modifiers::PROTECTED) {
        MemberAccess::Public
    } else {
        MemberAccess::Package(package)
    }
}

/// Who can inherit a class file's member with access flags `flags`.
fn access_of_flags(flags: u16, package: Name) -> MemberAccess {
    if flags & 0x0002 != 0 {
        MemberAccess::Private
    } else if flags & 0x0005 != 0 {
        MemberAccess::Public
    } else {
        MemberAccess::Package(package)
    }
}

fn inheritable(access: MemberAccess, package: Name) -> bool {
    match access {
        MemberAccess::Public => true,
        MemberAccess::Package(p) => p == package,
        MemberAccess::Private => false,
    }
}

impl Walker<'_, '_, '_> {
    fn walk(&mut self, tree: &SyntaxTree) -> Result<(), Cancelled> {
        let mut events = tree.root().preorder();
        while let Some(event) = events.next() {
            match event {
                WalkEvent::Enter(node) => {
                    if !self.enter(node)? {
                        events.skip_subtree();
                    }
                }
                WalkEvent::Leave(node) => self.leave(node)?,
            }
        }
        Ok(())
    }

    /// Walks a field initializer of the summary's type at `summary` alone, as the whole walk
    /// would: a region starts at it, in that type's scope.
    fn walk_initializer(&mut self, root: SyntaxNode<'_>, summary: usize) -> Result<(), Cancelled> {
        self.start_region(root, Some(summary), false);
        let mut events = root.preorder();
        while let Some(event) = events.next() {
            match event {
                WalkEvent::Enter(node) if node == root => {}
                WalkEvent::Enter(node) => {
                    if !self.enter(node)? {
                        events.skip_subtree();
                    }
                }
                WalkEvent::Leave(node) => self.leave(node)?,
            }
        }
        Ok(())
    }

    /// Polls for cancellation every 1,024 tokens.
    fn poll(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let token = node.first_token().id().index();
        if token >= self.next_check {
            self.r.reads.check()?;
            self.next_check = token + 1024;
        }
        Ok(())
    }

    fn text(&self, token: SyntaxToken<'_>) -> Name {
        name_text(token, &self.names)
    }

    fn join(&self, prefix: Name, segment: Name) -> Name {
        self.names.intern(&format!(
            "{}.{}",
            self.names.text(prefix),
            self.names.text(segment)
        ))
    }

    fn record(&mut self, node: SyntaxNode<'_>, symbol: Symbol) {
        let span = node.span();
        self.record_range(node, span.start, span.end, symbol);
    }

    fn record_range(&mut self, node: SyntaxNode<'_>, start: u32, end: u32, symbol: Symbol) {
        self.out.push(ResolvedName {
            node: node.id(),
            start,
            end,
            symbol,
        });
    }

    fn summary_index(&self) -> Option<usize> {
        self.regions.last().and_then(|r| r.summary)
    }

    /// The summary's class around the body and its outer classes, innermost first.
    fn summary_chain(&self) -> Vec<TypeKey> {
        let mut chain = Vec::new();
        let mut next = self.summary_index();
        while let Some(index) = next {
            chain.push(self.fs.keys[index]);
            next = self.fs.summary.types[index].outer.map(|o| o as usize);
        }
        chain
    }

    /// The class `this` is: the innermost local or anonymous class whose body the walk is in,
    /// else the summary's.
    fn current_class(&self) -> Option<TypeKey> {
        match self.classes.last() {
            Some(&(_, key)) => Some(key),
            None => self.summary_index().map(|i| self.fs.keys[i]),
        }
    }

    // ---------------------------------------------------------------------------------------
    // Outside bodies.

    /// Returns false to skip the node's subtree.
    fn enter(&mut self, node: SyntaxNode<'_>) -> Result<bool, Cancelled> {
        self.poll(node)?;
        if self.regions.is_empty() {
            return self.enter_declarations(node);
        }
        self.enter_body(node)
    }

    /// Follows the summary's types and their methods, and starts a body region where one
    /// starts.
    fn enter_declarations(&mut self, node: SyntaxNode<'_>) -> Result<bool, Cancelled> {
        let kind = node.kind();
        let Some(parent) = node.parent() else {
            // The compilation unit: a compact one's members are the implicit class's.
            if self.fs.is_compact() {
                self.summary_types.push(SummaryType {
                    node: node.id(),
                    index: 0,
                    methods: 0,
                });
                self.next_index = 1;
            }
            return Ok(true);
        };
        let top = self.summary_types.last().map(|t| t.node);
        let member = match parent.kind() {
            SyntaxKind::CompilationUnit => true,
            SyntaxKind::ClassBody | SyntaxKind::EnumBody => {
                parent.parent().is_some_and(|p| Some(p.id()) == top)
            }
            _ => false,
        };
        if is_type_decl(kind) {
            if !member {
                return Ok(false);
            }
            let header = Header::of(ast::TypeDecl::cast(node).unwrap());
            let name = header.name.map_or(Name::EMPTY, |n| self.text(n));
            let outer = self.summary_types.last().map(|t| t.index);
            let index = self.next_index;
            // The summary has the type at this index, unless they disagree, which they can't
            // for the same tree; the type's names are left out if they do.
            let matches = self
                .fs
                .summary
                .types
                .get(index)
                .is_some_and(|t| t.name == name && t.outer.map(|o| o as usize) == outer);
            if !matches {
                return Ok(false);
            }
            self.next_index += 1;
            self.summary_types.push(SummaryType {
                node: node.id(),
                index,
                methods: 0,
            });
            return Ok(true);
        }
        if is_method_like(kind) && member {
            if let Some(top) = self.summary_types.last_mut() {
                let owner = TypeVarOwner::Method(self.fs.keys[top.index], top.methods);
                top.methods += 1;
                self.member_method = Some((node.id(), owner));
            }
            return Ok(true);
        }
        let summary = self.summary_types.last().map(|t| t.index);
        match kind {
            SyntaxKind::Block if is_method_like(parent.kind()) => {
                self.start_region(node, summary, false);
                self.member_parameters(parent)?;
            }
            SyntaxKind::Block if parent.kind() == SyntaxKind::Initializer => {
                self.start_region(node, summary, false);
            }
            SyntaxKind::DefaultValue => self.start_region(node, summary, false),
            _ if parent.kind() == SyntaxKind::VariableDeclarator && kind != SyntaxKind::Dim => {
                self.start_region(node, summary, false)
            }
            SyntaxKind::Arguments if parent.kind() == SyntaxKind::EnumConstant => {
                self.start_region(node, summary, false)
            }
            SyntaxKind::ClassBody if parent.kind() == SyntaxKind::EnumConstant => {
                self.start_region(node, summary, false);
                return self.enter_body(node);
            }
            SyntaxKind::AnnotationArguments => {
                // A type's own annotations are in the scope around it.
                let on_top = parent
                    .parent()
                    .filter(|m| m.kind() == SyntaxKind::Modifiers)
                    .and_then(|m| m.parent())
                    .is_some_and(|d| Some(d.id()) == top);
                let summary = if on_top {
                    let n = self.summary_types.len();
                    (n >= 2).then(|| self.summary_types[n - 2].index)
                } else {
                    summary
                };
                self.start_region(node, summary, true);
            }
            _ => {}
        }
        Ok(true)
    }

    fn start_region(&mut self, node: SyntaxNode<'_>, summary: Option<usize>, only: bool) {
        self.regions.push(Region {
            node: node.id(),
            summary,
            expressions_only: only,
        });
        self.env.push(node.id());
    }

    /// A summary method's or constructor's type parameters and parameters, in scope in its
    /// body; a compact constructor's parameters are its record's components.
    fn member_parameters(&mut self, member: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let owner = match self.member_method {
            Some((node, owner)) if node == member.id() => owner,
            _ => return Ok(()),
        };
        let Some(index) = self.summary_index() else {
            return Ok(());
        };
        let (type_parameters, parameters) = match member.kind() {
            SyntaxKind::MethodDecl => {
                let m = ast::MethodDecl::cast(member).unwrap();
                (m.type_parameters(), m.formal_parameters())
            }
            SyntaxKind::ConstructorDecl => {
                let m = ast::ConstructorDecl::cast(member).unwrap();
                (m.type_parameters(), m.formal_parameters())
            }
            _ => (None, None),
        };
        let mut type_names = Vec::new();
        for (i, p) in type_parameters
            .iter()
            .flat_map(|p| p.parameters())
            .enumerate()
        {
            let name = p.name().map_or(Name::EMPTY, |n| self.text(n));
            type_names.push(name);
            self.env
                .bind_type(name, TypeResolution::TypeVar(owner, i as u32));
        }
        let scope = Scope {
            method: Some((owner, type_names)),
            ..Scope::of_type(self.fs, index, true)
        };
        let mut declared = Vec::new();
        for param in parameters.iter().flat_map(|p| p.parameters()) {
            let extra = param.dims().count() + usize::from(param.varargs().is_some());
            let ty = TypeRef::from_syntax(param.ty(), &self.names);
            let at = param.name().map(|n| n.span().start);
            declared.push((param.syntax(), param.name(), ty, extra as u32, at));
        }
        if member.kind() == SyntaxKind::CompactConstructorDecl
            && let Some(record) = member
                .parent()
                .and_then(|b| b.parent())
                .and_then(ast::RecordDecl::cast)
        {
            for component in record.record_header().iter().flat_map(|h| h.components()) {
                let extra = u32::from(component.varargs().is_some());
                let ty = TypeRef::from_syntax(component.ty(), &self.names);
                let at = Some(component_type_at(component));
                declared.push((component.syntax(), component.name(), ty, extra, at));
            }
        }
        for (node, name, ty, extra, at) in declared {
            let resolved = self.r.ty(&scope, &ty, extra)?;
            self.r.errors.clear();
            let ty = self.var_ty(&resolved);
            if let Some(index) = self.declare_local(node, name, ty)
                && let Some(at) = at
            {
                self.env.locals[index as usize].at = at;
            }
        }
        Ok(())
    }

    fn leave_declarations(&mut self, node: SyntaxNode<'_>) {
        if self
            .summary_types
            .last()
            .is_some_and(|t| t.node == node.id())
        {
            self.summary_types.pop();
        }
        if self.member_method.is_some_and(|(n, _)| n == node.id()) {
            self.member_method = None;
        }
    }

    // ---------------------------------------------------------------------------------------
    // Locals.

    /// What a resolved type says about a variable's members.
    fn var_ty(&mut self, ty: &Ty) -> VarTy {
        match ty.parts().first() {
            Some(TyPart::Class { ty, .. }) => VarTy::Class(*ty),
            Some(TyPart::Binary { binary, .. }) => {
                match self.r.reads.find_binary(self.set, *binary) {
                    Ok(Some(key)) => VarTy::Class(key),
                    _ => VarTy::Unknown,
                }
            }
            Some(TyPart::TypeVar { .. } | TyPart::UnknownTypeVar { .. }) => VarTy::TypeVar,
            Some(TyPart::Array { .. }) => VarTy::Array,
            Some(TyPart::Primitive(_) | TyPart::Void) => VarTy::Primitive,
            _ => VarTy::Unknown,
        }
    }

    /// The type a declaration's type node gives a variable, from what the walk resolved it to.
    fn declared_ty(&self, ty: Option<ast::Type<'_>>, array: bool) -> VarTy {
        if array {
            return VarTy::Array;
        }
        match ty {
            Some(ast::Type::ClassType(c)) => {
                if self.var_types.contains(&c.syntax().id()) {
                    return VarTy::Unknown;
                }
                match self.type_of.get(&c.syntax().id()) {
                    Some(TypeResolution::Type(key)) => VarTy::Class(*key),
                    Some(TypeResolution::TypeVar(..)) => VarTy::TypeVar,
                    Some(TypeResolution::Error(e)) => VarTy::Error(e.clone()),
                    _ => VarTy::Unknown,
                }
            }
            Some(ast::Type::ArrayType(_)) => VarTy::Array,
            Some(ast::Type::PrimitiveType(_)) => VarTy::Primitive,
            _ => VarTy::Unknown,
        }
    }

    fn new_local(&mut self, node: SyntaxNode<'_>, token: SyntaxToken<'_>, ty: VarTy) -> u32 {
        let name = self.text(token);
        self.env.locals.push(LocalVar {
            node: node.id(),
            name,
            at: token.span().start,
            ty,
        });
        self.env.locals.len() as u32 - 1
    }

    /// Declares a local in the innermost scope. `_` declares nothing (JLS 6.1).
    fn declare_local(
        &mut self,
        node: SyntaxNode<'_>,
        name: Option<SyntaxToken<'_>>,
        ty: VarTy,
    ) -> Option<u32> {
        let token = name.filter(|t| t.kind() != SyntaxKind::Underscore)?;
        let index = self.new_local(node, token, ty);
        let name = self.env.locals[index as usize].name;
        self.env.bind_var(name, Var::Local(index));
        Some(index)
    }

    fn bind_locals(&mut self, locals: &[u32]) {
        for &local in locals {
            let name = self.env.locals[local as usize].name;
            self.env.bind_var(name, Var::Local(local));
        }
    }

    fn bindings_of(&self, node: Option<NodeId>, when_true: bool) -> Vec<u32> {
        node.and_then(|n| self.bindings.get(&n))
            .map(|(t, f)| if when_true { t.clone() } else { f.clone() })
            .unwrap_or_default()
    }

    /// Opens a scope for `node` with the pattern bindings `locals`.
    fn push_bindings(&mut self, node: SyntaxNode<'_>, locals: &[u32]) {
        if locals.is_empty() {
            return;
        }
        self.env.push(node.id());
        self.bind_locals(locals);
    }

    // ---------------------------------------------------------------------------------------
    // Inside bodies.

    fn enter_body(&mut self, node: SyntaxNode<'_>) -> Result<bool, Cancelled> {
        let kind = node.kind();
        if let Some(parent) = node.parent() {
            self.enter_role(node, parent);
        }
        let only = self.regions.last().is_some_and(|r| r.expressions_only);
        match kind {
            SyntaxKind::Name => {
                // A qualified name is resolved whole, from its outermost node.
                if !only
                    && node
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::Annotation)
                {
                    self.annotation(node.parent().unwrap())?;
                }
                return Ok(false);
            }
            SyntaxKind::Block | SyntaxKind::CatchClause | SyntaxKind::LambdaExpr => {
                self.env.push(node.id())
            }
            SyntaxKind::TryStmt => self.env.push(node.id()),
            SyntaxKind::ForStmt | SyntaxKind::ForEachStmt => {
                self.env.push(node.id());
                self.breakable.push(Breakable {
                    node: node.id(),
                    label: None,
                });
            }
            SyntaxKind::WhileStmt
            | SyntaxKind::DoStmt
            | SyntaxKind::SwitchStmt
            | SyntaxKind::SwitchExpr => self.breakable.push(Breakable {
                node: node.id(),
                label: None,
            }),
            SyntaxKind::LabeledStmt => {
                let label = ast::LabeledStmt::cast(node)
                    .unwrap()
                    .label()
                    .map(|t| self.text(t));
                self.breakable.push(Breakable {
                    node: node.id(),
                    label,
                });
            }
            SyntaxKind::SwitchBlock => {
                let selector = node.parent().and_then(|p| match p.kind() {
                    SyntaxKind::SwitchStmt => id_of(ast::SwitchStmt::cast(p).unwrap().selector()),
                    SyntaxKind::SwitchExpr => id_of(ast::SwitchExpr::cast(p).unwrap().selector()),
                    _ => None,
                });
                let meaning = selector.and_then(|s| self.meanings.remove(&s));
                let selector = match meaning {
                    Some(Meaning::Value(VarTy::Class(key)) | Meaning::This(key)) => {
                        if self.kind_of(key)? == Some(TypeKind::Enum) {
                            Selector::Enum(key)
                        } else {
                            Selector::Other
                        }
                    }
                    Some(Meaning::Value(VarTy::Primitive | VarTy::Error(_))) => Selector::Other,
                    _ => Selector::Unknown,
                };
                self.switch_enums.push((node.id(), selector));
                // A local declared in a statement group is in scope in the rest of the switch
                // block, the later groups included (JLS 6.3).
                self.env.push(node.id());
            }
            SyntaxKind::SwitchLabel | SyntaxKind::InstanceofExpr => {
                self.pattern_marks.insert(node.id(), self.patterns.len());
            }
            SyntaxKind::Guard => {
                let mark = node
                    .parent()
                    .and_then(|l| self.pattern_marks.get(&l.id()))
                    .copied()
                    .unwrap_or(self.patterns.len());
                let locals = self.patterns[mark..].to_vec();
                self.push_bindings(node, &locals);
            }
            SyntaxKind::FieldAccessExpr => {
                let context = self.context_of(node);
                self.contexts.insert(node.id(), context);
            }
            SyntaxKind::ClassType => self.class_type(node)?,
            SyntaxKind::VariableDeclarator => self.local_declarator(node),
            _ if is_type_decl(kind) => self.enter_local_type(node),
            SyntaxKind::ClassBody | SyntaxKind::EnumBody => self.enter_class_body(node)?,
            _ if is_method_like(kind) => self.enter_local_method(node),
            _ => {}
        }
        Ok(true)
    }

    /// The scope a node's role in its parent opens: an operand or branch where a condition's
    /// pattern bindings are in scope, a loop's body, a `case`'s body.
    fn enter_role(&mut self, node: SyntaxNode<'_>, parent: SyntaxNode<'_>) {
        match parent.kind() {
            SyntaxKind::BinaryExpr => {
                let b = ast::BinaryExpr::cast(parent).unwrap();
                if is(node, b.rhs()) {
                    let lhs = id_of(b.lhs());
                    let locals = match b.op().map(|t| t.kind()) {
                        Some(SyntaxKind::AmpAmp) => self.bindings_of(lhs, true),
                        Some(SyntaxKind::PipePipe) => self.bindings_of(lhs, false),
                        _ => Vec::new(),
                    };
                    self.push_bindings(node, &locals);
                }
            }
            SyntaxKind::ConditionalExpr => {
                let c = ast::ConditionalExpr::cast(parent).unwrap();
                let condition = id_of(c.condition());
                if is(node, c.then_expr()) {
                    let locals = self.bindings_of(condition, true);
                    self.push_bindings(node, &locals);
                } else if is(node, c.else_expr()) {
                    let locals = self.bindings_of(condition, false);
                    self.push_bindings(node, &locals);
                }
            }
            SyntaxKind::IfStmt => {
                let s = ast::IfStmt::cast(parent).unwrap();
                let condition = id_of(s.condition());
                if is(node, s.then_branch()) {
                    let locals = self.bindings_of(condition, true);
                    self.push_bindings(node, &locals);
                } else if is(node, s.else_branch()) {
                    let locals = self.bindings_of(condition, false);
                    self.push_bindings(node, &locals);
                }
            }
            SyntaxKind::WhileStmt => {
                let s = ast::WhileStmt::cast(parent).unwrap();
                if is(node, s.body()) {
                    let locals = self.bindings_of(id_of(s.condition()), true);
                    self.push_bindings(node, &locals);
                }
            }
            SyntaxKind::ForStmt => {
                let s = ast::ForStmt::cast(parent).unwrap();
                if is(node, s.body()) || is(node, s.for_update()) {
                    let locals = self.bindings_of(id_of(s.condition()), true);
                    self.push_bindings(node, &locals);
                }
            }
            SyntaxKind::ForEachStmt => {
                let s = ast::ForEachStmt::cast(parent).unwrap();
                if is(node, s.body())
                    && let Some(local) = self.for_each.remove(&parent.id())
                {
                    self.bind_locals(&[local]);
                }
            }
            SyntaxKind::SwitchRule => {
                let rule = ast::SwitchRule::cast(parent).unwrap();
                if is(node, rule.body()) {
                    let locals = rule
                        .switch_label()
                        .and_then(|l| self.label_bindings.get(&l.syntax().id()))
                        .cloned()
                        .unwrap_or_default();
                    self.push_bindings(node, &locals);
                }
            }
            SyntaxKind::SwitchGroup => {
                if node.kind() != SyntaxKind::SwitchLabel && self.open_groups.insert(parent.id()) {
                    let group = ast::SwitchGroup::cast(parent).unwrap();
                    let mut locals = Vec::new();
                    for label in group.labels() {
                        if let Some(l) = self.label_bindings.get(&label.syntax().id()) {
                            locals.extend_from_slice(l);
                        }
                    }
                    // The group's pattern bindings are its own; so, as built, are the locals
                    // of a group that has them.
                    if !locals.is_empty() {
                        self.env.push(parent.id());
                        self.bind_locals(&locals);
                    }
                }
            }
            SyntaxKind::TryStmt => {
                // A resource is in scope in the `try` block, not in `catch` or `finally`.
                if matches!(
                    node.kind(),
                    SyntaxKind::CatchClause | SyntaxKind::FinallyClause
                ) && let Some(frame) = self.env.frames.last_mut()
                    && frame.owner == parent.id()
                {
                    let resources = Frame {
                        owner: frame.owner,
                        vars: std::mem::take(&mut frame.vars),
                        types: Vec::new(),
                        methods: Vec::new(),
                    };
                    self.env.unbind(&resources);
                }
            }
            _ => {}
        }
    }

    fn local_declarator(&mut self, node: SyntaxNode<'_>) {
        let Some(decl) = node.parent().and_then(ast::LocalVarDecl::cast) else {
            return;
        };
        let declarator = ast::VariableDeclarator::cast(node).unwrap();
        let ty = self.declared_ty(decl.ty(), declarator.dims().next().is_some());
        let for_each = decl
            .syntax()
            .parent()
            .filter(|p| p.kind() == SyntaxKind::ForEachStmt);
        match for_each {
            // Not in scope in its own expression: bound when the body is entered.
            Some(statement) => {
                if let Some(token) = declarator
                    .name()
                    .filter(|t| t.kind() != SyntaxKind::Underscore)
                {
                    let local = self.new_local(node, token, ty);
                    self.for_each.insert(statement.id(), local);
                }
            }
            None => {
                self.declare_local(node, declarator.name(), ty);
            }
        }
    }

    fn leave(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        if self.regions.is_empty() {
            self.leave_declarations(node);
            return Ok(());
        }
        let id = node.id();
        match node.kind() {
            SyntaxKind::NameExpr => self.name_expr(node)?,
            SyntaxKind::FieldAccessExpr => self.field_access(node)?,
            SyntaxKind::MethodCallExpr => self.method_call(node)?,
            SyntaxKind::MethodRefExpr => self.method_ref(node)?,
            SyntaxKind::ThisExpr => self.this_expr(node)?,
            SyntaxKind::SuperExpr => self.super_expr(node)?,
            SyntaxKind::NewExpr => self.new_as_outer(node),
            SyntaxKind::ParenExpr => {
                let inner = id_of(ast::ParenExpr::cast(node).unwrap().expr());
                if let Some(b) = inner.and_then(|i| self.bindings.get(&i)).cloned() {
                    self.bindings.insert(id, b);
                }
            }
            SyntaxKind::PrefixExpr => {
                let e = ast::PrefixExpr::cast(node).unwrap();
                if e.op().is_some_and(|t| t.kind() == SyntaxKind::Bang)
                    && let Some((t, f)) = id_of(e.operand()).and_then(|i| self.bindings.get(&i))
                {
                    let swapped = (f.clone(), t.clone());
                    self.bindings.insert(id, swapped);
                }
            }
            SyntaxKind::BinaryExpr => {
                let e = ast::BinaryExpr::cast(node).unwrap();
                let (lhs, rhs) = (id_of(e.lhs()), id_of(e.rhs()));
                match e.op().map(|t| t.kind()) {
                    Some(SyntaxKind::AmpAmp) => {
                        let mut t = self.bindings_of(lhs, true);
                        t.extend(self.bindings_of(rhs, true));
                        if !t.is_empty() {
                            self.bindings.insert(id, (t, Vec::new()));
                        }
                    }
                    Some(SyntaxKind::PipePipe) => {
                        let mut f = self.bindings_of(lhs, false);
                        f.extend(self.bindings_of(rhs, false));
                        if !f.is_empty() {
                            self.bindings.insert(id, (Vec::new(), f));
                        }
                    }
                    _ => {}
                }
            }
            SyntaxKind::InstanceofExpr => {
                let mark = self
                    .pattern_marks
                    .remove(&id)
                    .unwrap_or(self.patterns.len());
                let claimed = self.patterns.split_off(mark.min(self.patterns.len()));
                if !claimed.is_empty() {
                    self.bindings.insert(id, (claimed, Vec::new()));
                }
            }
            SyntaxKind::TypePattern => {
                let p = ast::TypePattern::cast(node).unwrap();
                let ty = self.declared_ty(p.ty(), false);
                if let Some(token) = p.name().filter(|t| t.kind() != SyntaxKind::Underscore) {
                    let local = self.new_local(node, token, ty);
                    self.patterns.push(local);
                }
            }
            SyntaxKind::SwitchLabel => {
                let mark = self
                    .pattern_marks
                    .remove(&id)
                    .unwrap_or(self.patterns.len());
                let mut claimed = self.patterns.split_off(mark.min(self.patterns.len()));
                let guard = ast::SwitchLabel::cast(node)
                    .unwrap()
                    .guard()
                    .and_then(|g| id_of(g.condition()));
                claimed.extend(self.bindings_of(guard, true));
                self.label_bindings.insert(id, claimed);
            }
            SyntaxKind::FormalParameter => {
                let p = ast::FormalParameter::cast(node).unwrap();
                let array = p.dims().next().is_some() || p.varargs().is_some();
                let ty = self.declared_ty(p.ty(), array);
                self.declare_local(node, p.name(), ty);
            }
            SyntaxKind::LambdaParameter => {
                let p = ast::LambdaParameter::cast(node).unwrap();
                let array = p.dims().next().is_some() || p.varargs().is_some();
                let ty = self.declared_ty(p.ty(), array);
                self.declare_local(node, p.name(), ty);
            }
            SyntaxKind::CatchParameter => {
                let p = ast::CatchParameter::cast(node).unwrap();
                let ty = self.declared_ty(p.ty(), p.dims().next().is_some());
                self.declare_local(node, p.name(), ty);
            }
            SyntaxKind::BreakStmt => {
                let label = ast::BreakStmt::cast(node)
                    .unwrap()
                    .label()
                    .map(|t| self.text(t));
                let target = self
                    .breakable
                    .iter()
                    .rev()
                    .find(|b| b.label == label)
                    .map(|b| b.node);
                if let Some(target) = target {
                    self.broken.insert(target);
                }
                self.abrupt.insert(id);
            }
            _ => self.statement(node),
        }
        self.env.pop(id);
        if self.breakable.last().is_some_and(|b| b.node == id) {
            self.breakable.pop();
        }
        if self.switch_enums.last().is_some_and(|s| s.0 == id) {
            self.switch_enums.pop();
        }
        self.open_groups.remove(&id);
        if self.classes.last().is_some_and(|c| c.0 == id) {
            self.classes.pop();
        }
        if let Some(parent) = node.parent()
            && parent.kind() != SyntaxKind::LabeledStmt
            && let Some(introduced) = self.introduced.remove(&id)
            && matches!(parent.kind(), SyntaxKind::Block | SyntaxKind::SwitchGroup)
        {
            self.bind_locals(&introduced);
        }
        if self.regions.last().is_some_and(|r| r.node == id) {
            self.regions.pop();
        }
        Ok(())
    }

    /// Whether a statement can't complete normally, and the pattern bindings it introduces
    /// after it (JLS 6.3.2, 14.22).
    fn statement(&mut self, node: SyntaxNode<'_>) {
        let id = node.id();
        let abrupt = |w: &Self, n: Option<NodeId>| n.is_some_and(|n| w.abrupt.contains(&n));
        match node.kind() {
            SyntaxKind::ReturnStmt
            | SyntaxKind::ThrowStmt
            | SyntaxKind::ContinueStmt
            | SyntaxKind::YieldStmt => {
                self.abrupt.insert(id);
            }
            SyntaxKind::Block => {
                if node.children().any(|c| self.abrupt.contains(&c.id())) {
                    self.abrupt.insert(id);
                }
            }
            SyntaxKind::IfStmt => {
                let s = ast::IfStmt::cast(node).unwrap();
                let condition = id_of(s.condition());
                let then = abrupt(self, id_of(s.then_branch()));
                let introduced = match s.else_branch() {
                    None => {
                        if then {
                            self.bindings_of(condition, false)
                        } else {
                            Vec::new()
                        }
                    }
                    Some(e) => {
                        let otherwise = self.abrupt.contains(&e.syntax().id());
                        if then && otherwise {
                            self.abrupt.insert(id);
                        }
                        match (then, otherwise) {
                            (false, true) => self.bindings_of(condition, true),
                            (true, false) => self.bindings_of(condition, false),
                            _ => Vec::new(),
                        }
                    }
                };
                if !introduced.is_empty() {
                    self.introduced.insert(id, introduced);
                }
            }
            SyntaxKind::WhileStmt | SyntaxKind::DoStmt | SyntaxKind::ForStmt => {
                let condition = match node.kind() {
                    SyntaxKind::WhileStmt => ast::WhileStmt::cast(node).unwrap().condition(),
                    SyntaxKind::DoStmt => ast::DoStmt::cast(node).unwrap().condition(),
                    _ => ast::ForStmt::cast(node).unwrap().condition(),
                };
                let forever = is_true(condition)
                    || (node.kind() == SyntaxKind::ForStmt && condition.is_none());
                if self.broken.contains(&id) {
                    return;
                }
                if forever {
                    self.abrupt.insert(id);
                }
                let introduced = self.bindings_of(id_of(condition), false);
                if !introduced.is_empty() {
                    self.introduced.insert(id, introduced);
                }
            }
            SyntaxKind::LabeledStmt => {
                let body = id_of(ast::LabeledStmt::cast(node).unwrap().body());
                if abrupt(self, body) && !self.broken.contains(&id) {
                    self.abrupt.insert(id);
                }
                if let Some(introduced) = body.and_then(|b| self.introduced.remove(&b)) {
                    self.introduced.insert(id, introduced);
                }
            }
            SyntaxKind::SynchronizedStmt => {
                if abrupt(
                    self,
                    id_of(ast::SynchronizedStmt::cast(node).unwrap().body()),
                ) {
                    self.abrupt.insert(id);
                }
            }
            SyntaxKind::TryStmt => {
                let s = ast::TryStmt::cast(node).unwrap();
                let body = abrupt(self, id_of(s.body()));
                let catches = s.catches().all(|c| abrupt(self, id_of(c.body())));
                let finally = s
                    .finally_clause()
                    .and_then(|f| f.body())
                    .is_some_and(|b| self.abrupt.contains(&b.syntax().id()));
                if (body && catches) || finally {
                    self.abrupt.insert(id);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------------------------------
    // Names in expressions.

    /// Keeps a qualifier's meaning for the name it qualifies, if its parent is one that reads
    /// it.
    fn store(&mut self, node: SyntaxNode<'_>, meaning: Meaning) {
        let reads = node.parent().is_some_and(|p| {
            matches!(
                p.kind(),
                SyntaxKind::FieldAccessExpr
                    | SyntaxKind::MethodCallExpr
                    | SyntaxKind::MethodRefExpr
                    | SyntaxKind::ThisExpr
                    | SyntaxKind::SuperExpr
                    | SyntaxKind::SwitchStmt
                    | SyntaxKind::SwitchExpr
                    | SyntaxKind::NewExpr
            )
        });
        if reads {
            self.meanings.insert(node.id(), meaning);
        }
    }

    /// `new Outer().new Inner()`: the outer instance is of the class it creates, whose member
    /// types `Inner` is looked up in. Kept for that alone; a created value's other members
    /// are B3's.
    fn new_as_outer(&mut self, node: SyntaxNode<'_>) {
        let new = ast::NewExpr::cast(node).unwrap();
        let is_outer = node
            .parent()
            .and_then(ast::NewExpr::cast)
            .and_then(|p| p.outer())
            .is_some_and(|outer| outer.syntax() == node);
        if !is_outer {
            return;
        }
        // An anonymous class: its own member types and those it inherits.
        if let Some(body) = new.body() {
            if let Some(&index) = self.env.local_of_node.get(&body.syntax().id()) {
                let key = self.env.local_types[index as usize].key;
                self.store(node, Meaning::Value(VarTy::Class(key)));
            }
            return;
        }
        let created = match new.ty() {
            Some(ast::Type::ClassType(c)) => self.type_of.get(&c.syntax().id()).cloned(),
            _ => None,
        };
        if let Some(TypeResolution::Type(key)) = created {
            self.store(node, Meaning::Value(VarTy::Class(key)));
        }
    }

    fn take_meaning(&mut self, node: Option<SyntaxNode<'_>>) -> Meaning {
        node.and_then(|n| self.meanings.remove(&n.id()))
            .unwrap_or(Meaning::Unknown)
    }

    /// The context of a name in an expression, from its parent (JLS 6.5.1).
    fn context_of(&self, node: SyntaxNode<'_>) -> Context {
        let Some(parent) = node.parent() else {
            return Context::Expr;
        };
        match parent.kind() {
            SyntaxKind::FieldAccessExpr => match self.contexts.get(&parent.id()) {
                Some(Context::Type | Context::PackageOrType) => Context::PackageOrType,
                _ => Context::Ambiguous,
            },
            // `C::new` names a class type (JLS 15.13), which a variable `C` doesn't obscure.
            SyntaxKind::MethodRefExpr
                if ast::MethodRefExpr::cast(parent)
                    .and_then(|r| r.new_kw())
                    .is_some() =>
            {
                Context::Type
            }
            SyntaxKind::MethodCallExpr | SyntaxKind::MethodRefExpr => Context::Ambiguous,
            SyntaxKind::ClassLiteralExpr | SyntaxKind::ThisExpr | SyntaxKind::SuperExpr => {
                Context::Type
            }
            SyntaxKind::SwitchLabel => match self.switch_enums.last() {
                Some((_, Selector::Enum(enumeration))) => Context::Case(*enumeration),
                Some((_, Selector::Unknown)) => Context::CaseOfUnknown,
                _ => Context::Expr,
            },
            _ => Context::Expr,
        }
    }

    fn not_found(name: Name, incomplete: Option<TypeKey>) -> ErrorSymbol {
        ErrorSymbol {
            name,
            cause: match incomplete {
                Some(ty) => Cause::Incomplete { ty },
                None => Cause::NotFound,
            },
        }
    }

    fn name_expr(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let Some(token) = ast::NameExpr::cast(node).unwrap().name() else {
            return Ok(());
        };
        let name = self.text(token);
        let (symbol, meaning) = match self.context_of(node) {
            Context::Expr => self.expression_name(name)?,
            Context::Case(enumeration) => {
                let maps = self.named(enumeration, name)?;
                match maps.fields.get(&name) {
                    Some((found, _)) => {
                        let found = found.clone();
                        self.field_symbol(found, name)?
                    }
                    None => self.expression_name(name)?,
                }
            }
            Context::CaseOfUnknown => self.case_of_unknown(name)?,
            Context::Ambiguous => {
                let (symbol, meaning) = self.expression_name(name)?;
                match &symbol {
                    Symbol::Error(ErrorSymbol {
                        cause: Cause::NotFound | Cause::Incomplete { .. },
                        ..
                    }) => self.type_or_package(name, true)?,
                    _ => (symbol, meaning),
                }
            }
            Context::PackageOrType => self.type_or_package(name, true)?,
            Context::Type => self.type_or_package(name, false)?,
        };
        self.record(node, symbol);
        self.store(node, meaning);
        Ok(())
    }

    /// A `case` label's simple name where the selector's type needs attribution: an enum
    /// constant of that type, if it is an enum, or else a constant variable in scope (JLS
    /// 14.11.1). A variable in scope that could be a `case` constant, of a primitive type, of
    /// `String`, or of an enum type, is taken; any other (helidon's `case STRING ->` on an enum,
    /// with a `TypeName STRING` imported) is left to B3 with names no variable has.
    fn case_of_unknown(&mut self, name: Name) -> Result<(Symbol, Meaning), Cancelled> {
        let (symbol, meaning) = self.expression_name(name)?;
        let constant = match &meaning {
            Meaning::Value(VarTy::Primitive) => true,
            Meaning::Value(VarTy::Class(key)) => {
                let string = self.names.intern("java/lang/String");
                self.r.reads.find_binary(self.set, string)? == Some(*key)
                    || self.kind_of(*key)? == Some(TypeKind::Enum)
            }
            _ => false,
        };
        if constant && !matches!(symbol, Symbol::Error(_)) {
            return Ok((symbol, meaning));
        }
        Ok((Symbol::LeftToB3(LeftToB3::ExpressionType), Meaning::Unknown))
    }

    /// A simple expression name (JLS 6.5.6.1): the innermost local, or field of a local or
    /// anonymous class around it; then a field of the summary's classes around the body,
    /// declared or inherited, innermost first; then a single-static import; then a
    /// static-import-on-demand.
    fn expression_name(&mut self, name: Name) -> Result<(Symbol, Meaning), Cancelled> {
        if let Some(var) = self.env.vars.get(&name).and_then(|v| v.last()).cloned() {
            return match var {
                Var::Local(index) => {
                    let local = &self.env.locals[index as usize];
                    Ok((
                        Symbol::Local {
                            file: self.file,
                            node: local.node,
                            at: local.at,
                        },
                        Meaning::Value(local.ty.clone()),
                    ))
                }
                Var::Field(found) => self.field_symbol(found, name),
            };
        }
        let mut incomplete = None;
        for key in self.summary_chain() {
            let maps = self.named(key, name)?;
            if let Some((found, _)) = maps.fields.get(&name) {
                let found = found.clone();
                return self.field_symbol(found, name);
            }
            if maps.incomplete {
                incomplete.get_or_insert(key);
            }
        }
        for owner in self.static_imports(name, true)? {
            let maps = self.named(owner, name)?;
            if let Some((found, _)) = maps.fields.get(&name) {
                let found = found.clone();
                return self.field_symbol(found, name);
            }
        }
        let mut found: Vec<FieldFound> = Vec::new();
        for owner in self.static_imports(name, false)? {
            let maps = self.named(owner, name)?;
            if let Some((f, _)) = maps.fields.get(&name)
                && !found.contains(f)
            {
                found.push(f.clone());
            }
        }
        match found.len() {
            0 => {}
            1 => return self.field_symbol(found.pop().unwrap(), name),
            _ => {
                let owners = found
                    .iter()
                    .filter_map(|f| match f {
                        FieldFound::One(owner, _) => Some(*owner),
                        FieldFound::Ambiguous(_) => None,
                    })
                    .collect();
                let error = ErrorSymbol {
                    name,
                    cause: Cause::Ambiguous(owners),
                };
                return Ok((Symbol::Error(error.clone()), Meaning::Error(error)));
            }
        }
        let error = Self::not_found(name, incomplete);
        Ok((Symbol::Error(error.clone()), Meaning::Error(error)))
    }

    /// The types the file's static imports of `name` name: single-static imports of it, or
    /// with `single` false, every static-import-on-demand.
    fn static_imports(&mut self, name: Name, single: bool) -> Result<Vec<TypeKey>, Cancelled> {
        let mut owners = Vec::new();
        let fs = self.fs;
        for import in fs.summary.imports.iter() {
            let owner_name = match import.kind {
                ImportKind::SingleStatic if single && import.name.last(&self.names) == name => {
                    QualifiedName(self.r.owner_of(import.name))
                }
                ImportKind::StaticOnDemand if !single => import.name,
                _ => continue,
            };
            if let Some(TypeResolution::Type(owner)) = self.r.canonical(owner_name)?.last().cloned()
                && !owners.contains(&owner)
            {
                owners.push(owner);
            }
        }
        Ok(owners)
    }

    fn field_symbol(
        &mut self,
        found: FieldFound,
        name: Name,
    ) -> Result<(Symbol, Meaning), Cancelled> {
        match found {
            FieldFound::One(owner, index) => {
                let ty = self.field_ty(owner, index)?;
                Ok((Symbol::Field { owner, name }, Meaning::Value(ty)))
            }
            FieldFound::Ambiguous(owners) => {
                let error = ErrorSymbol {
                    name,
                    cause: Cause::Ambiguous(owners),
                };
                Ok((Symbol::Error(error.clone()), Meaning::Error(error)))
            }
        }
    }

    /// The declared type of the `index`th field of `owner`.
    fn field_ty(&mut self, owner: TypeKey, index: u32) -> Result<VarTy, Cancelled> {
        let ty = match owner {
            TypeKey::Local { .. } => self
                .env
                .state(owner)
                .and_then(|s| s.entry.as_ref())
                .and_then(|e| e.fields.get(index as usize))
                .map(|f| f.ty.clone()),
            TypeKey::Class { .. } => self
                .r
                .reads
                .snapshot
                .class_entry(owner)
                .and_then(|e| e.fields.get(index as usize).map(|f| f.ty.clone())),
            _ => self
                .r
                .reads
                .type_entry(owner)?
                .fields
                .get(index as usize)
                .map(|f| f.ty.clone()),
        };
        Ok(match ty {
            Some(ty) => self.var_ty(&ty),
            None => VarTy::Unknown,
        })
    }

    /// A simple name as a type, or with `package` as a package if no type has it.
    fn type_or_package(
        &mut self,
        name: Name,
        package: bool,
    ) -> Result<(Symbol, Meaning), Cancelled> {
        let scope = scope!(self);
        let resolved = self.r.simple(&scope, name)?;
        Ok(match resolved {
            TypeResolution::Type(key) => (Symbol::Type(key), Meaning::Type(key)),
            TypeResolution::TypeVar(owner, index) => {
                (Symbol::TypeVar(owner, index), Meaning::TypeVar)
            }
            TypeResolution::Error(ErrorSymbol {
                cause: Cause::NotFound | Cause::Incomplete { .. },
                ..
            }) if package => (Symbol::Package(name), Meaning::Package(name, 1)),
            TypeResolution::Error(error) => (Symbol::Error(error.clone()), Meaning::Error(error)),
            TypeResolution::Package(p) => (Symbol::Package(p), Meaning::Package(p, 1)),
        })
    }

    fn field_access(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let access = ast::FieldAccessExpr::cast(node).unwrap();
        let context = self.contexts.remove(&node.id()).unwrap_or(Context::Expr);
        let target = access.target().map(|t| t.syntax());
        let qualifier = self.take_meaning(target);
        let Some(token) = access.name() else {
            return Ok(());
        };
        let name = self.text(token);
        let (symbol, meaning) = self.member(qualifier, name, context)?;
        self.record(node, symbol);
        self.store(node, meaning);
        Ok(())
    }

    /// `name` qualified by `qualifier`, in `context` (JLS 6.5.2, 6.5.6.2): after a package, a
    /// type or a subpackage; after a type, a static field, or a member type where a type can
    /// be; after a variable of a declared class type, a field of it. What needs an expression's
    /// type is B3's.
    fn member(
        &mut self,
        qualifier: Meaning,
        name: Name,
        context: Context,
    ) -> Result<(Symbol, Meaning), Cancelled> {
        let error = |error: ErrorSymbol| (Symbol::Error(error.clone()), Meaning::Error(error));
        let b3 = |why: LeftToB3| (Symbol::LeftToB3(why), Meaning::Unknown);
        let types = matches!(context, Context::Type | Context::PackageOrType);
        Ok(match qualifier {
            Meaning::Package(package, segments) => {
                let written = self.join(package, name);
                if segments as usize >= MAX_SEGMENTS {
                    return Ok(error(ErrorSymbol {
                        name: written,
                        cause: Cause::TooLong,
                    }));
                }
                let found = self.r.reads.find_top_level(self.set, package, name)?;
                match self.r.found(found, written, package) {
                    Some(TypeResolution::Type(key)) => (Symbol::Type(key), Meaning::Type(key)),
                    Some(TypeResolution::Error(e)) => error(e),
                    _ if matches!(context, Context::Ambiguous | Context::PackageOrType) => (
                        Symbol::Package(written),
                        Meaning::Package(written, segments + 1),
                    ),
                    _ => match self.r.missing_in_package(written, package)? {
                        TypeResolution::Error(e) => error(e),
                        _ => error(Self::not_found(written, None)),
                    },
                }
            }
            Meaning::Type(owner) => {
                let maps = self.named(owner, name)?;
                if !types && let Some((found, _)) = maps.fields.get(&name) {
                    let found = found.clone();
                    return self.field_symbol(found, name);
                }
                if context != Context::Expr {
                    let body = &self.env;
                    match self.r.member_type(owner, name, Some(body))? {
                        MemberLookup::Found(key) => {
                            return Ok((Symbol::Type(key), Meaning::Type(key)));
                        }
                        MemberLookup::Ambiguous(keys) => {
                            return Ok(error(ErrorSymbol {
                                name,
                                cause: Cause::Ambiguous(keys),
                            }));
                        }
                        MemberLookup::Cycle => {
                            return Ok(error(ErrorSymbol {
                                name,
                                cause: Cause::Cycle,
                            }));
                        }
                        MemberLookup::None { .. } => {}
                    }
                }
                error(Self::not_found(name, maps.incomplete.then_some(owner)))
            }
            Meaning::This(owner) | Meaning::Super(owner) | Meaning::Value(VarTy::Class(owner)) => {
                let maps = self.named(owner, name)?;
                match maps.fields.get(&name) {
                    Some((found, _)) => {
                        let found = found.clone();
                        return self.field_symbol(found, name);
                    }
                    None => error(Self::not_found(name, maps.incomplete.then_some(owner))),
                }
            }
            Meaning::TypeVar | Meaning::Value(VarTy::TypeVar) => b3(LeftToB3::TypeVariable),
            Meaning::Value(VarTy::Array) => b3(LeftToB3::Array),
            Meaning::Error(e) | Meaning::Value(VarTy::Error(e)) => error(e),
            Meaning::Value(VarTy::Primitive | VarTy::Unknown) | Meaning::Unknown => {
                b3(LeftToB3::ExpressionType)
            }
        })
    }

    fn method_call(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let call = ast::MethodCallExpr::cast(node).unwrap();
        let receiver = call.receiver().map(|r| r.syntax());
        let qualifier = receiver.map(|r| self.take_meaning(Some(r)));
        let Some(token) = call.name() else {
            return Ok(());
        };
        let name = self.text(token);
        let symbol = match qualifier {
            None => self.simple_method(name)?,
            Some(qualifier) => self.methods_of(qualifier, name)?,
        };
        let (start, end) = (node.span().start, token.span().end);
        self.record_range(node, start, end, symbol);
        self.store(node, Meaning::Unknown);
        Ok(())
    }

    /// The methods `name` of what a qualifier means (JLS 15.12.1): a type's, `this`'s class's,
    /// or `super`'s. A call on any other expression is B3's.
    fn methods_of(&mut self, qualifier: Meaning, name: Name) -> Result<Symbol, Cancelled> {
        Ok(match qualifier {
            Meaning::Type(owner) | Meaning::This(owner) | Meaning::Super(owner) => {
                self.methods_in(owner, name)?
            }
            Meaning::Package(package, _) => Symbol::Error(ErrorSymbol {
                name: package,
                cause: Cause::NotFound,
            }),
            Meaning::Error(e) | Meaning::Value(VarTy::Error(e)) => Symbol::Error(e),
            Meaning::TypeVar | Meaning::Value(VarTy::TypeVar) => {
                Symbol::LeftToB3(LeftToB3::TypeVariable)
            }
            Meaning::Value(VarTy::Array) => Symbol::LeftToB3(LeftToB3::Array),
            _ => Symbol::LeftToB3(LeftToB3::MethodOfExpression),
        })
    }

    /// The member methods `name` of `owner`, declared or inherited, as candidates.
    fn methods_in(&mut self, owner: TypeKey, name: Name) -> Result<Symbol, Cancelled> {
        if let Some(candidates) = self.candidates.get(&(owner, name)) {
            return Ok(Symbol::Methods {
                search: owner,
                name,
                candidates: candidates.clone(),
            });
        }
        let maps = self.named(owner, name)?;
        Ok(match maps.methods.get(&name) {
            Some(methods) if !methods.is_empty() => {
                // Shared by every call of the name on the type.
                let candidates: Arc<[MethodRef]> = methods.iter().map(|(m, _)| *m).collect();
                self.candidates.insert((owner, name), candidates.clone());
                Symbol::Methods {
                    search: owner,
                    name,
                    candidates,
                }
            }
            _ => Symbol::Error(Self::not_found(name, maps.incomplete.then_some(owner))),
        })
    }

    /// A simple method name (JLS 15.12.1): the innermost class around the call with a member
    /// method of the name, declared or inherited, is the type to search; else the types the
    /// static imports of it name.
    fn simple_method(&mut self, name: Name) -> Result<Symbol, Cancelled> {
        if let Some((search, candidates)) = self.env.methods.get(&name).and_then(|v| v.last()) {
            return Ok(Symbol::Methods {
                search: *search,
                name,
                candidates: candidates.clone(),
            });
        }
        let mut incomplete = None;
        for key in self.summary_chain() {
            let maps = self.named(key, name)?;
            if maps.methods.get(&name).is_some_and(|m| !m.is_empty()) {
                return self.methods_in(key, name);
            }
            if maps.incomplete {
                incomplete.get_or_insert(key);
            }
        }
        for single in [true, false] {
            let mut search = None;
            let mut candidates: Vec<MethodRef> = Vec::new();
            for owner in self.static_imports(name, single)? {
                let maps = self.named(owner, name)?;
                for (method, _) in maps.methods.get(&name).into_iter().flatten() {
                    search.get_or_insert(owner);
                    if !candidates.contains(method) {
                        candidates.push(*method);
                    }
                }
            }
            if let Some(mut search) = search {
                // JLS 15.12.1 has no type to search for a statically imported method; when its
                // candidates are one type's, that type is the one the method is found in, as
                // the javac comparison has it (semantic-oracle-normalization.md).
                if candidates.iter().all(|m| m.owner == candidates[0].owner) {
                    search = candidates[0].owner;
                }
                return Ok(Symbol::Methods {
                    search,
                    name,
                    candidates: Arc::from(candidates),
                });
            }
        }
        Ok(Symbol::Error(Self::not_found(name, incomplete)))
    }

    fn method_ref(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let reference = ast::MethodRefExpr::cast(node).unwrap();
        let qualifier = match reference.qualifier() {
            Some(ast::ExprOrType::Expr(e)) => self.take_meaning(Some(e.syntax())),
            Some(ast::ExprOrType::Type(ast::Type::ClassType(c))) => {
                match self.type_of.get(&c.syntax().id()) {
                    Some(TypeResolution::Type(key)) => Meaning::Type(*key),
                    Some(TypeResolution::TypeVar(..)) => Meaning::TypeVar,
                    Some(TypeResolution::Error(e)) => Meaning::Error(e.clone()),
                    _ => Meaning::Unknown,
                }
            }
            Some(ast::ExprOrType::Type(ast::Type::ArrayType(_))) => Meaning::Value(VarTy::Array),
            _ => Meaning::Unknown,
        };
        if reference.new_kw().is_some() {
            // A constructor reference: which constructor needs the types.
            self.record(node, Symbol::LeftToB3(LeftToB3::MethodOfExpression));
            return Ok(());
        }
        let Some(token) = reference.name() else {
            return Ok(());
        };
        let name = self.text(token);
        let symbol = self.methods_of(qualifier, name)?;
        self.record(node, symbol);
        Ok(())
    }

    fn this_expr(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let qualifier = ast::ThisExpr::cast(node)
            .unwrap()
            .qualifier()
            .map(|q| q.syntax());
        let meaning = match qualifier {
            None => self.current_class().map_or(Meaning::Unknown, Meaning::This),
            Some(q) => match self.take_meaning(Some(q)) {
                Meaning::Type(key) => Meaning::This(key),
                Meaning::Error(e) => Meaning::Error(e),
                _ => Meaning::Unknown,
            },
        };
        self.store(node, meaning);
        Ok(())
    }

    fn super_expr(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let qualifier = ast::SuperExpr::cast(node)
            .unwrap()
            .qualifier()
            .map(|q| q.syntax());
        let class = match qualifier {
            None => self.current_class(),
            Some(q) => match self.take_meaning(Some(q)) {
                // `I.super` for a superinterface `I` searches `I`.
                Meaning::Type(key) if self.kind_of(key)? == Some(TypeKind::Interface) => {
                    self.store(node, Meaning::Super(key));
                    return Ok(());
                }
                Meaning::Type(key) => Some(key),
                Meaning::Error(e) => {
                    self.store(node, Meaning::Error(e));
                    return Ok(());
                }
                _ => None,
            },
        };
        let meaning = match class {
            Some(class) => match self.superclass_of(class)? {
                Some(Ok(superclass)) => Meaning::Super(superclass),
                Some(Err(e)) => Meaning::Error(e),
                None => Meaning::Unknown,
            },
            None => Meaning::Unknown,
        };
        self.store(node, meaning);
        Ok(())
    }

    fn superclass_of(
        &mut self,
        key: TypeKey,
    ) -> Result<Option<Result<TypeKey, ErrorSymbol>>, Cancelled> {
        if let TypeKey::Local { .. } = key {
            return Ok(self
                .env
                .state(key)
                .and_then(|s| s.supertypes.as_ref())
                .and_then(|s| s.superclass.clone()));
        }
        Ok(self
            .r
            .reads
            .supertypes(self.set, key)?
            .and_then(|s| s.superclass.clone()))
    }

    fn kind_of(&mut self, key: TypeKey) -> Result<Option<TypeKind>, Cancelled> {
        Ok(match key {
            TypeKey::Local { .. } => self.env.state(key).map(|s| s.kind),
            TypeKey::Class { .. } => self.r.reads.snapshot.class_entry(key).map(|e| e.kind()),
            _ => Some(self.r.reads.type_entry(key)?.kind),
        })
    }

    // ---------------------------------------------------------------------------------------
    // Type names and annotations.

    fn class_type(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let ty = ast::ClassType::cast(node).unwrap();
        // A qualifier is resolved with the type it qualifies.
        if let Some(parent) = node.parent().and_then(ast::ClassType::cast)
            && parent.qualifier() == Some(ty)
        {
            return Ok(());
        }
        let mut chain = Vec::new();
        let mut next = Some(ty);
        while let Some(t) = next {
            let segment = t.name().map_or(Name::EMPTY, |n| self.text(n));
            chain.push((t.syntax(), segment));
            next = t.qualifier();
        }
        chain.reverse();
        let segments: Vec<Name> = chain.iter().map(|(_, n)| *n).collect();
        // `var` where a local's type would be isn't a type name (JLS 14.4.1).
        let parent = node.parent().map(|p| p.kind());
        if self.release >= 10
            && segments.len() == 1
            && ty.type_arguments().is_none()
            && &*self.names.text(segments[0]) == "var"
            && matches!(
                parent,
                Some(
                    SyntaxKind::LocalVarDecl
                        | SyntaxKind::LambdaParameter
                        | SyntaxKind::TypePattern
                )
            )
        {
            self.var_types.insert(node.id());
            return Ok(());
        }
        // `outer.new Inner()`: `Inner` is a member type of `outer`'s type.
        let outer = node
            .parent()
            .and_then(ast::NewExpr::cast)
            .and_then(|n| n.outer());
        let mut resolved = match outer {
            Some(outer) => match self.take_meaning(Some(outer.syntax())) {
                Meaning::Value(VarTy::Class(key)) | Meaning::This(key) => {
                    let first = match self.r.member_type(key, segments[0], Some(&self.env))? {
                        MemberLookup::Found(found) => TypeResolution::Type(found),
                        _ => TypeResolution::Error(Self::not_found(segments[0], None)),
                    };
                    self.r.steps(first, &segments, Some(&self.env))?
                }
                _ => {
                    for (node, _) in &chain {
                        self.record(*node, Symbol::LeftToB3(LeftToB3::ExpressionType));
                    }
                    return Ok(());
                }
            },
            None => {
                let scope = scope!(self);
                self.r.qualified(&scope, &segments)?
            }
        };
        if let Some(TypeResolution::Package(_)) = resolved.last() {
            let n = resolved.len();
            let package = if n >= 2 {
                self.r.written(&segments[..n - 1])
            } else {
                Name::EMPTY
            };
            let written = self.r.written(&segments);
            resolved[n - 1] = self.r.missing_in_package(written, package)?;
        }
        if let Some(last) = resolved.last() {
            self.type_of.insert(node.id(), last.clone());
        }
        for ((node, _), resolution) in chain.iter().zip(resolved) {
            self.record(*node, resolution.into());
        }
        Ok(())
    }

    /// An annotation in a body: its name, and its elements' names.
    fn annotation(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let annotation = ast::Annotation::cast(node).unwrap();
        let Some(name) = annotation.name() else {
            return Ok(());
        };
        let mut chain = Vec::new();
        let mut next = Some(name);
        while let Some(n) = next {
            let segment = n.name().map_or(Name::EMPTY, |t| self.text(t));
            chain.push((n.syntax(), segment));
            next = n.qualifier();
        }
        chain.reverse();
        let segments: Vec<Name> = chain.iter().map(|(_, n)| *n).collect();
        let scope = scope!(self);
        let mut resolved = self.r.qualified(&scope, &segments)?;
        if let Some(TypeResolution::Package(_)) = resolved.last() {
            let n = resolved.len();
            let package = if n >= 2 {
                self.r.written(&segments[..n - 1])
            } else {
                Name::EMPTY
            };
            let written = self.r.written(&segments);
            resolved[n - 1] = self.r.missing_in_package(written, package)?;
        }
        let owner = match resolved.last() {
            Some(TypeResolution::Type(key)) => Some(*key),
            _ => None,
        };
        for ((node, _), resolution) in chain.iter().zip(resolved) {
            self.record(*node, resolution.into());
        }
        for pair in annotation
            .annotation_arguments()
            .iter()
            .flat_map(|a| a.pairs())
        {
            let Some(token) = pair.name() else { continue };
            let element = self.text(token);
            let symbol = match owner {
                Some(owner) => Symbol::Element {
                    owner,
                    name: element,
                },
                None => Symbol::Error(Self::not_found(element, None)),
            };
            let span = token.span();
            self.record_range(pair.syntax(), span.start, span.end, symbol);
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------
    // Local and anonymous classes.

    fn new_local_type(
        &mut self,
        node: SyntaxNode<'_>,
        at: u32,
        kind: TypeKind,
        access: MemberAccess,
    ) -> u32 {
        let index = self.env.local_types.len() as u32;
        let lexical = self.current_class();
        self.env.local_types.push(LocalState {
            key: TypeKey::Local {
                file: self.file,
                index,
            },
            node: node.id(),
            at,
            kind,
            access,
            declared: Map::default(),
            summary: None,
            supertypes: None,
            member_types: None,
            entry: None,
            maps: None,
            methods: 0,
            lexical,
        });
        self.env.local_of_node.insert(node.id(), index);
        index
    }

    /// Registers the member types of the local type at `index`, and theirs, so that they can
    /// be named before the walk reaches them.
    fn register_members<'t>(&mut self, index: u32, members: impl Iterator<Item = ast::Member<'t>>) {
        let mut work: Vec<(u32, ast::TypeDecl<'t>)> = members
            .filter_map(|m| match m {
                ast::Member::TypeDecl(d) => Some((index, d)),
                _ => None,
            })
            .collect();
        while let Some((outer, decl)) = work.pop() {
            if self.env.local_of_node.contains_key(&decl.syntax().id()) {
                continue;
            }
            let header = Header::of(decl);
            let name = header.name.map_or(Name::EMPTY, |n| self.text(n));
            let at = declaration_at(decl.syntax());
            let access = access_of(modifiers(header.modifiers, &self.names).0, self.env.package);
            let member = self.new_local_type(decl.syntax(), at, header.kind, access);
            self.env.local_types[member as usize].lexical =
                Some(self.env.local_types[outer as usize].key);
            self.env.local_types[outer as usize]
                .declared
                .entry(name)
                .or_insert(member);
            work.extend(
                header
                    .members
                    .into_iter()
                    .flatten()
                    .filter_map(|m| match m {
                        ast::Member::TypeDecl(d) => Some((member, d)),
                        _ => None,
                    }),
            );
        }
    }

    /// A local class, or a member type of a local or anonymous class: in scope from its
    /// declaration, in its own declaration too, with its type parameters in scope in its header
    /// and body.
    fn enter_local_type(&mut self, node: SyntaxNode<'_>) {
        let decl = ast::TypeDecl::cast(node).unwrap();
        let header = Header::of(decl);
        let name = header.name.map_or(Name::EMPTY, |n| self.text(n));
        let index = match self.env.local_of_node.get(&node.id()) {
            Some(&index) => index,
            None => {
                let at = declaration_at(node);
                let index = self.new_local_type(node, at, header.kind, MemberAccess::Public);
                let key = self.env.local_types[index as usize].key;
                self.env.bind_type(name, TypeResolution::Type(key));
                index
            }
        };
        let summary = local_type(decl, &self.names);
        self.register_members(index, Header::of(decl).members.into_iter().flatten());
        let key = self.env.local_types[index as usize].key;
        self.env.push(node.id());
        for (i, p) in summary.type_parameters.iter().enumerate() {
            self.env.bind_type(
                p.name,
                TypeResolution::TypeVar(TypeVarOwner::Type(key), i as u32),
            );
        }
        self.env.local_types[index as usize].summary = Some(summary);
    }

    /// A method or constructor of a local or anonymous class: its type parameters are in scope
    /// in it, and a compact constructor's parameters are its record's components.
    fn enter_local_method(&mut self, node: SyntaxNode<'_>) {
        let Some(&owner) = node
            .parent()
            .and_then(|b| self.env.local_of_node.get(&b.id()))
        else {
            return;
        };
        let state = &mut self.env.local_types[owner as usize];
        let method = state.methods;
        state.methods += 1;
        let key = state.key;
        self.env.push(node.id());
        let type_parameters = match node.kind() {
            SyntaxKind::MethodDecl => ast::MethodDecl::cast(node).unwrap().type_parameters(),
            SyntaxKind::ConstructorDecl => {
                ast::ConstructorDecl::cast(node).unwrap().type_parameters()
            }
            _ => None,
        };
        for (i, p) in type_parameters
            .iter()
            .flat_map(|p| p.parameters())
            .enumerate()
        {
            let name = p.name().map_or(Name::EMPTY, |n| self.text(n));
            self.env.bind_type(
                name,
                TypeResolution::TypeVar(TypeVarOwner::Method(key, method), i as u32),
            );
        }
        if node.kind() == SyntaxKind::CompactConstructorDecl
            && let Some(record) = node
                .parent()
                .and_then(|b| b.parent())
                .and_then(ast::RecordDecl::cast)
        {
            for component in record.record_header().iter().flat_map(|h| h.components()) {
                let ty = self.declared_ty(component.ty(), component.varargs().is_some());
                if let Some(index) = self.declare_local(component.syntax(), component.name(), ty) {
                    self.env.locals[index as usize].at = component_type_at(component);
                }
            }
        }
    }

    /// A class body in a body: a local class's, an anonymous class's, or an enum constant's.
    fn enter_class_body(&mut self, node: SyntaxNode<'_>) -> Result<(), Cancelled> {
        let Some(parent) = node.parent() else {
            return Ok(());
        };
        if let Some(&index) = self.env.local_of_node.get(&parent.id()) {
            let summary = self.env.local_types[index as usize].summary.take();
            if let Some(summary) = summary {
                self.set_up_class(index, node, summary, None)?;
            }
            return Ok(());
        }
        let members = || {
            ast::ClassBody::cast(node)
                .map(|b| b.members())
                .into_iter()
                .flatten()
        };
        let mut generic = None;
        let (at, superclass) = match parent.kind() {
            SyntaxKind::NewExpr => {
                let new = ast::NewExpr::cast(parent).unwrap();
                let ty = match new.ty() {
                    Some(ast::Type::ClassType(c)) => self.type_of.get(&c.syntax().id()).cloned(),
                    _ => None,
                };
                let superclass = match ty {
                    Some(TypeResolution::Type(key)) => Ok(key),
                    Some(TypeResolution::Error(e)) => Err(e),
                    _ => Err(Self::not_found(Name::EMPTY, None)),
                };
                // The class or interface as written, with its arguments, for the entry's
                // generic supertype; not with `<>`, whose arguments inference gives (B3c), nor
                // after an outer instance.
                let diamond = match new.ty() {
                    Some(ast::Type::ClassType(c)) => c
                        .type_arguments()
                        .is_some_and(|a| a.arguments().next().is_none()),
                    _ => true,
                };
                if !diamond && new.outer().is_none() && superclass.is_ok() {
                    generic = Some(TypeRef::from_syntax(new.ty(), &self.names));
                }
                // Where javac's tree for the class is: its body's `{`.
                (node.span().start, superclass)
            }
            SyntaxKind::EnumConstant => {
                let constant = ast::EnumConstant::cast(parent).unwrap();
                let at = constant
                    .name()
                    .map_or(parent.span().start, |n| n.span().start);
                // The enum: the summary's type, or a local enum.
                let enumeration = parent
                    .parent()
                    .and_then(|b| b.parent())
                    .and_then(|d| self.env.local_of_node.get(&d.id()))
                    .map(|&i| self.env.local_types[i as usize].key)
                    .or_else(|| self.summary_index().map(|i| self.fs.keys[i]));
                (
                    at,
                    enumeration.ok_or_else(|| Self::not_found(Name::EMPTY, None)),
                )
            }
            _ => return Ok(()),
        };
        let names = self.names.clone();
        let summary = anonymous_type(members(), &names);
        let index = self.new_local_type(node, at, TypeKind::Class, MemberAccess::Private);
        self.register_members(index, members());
        self.set_up_class(index, node, summary, Some((superclass, generic)))
    }

    /// Resolves a local or anonymous class's header, in the scope of its header, then opens
    /// its body's scope with its member types, fields, and methods, declared and inherited.
    /// `anonymous` is an anonymous class's class or interface, with its type as written when it
    /// has one to give (an enum constant's enum has none).
    fn set_up_class(
        &mut self,
        index: u32,
        body: SyntaxNode<'_>,
        summary: TypeSummary,
        anonymous: Option<(Result<TypeKey, ErrorSymbol>, Option<TypeRef>)>,
    ) -> Result<(), Cancelled> {
        let (anonymous, generic) = match anonymous {
            Some((class, generic)) => (Some(class), generic),
            None => (None, None),
        };
        let key = self.env.local_types[index as usize].key;
        let kind = summary.kind;
        let anonymous_kind = match &anonymous {
            Some(Ok(class)) => self.kind_of(*class)?,
            _ => None,
        };
        self.env.local_of_node.insert(body.id(), index);
        let saved = std::mem::take(&mut self.r.errors);

        // The header, in the scope around the body: the type parameters, not the members.
        let scope = scope!(self);
        let type_parameters = type_params(&mut self.r, &scope, &summary.type_parameters)?;
        let superclass_ty = match (kind, summary.extends.first()) {
            (TypeKind::Class, Some(written)) => Some(self.r.ty(&scope, written, 0)?),
            _ => None,
        };
        let written: &[TypeRef] = match kind {
            TypeKind::Interface => &summary.extends,
            TypeKind::Annotation => &[],
            _ => &summary.implements,
        };
        let mut interfaces_ty = Vec::new();
        for w in written {
            interfaces_ty.push(self.r.ty(&scope, w, 0)?);
        }
        // An anonymous class's generic supertype: the class it extends, or the interface it
        // implements; an enum constant's body extends its enum.
        let mut superclass_ty = superclass_ty;
        let mut generic_interface = None;
        match (&anonymous, generic) {
            (Some(Ok(_)), Some(written)) => {
                let ty = self.r.ty(&scope, &written, 0)?;
                if matches!(
                    anonymous_kind,
                    Some(TypeKind::Interface | TypeKind::Annotation)
                ) {
                    generic_interface = Some(ty);
                } else {
                    superclass_ty = Some(ty);
                }
            }
            (Some(Ok(class)), None) if anonymous_kind == Some(TypeKind::Enum) => {
                superclass_ty = Some(Ty::of(vec![TyPart::Class {
                    ty: *class,
                    outer: false,
                    arguments: TyPart::NO_ARGUMENTS,
                }]));
            }
            _ => {}
        }
        let mut permits = Vec::new();
        for w in summary.permits.iter() {
            permits.push(self.r.ty(&scope, w, 0)?);
        }

        // The erased supertypes.
        let erased = |w: &Self, ty: &Ty| -> Result<TypeKey, ErrorSymbol> {
            match ty.parts().first() {
                Some(TyPart::Class { ty, .. }) => Ok(*ty),
                Some(TyPart::Error(e)) => Err(w.r.errors[*e as usize].clone()),
                _ => Err(Self::not_found(Name::EMPTY, None)),
            }
        };
        let mut interfaces: Vec<Result<TypeKey, ErrorSymbol>> =
            interfaces_ty.iter().map(|t| erased(self, t)).collect();
        // The erased list has an anonymous class's interface from `anonymous`, below.
        interfaces_ty.extend(generic_interface);
        let superclass = match (kind, anonymous) {
            (_, Some(Ok(class)))
                if matches!(
                    self.kind_of(class)?,
                    Some(TypeKind::Interface | TypeKind::Annotation)
                ) =>
            {
                interfaces.push(Ok(class));
                Some(self.lang("Object")?)
            }
            (_, Some(class)) => Some(class),
            (TypeKind::Class, None) => match &superclass_ty {
                Some(ty) => Some(erased(self, ty)),
                None => Some(self.lang("Object")?),
            },
            (TypeKind::Enum, None) => Some(self.lang("Enum")?),
            (TypeKind::Record, None) => Some(self.lang("Record")?),
            _ => None,
        };
        if kind == TypeKind::Annotation {
            let name = self.names.intern("Annotation");
            let package = self.names.intern("java.lang.annotation");
            interfaces.push(
                match self.r.reads.find_top_level(self.set, package, name)? {
                    Found::Type(key) => Ok(key),
                    _ => Err(Self::not_found(name, None)),
                },
            );
        }
        // A local class can name itself, or a member of its own, as a supertype.
        let cyclic = superclass
            .iter()
            .chain(&interfaces)
            .any(|s| s.as_ref().is_ok_and(|s| self.reaches(*s, key)));
        let supertypes = if cyclic {
            let cycle = |s: &Result<TypeKey, ErrorSymbol>| -> Result<TypeKey, ErrorSymbol> {
                Err(ErrorSymbol {
                    name: s.as_ref().err().map_or(Name::EMPTY, |e| e.name),
                    cause: Cause::Cycle,
                })
            };
            Supertypes {
                superclass: superclass.as_ref().map(cycle),
                interfaces: interfaces.iter().map(cycle).collect(),
                cycle: Arc::from([key]),
            }
        } else {
            Supertypes {
                superclass,
                interfaces: interfaces.into_boxed_slice(),
                cycle: Arc::from([]),
            }
        };
        let supers: Vec<Result<TypeKey, ()>> = supertypes
            .superclass
            .iter()
            .chain(supertypes.interfaces.iter())
            .map(|s| s.as_ref().copied().map_err(|_| ()))
            .collect();
        self.env.local_types[index as usize].supertypes = Some(Arc::new(supertypes));

        // The member types, declared and inherited, bound in the body's scope.
        let member_types = self.local_member_types(index, &supers)?;
        self.env.push(body.id());
        for (name, member) in member_types.iter() {
            let resolution = match member {
                MemberType::One(key) => TypeResolution::Type(*key),
                MemberType::Ambiguous(keys) => TypeResolution::Error(ErrorSymbol {
                    name,
                    cause: Cause::Ambiguous(keys.clone()),
                }),
            };
            self.env.bind_type(name, resolution);
        }
        let incomplete_members = member_types.incomplete;
        self.env.local_types[index as usize].member_types = Some(Arc::new(member_types));

        // The fields and methods, in the scope of the body.
        let mut lombok = false;
        let scope = scope!(self);
        let (fields, methods) = members_of(&mut self.r, key, &summary, &scope, &mut lombok)?;
        let errors = std::mem::replace(&mut self.r.errors, saved);
        let entry = TypeEntry {
            key,
            kind,
            modifiers: summary.modifiers,
            annotations: Box::default(),
            type_parameters,
            superclass: superclass_ty,
            interfaces: interfaces_ty.into_boxed_slice(),
            permits: permits.into_boxed_slice(),
            fields: fields.into_boxed_slice(),
            methods: methods.into_boxed_slice(),
            incomplete: incomplete_members || cyclic || lombok,
            lombok,
            generated: false,
            errors: errors.into_boxed_slice(),
        };
        self.env.local_types[index as usize].entry = Some(Arc::new(entry));

        // Its fields and methods, declared and inherited, bound in the body's scope.
        let maps = self.local_maps(index, &supers)?;
        for (name, (found, _)) in &maps.fields {
            self.env.bind_var(*name, Var::Field(found.clone()));
        }
        for (name, methods) in &maps.methods {
            let refs: Arc<[MethodRef]> = methods.iter().map(|(m, _)| *m).collect();
            self.env.bind_methods(*name, key, refs);
        }
        self.env.local_types[index as usize].maps = Some(Arc::new(maps));
        self.classes.push((body.id(), key));
        Ok(())
    }

    /// The `java.lang` class `simple`, as an implicit supertype.
    fn lang(&mut self, simple: &str) -> Result<Result<TypeKey, ErrorSymbol>, Cancelled> {
        let name = self.names.intern(simple);
        let java_lang = self.r.java_lang;
        Ok(
            match self.r.reads.find_top_level(self.set, java_lang, name)? {
                Found::Type(key) => Ok(key),
                _ => Err(Self::not_found(
                    self.names.intern(&format!("java.lang.{simple}")),
                    None,
                )),
            },
        )
    }

    /// Whether `from` is `to`, or has it among its local supertypes, transitively.
    fn reaches(&self, from: TypeKey, to: TypeKey) -> bool {
        let mut work = vec![from];
        let mut seen = Set::default();
        while let Some(key) = work.pop() {
            if key == to {
                return true;
            }
            if !seen.insert(key) {
                continue;
            }
            if let Some(supertypes) = self.env.state(key).and_then(|s| s.supertypes.as_ref()) {
                for s in supertypes
                    .superclass
                    .iter()
                    .chain(supertypes.interfaces.iter())
                {
                    if let Ok(s) = s
                        && matches!(s, TypeKey::Local { .. })
                    {
                        work.push(*s);
                    }
                }
            }
        }
        false
    }

    /// A local type's member types: the declared ones, then those its supertypes have that
    /// it can inherit, one inherited as two different types being ambiguous.
    fn local_member_types(
        &mut self,
        index: u32,
        supers: &[Result<TypeKey, ()>],
    ) -> Result<MemberTypes, Cancelled> {
        let package = self.env.package;
        let mut members: Map<Name, (MemberType, MemberAccess)> = Map::default();
        for (&name, &member) in &self.env.local_types[index as usize].declared {
            let state = &self.env.local_types[member as usize];
            members.insert(name, (MemberType::One(state.key), state.access));
        }
        let mut incomplete = false;
        let mut inherited: Map<Name, (MemberType, MemberAccess)> = Map::default();
        for supertype in supers {
            let Ok(supertype) = *supertype else {
                incomplete = true;
                continue;
            };
            let theirs = match supertype {
                TypeKey::Local { .. } => self
                    .env
                    .state(supertype)
                    .and_then(|s| s.member_types.clone()),
                _ => self.r.reads.member_types(self.set, supertype)?,
            };
            let Some(theirs) = theirs else {
                incomplete = true;
                continue;
            };
            incomplete |= theirs.incomplete;
            for (name, (member, access)) in theirs.members.iter() {
                if !inheritable(*access, package) || members.contains_key(name) {
                    continue;
                }
                match inherited.get_mut(name) {
                    None => {
                        inherited.insert(*name, (member.clone(), *access));
                    }
                    Some((existing, _)) if existing == member => {}
                    Some((existing, _)) => {
                        let mut keys: Vec<TypeKey> = Vec::new();
                        for m in [&*existing, member] {
                            match m {
                                MemberType::One(k) => keys.push(*k),
                                MemberType::Ambiguous(ks) => keys.extend(ks.iter()),
                            }
                        }
                        keys.sort();
                        keys.dedup();
                        *existing = MemberType::Ambiguous(keys.into_boxed_slice());
                    }
                }
            }
        }
        members.extend(inherited);
        let mut members: Vec<(Name, (MemberType, MemberAccess))> = members.into_iter().collect();
        members.sort_by_key(|(name, _)| *name);
        Ok(MemberTypes {
            members: members.into_boxed_slice(),
            incomplete,
        })
    }

    /// A local type's fields and methods by name, declared and inherited.
    fn local_maps(
        &mut self,
        index: u32,
        supers: &[Result<TypeKey, ()>],
    ) -> Result<ClassMaps, Cancelled> {
        let state = &self.env.local_types[index as usize];
        let key = state.key;
        let entry = state.entry.clone().expect("set up before its maps");
        let interface = matches!(entry.kind, TypeKind::Interface | TypeKind::Annotation);
        let mut maps = ClassMaps::default();
        let package = self.env.package;
        for (i, field) in entry.fields.iter().enumerate() {
            let access = if interface {
                MemberAccess::Public
            } else {
                access_of(field.modifiers, package)
            };
            maps.fields
                .entry(field.name)
                .or_insert((FieldFound::One(key, i as u32), access));
        }
        for (i, method) in entry.methods.iter().enumerate() {
            if method.kind != MethodKind::Method {
                continue;
            }
            let access = if interface && method.modifiers.contains(Modifiers::STATIC) {
                MemberAccess::Private
            } else if interface {
                MemberAccess::Public
            } else {
                access_of(method.modifiers, package)
            };
            maps.methods.entry(method.name).or_default().push((
                MethodRef {
                    owner: key,
                    index: i as u32,
                },
                access,
            ));
        }
        let mut theirs = Vec::new();
        for supertype in supers {
            match supertype {
                Ok(s) => theirs.push(Some(self.class_maps(*s)?)),
                Err(()) => theirs.push(None),
            }
        }
        inherit(&mut maps, &theirs, package);
        Ok(maps)
    }

    /// A type's fields and methods by name, declared and inherited, made once per body
    /// result for each type asked about: what a local or anonymous class inherits.
    fn class_maps(&mut self, key: TypeKey) -> Result<Arc<ClassMaps>, Cancelled> {
        self.maps_of(key, None)
    }

    /// A type's fields and methods named `name`, declared and inherited: what a lookup by name
    /// needs, which costs the hierarchy's members of that name rather than all of them.
    fn named(&mut self, key: TypeKey, name: Name) -> Result<Arc<ClassMaps>, Cancelled> {
        self.maps_of(key, Some(name))
    }

    /// A type's fields and methods, of every name or of `only`, its supertypes' first, by a
    /// loop rather than recursion, so that a long chain of supertypes doesn't overflow the
    /// stack.
    fn maps_of(&mut self, key: TypeKey, only: Option<Name>) -> Result<Arc<ClassMaps>, Cancelled> {
        if let TypeKey::Local { .. } = key {
            return Ok(self
                .env
                .state(key)
                .and_then(|s| s.maps.clone())
                .unwrap_or_else(|| {
                    Arc::new(ClassMaps {
                        incomplete: true,
                        ..ClassMaps::default()
                    })
                }));
        }
        if let Some(maps) = self.class_maps.get(&(key, only)) {
            return Ok(maps.clone());
        }
        let mut stack = vec![(key, false)];
        let mut open: Set<TypeKey> = Set::default();
        while let Some((next, ready)) = stack.pop() {
            self.r.reads.check()?;
            if self.class_maps.contains_key(&(next, only)) {
                continue;
            }
            let (supers, incomplete) = self.direct_supertypes(next)?;
            if !ready {
                // On the stack already: a cycle, whose type is made without it.
                if !open.insert(next) {
                    continue;
                }
                stack.push((next, true));
                for &s in &supers {
                    if !self.class_maps.contains_key(&(s, only)) && !open.contains(&s) {
                        stack.push((s, false));
                    }
                }
            } else {
                let maps = self.declared_maps(next, &supers, incomplete, only)?;
                self.class_maps.insert((next, only), Arc::new(maps));
                open.remove(&next);
            }
        }
        Ok(self.class_maps[&(key, only)].clone())
    }

    /// A summary type's or class file's direct supertypes, and whether any didn't resolve.
    fn direct_supertypes(&mut self, key: TypeKey) -> Result<(Vec<TypeKey>, bool), Cancelled> {
        if let Some(found) = self.supertypes.get(&key) {
            return Ok(found.clone());
        }
        let found = self.read_supertypes(key)?;
        self.supertypes.insert(key, found.clone());
        Ok(found)
    }

    fn read_supertypes(&mut self, key: TypeKey) -> Result<(Vec<TypeKey>, bool), Cancelled> {
        let Some(supertypes) = self.r.reads.supertypes(self.set, key)? else {
            return Ok((Vec::new(), true));
        };
        let mut incomplete = supertypes.cyclic();
        let mut keys = Vec::new();
        for s in supertypes
            .superclass
            .iter()
            .chain(supertypes.interfaces.iter())
        {
            match s {
                Ok(s) => keys.push(*s),
                Err(_) => incomplete = true,
            }
        }
        // An interface without superinterfaces has `Object`'s public methods as members (JLS
        // 9.2): `getClass()` in an interface's default method is searched in the interface.
        // Only `Object` itself has no superclass otherwise.
        if supertypes.superclass.is_none() && supertypes.interfaces.is_empty() {
            let object = self.names.intern("java/lang/Object");
            if let Some(found) = self.r.reads.find_binary(self.set, object)?
                && found != key
            {
                keys.push(found);
            }
        }
        Ok((keys, incomplete))
    }

    /// A summary type's or class file's own fields and methods, of every name or of `only`,
    /// then those its supertypes' maps (made already) let it inherit.
    fn declared_maps(
        &mut self,
        key: TypeKey,
        supers: &[TypeKey],
        incomplete: bool,
        only: Option<Name>,
    ) -> Result<ClassMaps, Cancelled> {
        let wanted = |name: Name| only.is_none_or(|o| o == name);
        let mut maps = ClassMaps {
            incomplete,
            ..ClassMaps::default()
        };
        let package = package_of(key, &self.names);
        if let TypeKey::Class { .. } = key {
            match self.r.reads.snapshot.class_entry(key) {
                None => maps.incomplete = true,
                Some(entry) => {
                    let interface = entry.access & 0x0200 != 0;
                    maps.incomplete |= entry.problem.is_some();
                    for (i, field) in entry.fields.iter().enumerate() {
                        if !wanted(field.name) {
                            continue;
                        }
                        maps.fields.entry(field.name).or_insert((
                            FieldFound::One(key, i as u32),
                            access_of_flags(field.access, package),
                        ));
                    }
                    for (i, method) in entry.methods.iter().enumerate() {
                        if !wanted(method.name) || self.names.text(method.name).starts_with('<') {
                            continue;
                        }
                        let access = if interface && method.access & 0x0008 != 0 {
                            MemberAccess::Private
                        } else {
                            access_of_flags(method.access, package)
                        };
                        maps.methods.entry(method.name).or_default().push((
                            MethodRef {
                                owner: key,
                                index: i as u32,
                            },
                            access,
                        ));
                    }
                }
            }
        } else {
            let entry = self.r.reads.type_entry(key)?;
            let interface = matches!(entry.kind, TypeKind::Interface | TypeKind::Annotation);
            maps.incomplete |= entry.incomplete;
            for (i, field) in entry.fields.iter().enumerate() {
                if !wanted(field.name) {
                    continue;
                }
                let access = if interface {
                    MemberAccess::Public
                } else {
                    access_of(field.modifiers, package)
                };
                maps.fields
                    .entry(field.name)
                    .or_insert((FieldFound::One(key, i as u32), access));
            }
            for (i, method) in entry.methods.iter().enumerate() {
                if method.kind != MethodKind::Method || !wanted(method.name) {
                    continue;
                }
                let access = if interface && method.modifiers.contains(Modifiers::STATIC) {
                    MemberAccess::Private
                } else if interface {
                    MemberAccess::Public
                } else {
                    access_of(method.modifiers, package)
                };
                maps.methods.entry(method.name).or_default().push((
                    MethodRef {
                        owner: key,
                        index: i as u32,
                    },
                    access,
                ));
            }
        }
        let theirs: Vec<Option<Arc<ClassMaps>>> = supers
            .iter()
            .map(|s| self.class_maps.get(&(*s, only)).cloned())
            .collect();
        inherit(&mut maps, &theirs, package);
        Ok(maps)
    }
}

/// Adds to `maps`, a type's own fields and methods, what it inherits from its supertypes'
/// `theirs` (`None` for one that didn't resolve or is in a cycle).
fn inherit(maps: &mut ClassMaps, theirs: &[Option<Arc<ClassMaps>>], package: Name) {
    let declared: Set<Name> = maps.fields.keys().copied().collect();
    for supertype in theirs {
        let Some(supertype) = supertype else {
            maps.incomplete = true;
            continue;
        };
        maps.incomplete |= supertype.incomplete;
        for (name, (found, access)) in &supertype.fields {
            if !inheritable(*access, package) || declared.contains(name) {
                continue;
            }
            match maps.fields.get_mut(name) {
                None => {
                    maps.fields.insert(*name, (found.clone(), *access));
                }
                Some((existing, _)) if existing == found => {}
                Some((existing, _)) => {
                    let mut owners: Vec<TypeKey> = Vec::new();
                    for f in [&*existing, found] {
                        match f {
                            FieldFound::One(owner, _) => owners.push(*owner),
                            FieldFound::Ambiguous(os) => owners.extend(os.iter()),
                        }
                    }
                    owners.sort();
                    owners.dedup();
                    *existing = FieldFound::Ambiguous(owners.into_boxed_slice());
                }
            }
        }
        for (name, methods) in &supertype.methods {
            for (method, access) in methods {
                if !inheritable(*access, package) {
                    continue;
                }
                let ours = maps.methods.entry(*name).or_default();
                if !ours.iter().any(|(m, _)| m == method) {
                    ours.push((*method, *access));
                }
            }
        }
    }
}
