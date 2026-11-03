//! `ProjectInput::from_project`: a project model's source sets become the engine's, with their
//! files, generated roots, and the source sets on their classpaths, in order.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrion_classfile::testing::{ClassBuilder, ZipBuilder};
use ferrion_classfile::{ArchiveCache, ClassName};
use ferrion_project::model::{ModelEntry, ModelSourceSet, PlatformKind, ProjectModel};
use ferrion_project::{Environment, InputFlags, Project, TypeInputs};
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

/// A jar holding a class of each name.
fn jar(path: &Path, classes: &[&str]) {
    let mut b = ZipBuilder::new();
    for class in classes {
        b.stored(
            &format!("{class}.class"),
            &ClassBuilder::new(class).finish(),
        );
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b.finish()).unwrap();
}

/// Loads `model`, written to `dir`, with its type inputs opened.
fn load(dir: &Path, model: &ProjectModel) -> (Project, TypeInputs) {
    let model_path = dir.join("model.json");
    fs::write(&model_path, model.to_json().unwrap()).unwrap();
    let flags = InputFlags {
        project_model: Some(model_path),
        ..InputFlags::default()
    };
    let env = Environment {
        working_dir: dir.to_path_buf(),
        java_home: None,
    };
    let project = Project::load(&flags, &env).unwrap();
    let (types, _) = TypeInputs::open(&project, &ArchiveCache::new()).unwrap();
    (project, types)
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
            ..ModelSourceSet::default()
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
            // With `/` between the parts on every platform.
            (path.display().to_string().replace('\\', "/"), f.generated)
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

/// Source sets with the same platform and compile options share one platform, so the engine has
/// one class source for it: two would make the same class two types, and a member type
/// inherited through a dependency's type and the reader's own (guava's `Map.Entry`) ambiguous. A
/// patched platform is another, and names the patch as where its classes come from.
#[test]
fn source_sets_with_the_same_options_share_a_platform() {
    let dir = scratch("project-input-platforms");
    // JDK 8, which `-source 8` sees whole.
    write(&dir.join("jdk/release"), "JAVA_VERSION=\"1.8.0_502\"\n");
    jar(
        &dir.join("jdk/jre/lib/rt.jar"),
        &["java/lang/Object", "java/util/Map", "java/util/Map$Entry"],
    );
    jar(&dir.join("api.jar"), &["java/lang/foreign/Arena"]);
    for root in ["main", "test", "patched"] {
        fs::create_dir_all(dir.join(root)).unwrap();
    }
    let set = |id: &str, classpath: Vec<ModelEntry>| ModelSourceSet {
        id: id.to_owned(),
        java_release: 8,
        platform: PlatformKind::Jdk,
        jdk: Some(dir.join("jdk")),
        source_roots: vec![dir.join(id)],
        classpath,
        ..ModelSourceSet::default()
    };
    let mut patched = set("patched", Vec::new());
    patched
        .patch_modules
        .insert("java.base".to_owned(), vec![dir.join("api.jar")]);
    let model = ProjectModel {
        format_version: ferrion_project::model::FORMAT_VERSION,
        source_sets: vec![
            set("main", Vec::new()),
            set("test", vec![ModelEntry::source_set("main")]),
            patched,
        ],
    };
    let (project, types) = load(&dir, &model);
    let (input, _) = ProjectInput::from_project(&project, Some(&types));
    let platform = |i: usize| input.source_sets[i].platform.clone().unwrap();
    assert!(Arc::ptr_eq(&platform(0), &platform(1)));
    assert!(!Arc::ptr_eq(&platform(0), &platform(2)));

    let arena = ClassName::new("java/lang/foreign/Arena");
    let object = ClassName::new("java/lang/Object");
    assert_eq!(platform(2).patched_from(&arena), Some(dir.join("api.jar")));
    assert_eq!(platform(2).patched_from(&object), None);
    assert!(platform(2).find(&object).unwrap().is_some());
    assert_eq!(platform(0).patched_from(&object), None);
}
