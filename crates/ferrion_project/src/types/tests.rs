use super::*;
use crate::testing::{Scratch, render};
use crate::{InputFlags, PATH_SEPARATOR};
use ferrion_classfile::ClassName;
use ferrion_classfile::testing::{ZipBuilder, sample_module_info};

/// The flags for a run at `release` with `classpath` (entries joined with the platform's
/// separator) and the JDK at `jdk`.
fn flags(release: u32, jdk: &Path, classpath: &[&str]) -> InputFlags {
    InputFlags {
        java_version: Some(release),
        jdk: Some(jdk.into()),
        classpath: Some(classpath.join(&PATH_SEPARATOR.to_string())),
        ..InputFlags::default()
    }
}

fn open(flags: &InputFlags, scratch: &Scratch) -> Result<(TypeInputs, Vec<InputError>), String> {
    let project = Project::load(flags, &scratch.env()).unwrap();
    TypeInputs::open(&project, &ArchiveCache::new()).map_err(|e| render(&e, &scratch.0))
}

/// The entries' paths relative to the scratch directory.
fn entries(types: &SourceSetTypes, scratch: &Scratch) -> Vec<String> {
    types
        .entries
        .iter()
        .map(|entry| match entry {
            TypeEntry::Classes(classes) => classes
                .path
                .strip_prefix(&scratch.0)
                .unwrap()
                .display()
                .to_string(),
            TypeEntry::SourceSet(id) => format!("source set {}", id.value.index()),
        })
        .collect()
}

#[test]
fn opens_the_jdk_and_classpath() {
    let scratch = Scratch::new("open");
    let jdk = scratch.jdk8("jdk8");
    scratch.jar("lib/a.jar", &["p/A"], "");
    scratch.write("classes/p/B.class", b"");
    let (types, warnings) = open(&flags(8, &jdk, &["lib/a.jar", "classes"]), &scratch).unwrap();
    assert_eq!(warnings, []);
    let set = &types.source_sets()[0];
    assert_eq!(set.platform.release(), 8);
    assert_eq!(set.jdk.feature_version(), 8);
    assert_eq!(entries(set, &scratch), ["lib/a.jar", "classes"]);
    assert!(
        set.class_path
            .find(&ClassName::new("p/A"))
            .unwrap()
            .is_some()
    );
    assert!(
        set.class_path
            .find(&ClassName::new("p/B"))
            .unwrap()
            .is_some()
    );
    let TypeEntry::Classes(first) = &set.entries[0] else {
        panic!("a jar");
    };
    assert_eq!(first.origin, Origin::Flag("--classpath"));
    assert!(!first.on_module_path && first.module.is_none());
}

#[test]
fn missing_type_inputs() {
    let scratch = Scratch::new("missing");
    let flags = InputFlags {
        java_version: Some(21),
        ..InputFlags::default()
    };
    assert_eq!(
        open(&flags, &scratch).unwrap_err(),
        "error: rules that need types are enabled, but no JDK was given\n  from: --jdk\n  help: \
         pass --jdk, or set JAVA_HOME\n\n\
         error: rules that need types are enabled, but no classpath was given\n  from: \
         --classpath\n  help: pass --classpath or --classpath-file (`--classpath \"\"` for code \
         with no dependencies), or --project-model"
    );
}

#[test]
fn jdk_problems() {
    let scratch = Scratch::new("jdks");
    let jdk8 = scratch.jdk8("jdk8");
    let runtime = scratch.runtime("jre17", 17);
    assert_eq!(
        open(&flags(17, &jdk8, &[]), &scratch).unwrap_err(),
        "error: release 17 can't be served by the JDK at `$DIR/jdk8` (JDK 8)\n  from: --jdk\n  \
         help: pass --jdk with JDK 17 or later"
    );
    // From JAVA_HOME, the same.
    let mut env = scratch.env();
    env.java_home = Some(jdk8.clone());
    let mut home = flags(17, &jdk8, &[]);
    home.jdk = None;
    let project = Project::load(&home, &env).unwrap();
    let errors = TypeInputs::open(&project, &ArchiveCache::new()).unwrap_err();
    assert_eq!(
        render(&errors, &scratch.0),
        "error: release 17 can't be served by the JDK at `$DIR/jdk8` (JDK 8)\n  from: JAVA_HOME\n  \
         help: pass --jdk with JDK 17 or later"
    );
    // B1's messages pass through, with the JDK's origin.
    assert_eq!(
        open(&flags(11, &runtime, &[]), &scratch).unwrap_err(),
        "error: $DIR/jre17 has no lib/ct.sym, so it serves only release 17\n  from: --jdk"
    );
    let errors = open(&flags(11, &scratch.dir("empty"), &[]), &scratch).unwrap_err();
    assert!(
        errors.starts_with("error: $DIR/empty isn't a JDK: No such file or directory"),
        "{errors}"
    );
    assert!(
        errors.ends_with("reading its release file\n  from: --jdk"),
        "{errors}"
    );
    assert!(open(&flags(17, &runtime, &[]), &scratch).is_ok());
}

