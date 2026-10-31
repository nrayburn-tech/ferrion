//! The table's consistency.

use std::collections::HashSet;
use std::path::Path;

use ferrion_java_version::{LATEST_RELEASE, OLDEST_RELEASE, SUPPORTED_RELEASES};

use super::*;
use crate::JavacPhase;

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// -------------------------------------------------------------------------------------------
// The table.

#[test]
fn rows_are_in_the_order_of_the_variants() {
    for (i, row) in FEATURES.iter().enumerate() {
        assert_eq!(row.feature as usize, i, "{:?}", row.feature);
        assert_eq!(row.feature.row().feature, row.feature);
    }
}

#[test]
fn rows_are_ordered_by_release_with_preview_features_last() {
    let mut previous = OLDEST_RELEASE + 1;
    let mut preview = false;
    for row in &FEATURES {
        match row.status {
            Status::Final(release) => {
                assert!(
                    !preview,
                    "{:?} is final after a preview feature",
                    row.feature
                );
                assert!(
                    release >= previous && release <= LATEST_RELEASE,
                    "{:?}: release {release} is out of order or unsupported",
                    row.feature
                );
                assert!(!row.feature.is_in(release - 1) && row.feature.is_in(release));
                previous = release;
            }
            Status::Preview => {
                preview = true;
                assert!(SUPPORTED_RELEASES.clone().all(|r| !row.feature.is_in(r)));
            }
        }
    }
}

#[test]
fn names_and_ids_are_unique() {
    let names: HashSet<_> = FEATURES.iter().map(|row| row.name).collect();
    let ids: HashSet<_> = Feature::all().map(Feature::id).collect();
    assert_eq!(names.len(), FEATURES.len());
    assert_eq!(ids.len(), FEATURES.len());
    assert_eq!(Feature::Records.id(), "records");
    assert_eq!(Feature::ImplicitClasses.id(), "implicit_classes");
}

#[test]
fn javac_codes_fit_the_phase() {
    for row in &FEATURES {
        let not_supported = row
            .javac
            .code
            .starts_with("feature.not.supported.in.source");
        if not_supported || row.javac.code == "preview.feature.disabled.plural" {
            assert!(row.javac.argument.is_some(), "{:?}", row.feature);
        }
        if row.status == Status::Preview {
            assert_eq!(row.javac_phase, JavacPhase::Later, "{:?}", row.feature);
        }
    }
}

#[test]
fn messages() {
    assert_eq!(
        Feature::Records.message(11),
        "records require Java 16+ (the target is Java 11)"
    );
    assert_eq!(
        Feature::CaseNull.message(17),
        "'case null' requires Java 21+ (the target is Java 17)"
    );
    assert_eq!(
        Feature::PrimitivePatterns.message(LATEST_RELEASE),
        "primitive types in patterns are a preview feature, not supported"
    );
}

#[test]
fn restricted_names() {
    for name in RestrictedName::ALL {
        assert_eq!(RestrictedName::from_text(name.text()), Some(name));
        assert!(SUPPORTED_RELEASES.contains(&name.restricted_from()));
    }
    assert_eq!(RestrictedName::from_text("when"), None);
    assert_eq!(RestrictedName::from_text("module"), None);
}

/// The feature table in docs/design/parser-language-versions.md lists every row, with its
/// release and javac's phase.
#[test]
fn the_design_document_lists_every_row() {
    let doc = read(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/design/parser-language-versions.md"),
    );
    let rows: Vec<Vec<&str>> = doc
        .lines()
        .skip_while(|l| !l.starts_with("| Feature | Needs | Anchor | javac phase |"))
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .map(|l| l.trim_matches('|').split(" | ").map(str::trim).collect())
        .collect();
    assert_eq!(rows.len(), FEATURES.len(), "rows in the document: {rows:?}");
    for (row, cells) in FEATURES.iter().zip(&rows) {
        assert_eq!(
            cells[0], row.name,
            "the document's rows are in the table's order"
        );
        let needs = match row.status {
            Status::Final(release) => release.to_string(),
            Status::Preview => "preview".to_owned(),
        };
        assert_eq!(cells[1], needs, "{:?}", row.feature);
        let phase = match row.javac_phase {
            JavacPhase::Parse => "parse",
            JavacPhase::Later => "later",
            JavacPhase::Never => "never",
        };
        assert!(
            cells[3].starts_with(phase),
            "{:?}: {}",
            row.feature,
            cells[3]
        );
    }
}
