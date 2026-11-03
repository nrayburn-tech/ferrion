use std::sync::Arc;

use super::*;

const OPTIONS: FormatOptions = FormatOptions { release: 27 };

const TEXT: &str = "class A {\n  // a  comment\n  int   x;\n  //b\n  int   y;\n}\n";

fn source(text: &str) -> SourceText {
    SourceText::decode(text).unwrap()
}

/// The range of `needle`'s first occurrence in `TEXT`.
fn range_of(needle: &str) -> Range<u32> {
    let start = TEXT.find(needle).unwrap() as u32;
    start..start + needle.len() as u32
}

/// An edit replacing `old`, the first occurrence in `TEXT`, with `new`.
fn replace(old: &str, new: &str) -> Edit {
    Edit {
        range: range_of(old),
        text: new.to_owned(),
    }
}

fn check(range: Range<u32>, edits: &[Edit]) -> Result<(), Violation> {
    verify_range(&source(TEXT), range, edits, &OPTIONS)
}

#[test]
fn format_ranges_edits_pass() {
    for needle in ["x", "//b", "y;", "a  comment", "}"] {
        let range = range_of(needle);
        let tree = parse(Arc::from(TEXT), ParseOptions { release: 27 }).tree;
        let edits = format_range(&tree, &source(TEXT), range.clone()).unwrap();
        check(range, &edits).unwrap();
    }
}

#[test]
fn an_edit_outside_the_range_is_reported() {
    let violation = check(range_of("x"), &[replace("   y", " y")]).unwrap_err();
    assert_eq!(violation.property, Property::Range);
}

#[test]
fn a_range_without_tokens_has_no_edits() {
    let violation = check(1..1, &[replace("   x", " x")]).unwrap_err();
    assert_eq!(violation.property, Property::Range);
    check(9..10, &[]).unwrap();
}

#[test]
fn an_edit_of_code_or_comments_outside_the_units_is_reported() {
    // The comment before `y` is in its unit, not `x`'s.
    let violation = check(range_of("x"), &[replace("//b", "// b")]).unwrap_err();
    assert_eq!(violation.property, Property::Range);
}

#[test]
fn an_edit_splitting_a_token_is_reported() {
    let violation = check(range_of("x"), &[replace("nt   x", "nt x")]).unwrap_err();
    assert_eq!(violation.property, Property::Range);
}

#[test]
fn a_changed_tree_or_comment_is_reported() {
    let violation = check(range_of("x"), &[replace("x", "z")]).unwrap_err();
    assert_eq!(violation.property, Property::Equivalence);
    let violation = check(
        range_of("a  comment"),
        &[replace("// a  comment", "// a comment")],
    )
    .unwrap_err();
    assert_eq!(violation.property, Property::Comments);
}

#[test]
fn a_comment_in_the_range_may_get_its_space() {
    let edits = [replace("//b", "// b"), replace("   y", " y")];
    check(range_of("y"), &edits).unwrap();
}

#[test]
fn a_range_left_unformatted_is_reported() {
    let violation = check(range_of("x"), &[]).unwrap_err();
    assert_eq!(violation.property, Property::Idempotence);
}

#[test]
fn moved_offsets_keep_their_tokens() {
    let edits = [
        Edit {
            range: 5..5,
            text: "  ".to_owned(),
        },
        Edit {
            range: 10..14,
            text: "x".to_owned(),
        },
    ];
    // An insertion at the start of a range comes before it, at its end after it.
    assert_eq!(moved(&edits, 5, false), 7);
    assert_eq!(moved(&edits, 5, true), 5);
    // An offset in a replaced span moves to the replacement's start, or for an end, its end.
    assert_eq!(moved(&edits, 12, false), 12);
    assert_eq!(moved(&edits, 12, true), 13);
    assert_eq!(moved(&edits, 20, false), 19);
}
