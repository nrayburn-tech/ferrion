//! Modifiers, annotations, and annotation arguments.
//!
//! Where an annotation goes depends on the declaration (observed in GJF):
//!
//! - **Own lines:** the annotations before the first keyword modifier of a type, method,
//!   constructor, enum constant, package, or module each go on their own line. Annotations after
//!   a keyword modifier stay on its line: `@A` then `public @B void m()`.
//! - **Fields:** the same if any of those annotations has arguments; otherwise they stay on the
//!   field's line, `@Inject Foo foo;`, unless the field doesn't fit on it, when they go on a line
//!   of their own before it.
//! - **Locals:** on their own lines if there are two or more, if one has arguments, or if the
//!   declaration doesn't fit on its line; otherwise inline, `@A int x = 1;`.
//! - **Parameters and record components:** inline; when the parameter doesn't fit, the
//!   annotations break onto their own lines at +4 as a group, then the rest of the parameter.

use std::cell::Cell;

use ferrion_syntax::{NodeOrToken, SyntaxKind as K, SyntaxNode};

use super::{
    ListLayout, delimited_list, emit, first_token, fits_flat, fits_on_line,
    leads_on_separator_line, parts,
};
use crate::document::IndentKind;
use crate::format::Formatter;
use crate::modifiers::{self, Item};

/// How a declaration's modifiers are laid out; see the module's documentation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Layout {
    /// Each leading annotation followed by a line break.
    OwnLines,
    /// The same, except for type-use annotations right before the type, which stay on its line:
    /// a method's or a field's.
    BeforeType,
    /// Leading annotations with a space between them and a `Line` after the last, in the
    /// group of the declaration around them: a field's marker annotations.
    LineAfter,
    /// Leading annotations in a group with a `Line` between them, at +4 after the first, and a
    /// `Line` after the last, in the group of the parameter around them. The rest of the
    /// parameter is at +4 too: this leaves an indent open, which the caller ends after it.
    Parameter,
    /// Everything on one line, with a space between items.
    Inline,
}

/// What a parent puts between a `Modifiers` node and what follows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum After {
    /// A space.
    Space,
    /// Nothing: the modifiers ended with a line break, or a `Line`.
    Nothing,
}

/// The rule for a `Modifiers` node formatted on its own, as one in another area's construct is
/// (a local variable, a catch or lambda parameter, a pattern): local variables in a block get the
/// locals' layout, and the rest are inline. A line break after the modifiers is emitted here,
/// since the parent puts only a space after them.
pub(super) fn modifiers_node<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let layout = match node.parent() {
        Some(local)
            if local.kind() == K::LocalVarDecl
                && local
                    .parent()
                    .is_some_and(|p| matches!(p.kind(), K::Block | K::SwitchGroup)) =>
        {
            local_layout(node, local)
        }
        Some(parent) if super::super::is_member(parent.kind()) => match parent.kind() {
            K::FieldDecl => field_layout(node),
            K::MethodDecl => Layout::BeforeType,
            _ => Layout::OwnLines,
        },
        Some(parent) if parent.kind() == K::EnumConstant => Layout::OwnLines,
        _ => Layout::Inline,
    };
    let layout = if layout == Layout::LineAfter {
        Layout::Inline
    } else {
        layout
    };
    modifiers(f, node, layout);
}

/// The layout of a local variable's modifiers, `node`, in `local`.
fn local_layout(node: SyntaxNode<'_>, local: SyntaxNode<'_>) -> Layout {
    let leading = leading_annotations(node);
    if leading.is_empty() {
        Layout::Inline
    } else if leading.len() > 1
        || leading.iter().any(|&a| has_arguments(a))
        || !fits_on_line(local, true)
    {
        Layout::OwnLines
    } else {
        Layout::Inline
    }
}

