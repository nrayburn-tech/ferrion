//! Resolution: which entries apply to a path, and the settings they give it.

use ferrion_config::{Config, Level};

fn config(text: &str) -> Config {
    Config::from_text(text).unwrap_or_else(|e| panic!("{}", e.render("ferrion.jsonc", text)))
}

/// The rules a path gets: name, level, options as JSON, and the entry they came from.
fn rules(config: &Config, path: &str) -> Vec<(String, Level, Option<String>, usize)> {
    config
        .resolve(path)
        .rules
        .values()
        .map(|r| {
            (
                r.rule.name.clone(),
                r.rule.level,
                r.rule.options.as_ref().map(|o| o.to_json()),
                r.entry,
            )
        })
        .collect()
}

const SCOPE_EXAMPLE: &str = r#"{
  "configs": [
    { "ignores": ["**/legacy/**"] },
    {
      "files": ["**/*.java"],
      "rules": {
        "no-equals-without-hashcode": "error",
        "no-unused-variable": { "level": "warn", "options": { "ignorePattern": "^_" } }
      }
    },
    {
      "name": "tests",
      "files": ["src/test/**"],
      "rules": { "no-unused-variable": "off" },
      "format": { "enabled": false }
    }
  ]
}"#;

#[test]
fn the_last_entry_that_mentions_a_rule_gives_all_of_it() {
    let config = config(SCOPE_EXAMPLE);
    let test = config.resolve("src/test/java/app/FooTest.java");
    assert!(test.ignored.is_none());
    assert_eq!(test.entries, [1, 2]);
    // Nothing is merged: the test entry's setting has no options, though the earlier one did.
    assert_eq!(
        rules(&config, "src/test/java/app/FooTest.java"),
        [
            ("no-equals-without-hashcode".into(), Level::Error, None, 1),
            ("no-unused-variable".into(), Level::Off, None, 2),
        ]
    );
    assert_eq!(
        rules(&config, "src/main/java/app/Foo.java"),
        [
            ("no-equals-without-hashcode".into(), Level::Error, None, 1),
            (
                "no-unused-variable".into(),
                Level::Warn,
                Some(r#"{ "ignorePattern": "^_" }"#.into()),
                1
            ),
        ]
    );
    let setting = &config.resolve("src/test/A.java").rules["no-unused-variable"];
    assert_eq!(setting.rule.value.to_json(), r#""off""#);
}

#[test]
fn the_last_entry_with_format_gives_all_of_it() {
    let config = config(SCOPE_EXAMPLE);
    let test = config.resolve("src/test/java/app/FooTest.java");
    assert!(!test.format.value.enabled);
    assert_eq!(test.format.entry, Some(2));
    let main = config.resolve("src/main/java/app/Foo.java");
    assert!(main.format.value.enabled);
    assert_eq!(main.format.entry, None);

    // An empty `format` object gives its defaults, from its entry.
    let config = self::config(
        r#"{ "configs": [
            { "format": { "enabled": false } },
            { "files": ["src/main/**"], "format": {} },
            { "files": ["src/main/x/**"], "rules": { "r": "warn" } }
        ] }"#,
    );
    let main = config.resolve("src/main/x/A.java");
    assert!(main.format.value.enabled);
    assert_eq!(main.format.entry, Some(1));
    let other = config.resolve("src/other/A.java");
    assert!(!other.format.value.enabled);
    assert_eq!(other.format.entry, Some(0));
}

#[test]
fn global_ignores() {
    let config = config(
        r#"{ "configs": [
            { "rules": { "r": "warn" } },
            { "name": "generated", "ignores": ["build/", "**/generated/**"] },
            { "ignores": ["**/legacy/**", "*.txt"] },
            { "ignores": ["src/main/legacy/**"], "rules": { "r": "off" } }
        ] }"#,
    );
    let ignored = config.resolve("src/legacy/Old.java").ignored.unwrap();
    assert_eq!(ignored.entry.index, 2);
    assert_eq!(ignored.glob.raw, "**/legacy/**");
    assert_eq!(
        ignored.to_string(),
        r#"ignored by configs[2], "**/legacy/**""#
    );
    // The first entry's first matching glob is the one reported.
    let ignored = config.ignored_by("build/generated/legacy/A.java").unwrap();
    assert_eq!(
        ignored.to_string(),
        r#"ignored by configs[1] "generated", "build/""#
    );
    // A global ignore applies to any file, not only `.java` files.
    assert_eq!(config.ignored_by("notes.txt").unwrap().glob.raw, "*.txt");
    assert!(config.ignored_by("src/notes.txt").is_none());
    // An entry with `ignores` and anything but `name` isn't a global ignore: it leaves files out
    // of itself only.
    assert!(config.ignored_by("src/main/legacy/A.java").is_some());
    assert!(!config.entries[3].is_global_ignore());
    let resolution = config.resolve("src/main/x/A.java");
    assert_eq!(resolution.entries, [0, 3]);
    let resolution = config.resolve("src/main/A.java");
    assert!(resolution.ignored.is_none());
    assert_eq!(resolution.rules["r"].rule.level, Level::Off);
    // An ignored file gets nothing else.
    let ignored = config.resolve("build/A.java");
    assert!(ignored.entries.is_empty() && ignored.rules.is_empty());
}

