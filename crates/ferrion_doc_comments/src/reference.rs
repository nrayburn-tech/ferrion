//! References to program elements, such as `java.util.List#add(int, Object)`.
//!
//! The syntax follows the "References" section of the Documentation Comment Specification, and
//! what javadoc 27 was seen to accept: a module before `/`, a package or type that may have type
//! arguments, a member after `#` with an optional parameter list, or a fragment after `##`.
//! Parameter types may have type arguments, `[]`s, `...`, and a parameter name, which javadoc
//! ignores, and whitespace (including line breaks) is allowed inside the parentheses only.
//! `Type.member(params)` without `#` is a member too, as javadoc reads it. Markdown comments write
//! `[]` as `\[\]` inside a link label, so both spellings are read.

use crate::MAX_NESTING;
use crate::content::Content;
use crate::model::{
    Member, Parameter, Parameters, QualifiedName, RefType, Reference, Text, TypeArgument,
    WildcardBound,
};

/// Parses the reference in `content.text[start..end]`, returning it and whether it is
/// well-formed. A malformed reference has the parts that could be read before the problem.
pub(crate) fn parse(content: &Content, start: usize, end: usize) -> (Reference, bool) {
    let mut parser = RefParser {
        content,
        text: &content.text,
        pos: start,
        end,
        ok: true,
    };
    let reference = parser.reference(start);
    (reference, parser.ok && parser.pos == end)
}

/// Parses `text[start..end]` as a type, such as the type of `@serialField`.
pub(crate) fn parse_type(content: &Content, start: usize, end: usize) -> (Option<RefType>, bool) {
    let mut parser = RefParser {
        content,
        text: &content.text,
        pos: start,
        end,
        ok: true,
    };
    let ty = parser.ref_type(0, true);
    let ok = parser.ok && parser.pos == end;
    (ty, ok)
}

/// Whether `text` reads as a well-formed reference. Markdown comments use this to tell a link to
/// a program element, such as `[List]`, from brackets that are just text, such as `[1]`.
pub(crate) fn is_reference(text: &str) -> bool {
    let mut content = Content::new(0);
    for c in crate::escape::chars(text, 0, text.len()) {
        // Only the structure matters here, so escapes needn't be told apart.
        content.push_char(crate::escape::Char {
            escaped: false,
            ..c
        });
    }
    let len = content.len();
    !text.is_empty() && parse(&content, 0, len).1
}

pub(crate) fn is_identifier_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

pub(crate) fn is_identifier_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

struct RefParser<'a> {
    content: &'a Content,
    text: &'a str,
    pos: usize,
    end: usize,
    ok: bool,
}

