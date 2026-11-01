use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use corpus::Dirs;
use corpus::lock::ClasspathEntry;
use corpus::project::Project;
use ferrion_classfile::jar::Jar;
use ferrion_classfile::zip::ZipArchive;
use ferrion_classfile::{ArchiveCache, ClassFile, ClassName, Jdk, PlatformApi, descriptor};

use super::{Benchmark, Inputs, Prepared};

pub fn benchmarks() -> Vec<Benchmark> {
    vec![
        Benchmark {
            name: "classfile/jdk-open",
            description: "open the JDK: its image's index and its ct.sym's central directory",
            setup: jdk_open,
        },
        Benchmark {
            name: "classfile/platform-image",
            description: "open the platform API for the JDK's own release, from its image",
            setup: platform_image,
        },
        Benchmark {
            name: "classfile/platform-ctsym",
            description: "open the platform API for --release from the JDK's ct.sym",
            setup: platform_ctsym,
        },
        Benchmark {
            name: "classfile/read-image",
            description: "find and read every class of the JDK's own release, from its image",
            setup: read_image,
        },
        Benchmark {
            name: "classfile/read-ctsym",
            description: "find and read every class of --release, from the JDK's ct.sym",
            setup: read_ctsym,
        },
        Benchmark {
            name: "classfile/read-jars",
            description: "open the corpus projects' dependency jars and read every class in them, \
                          at the release of the source set that uses them",
            setup: read_jars,
        },
    ]
}

fn jdk_home(inputs: &Inputs) -> Result<PathBuf> {
    match &inputs.jdk {
        Some(home) => Ok(home.clone()),
        None => std::env::var_os("JAVA_HOME")
            .map(PathBuf::from)
            .context("no JDK: pass --jdk or set JAVA_HOME"),
    }
}

fn open(inputs: &Inputs) -> Result<Arc<Jdk>> {
    let home = jdk_home(inputs)?;
    Jdk::open(&home, &ArchiveCache::new()).with_context(|| format!("opening {}", home.display()))
}

fn describe(jdk: &Jdk) -> String {
    let name = jdk.home().file_name().unwrap_or_default().to_string_lossy();
    format!("JDK {} ({name})", jdk.feature_version())
}

fn ctsym_release(jdk: &Jdk, inputs: &Inputs) -> Result<u32> {
    let release = inputs.release;
    if jdk.ctsym().is_none() || release >= jdk.feature_version() {
        bail!(
            "{} has no ct.sym release {release}; pass an older --release",
            describe(jdk)
        );
    }
    Ok(release)
}

fn jdk_open(inputs: &Inputs) -> Result<Prepared> {
    let jdk = open(inputs)?;
    let home = jdk.home().to_owned();
    Ok(Prepared {
        input: describe(&jdk),
        bytes: None,
        // A fresh cache each time, so nothing is shared with the last iteration.
        run: Box::new(move || {
            black_box(Jdk::open(&home, &ArchiveCache::new()).unwrap());
        }),
    })
}

fn platform_image(inputs: &Inputs) -> Result<Prepared> {
    let jdk = open(inputs)?;
    platform_open(describe(&jdk), jdk.feature_version(), jdk)
}

fn platform_ctsym(inputs: &Inputs) -> Result<Prepared> {
    let jdk = open(inputs)?;
    let release = ctsym_release(&jdk, inputs)?;
    platform_open(
        format!("{}, release {release}", describe(&jdk)),
        release,
        jdk,
    )
}

/// Each iteration drops the platform API, so the next one builds it again rather than finding it
/// in the JDK's table.
fn platform_open(input: String, release: u32, jdk: Arc<Jdk>) -> Result<Prepared> {
    jdk.platform(release)?;
    Ok(Prepared {
        input,
        bytes: None,
        run: Box::new(move || {
            black_box(jdk.platform(release).unwrap());
        }),
    })
}

fn read_image(inputs: &Inputs) -> Result<Prepared> {
    let jdk = open(inputs)?;
    let release = jdk.feature_version();
    read_platform(describe(&jdk), &jdk, release)
}

