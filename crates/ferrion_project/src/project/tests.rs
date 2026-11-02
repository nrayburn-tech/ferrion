use super::*;
use crate::testing::{Scratch, render};

fn flags(java_version: u32) -> InputFlags {
    InputFlags {
        java_version: Some(java_version),
        ..InputFlags::default()
    }
}

fn load_errors(flags: &InputFlags, scratch: &Scratch) -> String {
    render(
        &Project::load(flags, &scratch.env()).unwrap_err(),
        &scratch.0,
    )
}

fn model_flags(scratch: &Scratch, json: &str) -> InputFlags {
    scratch.write("build/model.json", json.as_bytes());
    InputFlags {
        project_model: Some("build/model.json".into()),
        ..InputFlags::default()
    }
}

fn paths(items: &[ClassPathItem]) -> Vec<(PathBuf, Origin)> {
    items
        .iter()
        .map(|item| match item {
            ClassPathItem::Path(path) => (path.value.clone(), path.origin.clone()),
            ClassPathItem::SourceSet(_) => panic!("a source set"),
        })
        .collect()
}

#[test]
fn no_release() {
    let scratch = Scratch::new("no-release");
    assert_eq!(
        load_errors(&InputFlags::default(), &scratch),
        "error: no Java version was given\n  from: --java-version\n  help: pass --java-version, \
         or --project-model"
    );
}

#[test]
fn release_outside_the_supported_range() {
    let scratch = Scratch::new("range");
    assert_eq!(
        load_errors(&flags(7), &scratch),
        "error: Java 7 isn't supported; Ferrion supports Java 8 through 27\n  from: --java-version"
    );
    let flags = model_flags(
        &scratch,
        r#"{"formatVersion": 1, "sourceSets": [{"id": "a", "javaRelease": 99, "sourceRoots": []}]}"#,
    );
    assert_eq!(
        load_errors(&flags, &scratch),
        "error: Java 99 isn't supported; Ferrion supports Java 8 through 27\n  from: project \
         model `build/model.json`, source set `a`"
    );
}

#[test]
fn single_source_set_from_flags() {
    let scratch = Scratch::new("flags");
    let mut env = scratch.env();
    env.java_home = Some("/opt/home-jdk".into());
    let project = Project::load(&flags(17), &env).unwrap();
    assert!(!project.is_from_model());
    let [set] = project.source_sets() else {
        panic!("one source set");
    };
    assert_eq!(set.name, "default");
    assert_eq!(set.roots, std::slice::from_ref(&scratch.0));
    assert_eq!(set.release.origin, Origin::Flag("--java-version"));
    // Without a classpath flag, the type inputs are missing.
    assert_eq!(set.classpath, None);
    assert!(set.module_path.is_empty());
    assert_eq!(
        set.jdk,
        Some(Given {
            value: "/opt/home-jdk".into(),
            origin: Origin::JavaHome
        })
    );
    // Every file is in the one source set, wherever it is.
    let file = Path::new("/elsewhere/A.java");
    assert_eq!(project.source_sets_of(file), [set.id]);
    assert_eq!(
        project.release_of(file),
        Some((17, Origin::Flag("--java-version")))
    );
    assert!(!project.is_generated(file));

    // `--jdk` wins over JAVA_HOME, and relative paths are against the working directory.
    let flags = InputFlags {
        jdk: Some("jdks/21".into()),
        classpath: Some(String::new()),
        module_path: Some("mods:../lib/m.jar".into()),
        ..flags(21)
    };
    let project = Project::load(&flags, &env).unwrap();
    let set = &project.source_sets()[0];
    assert_eq!(
        set.jdk,
        Some(Given {
            value: scratch.path("jdks/21"),
            origin: Origin::Flag("--jdk")
        })
    );
    // An empty classpath is given, unlike an unset one.
    assert_eq!(set.classpath, Some(Vec::new()));
    assert_eq!(
        set.module_path,
        [
            Given {
                value: scratch.path("mods"),
                origin: Origin::Flag("--module-path")
            },
            Given {
                value: scratch.0.parent().unwrap().join("lib/m.jar"),
                origin: Origin::Flag("--module-path")
            },
        ]
    );
}

