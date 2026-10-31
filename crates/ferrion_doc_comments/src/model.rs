//! The parts of a doc comment that traditional and Markdown comments share: block tags, inline
//! tags, references, HTML, and diagnostics.
//!
//! Every part has a [`Span`] in the source text, covering it as written: Unicode escapes, and the
//! line breaks and leading `*`s or `///`s of the lines it spans. Text values are the content as
//! javadoc reads it, with Unicode escapes translated.

use crate::Span;

/// A run of text: its value, and where it is written.
///
/// An [`Inline::Text`], a name, or a reference part is on one line, and its value is exactly what
/// its span covers with Unicode escapes translated, so where the two differ, the text has escapes.
/// Quoted values (`@see "..."`, a snippet attribute's `"..."`) leave out the quotes their span
/// covers. In Markdown content, the value is what the Markdown parser reads, with backslash
/// escapes and entities resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text {
    pub span: Span,
    /// The text with Unicode escapes translated.
    pub text: String,
}

/// A problem found while parsing. The structure around it is still produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// An inline tag with no matching `}`. It runs to the end of the comment, as in javadoc. The
    /// span is the tag's `{@name`.
    UnclosedInlineTag,
    /// `@` or `{@` not followed by a tag name, such as `@ x` at the start of a line. The span is
    /// the `@` or `{@`.
    MissingTagName,
    /// A `<` that doesn't begin an HTML element, end tag, or comment. It is read as text.
    MalformedHtml,
    /// A `&` that doesn't begin an entity (`&name;`, `&#nn;`, or `&#xhh;`). It is read as text.
    MalformedEntity,
    /// A tag with no argument where one is needed, such as `@param` without a name, or `{@link}`
    /// without a reference. The span is the tag's name.
    MissingArgument,
    /// An argument that is malformed, such as `@param <T` or an unclosed quoted string.
    MalformedArgument,
    /// A reference that doesn't follow the reference syntax, such as `Foo#` or `#m(int`. The
    /// reference is still produced, with what could be read of it.
    MalformedReference,
    /// Content where a tag takes none, or more than it takes, such as `{@docRoot x}`, or text
    /// after a snippet's `:` on the same line.
    UnexpectedContent,
    /// Inline tags, references, or Markdown nested more deeply than the parsers follow (see
    /// [`MAX_NESTING`](crate::MAX_NESTING)). The span is where the limit was reached.
    NestingTooDeep,
}

/// Inline content: what a traditional comment's description and block tags hold, and what the
/// labels and descriptions of inline tags hold in both kinds of comment.
///
/// Nodes are in source order and don't overlap. Text is split at line breaks, so the formatter
/// can tell where the lines of the comment are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(Text),
    /// A line break: the line terminator, and the next line's leading whitespace and `*`s in a
    /// traditional comment, or its `///` and common indentation in a Markdown comment. Whitespace
    /// after the `*`s is content, which matters in `<pre>`.
    LineBreak(Span),
    /// `@@`, `@*`, or `@/` where javadoc reads it as an escape (traditional comments only).
    Escape(Escape),
    /// `&name;`, `&#nn;`, or `&#xhh;`.
    Entity(Entity),
    StartElement(StartElement),
    EndElement(EndElement),
    /// `<!-- ... -->`
    HtmlComment(HtmlComment),
    Tag(Box<InlineTag>),
}

impl Inline {
    pub fn span(&self) -> Span {
        match self {
            Inline::Text(text) => text.span,
            Inline::LineBreak(span) => *span,
            Inline::Escape(escape) => escape.span,
            Inline::Entity(entity) => entity.span,
            Inline::StartElement(element) => element.span,
            Inline::EndElement(element) => element.span,
            Inline::HtmlComment(comment) => comment.span,
            Inline::Tag(tag) => tag.span,
        }
    }
}

/// An escape sequence of a traditional comment: `@@` for `@` at the start of a line or after `{`,
/// `@*` for `*` at the start of a line, and `@/` for `/` after `*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Escape {
    pub span: Span,
    /// The character it stands for.
    pub ch: char,
}

/// An HTML character reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub span: Span,
    /// What is between `&` and `;`: a name such as `amp`, or `#64` or `#x40`. Whether a name
    /// exists is for consumers to check.
    pub name: String,
}

/// An HTML start tag, such as `<a href="x">` or `<br/>`. Elements are not matched with their end
/// tags: start and end tags are separate nodes, as in javadoc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartElement {
    pub span: Span,
    pub name: Text,
    pub attributes: Vec<Attribute>,
    /// Whether the tag ends with `/>`.
    pub self_closing: bool,
}