#[test]
fn missing_and_unreadable_entries() {
    let scratch = Scratch::new("entries");
    let jdk = scratch.jdk8("jdk8");
    scratch.write("deps/x.pom", b"<project/>");
    scratch.write("deps/bad.jar", b"not a zip");
    // A zip named otherwise is read as a jar, as javac does.
    scratch.write("deps/zipped.pom", &ZipBuilder::new().finish());
    let classpath = [
        "gone.jar",
        "target/classes",
        "deps/x.pom",
        "deps/bad.jar",
        "deps/zipped.pom",
    ];
    assert_eq!(
        open(&flags(8, &jdk, &classpath), &scratch).unwrap_err(),
        "error: classpath entry `$DIR/gone.jar` doesn't exist\n  from: --classpath\n\n\
         warning: classpath directory `$DIR/target/classes` doesn't exist\n  from: --classpath\n\n\
         warning: classpath entry `$DIR/deps/x.pom` isn't a jar or a directory, so it's skipped\n  \
         from: --classpath\n\n\
         error: can't read the classpath jar `$DIR/deps/bad.jar`: too short to be a zip \
         file\n  from: --classpath"
    );
    let (types, warnings) = open(
        &flags(
            8,
            &jdk,
            &["target/classes", "deps/x.pom", "deps/zipped.pom"],
        ),
        &scratch,
    )
    .unwrap();
    assert_eq!(warnings.len(), 2);
    // A directory that doesn't exist is kept, since it may be build output still to come.
    assert_eq!(
        entries(&types.source_sets()[0], &scratch),
        ["target/classes", "deps/zipped.pom"]
    );
}

#[test]
fn manifest_class_path() {
    let scratch = Scratch::new("manifest");
    let jdk = scratch.jdk8("jdk8");
    // a lists b and c; b lists d: depth first, so d comes before c.
    scratch.jar("lib/a.jar", &["a/A"], "Class-Path: b.jar sub/c.jar\n");
    scratch.jar("lib/b.jar", &["b/B"], "Class-Path: ../d.jar\n");
    scratch.jar("lib/sub/c.jar", &["lib/X"], "");
    scratch.jar("d.jar", &["lib/X"], "");
    let (types, warnings) = open(&flags(8, &jdk, &["lib/a.jar"]), &scratch).unwrap();
    assert_eq!(warnings, []);
    let set = &types.source_sets()[0];
    assert_eq!(
        entries(set, &scratch),
        ["lib/a.jar", "lib/b.jar", "d.jar", "lib/sub/c.jar"]
    );
    let TypeEntry::Classes(b) = &set.entries[1] else {
        panic!("a jar");
    };
    assert_eq!(
        b.origin,
        Origin::Manifest {
            jar: scratch.path("lib/a.jar")
        }
    );
    let found = set
        .class_path
        .find(&ClassName::new("lib/X"))
        .unwrap()
        .unwrap();
    assert!(
        found.origin.to_string().contains("d.jar"),
        "{}",
        found.origin
    );

    // An entry a manifest lists keeps the manifest's place, and its later repeat is left out.
    let (types, _) = open(
        &flags(8, &jdk, &["lib/a.jar", "lib/sub/c.jar", "lib/b.jar"]),
        &scratch,
    )
    .unwrap();
    assert_eq!(
        entries(&types.source_sets()[0], &scratch),
        ["lib/a.jar", "lib/b.jar", "d.jar", "lib/sub/c.jar"]
    );
}

#[test]
fn manifest_cycles_and_missing_entries() {
    let scratch = Scratch::new("manifest-cycles");
    let jdk = scratch.jdk8("jdk8");
    scratch.jar("e.jar", &["e/E"], "Class-Path: f.jar\n");
    scratch.jar(
        "f.jar",
        &["f/F"],
        "Class-Path: e.jar gone.jar classes/ http://example.com/x.jar\n",
    );
    scratch.dir("classes");
    scratch.dir("sp ace");
    scratch.jar("sp ace/s.jar", &["s/S"], "");
    scratch.jar("g.jar", &["g/G"], "Class-Path: sp%20ace/s.jar\n");
    scratch.write("h.jar", &{
        let mut b = ZipBuilder::new();
        b.stored("META-INF/MANIFEST.MF", b"no colon here\n");
        b.finish()
    });
    let (types, warnings) = open(&flags(8, &jdk, &["e.jar", "g.jar", "h.jar"]), &scratch).unwrap();
    assert_eq!(
        entries(&types.source_sets()[0], &scratch),
        [
            "e.jar",
            "f.jar",
            "classes",
            "g.jar",
            "sp ace/s.jar",
            "h.jar"
        ]
    );
    assert_eq!(
        render(&warnings, &scratch.0),
        "warning: Class-Path entry `http://example.com/x.jar` isn't a local file, so it's \
         skipped\n  from: Class-Path in the manifest of `$DIR/f.jar`\n\n\
         warning: classpath entry `$DIR/gone.jar` doesn't exist\n  from: Class-Path in the \
         manifest of `$DIR/f.jar`\n\n\
         warning: the manifest of `$DIR/h.jar` can't be read, so its Class-Path is ignored: a \
         manifest line without `:`\n  from: Class-Path in the manifest of `$DIR/h.jar`"
    );
}