#[test]
fn classpath_flag() {
    let scratch = Scratch::new("classpath");
    let flags = InputFlags {
        classpath: Some(format!(
            "lib/a.jar{PATH_SEPARATOR}{PATH_SEPARATOR}/abs/classes"
        )),
        ..flags(11)
    };
    let project = Project::load(&flags, &scratch.env()).unwrap();
    let classpath = project.source_sets()[0].classpath.as_ref().unwrap();
    assert_eq!(
        paths(classpath),
        [
            (scratch.path("lib/a.jar"), Origin::Flag("--classpath")),
            ("/abs/classes".into(), Origin::Flag("--classpath")),
        ]
    );
}

#[test]
fn classpath_files() {
    let dir = Path::new("/work");
    let file = Path::new("build/cp.txt");
    let origin = |line| Origin::ClassPathFile {
        path: file.into(),
        line,
    };
    // `mvn dependency:build-classpath` output: one line, no line break at the end.
    let maven = class_path_file("/m2/a.jar:/m2/b.jar", file, ':', dir);
    assert_eq!(
        paths(&maven),
        [
            ("/m2/a.jar".into(), origin(1)),
            ("/m2/b.jar".into(), origin(1)),
        ]
    );
    // One entry per line, with blank lines, CR LF, and a relative entry.
    let lines = class_path_file("/m2/a.jar\r\n\r\nlib/b.jar\n\n/classes\n", file, ':', dir);
    assert_eq!(
        paths(&lines),
        [
            ("/m2/a.jar".into(), origin(1)),
            ("/work/lib/b.jar".into(), origin(3)),
            ("/classes".into(), origin(5)),
        ]
    );
    // Windows separators: `:` is part of a path there.
    let windows = class_path_file(r"C:\m2\a.jar;C:\m2\b.jar", file, ';', dir);
    assert_eq!(windows.len(), 2);
    assert_eq!(
        paths(&windows)[0].0,
        absolute(dir, Path::new(r"C:\m2\a.jar"))
    );
    assert_eq!(
        Origin::ClassPathFile {
            path: file.into(),
            line: 3
        }
        .to_string(),
        "--classpath-file build/cp.txt, line 3"
    );
}

#[test]
fn classpath_file_flag() {
    let scratch = Scratch::new("classpath-file");
    scratch.write(
        "cp.txt",
        format!("a.jar{PATH_SEPARATOR}b.jar\nc.jar\n").as_bytes(),
    );
    let flags = InputFlags {
        classpath_file: Some("cp.txt".into()),
        ..flags(21)
    };
    let project = Project::load(&flags, &scratch.env()).unwrap();
    let classpath = paths(project.source_sets()[0].classpath.as_ref().unwrap());
    assert_eq!(classpath.len(), 3);
    assert_eq!(
        classpath[2],
        (
            scratch.path("c.jar"),
            Origin::ClassPathFile {
                path: "cp.txt".into(),
                line: 2
            }
        )
    );
}

#[test]
fn classpath_flag_problems() {
    let scratch = Scratch::new("classpath-problems");
    let both = InputFlags {
        classpath: Some(String::new()),
        classpath_file: Some("cp.txt".into()),
        ..flags(21)
    };
    assert_eq!(
        load_errors(&both, &scratch),
        "error: --classpath and --classpath-file can't both be given\n  from: --classpath-file"
    );
    let missing = InputFlags {
        classpath_file: Some("gone.txt".into()),
        ..flags(21)
    };
    let errors = load_errors(&missing, &scratch);
    assert!(
        errors.starts_with(
            "error: can't read the classpath file `gone.txt`: No such file or directory"
        ),
        "{errors}"
    );
    assert!(errors.ends_with("\n  from: --classpath-file"), "{errors}");
}

