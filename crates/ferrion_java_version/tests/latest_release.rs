//! Checks that what depends on the latest release agrees with `LATEST_RELEASE`: the JDK the
//! lexer's Unicode tables were generated with, and the JDK the workflows install. Raising
//! `LATEST_RELEASE` without the others fails these tests.

use ferrion_java_version::LATEST_RELEASE;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

const TABLES: &str = "crates/ferrion_lexer/src/unicode_tables.rs";

fn read(path: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{path}"))
        .unwrap_or_else(|error| panic!("reading {path}: {error}"))
}

#[test]
fn unicode_tables_come_from_the_latest_release() {
    let tables = read(TABLES);
    let jdk = tables
        .lines()
        .find_map(|line| line.strip_prefix("//! JDK: "))
        .unwrap_or_else(|| panic!("{TABLES} doesn't record its JDK"));
    assert_eq!(
        jdk.trim(),
        LATEST_RELEASE.to_string(),
        "{TABLES} was generated with JDK {jdk}, but the latest release is {LATEST_RELEASE}; \
         regenerate it as tools/unicode-tables/README.md describes"
    );
}

#[test]
fn workflows_install_the_latest_release() {
    let mut workflows: Vec<_> = std::fs::read_dir(format!("{ROOT}/.github/workflows"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".yml"))
        .collect();
    workflows.sort();
    let mut found = 0;
    let mut wrong = Vec::new();
    for name in workflows {
        let path = format!(".github/workflows/{name}");
        for (i, line) in read(&path).lines().enumerate() {
            let Some(version) = line.trim().strip_prefix("java-version:") else {
                continue;
            };
            found += 1;
            let version = version.trim().trim_matches(['"', '\'']);
            if version != LATEST_RELEASE.to_string() {
                wrong.push(format!("{path}:{}: java-version {version}", i + 1));
            }
        }
    }
    assert!(found > 0, "no workflow sets a java-version");
    assert!(
        wrong.is_empty(),
        "these workflows don't install JDK {LATEST_RELEASE}, the latest release:\n{}",
        wrong.join("\n")
    );
}