#[test]
fn manifest_entries() {
    let dir = Path::new("/m2/lib");
    assert_eq!(manifest_entry(dir, "a.jar"), Some("/m2/lib/a.jar".into()));
    assert_eq!(
        manifest_entry(dir, "../b/c.jar"),
        Some("/m2/b/c.jar".into())
    );
    assert_eq!(manifest_entry(dir, "/abs/d.jar"), Some("/abs/d.jar".into()));
    assert_eq!(
        manifest_entry(dir, "file:/abs/e.jar"),
        Some("/abs/e.jar".into())
    );
    assert_eq!(
        manifest_entry(dir, "file:///abs/f.jar"),
        Some("/abs/f.jar".into())
    );
    assert_eq!(
        manifest_entry(dir, "a%20b%2.jar"),
        Some("/m2/lib/a b%2.jar".into())
    );
    assert_eq!(manifest_entry(dir, "https://example.com/g.jar"), None);
}

#[test]
fn module_path() {
    let scratch = Scratch::new("modules");
    let jdk = scratch.jdk8("jdk8");
    let mut named = ZipBuilder::new();
    named.stored("module-info.class", &sample_module_info());
    scratch.write("mods/named.jar", &named.finish());
    scratch.jar(
        "mods/withname-1.0.jar",
        &["w/W"],
        "Automatic-Module-Name: com.example.named\n",
    );
    scratch.jar(
        "mods/commons-lang3-3.17.0.jar",
        &["c/C"],
        "Class-Path: ../cp.jar\n",
    );
    scratch.jar("cp.jar", &["cp/C"], "");
    scratch.write("mods/notes.txt", b"");
    scratch.write("exploded/module-info.class", &sample_module_info());
    scratch.jar("single-2.jar", &["s/S"], "");
    let mut flags = flags(8, &jdk, &["single-2.jar"]);
    flags.module_path =
        Some(["mods", "exploded", "single-2.jar"].join(&PATH_SEPARATOR.to_string()));
    let (types, warnings) = open(&flags, &scratch).unwrap();
    assert_eq!(warnings, []);
    let set = &types.source_sets()[0];
    // The module path comes after the classpath, its directory's jars in name order; manifests
    // aren't followed there, and an entry already on the classpath isn't repeated.
    let modules: Vec<(String, Option<&str>, bool)> = set
        .entries
        .iter()
        .map(|entry| match entry {
            TypeEntry::Classes(c) => (
                c.path
                    .strip_prefix(&scratch.0)
                    .unwrap()
                    .display()
                    .to_string(),
                c.module.as_deref(),
                c.on_module_path,
            ),
            TypeEntry::SourceSet(_) => panic!("a source set"),
        })
        .collect();
    let owned = |path: &str, module: Option<&'static str>, on: bool| (path.to_owned(), module, on);
    assert_eq!(
        modules,
        [
            owned("single-2.jar", None, false),
            owned("mods/commons-lang3-3.17.0.jar", Some("commons.lang3"), true),
            owned("mods/named.jar", Some("m"), true),
            owned("mods/withname-1.0.jar", Some("com.example.named"), true),
            owned("exploded", Some("m"), true),
        ]
    );
    // A missing module path jar is an error, as on the classpath.
    let mut missing = flags.clone();
    missing.module_path = Some("gone.jar".into());
    assert_eq!(
        open(&missing, &scratch).unwrap_err(),
        "error: module path entry `$DIR/gone.jar` doesn't exist\n  from: --module-path"
    );
}