const MODEL: &str = r#"{
  "formatVersion": 1,
  "sourceSets": [
    {
      "id": "core/main",
      "javaRelease": 21,
      "jdk": "/opt/jdk-21",
      "sourceRoots": ["../core/src/main/java"],
      "generatedSourceRoots": ["../core/target/generated-sources/annotations"],
      "classpath": [
        { "sourceSet": "api/main" },
        { "path": "/m2/guava.jar" },
        { "path": "../core/lib" }
      ],
      "modulePath": ["mods"]
    },
    {
      "id": "core/test",
      "javaRelease": 17,
      "sourceRoots": ["../core/src/test/java", "../core/src/main/java/shared"],
      "classpath": [{ "sourceSet": "core/main" }, { "sourceSet": "api/main" }]
    },
    {
      "id": "api/main",
      "javaRelease": 11,
      "sourceRoots": ["../api/src/main/java"]
    }
  ]
}"#;

fn write_roots(scratch: &Scratch) {
    for root in [
        "core/src/main/java/shared",
        "core/src/test/java",
        "core/target/generated-sources/annotations",
        "api/src/main/java",
    ] {
        scratch.dir(root);
    }
}

#[test]
fn project_model() {
    let scratch = Scratch::new("model");
    write_roots(&scratch);
    let mut flags = model_flags(&scratch, MODEL);
    flags.java_version = Some(8);
    flags.jdk = Some("/opt/flag-jdk".into());
    let project = Project::load(&flags, &scratch.env()).unwrap();
    assert!(project.is_from_model());
    assert_eq!(project.warnings(), []);
    let [main, test, api] = project.source_sets() else {
        panic!("three source sets");
    };
    let origin = |set: &str| Origin::ProjectModel {
        path: "build/model.json".into(),
        source_set: set.into(),
    };
    // Relative paths are against the model's directory.
    assert_eq!(main.roots, [scratch.path("core/src/main/java")]);
    assert_eq!(
        main.generated_roots,
        [scratch.path("core/target/generated-sources/annotations")]
    );
    assert_eq!(
        main.release,
        Given {
            value: 21,
            origin: origin("core/main")
        }
    );
    assert_eq!(
        main.jdk,
        Some(Given {
            value: "/opt/jdk-21".into(),
            origin: origin("core/main")
        })
    );
    assert_eq!(
        main.classpath.as_ref().unwrap(),
        &[
            ClassPathItem::SourceSet(Given {
                value: api.id,
                origin: origin("core/main")
            }),
            ClassPathItem::Path(Given {
                value: "/m2/guava.jar".into(),
                origin: origin("core/main")
            }),
            ClassPathItem::Path(Given {
                value: scratch.path("core/lib"),
                origin: origin("core/main")
            }),
        ]
    );
    assert_eq!(main.module_path[0].value, scratch.path("build/mods"));
    // A source set without a JDK takes `--jdk`, and one without a classpath has an empty one.
    assert_eq!(
        test.jdk,
        Some(Given {
            value: "/opt/flag-jdk".into(),
            origin: Origin::Flag("--jdk")
        })
    );
    assert_eq!(api.classpath, Some(Vec::new()));

    let shared = scratch.path("core/src/main/java/shared/S.java");
    assert_eq!(project.source_sets_of(&shared), [main.id, test.id]);
    assert_eq!(project.release_of(&shared), Some((21, origin("core/main"))));
    let relative = Path::new("core/src/test/java/./p/../T.java");
    assert_eq!(project.source_sets_of(relative), [test.id]);
    assert_eq!(
        project.release_of(relative),
        Some((17, origin("core/test")))
    );
    let generated = scratch.path("core/target/generated-sources/annotations/G.java");
    assert_eq!(project.source_sets_of(&generated), [main.id]);
    assert!(project.is_generated(&generated));
    assert!(!project.is_generated(&shared));
    // A file under no source set takes `--java-version`.
    let outside = scratch.path("scripts/Tool.java");
    assert_eq!(project.source_sets_of(&outside), []);
    assert_eq!(
        project.release_of(&outside),
        Some((8, Origin::Flag("--java-version")))
    );

    flags.java_version = None;
    flags.jdk = None;
    let mut env = scratch.env();
    env.java_home = Some("/opt/home-jdk".into());
    let project = Project::load(&flags, &env).unwrap();
    assert_eq!(project.release_of(&outside), None);
    assert_eq!(
        render(
            &[project.no_release(Path::new("scripts/Tool.java"))],
            &scratch.0
        ),
        "error: `scripts/Tool.java` isn't in any source set of the project model, so it has no \
         Java version\n  from: --project-model\n  help: pass --java-version for files outside \
         the model's source sets"
    );
    assert_eq!(
        project.source_sets()[1].jdk.as_ref().unwrap().origin,
        Origin::JavaHome
    );
}

