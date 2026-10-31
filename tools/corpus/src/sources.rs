//! The Java files to parse, each with the releases it's meant for: a project's files at their
//! source set's release, as its lock file records it, and javac's tests at the releases their
//! tests compile them at.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::project::Project;
use crate::{Dirs, fetch, jdk_tests, lock};

/// The names that select javac's tests instead of a project: all of them, or only its
/// deep-nesting stress tests.
pub const JDK_TESTS: &str = "jdk-tests";
pub const JDK_DEEP_NESTING: &str = "jdk-deep-nesting";

/// A file to parse.
pub struct Source {
    /// The path to read.
    pub path: PathBuf,
    /// The path to report: the project's name (or `jdk-tests`) and the path within it.
    pub display: String,
    /// The releases the file is meant for, in order. Never empty.
    pub releases: Vec<u32>,
}

/// The Java files in `project`'s source roots, sorted, each at its source set's release. A file in
/// more than one source set is listed once, at the first one's release.
pub fn project_sources(dirs: &Dirs, project: &Project) -> Result<Vec<Source>> {
    let lock =
        lock::read(dirs, project)?.with_context(|| format!("{} has no lock file", project.name))?;
    if lock.commit != project.commit {
        bail!(
            "{}: the lock file is for a different commit; run `cargo corpus lock`",
            project.name
        );
    }
    if !fetch::is_checked_out(project, dirs) {
        bail!(
            "{} isn't checked out at {}; run `cargo corpus checkout {}`",
            project.name,
            project.commit,
            project.name
        );
    }
    let checkout = dirs.checkout(project);
    let mut own = BTreeMap::new();
    for module in &lock.modules {
        for source_set in &module.source_sets {
            // A build that doesn't set a release compiles for its JDK's.
            let release = source_set.java_release.unwrap_or(lock.jdk);
            if !ferrion_java_version::is_supported(release) {
                bail!(
                    "{} {} {} is for Java {release}, which isn't supported",
                    project.name,
                    module.path,
                    source_set.name
                );
            }
            for root in &source_set.source_roots {
                for path in jdk_tests::java_files(&checkout.join(root))? {
                    own.entry(path).or_insert(release);
                }
            }
        }
    }
    Ok(own
        .into_iter()
        .map(|(path, release)| {
            let relative = lock::relative(&checkout, &path).unwrap();
            Source {
                display: format!("{}/{relative}", project.name),
                path,
                releases: vec![release],
            }
        })
        .collect())
}

/// javac's tests (see [`jdk_tests`]), or only its deep-nesting stress tests, each at the releases
/// its tests compile it at, with preview features at the latest release. Files compiled only
/// before Java 8 are left out, and their number returned.
pub fn jdk_test_sources(dirs: &Dirs, deep_only: bool) -> Result<(Vec<Source>, usize)> {
    let tests = jdk_tests::JdkTests::load(dirs)?;
    let mut sources = Vec::new();
    let mut unsupported = 0;
    for input in jdk_tests::inputs(&tests, dirs)? {
        if deep_only && !input.deep_nesting {
            continue;
        }
        if input.releases.is_empty() {
            unsupported += 1;
            continue;
        }
        sources.push(Source {
            display: format!("{JDK_TESTS}/{}", input.path),
            path: input.file,
            releases: input.releases.into_iter().collect(),
        });
    }
    Ok((sources, unsupported))
}

/// What a name on a tool's command line selects: a project, javac's tests, or javac's
/// deep-nesting tests.
pub enum Target<'a> {
    Project(&'a Project),
    JdkTests { deep_only: bool },
}

impl Target<'_> {
    pub fn name(&self) -> &str {
        match self {
            Target::Project(project) => &project.name,
            Target::JdkTests { deep_only: false } => JDK_TESTS,
            Target::JdkTests { deep_only: true } => JDK_DEEP_NESTING,
        }
    }

    /// The target's files, and how many were left out because they're only for releases before
    /// Java 8.
    pub fn sources(&self, dirs: &Dirs) -> Result<(Vec<Source>, usize)> {
        match self {
            Target::Project(project) => Ok((project_sources(dirs, project)?, 0)),
            Target::JdkTests { deep_only } => jdk_test_sources(dirs, *deep_only),
        }
    }
}

/// The targets `names` select, in that order, or every project if `names` is empty.
pub fn targets<'a>(projects: &'a [Project], names: &[String]) -> Result<Vec<Target<'a>>> {
    if names.is_empty() {
        return Ok(projects.iter().map(Target::Project).collect());
    }
    names
        .iter()
        .map(|name| match name.as_str() {
            JDK_TESTS => Ok(Target::JdkTests { deep_only: false }),
            JDK_DEEP_NESTING => Ok(Target::JdkTests { deep_only: true }),
            _ => Ok(Target::Project(
                crate::select(projects, std::slice::from_ref(name))?[0],
            )),
        })
        .collect()
}
