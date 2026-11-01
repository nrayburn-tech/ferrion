use super::*;

fn token(width: u32) -> (TokenId, u32, u32) {
    (TokenId::from_index(0), width, width)
}

fn text(b: &mut DocumentBuilder, width: u32) {
    let (id, len, width) = token(width);
    b.token(id, len, width);
}

fn group_at(document: &Document, index: u32) -> (u32, bool, bool) {
    match document.elements[index as usize] {
        Element::Group {
            width,
            forced,
            stopped,
            ..
        } => (width, forced, stopped),
        other => panic!("{other:?} isn't a group"),
    }
}

#[test]
fn a_groups_width_is_its_flat_width_plus_the_rest_of_the_line() {
    let mut b = DocumentBuilder::new();
    let outer = b.start_group();
    text(&mut b, 3);
    b.line();
    let inner = b.start_group();
    text(&mut b, 4);
    b.soft_line();
    text(&mut b, 5);
    b.end();
    b.end();
    // After the outer group, up to the next line element.
    text(&mut b, 2);
    b.space();
    text(&mut b, 1);
    b.hard_line();
    text(&mut b, 50);
    let document = b.finish();
    // 3 + 1 + 4 + 0 + 5, then 2 + 1 + 1 after it.
    assert_eq!(group_at(&document, outer), (13 + 4, false, false));
    // The inner group's rest is the same: nothing between its end and the outer's.
    assert_eq!(group_at(&document, inner), (9 + 4, false, false));
    let Element::Group { end, .. } = document.elements[outer as usize] else {
        unreachable!()
    };
    assert_eq!(document.elements[end as usize], Element::End);
}

#[test]
fn a_hard_line_forces_every_group_around_it() {
    let mut b = DocumentBuilder::new();
    let outer = b.start_group();
    let inner = b.start_group();
    text(&mut b, 3);
    b.hard_line();
    b.end();
    b.end();
    let after = b.start_group();
    text(&mut b, 3);
    b.end();
    let document = b.finish();
    assert!(group_at(&document, outer).1);
    assert!(group_at(&document, inner).1);
    assert_eq!(group_at(&document, after), (3, false, false));
}

#[test]
fn a_break_in_a_hug_only_stops_the_measure_outside_it() {
    let mut b = DocumentBuilder::new();
    let outer = b.start_group();
    text(&mut b, 10);
    b.start_hug();
    let inside = b.start_group();
    text(&mut b, 1);
    b.hard_line();
    text(&mut b, 80);
    b.end();
    b.end();
    text(&mut b, 20);
    b.end();
    text(&mut b, 30);
    let document = b.finish();
    // Measured up to the break, without the rest of the line.
    assert_eq!(group_at(&document, outer), (11, false, true));
    assert!(group_at(&document, inside).1);
}

#[test]
fn a_hug_in_a_group_spares_only_that_group() {
    let mut b = DocumentBuilder::new();
    let outer = b.start_group();
    text(&mut b, 10);
    let arguments = b.start_group();
    b.start_indent_if_broken(arguments);
    b.start_hug_in_group();
    let inside = b.start_group();
    text(&mut b, 1);
    b.hard_line();
    text(&mut b, 80);
    b.end();
    b.hard_line();
    b.end();
    b.end();
    b.end();
    b.end();
    let document = b.finish();
    assert_eq!(group_at(&document, arguments), (1, false, true));
    assert_eq!(group_at(&document, outer), (11, true, true));
    assert!(group_at(&document, inside).1);
}

#[test]
fn copied_text_with_a_line_break_forces_its_groups() {
    let mut b = DocumentBuilder::new();
    let group = b.start_group();
    text(&mut b, 4);
    b.verbatim(Verbatim {
        start: 0,
        end: 0,
        kind: VerbatimKind::Comment,
        first_width: 6,
        breaks: true,
    });
    b.end();
    let document = b.finish();
    assert_eq!(group_at(&document, group), (10, true, true));
}

#[test]
fn a_fill_break_records_the_width_of_the_next_item() {
    let mut b = DocumentBuilder::new();
    b.start_fill();
    text(&mut b, 2);
    b.fill_break(false);
    text(&mut b, 3);
    b.space();
    text(&mut b, 4);
    b.fill_break(true);
    text(&mut b, 5);
    b.end();
    let document = b.finish();
    let nexts: Vec<_> = document
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::FillBreak { soft, next } => Some((*soft, *next)),
            _ => None,
        })
        .collect();
    assert_eq!(nexts, [(false, 8), (true, 5)]);
}

#[test]
fn a_fills_last_break_counts_the_width_after_the_fill() {
    let mut b = DocumentBuilder::new();
    b.start_fill();
    text(&mut b, 2);
    b.fill_break(false);
    text(&mut b, 3);
    b.fill_break(false);
    text(&mut b, 4);
    b.end();
    text(&mut b, 6);
    b.line();
    text(&mut b, 7);
    let document = b.finish();
    let nexts: Vec<_> = document
        .elements
        .iter()
        .filter_map(|e| match e {
            Element::FillBreak { next, .. } => Some(*next),
            _ => None,
        })
        .collect();
    assert_eq!(nexts, [3, 10]);
}

#[test]
#[should_panic(expected = "still open")]
fn finishing_with_an_open_container_panics() {
    let mut b = DocumentBuilder::new();
    b.start_group();
    b.finish();
}

#[test]
fn deep_nesting_with_a_break_at_every_level_is_linear() {
    // Each break marks only the containers not marked yet, so this is quick at any depth.
    let mut b = DocumentBuilder::new();
    let depth = 200_000;
    for _ in 0..depth {
        b.start_group();
        text(&mut b, 1);
        b.hard_line();
    }
    for _ in 0..depth {
        b.end();
    }
    let document = b.finish();
    assert_eq!(document.elements.len(), depth * 4);
}

#[test]
fn widths_count_utf16_code_units() {
    assert_eq!(width("abc"), 3);
    assert_eq!(width("é"), 1);
    assert_eq!(width("中文"), 2);
    // Outside the Basic Multilingual Plane: two code units each.
    assert_eq!(width("😀😀"), 4);
}