/// The layout of a field's modifiers: own lines if a leading annotation has arguments, otherwise
/// a `Line` after the leading annotations, for the field's group to break.
pub(super) fn field_layout(node: SyntaxNode<'_>) -> Layout {
    let leading = leading_annotations(node);
    if leading.is_empty() {
        Layout::Inline
    } else if leading.iter().any(|&a| has_arguments(a)) {
        Layout::BeforeType
    } else {
        Layout::LineAfter
    }
}

/// The annotations of `modifiers` before its first keyword modifier.
pub(super) fn leading_annotations(modifiers: SyntaxNode<'_>) -> Vec<SyntaxNode<'_>> {
    modifiers
        .children_with_tokens()
        .filter(|part| !part.as_token().is_some_and(|t| t.is_trivia()))
        .map_while(|part| part.as_node().filter(|n| n.kind() == K::Annotation))
        .collect()
}

/// Whether `annotation` has arguments: `@A(1)`, but not `@A` or `@A()`.
pub(super) fn has_arguments(annotation: SyntaxNode<'_>) -> bool {
    annotation
        .children()
        .any(|child| child.kind() == K::AnnotationArguments && parts(child).len() > 2)
}

/// Emits a `Modifiers` node: its items in source order, keyword runs reordered unless the list
/// is in a disable region or holds a marker, laid out by `layout`. Returns what goes after them.
pub(super) fn modifiers<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>, layout: Layout) -> After {
    // A region that starts with a comment before the list is entered when that comment is
    // placed, so place it first.
    f.comments_before(node.first_token().id());
    let reorder = !f.in_region() && !f.has_marker(node);
    let items = modifiers::items(node);
    let leading = items
        .iter()
        .take_while(|item| matches!(item, Item::Annotation(_)))
        .count();
    // The leading annotations that go on lines of their own.
    let own_lines = match layout {
        Layout::BeforeType if leading == items.len() => leading - type_use_suffix(&items),
        Layout::OwnLines | Layout::BeforeType => leading,
        _ => 0,
    };
    let parameter = layout == Layout::Parameter && leading > 0;
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            match layout {
                _ if i <= own_lines => {}
                Layout::LineAfter if i == leading => f.line(),
                Layout::Parameter if i <= leading => f.line(),
                _ => f.space(),
            }
        }
        if parameter && i == 0 {
            f.start_group();
        }
        match item {
            Item::Annotation(annotation) => f.node(*annotation),
            Item::Run(run) => modifiers::run(f, run, reorder),
        }
        // A parameter's annotations after the first are at +4, and so is the rest of the
        // parameter. Each indent starts after text, since the printer indents a line by the
        // indent its first text is in.
        if parameter && i == 0 && leading > 1 {
            f.start_indent(IndentKind::Continuation);
        }
        if parameter && i + 1 == leading {
            if leading > 1 {
                f.end();
            }
            f.end();
            f.start_indent(IndentKind::Continuation);
        }
        if i < own_lines {
            // A block comment after the annotation, on its line, stays there.
            let next = match items.get(i + 1) {
                Some(Item::Annotation(annotation)) => Some(annotation.first_token()),
                Some(Item::Run(run)) => Some(run[0][0]),
                None => node.last_token().next_non_trivia(),
            };
            if let Some(next) = next
                && leads_on_separator_line(next)
                && !f.in_region()
            {
                f.comments_before(next.id());
            }
            f.no_blank_line();
            f.hard_line();
        }
    }
    match layout {
        _ if leading < items.len() => After::Space,
        _ if own_lines == items.len() => After::Nothing,
        Layout::LineAfter | Layout::Parameter => {
            f.line();
            After::Nothing
        }
        _ => After::Space,
    }
}