impl RefParser<'_> {
    fn reference(&mut self, start: usize) -> Reference {
        let mut reference = Reference {
            span: self.content.span(start..self.end),
            text: self.text[start..self.end].to_owned(),
            module: None,
            qualifier: None,
            member: None,
            fragment: None,
        };
        if start == self.end {
            self.ok = false;
            return reference;
        }
        // A module comes before a `/` that comes before any member or parameters.
        let head = &self.text[start..self.end];
        let head = &head[..head.find(['#', '(']).unwrap_or(head.len())];
        if let Some(slash) = head.find('/') {
            let module = self.qualified_name();
            if self.pos != start + slash {
                self.ok = false;
                return reference;
            }
            reference.module = module;
            self.pos += 1;
        }
        if self.pos < self.end && !self.at('#') {
            reference.qualifier = self.ref_type(0, false);
            if !self.ok {
                return reference;
            }
            if self.at('(') {
                // `Type.member(params)` or `member(params)`, without `#`.
                let Some(mut qualifier) = reference.qualifier.take() else {
                    return reference;
                };
                if !qualifier.type_arguments.is_empty() {
                    self.ok = false;
                    reference.qualifier = Some(qualifier);
                    return reference;
                }
                let name = qualifier.name.segments.pop();
                if let (Some(first), Some(last)) = (
                    qualifier.name.segments.first(),
                    qualifier.name.segments.last(),
                ) {
                    let span = first.span.cover(last.span);
                    qualifier.name.span = span;
                    qualifier.span = span;
                    reference.qualifier = Some(qualifier);
                }
                if let Some(name) = name {
                    let parameters = self.parameters();
                    reference.member = Some(Member { name, parameters });
                }
                return reference;
            }
        }
        if self.text[self.pos..self.end].starts_with("##") {
            self.pos += 2;
            if self.pos == self.end {
                self.ok = false;
            } else {
                reference.fragment = Some(self.text_node(self.pos, self.end));
                self.pos = self.end;
            }
        } else if self.eat('#') {
            match self.identifier() {
                Some(name) => {
                    let parameters = if self.at('(') {
                        self.parameters()
                    } else {
                        None
                    };
                    reference.member = Some(Member { name, parameters });
                }
                None => self.ok = false,
            }
        }
        reference
    }

    fn parameters(&mut self) -> Option<Parameters> {
        let start = self.pos;
        self.eat('(');
        let mut list = Vec::new();
        self.skip_whitespace();
        if !self.eat(')') {
            loop {
                self.skip_whitespace();
                let param_start = self.pos;
                let Some(ty) = self.ref_type(0, true) else {
                    break;
                };
                let after_type = self.pos;
                self.skip_whitespace();
                let name = if self.pos > after_type {
                    self.identifier()
                } else {
                    None
                };
                let param_end = if name.is_some() { self.pos } else { after_type };
                list.push(Parameter {
                    span: self.content.span(param_start..param_end),
                    ty,
                    name,
                });
                self.skip_whitespace();
                if self.eat(')') {
                    break;
                }
                if !self.eat(',') {
                    self.ok = false;
                    break;
                }
            }
        }
        // When the list is malformed, it ends where the problem is, and the caller sees that the
        // reference doesn't end there.
        Some(Parameters {
            span: self.content.span(start..self.pos),
            list,
        })
    }

    /// A type: a qualified name, type arguments, and, for a parameter, `[]`s and `...`.
    fn ref_type(&mut self, depth: usize, parameter: bool) -> Option<RefType> {
        let start = self.pos;
        let Some(name) = self.qualified_name() else {
            self.ok = false;
            return None;
        };
        let mut ty = RefType {
            span: name.span,
            name,
            type_arguments: Vec::new(),
            dimensions: 0,
            varargs: false,
        };
        if self.at('<') {
            if depth >= MAX_NESTING {
                self.ok = false;
                return Some(ty);
            }
            self.pos += 1;
            loop {
                self.skip_whitespace();
                match self.type_argument(depth + 1) {
                    Some(argument) => ty.type_arguments.push(argument),
                    None => {
                        self.ok = false;
                        break;
                    }
                }
                self.skip_whitespace();
                if self.eat('>') {
                    break;
                }
                if !self.eat(',') {
                    self.ok = false;
                    break;
                }
            }
        }
        if parameter && self.ok {
            while let Some(after) = self.dimension() {
                ty.dimensions += 1;
                self.pos = after;
            }
            if self.text[self.pos..self.end].starts_with("...") {
                ty.varargs = true;
                self.pos += 3;
            }
        }
        ty.span = self.content.span(start..self.pos);
        Some(ty)
    }

    /// Where a `[]` or `\[\]` at the current position ends, if there is one.
    fn dimension(&self) -> Option<usize> {
        let rest = &self.text[self.pos..self.end];
        let open = if rest.starts_with('[') {
            1
        } else if rest.starts_with("\\[") {
            2
        } else {
            return None;
        };
        let inner = rest[open..].trim_start_matches([' ', '\t', '\n']);
        let skipped = rest.len() - open - inner.len();
        let close = if inner.starts_with(']') {
            1
        } else if inner.starts_with("\\]") {
            2
        } else {
            return None;
        };
        Some(self.pos + open + skipped + close)
    }

    fn type_argument(&mut self, depth: usize) -> Option<TypeArgument> {
        let start = self.pos;
        if !self.eat('?') {
            return self.ref_type(depth, true).map(TypeArgument::Type);
        }
        let after_question = self.pos;
        self.skip_whitespace();
        let bound = if self.keyword("extends") {
            Some(WildcardBound::Extends)
        } else if self.keyword("super") {
            Some(WildcardBound::Super)
        } else {
            self.pos = after_question;
            None
        };
        let bound = match bound {
            Some(kind) => {
                self.skip_whitespace();
                Some((kind, self.ref_type(depth, true)?))
            }
            None => None,
        };
        Some(TypeArgument::Wildcard {
            span: self.content.span(start..self.pos),
            bound,
        })
    }

    /// Eats `word` if it is the next identifier.
    fn keyword(&mut self, word: &str) -> bool {
        match self.text[self.pos..self.end].strip_prefix(word) {
            Some(after) if !after.starts_with(is_identifier_part) => {
                self.pos += word.len();
                true
            }
            _ => false,
        }
    }

    fn qualified_name(&mut self) -> Option<QualifiedName> {
        let start = self.pos;
        let mut segments = vec![self.identifier()?];
        while self.at('.') && !self.text[self.pos..self.end].starts_with("...") {
            self.pos += 1;
            match self.identifier() {
                Some(segment) => segments.push(segment),
                None => {
                    self.ok = false;
                    break;
                }
            }
        }
        let span = self.content.span(start..self.pos);
        Some(QualifiedName { span, segments })
    }

    fn identifier(&mut self) -> Option<Text> {
        let start = self.pos;
        let mut chars = self.text[self.pos..self.end].char_indices();
        match chars.next() {
            Some((_, c)) if is_identifier_start(c) => {}
            _ => return None,
        }
        let len = chars
            .find(|&(_, c)| !is_identifier_part(c))
            .map_or(self.end - start, |(i, _)| i);
        self.pos = start + len;
        Some(self.text_node(start, self.pos))
    }

    fn text_node(&self, start: usize, end: usize) -> Text {
        Text {
            span: self.content.span(start..end),
            text: self.text[start..end].to_owned(),
        }
    }

    fn skip_whitespace(&mut self) {
        let rest = &self.text[self.pos..self.end];
        self.pos += rest.len() - rest.trim_start_matches([' ', '\t', '\x0C', '\n']).len();
    }

    fn at(&self, c: char) -> bool {
        self.text[self.pos..self.end].starts_with(c)
    }

    fn eat(&mut self, c: char) -> bool {
        let at = self.at(c);
        if at {
            self.pos += c.len_utf8();
        }
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

    fn content(text: &str) -> Content {
        let mut content = Content::new(0);
        for c in crate::escape::chars(text, 0, text.len()) {
            content.push_char(c);
        }
        content
    }

    fn reference(text: &str) -> (Reference, bool) {
        let content = content(text);
        parse(&content, 0, text.len())
    }

    fn names(name: &QualifiedName) -> Vec<&str> {
        name.segments.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn type_and_member() {
        let (r, ok) = reference("java.util.List#add(int, Object)");
        assert!(ok);
        let qualifier = r.qualifier.as_ref().unwrap();
        assert_eq!(names(&qualifier.name), ["java", "util", "List"]);
        assert_eq!(qualifier.span, Span::new(0, 14));
        let member = r.member.unwrap();
        assert_eq!(member.name.text, "add");
        let parameters = member.parameters.unwrap();
        assert_eq!(parameters.span, Span::new(18, 31));
        let types: Vec<_> = parameters.list.iter().map(|p| names(&p.ty.name)).collect();
        assert_eq!(types, [["int"], ["Object"]]);
        assert_eq!(parameters.list[1].span, Span::new(24, 30));
    }

    #[test]
    fn module_package_and_fragment() {
        let (r, ok) = reference("java.base/java.lang.String##chars");
        assert!(ok);
        assert_eq!(names(&r.module.unwrap()), ["java", "base"]);
        assert_eq!(
            names(&r.qualifier.unwrap().name),
            ["java", "lang", "String"]
        );
        assert_eq!(r.fragment.unwrap().text, "chars");
        // A module alone, as `[java.base/]` links to it.
        let (r, ok) = reference("java.base/");
        assert!(ok);
        assert!(r.module.is_some() && r.qualifier.is_none());
    }

    #[test]
    fn parameters_with_names_arrays_varargs_and_generics() {
        // javadoc 27 accepts parameter names, and generics in the type and the parameters.
        let (r, ok) = reference("#m( int count , String... rest, int[][] a, List<? extends T> l)");
        assert!(ok, "{r:?}");
        let list = r.member.unwrap().parameters.unwrap().list;
        assert_eq!(list[0].name.as_ref().unwrap().text, "count");
        assert!(list[1].ty.varargs);
        assert_eq!(list[2].ty.dimensions, 2);
        assert!(matches!(
            &list[3].ty.type_arguments[0],
            TypeArgument::Wildcard {
                bound: Some((WildcardBound::Extends, _)),
                ..
            }
        ));
        let (r, ok) = reference("java.util.List<String>");
        assert!(ok);
        assert_eq!(r.qualifier.unwrap().type_arguments.len(), 1);
    }

    #[test]
    fn markdown_escaped_brackets() {
        // JEP 467's example of an array parameter in a Markdown link.
        let (r, ok) = reference("String#copyValueOf(char\\[\\])");
        assert!(ok);
        assert_eq!(
            r.member.unwrap().parameters.unwrap().list[0].ty.dimensions,
            1
        );
    }

    #[test]
    fn member_without_hash() {
        // javadoc 27 links `[m()]` in a Markdown comment to the method `m()`.
        let (r, ok) = reference("m()");
        assert!(ok);
        assert!(r.qualifier.is_none());
        assert_eq!(r.member.unwrap().name.text, "m");
        let (r, ok) = reference("a.B.m(int)");
        assert!(ok);
        assert_eq!(names(&r.qualifier.unwrap().name), ["a", "B"]);
    }

    #[test]
    fn malformed() {
        // javadoc 27 reports "syntax error in reference" for `C#` and `C.`.
        for text in [
            "C#", "C.", "#m(", "#m(int", "#m(int,)", "1x", "", "a/b/c", "A<B", "A##", "#",
        ] {
            let (_, ok) = reference(text);
            assert!(!ok, "{text:?}");
        }
        let (r, _) = reference("#m(int");
        assert_eq!(r.member.unwrap().parameters.unwrap().list.len(), 1);
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = format!("A{}", "<A".repeat(10_000));
        assert!(!reference(&deep).1);
    }

    #[test]
    fn types_are_listed_in_source_order() {
        let (r, _) = reference("Map<K, List<V>>#put(Key, Set<? super Val>)");
        let found: Vec<_> = r
            .types()
            .iter()
            .map(|t| t.name.segments[0].text.as_str())
            .collect();
        assert_eq!(found, ["Map", "K", "List", "V", "Key", "Set", "Val"]);
    }

    #[test]
    fn looks_like_a_reference() {
        for text in [
            "List",
            "java.util.List",
            "List#add(Object)",
            "#m",
            "java.base/",
            "d",
        ] {
            assert!(is_reference(text), "{text:?}");
        }
        for text in ["1", "see above", "", "a b", "x]"] {
            assert!(!is_reference(text), "{text:?}");
        }
    }
}