#[test]
fn missing_source_roots_are_warnings() {
    let scratch = Scratch::new("model-roots");
    let project = Project::load(&model_flags(&scratch, MODEL), &scratch.env()).unwrap();
    assert_eq!(
        render(&project.warnings()[..2], &scratch.0),
        "warning: source root `$DIR/core/src/main/java` doesn't exist\n  from: project model \
         `build/model.json`, source set `core/main`\n\n\
         warning: generated-source root `$DIR/core/target/generated-sources/annotations` doesn't \
         exist\n  from: project model `build/model.json`, source set `core/main`"
    );
    assert_eq!(project.warnings().len(), 5);
}

#[test]
fn flags_that_conflict_with_a_model() {
    let scratch = Scratch::new("model-conflicts");
    write_roots(&scratch);
    let mut flags = model_flags(&scratch, MODEL);
    flags.classpath = Some(String::new());
    flags.module_path = Some(String::new());
    assert_eq!(
        load_errors(&flags, &scratch),
        "error: --classpath can't be used with --project-model\n  from: --classpath\n  help: the \
         project model gives each source set's classpath and module path\n\n\
         error: --module-path can't be used with --project-model\n  from: --module-path\n  help: \
         the project model gives each source set's classpath and module path"
    );
}

#[test]
fn unreadable_and_malformed_models() {
    let scratch = Scratch::new("model-malformed");
    let flags = InputFlags {
        project_model: Some("gone.json".into()),
        ..InputFlags::default()
    };
    let errors = load_errors(&flags, &scratch);
    assert!(
        errors.starts_with("error: can't read the project model `gone.json`: No such file"),
        "{errors}"
    );
    assert!(errors.ends_with("\n  from: --project-model"), "{errors}");

    let cases = [
        (
            "{",
            "the project model isn't valid: EOF while parsing an object at line 1 column 1",
        ),
        (
            r#"{"sourceSets": []}"#,
            "the project model isn't valid: no `formatVersion`",
        ),
        (
            r#"{"formatVersion": 0, "sourceSets": []}"#,
            "the project model isn't valid: format version 0 doesn't exist",
        ),
        (
            "{\"formatVersion\": 1,\n \"sourceSets\": [{\"id\": \"a\", \"javaRelease\": 21, \
             \"sourceRoots\": [], \"sourcePath\": []}]}",
            "the project model isn't valid: unknown field `sourcePath`, expected one of `id`, \
             `javaRelease`, `platform`, `enablePreview`, `addModules`, `addExports`, `addReads`, \
             `patchModules`, `jdk`, `sourceRoots`, `generatedSourceRoots`, `classpath`, \
             `modulePath`, `processorPath` at line 2 column 78",
        ),
        (
            r#"{"formatVersion": 1, "sourceSets": [{"id": "a", "sourceRoots": []}]}"#,
            "the project model isn't valid: missing field `javaRelease` at line 1 column 66",
        ),
    ];
    for (json, message) in cases {
        assert_eq!(
            load_errors(&model_flags(&scratch, json), &scratch),
            format!("error: {message}\n  from: project model `build/model.json`"),
            "{json}"
        );
    }
}