fn read_ctsym(inputs: &Inputs) -> Result<Prepared> {
    let jdk = open(inputs)?;
    let release = ctsym_release(&jdk, inputs)?;
    read_platform(
        format!("{}, release {release}", describe(&jdk)),
        &jdk,
        release,
    )
}

fn read_platform(input: String, jdk: &Jdk, release: u32) -> Result<Prepared> {
    let platform: Arc<PlatformApi> = jdk.platform(release)?;
    let names: Vec<ClassName> = platform
        .class_names()?
        .iter()
        .map(|name| ClassName::new(name))
        .collect();
    let input = format!("{input}: {} classes", names.len());
    Ok(Prepared {
        input,
        bytes: None,
        run: Box::new(move || {
            for name in &names {
                if let Ok(Some(found)) = platform.find(name) {
                    black_box(read_class(found.bytes));
                }
            }
        }),
    })
}

fn read_jars(inputs: &Inputs) -> Result<Prepared> {
    let dirs = Dirs::new()?;
    let projects = Project::load_all(&dirs.projects_file())?;
    // Each jar at the first release that uses it.
    let mut jars: BTreeMap<PathBuf, u32> = BTreeMap::new();
    for project in corpus::select(&projects, &inputs.projects)? {
        let lock = corpus::lock::read(&dirs, project)?
            .with_context(|| format!("{} has no lock file", project.name))?;
        for module in &lock.modules {
            for source_set in &module.source_sets {
                let release = source_set.java_release.unwrap_or(lock.jdk);
                for entry in &source_set.classpath {
                    if let ClasspathEntry::Jar { jar } = entry {
                        jars.entry(dirs.jars().join(jar)).or_insert(release);
                    }
                }
            }
        }
    }
    if let Some(missing) = jars.keys().find(|jar| !jar.is_file()) {
        bail!(
            "{} isn't fetched; run `cargo corpus fetch {}`",
            missing.display(),
            inputs.projects.join(" ")
        );
    }
    let bytes = jars
        .keys()
        .map(|jar| Ok(std::fs::metadata(jar)?.len()))
        .sum::<Result<u64>>()?;
    let input = format!(
        "{}: {} jars, {:.1} MB",
        inputs.projects.join(", "),
        jars.len(),
        bytes as f64 / 1e6
    );
    Ok(Prepared {
        input,
        bytes: Some(bytes),
        run: Box::new(move || {
            for (path, &release) in &jars {
                black_box(read_jar(path, release));
            }
        }),
    })
}

/// Opens the jar at `path` and reads every class in it, returning how many it read.
fn read_jar(path: &Path, release: u32) -> usize {
    let Ok(archive) = ZipArchive::open(path) else {
        return 0;
    };
    let jar = Jar::new(Arc::new(archive), release);
    let mut read = 0;
    for (name, entry) in jar.entries() {
        if name.ends_with(".class")
            && let Ok(bytes) = entry.read()
        {
            read += read_class(bytes.into());
        }
    }
    read
}

/// Reads what B2 needs from a class: its names, supertypes, members, descriptors, and attributes.
/// Returns a count, so the reads aren't optimized away.
fn read_class(bytes: Arc<[u8]>) -> usize {
    let Ok(class) = ClassFile::parse(bytes) else {
        return 0;
    };
    let mut read = usize::from(class.this_class().is_ok());
    read += usize::from(class.super_class().is_ok());
    read += class.interfaces().filter(Result::is_ok).count();
    read += class.attributes().filter(|a| a.decode().is_ok()).count();
    for field in class.fields() {
        if let Ok(text) = field.descriptor().and_then(|d| d.to_str()) {
            read += usize::from(descriptor::parse_field(&text).is_ok());
        }
        read += usize::from(field.name().is_ok());
        read += field.attributes().filter(|a| a.decode().is_ok()).count();
    }
    for method in class.methods() {
        if let Ok(text) = method.descriptor().and_then(|d| d.to_str()) {
            read += usize::from(descriptor::parse_method(&text).is_ok());
        }
        read += usize::from(method.name().is_ok());
        read += method.attributes().filter(|a| a.decode().is_ok()).count();
    }
    read
}