/// How many of the annotations at the end of `items`, all annotations, are type-use annotations
/// that GJF keeps on the line of the type after them (see [`is_type_use`]).
fn type_use_suffix(items: &[Item<'_>]) -> usize {
    items
        .iter()
        .rev()
        .take_while(|item| matches!(item, Item::Annotation(a) if is_type_use(*a)))
        .count()
}

/// The type-use annotations GJF recognizes, by their imports (observed: `@Nullable String m()`
/// stays on one line when `Nullable` is imported from one of these, and not from other
/// packages, nor with a wildcard import or a qualified name).
const TYPE_USE: &[&str] = &[
    "org.jspecify.annotations.Nullable",
    "org.jspecify.annotations.NonNull",
    "org.checkerframework.checker.nullness.qual.Nullable",
    "org.checkerframework.checker.nullness.qual.NonNull",
];

thread_local! {
    /// The simple names of the [`TYPE_USE`] annotations the file being formatted imports: set by
    /// [`with_type_use`] while the compilation unit's rule formats the file, so that the rules
    /// below it can look them up without scanning the imports again.
    static IMPORTED_TYPE_USE: Cell<[bool; TYPE_USE.len()]> = const { Cell::new([false; TYPE_USE.len()]) };
}

/// Runs `format` with the type-use annotations `unit`, a compilation unit, imports, for
/// [`is_type_use`] to see.
pub(super) fn with_type_use<R>(unit: SyntaxNode<'_>, format: impl FnOnce() -> R) -> R {
    let mut imported = [false; TYPE_USE.len()];
    for import in unit
        .children()
        .take_while(|n| matches!(n.kind(), K::PackageDecl | K::ImportDecl))
    {
        if import.kind() != K::ImportDecl {
            continue;
        }
        let parts = parts(import);
        if let [_, NodeOrToken::Node(name), _] = parts.as_slice()
            && name.kind() == K::Name
        {
            for (i, &qualified) in TYPE_USE.iter().enumerate() {
                imported[i] |= name_is(*name, qualified);
            }
        }
    }
    struct Restore([bool; TYPE_USE.len()]);
    impl Drop for Restore {
        fn drop(&mut self) {
            IMPORTED_TYPE_USE.set(self.0);
        }
    }
    let _restore = Restore(IMPORTED_TYPE_USE.replace(imported));
    format()
}

/// Whether `name`, a `Name` node, is the qualified name `qualified`, comparing its identifiers.
/// It stops at the first one that differs.
fn name_is(name: SyntaxNode<'_>, qualified: &str) -> bool {
    let mut rest = qualified;
    for token in name.tokens().filter(|t| !t.is_trivia()) {
        let text = match token.kind() {
            K::Dot => ".",
            _ => token.text(),
        };
        match rest.strip_prefix(text) {
            Some(tail) => rest = tail,
            None => return false,
        }
    }
    rest.is_empty()
}

/// Whether `annotation` is one of the [`TYPE_USE`] annotations the file imports, without
/// arguments.
fn is_type_use(annotation: SyntaxNode<'_>) -> bool {
    let annotation = parts(annotation);
    let [_, NodeOrToken::Node(name)] = annotation.as_slice() else {
        return false;
    };
    let name = parts(*name);
    let [NodeOrToken::Token(identifier)] = name.as_slice() else {
        return false;
    };
    let imported = IMPORTED_TYPE_USE.get();
    TYPE_USE.iter().zip(imported).any(|(qualified, imported)| {
        imported && qualified.rsplit('.').next() == Some(identifier.text())
    })
}

/// Emits an annotation of a package or a module, which has no `Modifiers` node, on its own line.
pub(super) fn own_lines<'t>(f: &mut Formatter<'t>, annotation: SyntaxNode<'t>) {
    f.node(annotation);
    f.hard_line();
}

/// The rule for an annotation: `@`, its name, and its arguments, with no spaces.
pub(super) fn annotation<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    for part in parts(node) {
        emit(f, part);
    }
}

