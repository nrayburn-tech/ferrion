//! `ProjectInput::from_project`: a project model's source sets become the engine's, with their
//! files, generated roots, and the source sets on their classpaths, in order.

use std::fs;
use std::path::{Path, PathBuf};

use ferrion_project::model::{ModelEntry, ModelSourceSet, ProjectModel};
use ferrion_project::{Environment, InputFlags, Project};
use ferrion_semantic::{ClassPathInput, ProjectInput, SourceSetId};

/// A directory under the target directory, emptied first.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_model_becomes_the_engine_inputs() {
    let dir = scratch("project-input");
    write(&dir.join("main/src/p/A.java"), "package p; class A {}\n");
    write(
        &dir.join("main/src/p/q/B.java"),
        "package p.q; class B {}\n",
    );
    write(&dir.join("main/src/p/notes.txt"), "not java\n");
    write(&dir.join("main/gen/p/G.java"), "package p; class G {}\n");
    write(
        &dir.join("test/src/p/ATest.java"),
        "package p; class ATest {}\n",
    );
    fs::write(
        dir.join("main/src/p/Latin1.java"),
        b"class L { char c = '\xe9'; }\n",
    )
    .unwrap();
    let set =
        |id: &str, roots: &[&str], generated: &[&str], classpath: Vec<ModelEntry>| ModelSourceSet {
            id: id.to_owned(),
            java_release: 21,
            jdk: None,
            source_roots: roots.iter().map(|r| dir.join(r)).collect(),
            generated_source_roots: generated.iter().map(|r| dir.join(r)).collect(),
            classpath,
            module_path: Vec::new(),
        };
    let model = ProjectModel {
        format_version: ferrion_project::model::FORMAT_VERSION,
        source_sets: vec![
            set("main", &["main/src"], &["main/gen"], Vec::new()),
            set(
                "test",
                &["test/src"],
                &[],
                vec![ModelEntry::source_set("main")],
            ),
        ],
    };
    let model_path = dir.join("model.json");
    fs::write(&model_path, model.to_json().unwrap()).unwrap();
    let flags = InputFlags {
        project_model: Some(model_path),
        ..InputFlags::default()
    };
    let env = Environment {
        working_dir: dir.clone(),
        java_home: None,
    };
    let project = Project::load(&flags, &env).unwrap();
    let (input, skipped) = ProjectInput::from_project(&project, None);

    let main = &input.source_sets[0];
    assert_eq!(main.name, "main");
    assert_eq!(main.release, 21);
    let files: Vec<(String, bool)> = main
        .files
        .iter()
        .map(|f| {
            let path = f.path.strip_prefix(&dir).unwrap();
            (path.display().to_string(), f.generated)
        })
        .collect();
    assert_eq!(
        files,
        [
            ("main/src/p/A.java".to_owned(), false),
            ("main/src/p/q/B.java".to_owned(), false),
            ("main/gen/p/G.java".to_owned(), true),
        ]
    );
    // A file that isn't UTF-8 is left out and listed.
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].path.ends_with("Latin1.java"));

    let test = &input.source_sets[1];
    assert_eq!(test.files.len(), 1);
    assert!(matches!(
        test.classpath.as_slice(),
        [ClassPathInput::SourceSet(id)] if *id == SourceSetId::from_index(0)
    ));
    // Without type inputs, there is no platform.
    assert!(test.platform.is_none());
}