#[test]
fn newer_format_version() {
    let scratch = Scratch::new("model-newer");
    // Fields a newer format may have aren't reported.
    let flags = model_flags(
        &scratch,
        r#"{"formatVersion": 3, "sourceSets": [], "toolchains": {}}"#,
    );
    assert_eq!(
        load_errors(&flags, &scratch),
        "error: the project model has format version 3, and this Ferrion reads versions up to \
         2\n  from: project model `build/model.json`\n  help: upgrade Ferrion to the version of \
         the integration that wrote the model"
    );
}

#[test]
fn compile_options() {
    let scratch = Scratch::new("model-compile");
    // Version 1 has none of them: the defaults.
    let old = r#"{"formatVersion": 1, "sourceSets": [{"id": "a", "javaRelease": 8,
        "sourceRoots": []}]}"#;
    let project = Project::load(&model_flags(&scratch, old), &scratch.env()).unwrap();
    let set = &project.source_sets()[0];
    assert_eq!(set.compile, CompileOptions::default());
    assert_eq!(set.compile.platform, PlatformKind::Release);
    assert_eq!(set.processor_path, None);

    let json = r#"{"formatVersion": 2, "sourceSets": [{"id": "a", "javaRelease": 8,
        "platform": "jdk", "enablePreview": true,
        "addModules": ["jdk.incubator.vector", "ALL-MODULE-PATH"],
        "addExports": ["java.base/jdk.internal.misc=ALL-UNNAMED,m.x"],
        "addReads": ["java.base=ALL-UNNAMED"],
        "patchModules": {"java.base": ["lib/api.jar"]},
        "processorPath": [{"path": "lib/p.jar"}, {"sourceSet": "b"}],
        "sourceRoots": []}, {"id": "b", "javaRelease": 21, "sourceRoots": []}]}"#;
    let project = Project::load(&model_flags(&scratch, json), &scratch.env()).unwrap();
    let [a, b] = project.source_sets() else {
        panic!("two source sets");
    };
    let options = &a.compile;
    assert_eq!(options.platform, PlatformKind::Jdk);
    assert!(options.enable_preview);
    assert_eq!(
        options.add_modules,
        ["jdk.incubator.vector", "ALL-MODULE-PATH"]
    );
    assert_eq!(
        options.add_exports,
        [AddExports {
            module: "java.base".into(),
            package: "jdk.internal.misc".into(),
            targets: vec!["ALL-UNNAMED".into(), "m.x".into()],
        }]
    );
    assert!(options.add_exports[0].to_unnamed());
    assert_eq!(
        options.add_reads,
        [AddReads {
            module: "java.base".into(),
            targets: vec!["ALL-UNNAMED".into()],
        }]
    );
    assert_eq!(options.patch_modules[0].module, "java.base");
    assert_eq!(
        options.patch_modules[0].paths[0].value,
        scratch.path("build/lib/api.jar")
    );
    let processors = a.processor_path.as_ref().unwrap();
    assert_eq!(
        paths(&processors[..1])[0].0,
        scratch.path("build/lib/p.jar")
    );
    assert!(matches!(&processors[1], ClassPathItem::SourceSet(id) if id.value == b.id));
    assert_eq!(b.compile, CompileOptions::default());

    let bad = r#"{"formatVersion": 2, "sourceSets": [{"id": "a", "javaRelease": 21,
        "addModules": ["no way"], "addExports": ["java.base=ALL-UNNAMED"],
        "addReads": ["java.base"], "patchModules": {"1x": []},
        "processorPath": [{"sourceSet": "gone"}], "sourceRoots": []}]}"#;
    assert_eq!(
        load_errors(&model_flags(&scratch, bad), &scratch),
        "error: source set `a` depends on `gone`, which isn't in the project model\n  from: \
         project model `build/model.json`, source set `a`\n\n\
         error: `addModules` entry `no way` of source set `a` isn't a module name\n  from: \
         project model `build/model.json`, source set `a`\n  help: write it as javac's \
         --add-modules takes it\n\n\
         error: `addExports` entry `java.base=ALL-UNNAMED` of source set `a` isn't \
         `<module>/<package>=<target>(,<target>)*`\n  from: project model `build/model.json`, \
         source set `a`\n  help: write it as javac's --add-exports takes it\n\n\
         error: `addReads` entry `java.base` of source set `a` isn't \
         `<module>=<target>(,<target>)*`\n  from: project model `build/model.json`, source set \
         `a`\n  help: write it as javac's --add-reads takes it\n\n\
         error: `patchModules` entry `1x` of source set `a` isn't a module name\n  from: project \
         model `build/model.json`, source set `a`\n  help: write it as javac's --patch-module \
         takes it"
    );
}

