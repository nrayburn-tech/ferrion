//! Tests of the type relations: the JLS's examples, and each observation of javac with its
//! answer beside it (docs/design/semantic-type-relations.md#observations).

use crate::testing::case_input;
use crate::types::{Printed, TypeContext, TypeId};
use crate::{Engine, SourceSetId};

/// An engine over a case's files, in the stand-in platform's view.
fn engine(text: &str) -> Engine {
    Engine::new(case_input(text))
}

/// The type of the field `field` of the class `binary`.
fn field(cx: &mut TypeContext<'_>, binary: &str, field: &str) -> TypeId {
    let key = cx
        .lang(binary)
        .unwrap()
        .unwrap_or_else(|| panic!("no class {binary}"));
    let index = cx
        .field_index(key, field)
        .unwrap()
        .unwrap_or_else(|| panic!("no field {field}"));
    cx.field_type(key, index).unwrap()
}

fn text(cx: &mut TypeContext<'_>, t: TypeId) -> String {
    cx.type_string(t, Printed::Captures).unwrap()
}

#[test]
fn translates_a_field() {
    let engine = engine(
        "//- set main 21\n//- file main p/A.java\npackage p; import java.util.List; class A<T> { List<String> f; T t; }\n",
    );
    let snapshot = engine.snapshot();
    let mut cx = snapshot.types(SourceSetId::from_index(0));
    let f = field(&mut cx, "p/A", "f");
    assert_eq!(text(&mut cx, f), "java/util/List<java/lang/String>");
    let t = field(&mut cx, "p/A", "t");
    assert_eq!(text(&mut cx, t), "T:p/A");
}