#[test]
fn automatic_module_names() {
    // As `jar --describe-module` (Temurin 27.0.1) reported for jars with these names.
    for (file, name) in [
        ("guava-33.4.0-jre.jar", Some("guava")),
        ("commons-lang3-3.17.0.jar", Some("commons.lang3")),
        ("foo_bar--baz.jar", Some("foo.bar.baz")),
        (".x.y..jar", Some("x.y")),
        ("my-lib-1.jar", Some("my.lib")),
        ("a-b-c.jar", Some("a.b.c")),
        ("junit-4.13.2.jar", Some("junit")),
        ("foo-bar-1.0-SNAPSHOT.jar", Some("foo.bar")),
        ("lib-1a.jar", None),
        ("1lib-2.0.jar", None),
        ("Foo.Bar.JAR", None),
    ] {
        assert_eq!(
            automatic_module_name(Path::new(file)).as_deref(),
            name,
            "{file}"
        );
    }
}

#[test]
fn source_sets_in_a_model() {
    let scratch = Scratch::new("model-types");
    let jdk = scratch.jdk8("jdk8");
    scratch.jar("a.jar", &["p/A"], "");
    scratch.dir("main");
    scratch.dir("test");
    let model = format!(
        r#"{{"formatVersion": 1, "sourceSets": [
            {{"id": "main", "javaRelease": 8, "jdk": "{jdk}", "sourceRoots": ["main"],
              "classpath": [{{"path": "a.jar"}}]}},
            {{"id": "test", "javaRelease": 8, "sourceRoots": ["test"],
              "classpath": [{{"path": "a.jar"}}, {{"sourceSet": "main"}}, {{"path": "a.jar"}}]}},
            {{"id": "newer", "javaRelease": 11, "jdk": "jdk8", "sourceRoots": []}}
        ]}}"#,
        jdk = jdk.display()
    );
    scratch.write("model.json", model.as_bytes());
    let flags = InputFlags {
        project_model: Some("model.json".into()),
        ..InputFlags::default()
    };
    assert_eq!(
        open(&flags, &scratch).unwrap_err(),
        "error: rules that need types are enabled, but no JDK was given\n  from: project model \
         `model.json`, source set `test`\n  help: pass --jdk, or set JAVA_HOME\n\n\
         error: release 11 can't be served by the JDK at `$DIR/jdk8` (JDK 8)\n  from: project \
         model `model.json`, source set `newer`\n  help: give source set `newer` JDK 11 or later \
         in the build"
    );
    let model = model
        .replace(r#""jdk": "jdk8", "#, "")
        .replace(r#""javaRelease": 11"#, r#""javaRelease": 8"#);
    scratch.write("model.json", model.as_bytes());
    let mut env = scratch.env();
    env.java_home = Some(jdk);
    let project = Project::load(&flags, &env).unwrap();
    let (types, _) = TypeInputs::open(&project, &ArchiveCache::new()).unwrap();
    let test = &types.source_sets()[1];
    assert_eq!(entries(test, &scratch), ["a.jar", "source set 0"]);
    assert_eq!(test.class_path.entries().len(), 1);
    // Source sets with the same JDK and release share one platform API.
    assert!(Arc::ptr_eq(
        &types.source_sets()[0].platform,
        &test.platform
    ));
}

#[test]
fn patched_modules_and_the_jdks_own_api() {
    let scratch = Scratch::new("patches");
    let jdk = scratch.jdk8("jdk8");
    scratch.jar("api.jar", &["java/lang/foreign/Arena"], "");
    scratch.dir("main");
    let model = |patch: &str| {
        format!(
            r#"{{"formatVersion": 2, "sourceSets": [{{"id": "main", "javaRelease": 8,
                "platform": "jdk", "jdk": "{jdk}", "sourceRoots": ["main"],
                "patchModules": {{"java.base": ["{patch}"]}}}}]}}"#,
            jdk = jdk.display()
        )
    };
    let flags = InputFlags {
        project_model: Some("model.json".into()),
        ..InputFlags::default()
    };
    scratch.write("model.json", model("api.jar").as_bytes());
    let (types, warnings) = open(&flags, &scratch).unwrap();
    assert_eq!(warnings, []);
    let set = &types.source_sets()[0];
    // `-source` and `-target` see the JDK's own API: JDK 8's here.
    assert_eq!(set.platform.release(), 8);
    let [(module, patches)] = set.patches.as_slice() else {
        panic!("one patched module");
    };
    assert_eq!(module, "java.base");
    assert_eq!(patches[0].path, scratch.path("api.jar"));
    assert_eq!(patches[0].module.as_deref(), Some("java.base"));
    assert!(
        patches[0]
            .entry
            .find(&ClassName::new("java/lang/foreign/Arena"))
            .unwrap()
            .is_some()
    );
    // Patches aren't classpath entries.
    assert!(set.entries.is_empty());

    scratch.write("model.json", model("gone.jar").as_bytes());
    assert_eq!(
        open(&flags, &scratch).unwrap_err(),
        "error: patch-module entry `$DIR/gone.jar` doesn't exist\n  from: project model \
         `model.json`, source set `main`"
    );
}