#[test]
fn files_and_ignores_together() {
    let config = config(
        r#"{ "configs": [
            { "files": ["src/**"], "ignores": ["src/gen/**", "**/*Test.java"], "rules": { "r": "error" } },
            { "files": ["*.java"], "rules": { "top": "warn" } }
        ] }"#,
    );
    assert_eq!(config.resolve("src/a/A.java").entries, [0]);
    assert!(config.resolve("src/gen/A.java").entries.is_empty());
    assert!(config.resolve("src/a/ATest.java").entries.is_empty());
    // `files` selects files of any extension; only an entry without `files` is limited to `.java`.
    assert_eq!(config.resolve("src/a/notes.txt").entries, [0]);
    // Globs are anchored: `*.java` matches only top-level files.
    assert_eq!(config.resolve("A.java").entries, [1]);
    assert!(config.resolve("lib/A.java").entries.is_empty());
}

#[test]
fn entries_without_files_apply_to_java_files() {
    let config = config(
        r#"{ "configs": [{ "rules": { "r": "warn" } }, { "ignores": ["a/**"], "rules": { "s": "warn" } }] }"#,
    );
    assert_eq!(config.resolve("x/y/A.java").entries, [0, 1]);
    assert_eq!(config.resolve("a/A.java").entries, [0]);
    assert!(config.resolve("x/y/A.JAVA").entries.is_empty());
    assert!(config.resolve("x/README.md").entries.is_empty());
}

#[test]
fn without_a_path_only_unscoped_entries_apply() {
    let config = config(
        r#"{ "configs": [
            { "ignores": ["**"] },
            { "rules": { "r": "warn" } },
            { "files": ["**"], "rules": { "r": "error" } },
            { "ignores": ["x/**"], "format": { "enabled": false } },
            { "format": { "enabled": true }, "rules": { "s": "info" } }
        ] }"#,
    );
    let resolution = config.resolve_without_path();
    assert!(resolution.ignored.is_none());
    assert_eq!(resolution.entries, [1, 4]);
    assert_eq!(resolution.rules["r"].rule.level, Level::Warn);
    assert_eq!(resolution.format.entry, Some(4));
}

#[test]
fn no_config() {
    let config = Config::default();
    let resolution = config.resolve("src/A.java");
    assert!(resolution.ignored.is_none());
    assert!(resolution.entries.is_empty() && resolution.rules.is_empty());
    assert!(resolution.format.value.enabled);
    assert_eq!(resolution.format.entry, None);
    assert!(config.prunes("src").is_none());
    let empty = self::config("{}");
    assert!(empty.entries.is_empty());
}

#[test]
fn pruning_folders() {
    let config = config(
        r#"{ "configs": [
            { "ignores": ["**/legacy/**", "build/", "out/*", "*.tmp/**"] },
            { "files": ["gen/**"], "ignores": ["gen/**"] }
        ] }"#,
    );
    let prunes = |folder: &str| config.prunes(folder).map(|i| i.glob.raw.clone());
    assert_eq!(prunes("legacy").as_deref(), Some("**/legacy/**"));
    assert_eq!(prunes("src/main/legacy").as_deref(), Some("**/legacy/**"));
    assert_eq!(prunes("build").as_deref(), Some("build/"));
    assert_eq!(prunes("a.tmp").as_deref(), Some("*.tmp/**"));
    assert_eq!(prunes("src/a.tmp"), None);
    assert_eq!(prunes("src/build"), None);
    assert_eq!(prunes("src"), None);
    // `out/*` doesn't end in `/**`: files directly in `out` are ignored, but not deeper ones.
    assert_eq!(prunes("out"), None);
    assert!(config.ignored_by("out/A.java").is_some());
    assert!(config.ignored_by("out/x/A.java").is_none());
    // Only global ignores prune.
    assert_eq!(prunes("gen"), None);
}