#[test]
fn source_set_references() {
    let scratch = Scratch::new("model-references");
    let set = |id: &str, deps: &[&str]| {
        let classpath: Vec<String> = deps
            .iter()
            .map(|dep| format!(r#"{{"sourceSet": "{dep}"}}"#))
            .collect();
        format!(
            r#"{{"id": "{id}", "javaRelease": 21, "sourceRoots": [], "classpath": [{}]}}"#,
            classpath.join(", ")
        )
    };
    let model = |sets: &[String]| {
        format!(
            r#"{{"formatVersion": 1, "sourceSets": [{}]}}"#,
            sets.join(", ")
        )
    };
    let duplicate = model(&[set("a", &[]), set("a", &[])]);
    assert_eq!(
        load_errors(&model_flags(&scratch, &duplicate), &scratch),
        "error: two source sets have the id `a`\n  from: project model `build/model.json`, source \
         set `a`"
    );
    let unknown = model(&[set("a", &["b"])]);
    assert_eq!(
        load_errors(&model_flags(&scratch, &unknown), &scratch),
        "error: source set `a` depends on `b`, which isn't in the project model\n  from: project \
         model `build/model.json`, source set `a`"
    );
    let cycles = model(&[
        set("core/main", &["core/test"]),
        set("core/test", &["core/main"]),
        set("x", &["y"]),
        set("y", &["z"]),
        set("z", &["x", "z"]),
        set("self", &["self"]),
        set("fine", &["core/main"]),
    ]);
    assert_eq!(
        load_errors(&model_flags(&scratch, &cycles), &scratch),
        "error: source sets `core/main` and `core/test` depend on each other\n  from: project \
         model `build/model.json`\n\n\
         error: source sets `x`, `y`, and `z` depend on each other\n  from: project model \
         `build/model.json`\n\n\
         error: source set `self` depends on itself\n  from: project model `build/model.json`"
    );
    let both = r#"{"formatVersion": 1, "sourceSets": [{"id": "a", "javaRelease": 21,
        "sourceRoots": [], "classpath": [{"path": "x.jar", "sourceSet": "a"}, {}]}, {"id": "",
        "javaRelease": 21, "sourceRoots": []}]}"#;
    assert_eq!(
        load_errors(&model_flags(&scratch, both), &scratch),
        "error: source set 2 has an empty `id`\n  from: project model `build/model.json`\n\n\
         error: a classpath entry of source set `a` has both or neither of `path` and \
         `sourceSet`\n  from: project model `build/model.json`, source set `a`"
    );
}

#[test]
fn models_round_trip() {
    let model: ProjectModel = serde_json::from_str(MODEL).unwrap();
    let json = model.to_json().unwrap();
    assert_eq!(serde_json::from_str::<ProjectModel>(&json).unwrap(), model);
    // Empty optional lists aren't written.
    assert!(!json.contains("\"modulePath\": []"));
}