/// An HTML end tag, such as `</a>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndElement {
    pub span: Span,
    pub name: Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// The name, and the value if there is one.
    pub span: Span,
    pub name: Text,
    pub value: Option<AttributeValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeValue {
    /// The value, with its quotes if it has them.
    pub span: Span,
    /// `'"'`, `'\''`, or `None` for an unquoted value.
    pub quote: Option<char>,
    /// The value without its quotes: text, line breaks, entities, and inline tags, as in
    /// `href="{@docRoot}/x.html"`.
    pub content: Vec<Inline>,
}

/// `<!-- ... -->`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlComment {
    pub span: Span,
    /// What is between `<!--` and `-->`: text and line breaks.
    pub content: Vec<Inline>,
}

/// An inline tag, `{@name ...}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineTag {
    /// From `{` to the closing `}`, or to the end of the comment when it isn't closed.
    pub span: Span,
    /// The tag name, without the `@`.
    pub name: Text,
    pub content: InlineTagContent,
    /// Whether the tag has its closing `}`. An unclosed tag runs to the end of the comment and
    /// has an [`UnclosedInlineTag`](DiagnosticKind::UnclosedInlineTag) diagnostic.
    pub closed: bool,
}

/// What an inline tag holds, by tag. Content that is read as written ("literal text" in the
/// specification) holds only [`Inline::Text`] and [`Inline::LineBreak`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineTagContent {
    /// `{@code text}`, literal text. Braces in it are balanced. One space or tab after the name
    /// separates it from the text and isn't part of it, as in javadoc.
    Code(Vec<Inline>),
    /// `{@literal text}`, read like `{@code}`.
    Literal(Vec<Inline>),
    /// `{@docRoot}`
    DocRoot,
    /// `{@index term description}`. The term is a word, or a phrase in double quotes; its text
    /// leaves the quotes out.
    Index {
        term: Option<Text>,
        description: Vec<Inline>,
    },
    /// `{@inheritDoc}` or `{@inheritDoc supertype}`.
    InheritDoc { supertype: Option<Reference> },
    /// `{@link reference label}`
    Link {
        reference: Option<Reference>,
        label: Vec<Inline>,
    },
    /// `{@linkplain reference label}`
    LinkPlain {
        reference: Option<Reference>,
        label: Vec<Inline>,
    },
    /// `{@return description}`
    Return(Vec<Inline>),
    /// `{@snippet attributes : body}`
    Snippet(Snippet),
    /// `{@summary description}`
    Summary(Vec<Inline>),
    /// `{@systemProperty name}`
    SystemProperty { name: Option<Text> },
    /// `{@value format reference}`, both optional. The format is a word starting with `%`, or a
    /// string in double quotes, whose text leaves the quotes out.
    Value {
        format: Option<Text>,
        reference: Option<Reference>,
    },
    /// Any other tag. User-defined tags take literal text, so this holds the content after the
    /// name and any whitespace, as written. Tags nested more deeply than
    /// [`MAX_NESTING`](crate::MAX_NESTING) are read this way too, whatever their name.
    Other(Vec<Inline>),
}

/// `{@snippet attributes : body}` (JEP 413).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub attributes: Vec<SnippetAttribute>,
    /// The inline body: the lines after the line with the `:`, up to the closing `}`, as written,
    /// as text and line breaks. `None` for an external snippet, which has no `:`. Markup comments
    /// (`// @highlight ...`) are part of the text; they aren't parsed.
    pub body: Option<Vec<Inline>>,
}

/// A snippet attribute, `name`, `name=value`, `name="value"`, or `name='value'`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetAttribute {
    pub span: Span,
    pub name: Text,
    /// The value, without its quotes. Its span covers the quotes.
    pub value: Option<Text>,
}

/// A block tag, `@name ...`, which runs to the next block tag or the end of the comment. `B` is
/// the body: [`Inline`] content in a traditional comment, and Markdown blocks in a Markdown
/// comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockTag<B> {
    /// From the `@` to the end of the body, leaving out trailing whitespace and line breaks.
    pub span: Span,
    /// The tag name, without the `@`. Empty, with a
    /// [`MissingTagName`](DiagnosticKind::MissingTagName) diagnostic, for an `@` that isn't
    /// followed by one.
    pub name: Text,
    pub kind: BlockTagKind,
    pub argument: Option<TagArgument>,
    /// What follows the name and argument: the description, or the label of an `@see`
    /// reference. Leading and trailing whitespace and line breaks are left out.
    pub body: B,
}

/// The block tags of the Documentation Comment Specification for the Standard Doclet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockTagKind {
    Author,
    Deprecated,
    /// `@exception`, a synonym for `@throws`.
    Exception,
    Hidden,
    Param,
    Provides,
    Return,
    See,
    Serial,
    SerialData,
    SerialField,
    Since,
    Spec,
    Throws,
    Uses,
    Version,
    /// Any other tag, including ones the JDK defines with `-tag`, such as `@apiNote`.
    Other,
}