/// The rule for an annotation's arguments: one per line at +4 when they don't fit on the line,
/// and always when there are two or more and one is a pair whose value is an array, as in
/// `x = {1}` (observed in GJF). A single array of values hugs the parentheses, `@A({` with the
/// values below.
pub(super) fn arguments<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let parts = parts(node);
    if let [open, array, close] = parts.as_slice()
        && array.kind() == K::ElementValueArray
    {
        emit(f, *open);
        emit(f, *array);
        emit(f, *close);
        return;
    }
    let arguments = parts.iter().filter(|part| part.as_node().is_some()).count();
    let array_pair = parts.iter().any(|part| {
        part.as_node().is_some_and(|pair| {
            pair.kind() == K::ElementValuePair
                && pair
                    .children()
                    .any(|value| value.kind() == K::ElementValueArray)
        })
    });
    let layout = if arguments >= 2 && array_pair {
        ListLayout::Broken
    } else {
        ListLayout::OnePerLine
    };
    delimited_list(f, node, layout);
}

/// The rule for an element-value pair, `name = value`, breaking after the `=` as a variable
/// declarator does.
pub(super) fn pair<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    let mut after_eq = false;
    for (i, part) in parts(node).into_iter().enumerate() {
        if after_eq {
            value_after_eq(f, part);
            after_eq = false;
            continue;
        }
        if i > 0 {
            f.space();
        }
        emit(f, part);
        after_eq = part.kind() == K::Eq;
    }
}

/// Emits the value after an `=`, in a variable declarator or an element-value pair: on the line
/// if it's an array initializer, whose braces break instead, and otherwise in a group that breaks
/// before it, at +4.
pub(super) fn value_after_eq<'t>(f: &mut Formatter<'t>, value: ferrion_syntax::NodeOrToken<'t>) {
    if matches!(value.kind(), K::ArrayInitializer | K::ElementValueArray) {
        f.space();
        emit(f, value);
    } else {
        f.group(|f| {
            f.indent(IndentKind::Continuation, |f| {
                f.line();
                emit(f, value);
            });
        });
    }
}

/// The widest an element can be, in columns, for an array of values to be filled, as many to a
/// line as fit, rather than one per line (observed in GJF).
const FILL_WIDTH: u32 = 9;

/// The rule for an array of element values, `{a, b}`: on one line if it fits; otherwise the
/// values go on the lines between the braces at +2, as many to a line as fit if each is at most
/// [`FILL_WIDTH`] wide, and otherwise all on one line or one per line. A trailing comma keeps the
/// braces broken.
pub(super) fn array<'t>(f: &mut Formatter<'t>, node: SyntaxNode<'t>) {
    f.comments_before(node.first_token().id());
    let parts = parts(node);
    let [open, items @ .., close] = parts.as_slice() else {
        parts.into_iter().for_each(|part| emit(f, part));
        return;
    };
    if items.is_empty() {
        emit(f, *open);
        super::dangling(f, first_token(*close));
        emit(f, *close);
        return;
    }
    let trailing_comma = items.last().is_some_and(|part| part.kind() == K::Comma);
    let fill = items.iter().all(|part| match part.as_node() {
        Some(value) => fits_flat(value, FILL_WIDTH, true),
        None => part.kind() == K::Comma,
    });
    f.group(|f| {
        emit(f, *open);
        f.indent(IndentKind::Block, |f| {
            f.soft_line();
            if fill {
                f.fill(|f| filled(f, items));
            } else {
                f.group(|f| super::separated(f, items, false));
            }
            f.comments_before(first_token(*close).id());
        });
        if trailing_comma {
            f.hard_line();
        } else {
            f.soft_line();
        }
    });
    // After the group, so that a line comment after it doesn't break the array.
    emit(f, *close);
}

/// Emits `items`, values and the commas between them, in a fill.
fn filled<'t>(f: &mut Formatter<'t>, items: &[ferrion_syntax::NodeOrToken<'t>]) {
    let mut after_comma = false;
    for &part in items {
        if part.kind() == K::Comma {
            emit(f, part);
            after_comma = true;
            continue;
        }
        if after_comma {
            f.fill_break(false);
        }
        emit(f, part);
        after_comma = false;
    }
}