impl BlockTagKind {
    pub fn from_name(name: &str) -> BlockTagKind {
        match name {
            "author" => BlockTagKind::Author,
            "deprecated" => BlockTagKind::Deprecated,
            "exception" => BlockTagKind::Exception,
            "hidden" => BlockTagKind::Hidden,
            "param" => BlockTagKind::Param,
            "provides" => BlockTagKind::Provides,
            "return" => BlockTagKind::Return,
            "see" => BlockTagKind::See,
            "serial" => BlockTagKind::Serial,
            "serialData" => BlockTagKind::SerialData,
            "serialField" => BlockTagKind::SerialField,
            "since" => BlockTagKind::Since,
            "spec" => BlockTagKind::Spec,
            "throws" => BlockTagKind::Throws,
            "uses" => BlockTagKind::Uses,
            "version" => BlockTagKind::Version,
            _ => BlockTagKind::Other,
        }
    }
}

/// The argument of a block tag, which comes before its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagArgument {
    /// `@param name` or `@param <T>`.
    Param(ParamName),
    /// The exception of `@throws` and `@exception`, the service type of `@provides` and `@uses`,
    /// and the reference of `@see reference label`.
    Reference(Reference),
    /// `@see "text"`. The text leaves the quotes out; the span covers them.
    Quoted(Text),
    /// `@serialField name type`.
    SerialField { name: Text, ty: Option<RefType> },
    /// The URL of `@spec URL title`.
    Url(Text),
}

/// The name in `@param name` or `@param <T>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamName {
    /// The name, with its angle brackets for a type parameter.
    pub span: Span,
    pub name: Text,
    /// Whether the name is in angle brackets, naming a type parameter.
    pub type_parameter: bool,
}

/// A reference to a program element: `module/package.Type#member(parameters)`, or
/// `module/package.Type##fragment`.
///
/// Which parts are present is all the parser knows. A qualifier such as `a.b` may name a package
/// or a type, and resolving it is for consumers. `Type.member(params)`, without `#`, is read as a
/// member, as javadoc does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub span: Span,
    /// The reference as javadoc reads it, with any line breaks as `\n`.
    pub text: String,
    /// The module, before a `/`.
    pub module: Option<QualifiedName>,
    /// The package or type, before any `#`.
    pub qualifier: Option<RefType>,
    pub member: Option<Member>,
    /// What follows `##`, naming an anchor in the element's documentation.
    pub fragment: Option<Text>,
}

/// Dot-separated identifiers, such as `java.util.List`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualifiedName {
    pub span: Span,
    /// The identifiers, without the dots.
    pub segments: Vec<Text>,
}

/// A type in a reference: a qualifier such as `java.util.List<String>`, or a parameter type such as
/// `int[]` or `String...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefType {
    pub span: Span,
    pub name: QualifiedName,
    pub type_arguments: Vec<TypeArgument>,
    /// The number of `[]`s (written `\[\]` in Markdown comments).
    pub dimensions: u32,
    /// Whether the type ends with `...`.
    pub varargs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeArgument {
    Type(RefType),
    /// `?`, `? extends T`, or `? super T`.
    Wildcard {
        span: Span,
        bound: Option<(WildcardBound, RefType)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WildcardBound {
    Extends,
    Super,
}

/// The member part of a reference, after `#`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: Text,
    /// The parameter list, for a method or constructor written with one.
    pub parameters: Option<Parameters>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameters {
    /// From `(` to `)`.
    pub span: Span,
    pub list: Vec<Parameter>,
}

/// A parameter in a reference: a type, optionally followed by a name, which javadoc allows and
/// ignores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub span: Span,
    pub ty: RefType,
    pub name: Option<Text>,
}

impl Reference {
    /// The types the reference names: its qualifier, its parameter types, and their type
    /// arguments, in source order. Each is a name that may need an import.
    pub fn types(&self) -> Vec<&RefType> {
        let mut types = Vec::new();
        let mut pending: Vec<&RefType> = Vec::new();
        if let Some(member) = &self.member
            && let Some(parameters) = &member.parameters
        {
            pending.extend(parameters.list.iter().rev().map(|p| &p.ty));
        }
        pending.extend(&self.qualifier);
        // Depth-first, in source order, without recursion.
        while let Some(ty) = pending.pop() {
            types.push(ty);
            for argument in ty.type_arguments.iter().rev() {
                match argument {
                    TypeArgument::Type(ty) => pending.push(ty),
                    TypeArgument::Wildcard {
                        bound: Some((_, ty)),
                        ..
                    } => pending.push(ty),
                    TypeArgument::Wildcard { bound: None, .. } => {}
                }
            }
        }
        types
    }
}
