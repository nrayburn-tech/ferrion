use super::*;
use crate::source::tests::Scratch;
use crate::testing::{ClassBuilder, ImageBuilder, ZipBuilder};

fn class(name: &str) -> Vec<u8> {
    ClassBuilder::new(name).finish()
}

/// A module-info with unqualified `exports`, `requires`, and a `ModuleResolution` value.
fn module_info(
    name: &str,
    exports: &[&str],
    requires: &[&str],
    resolution: Option<u16>,
) -> Vec<u8> {
    let mut b = ClassBuilder::new("module-info");
    b.set_super(None);
    b.access = 0x8000;
    let module = b.module(name);
    let mut c = Vec::new();
    for value in [module, 0, 0, requires.len() as u16] {
        c.extend_from_slice(&value.to_be_bytes());
    }
    for required in requires {
        let required = b.module(required);
        for value in [required, 0, 0] {
            c.extend_from_slice(&value.to_be_bytes());
        }
    }
    c.extend_from_slice(&(exports.len() as u16).to_be_bytes());
    for export in exports {
        let package = b.package(export);
        for value in [package, 0, 0] {
            c.extend_from_slice(&value.to_be_bytes());
        }
    }
    c.extend_from_slice(&[0; 6]);
    let attribute = b.attribute("Module", &c);
    b.class_attribute(attribute);
    if let Some(value) = resolution {
        let attribute = b.attribute("ModuleResolution", &value.to_be_bytes());
        b.class_attribute(attribute);
    }
    b.finish()
}

fn image() -> Vec<u8> {
    let mut b = ImageBuilder::new();
    b.add(
        "java.base",
        "module-info.class",
        &module_info("java.base", &["java/lang"], &[], None),
    )
    .add(
        "java.base",
        "java/lang/Object.class",
        &class("java/lang/Object"),
    )
    .add(
        "java.base",
        "jdk/internal/misc/Unsafe.class",
        &class("jdk/internal/misc/Unsafe"),
    )
    .add(
        "jdk.unsupported.desktop",
        "module-info.class",
        &module_info("jdk.unsupported.desktop", &["jdk/swing/interop"], &[], None),
    )
    .add(
        "jdk.unsupported.desktop",
        "jdk/swing/interop/X.class",
        &class("jdk/swing/interop/X"),
    )
    .add(
        "jdk.incubator.vector",
        "module-info.class",
        &module_info(
            "jdk.incubator.vector",
            &["jdk/incubator/vector"],
            &["java.base"],
            Some(9),
        ),
    )
    .add(
        "jdk.incubator.vector",
        "jdk/incubator/vector/V.class",
        &class("jdk/incubator/vector/V"),
    );
    b.finish()
}

fn write_jdk(scratch: &Scratch, name: &str, version: &str, ctsym: Option<&ZipBuilder>) -> PathBuf {
    let home = scratch.0.join(name);
    scratch.write(
        &format!("{name}/release"),
        format!("JAVA_VERSION=\"{version}\"\nIMPLEMENTOR=\"Test\"\n").as_bytes(),
    );
    scratch.write(&format!("{name}/lib/modules"), &image());
    if let Some(ctsym) = ctsym {
        scratch.write(&format!("{name}/lib/ct.sym"), &ctsym.finish());
    }
    home
}

#[test]
fn versions() {
    assert_eq!(feature_version("JAVA_VERSION=\"1.8.0_502\"\n"), Some(8));
    assert_eq!(
        feature_version("IMPLEMENTOR=\"x\"\nJAVA_VERSION=\"21.0.11\"\n"),
        Some(21)
    );
    assert_eq!(feature_version("JAVA_VERSION=\"27\""), Some(27));
    assert_eq!(feature_version("JAVA_VERSION=\"25-ea\""), Some(25));
    assert_eq!(feature_version("OTHER=1"), None);
}

#[test]
fn own_release_from_the_image() {
    // JDK 21's shape: ct.sym has release 20 in full and only system-modules for 21.
    let mut ctsym = ZipBuilder::new();
    ctsym
        .deflated(
            "KL/java.base/module-info.sig",
            &module_info("java.base", &["java/lang"], &[], None),
        )
        .deflated(
            "K/java.base/java/lang/Object.sig",
            &class("java/lang/Object"),
        )
        .deflated("L/system-modules", b"java.base\njdk.incubator.vector\n");
    let scratch = Scratch::new("jdk21");
    let home = write_jdk(&scratch, "jdk", "21.0.11", Some(&ctsym));
    let cache = ArchiveCache::new();
    let jdk = Jdk::open(&home, &cache).unwrap();
    assert!(Arc::ptr_eq(&jdk, &Jdk::open(&home, &cache).unwrap()));
    assert_eq!(jdk.feature_version(), 21);
    assert_eq!(jdk.releases().unwrap(), [20, 21]);

    let platform = jdk.platform(21).unwrap();
    assert!(Arc::ptr_eq(&platform, &jdk.platform(21).unwrap()));
    assert_eq!(platform.source(), PlatformSource::Image);
    let found = platform
        .find(&ClassName::new("java/lang/Object"))
        .unwrap()
        .unwrap();
    assert!(
        ClassFile::parse(found.bytes)
            .unwrap()
            .this_class()
            .unwrap()
            .eq_str("java/lang/Object")
    );
    // Unexported classes are found but not visible.
    assert!(
        platform
            .find(&ClassName::new("jdk/internal/misc/Unsafe"))
            .unwrap()
            .is_some()
    );
    assert!(!platform.is_visible(&PackageName::new("jdk/internal/misc")));
    assert!(platform.is_visible(&PackageName::new("java/lang")));
    // Modules outside system-modules don't exist for the own release.
    assert!(
        platform
            .find(&ClassName::new("jdk/swing/interop/X"))
            .unwrap()
            .is_none()
    );
    assert!(
        platform
            .package(&PackageName::new("jdk/swing/interop"))
            .unwrap()
            .is_none()
    );
    // Not resolved by default: found, but not visible.
    assert!(
        platform
            .find(&ClassName::new("jdk/incubator/vector/V"))
            .unwrap()
            .is_some()
    );
    assert!(!platform.is_visible(&PackageName::new("jdk/incubator/vector")));
    // `--add-modules` makes it a root; `ALL-SYSTEM` makes every module one.
    for roots in [["jdk.incubator.vector"], ["ALL-SYSTEM"]] {
        let visible = platform.visible_with_roots(&roots).unwrap();
        assert!(visible.contains("jdk/incubator/vector"), "{roots:?}");
        assert!(visible.contains("java/lang"), "{roots:?}");
        assert!(!visible.contains("jdk/internal/misc"), "{roots:?}");
    }
    assert_eq!(
        platform.visible_with_roots(&[]).unwrap().len(),
        platform.visible_packages().unwrap().len()
    );
    assert_eq!(platform.modules().len(), 2);
    assert_eq!(
        platform.class_names().unwrap(),
        [
            "java/lang/Object",
            "jdk/incubator/vector/V",
            "jdk/internal/misc/Unsafe",
            "module-info"
        ]
    );

    let older = jdk.platform(20).unwrap();
    assert_eq!(older.source(), PlatformSource::CtSym);
    assert!(
        older
            .find(&ClassName::new("java/lang/Object"))
            .unwrap()
            .is_some()
    );
    assert_eq!(older.visible_packages().unwrap(), ["java/lang"]);
    let listing = older
        .package(&PackageName::new("java/lang"))
        .unwrap()
        .unwrap();
    assert_eq!(listing.names().collect::<Vec<_>>(), ["Object"]);

    for release in [7, 19, 22] {
        assert_eq!(
            jdk.platform(release).unwrap_err().kind(),
            ErrorKind::Usage,
            "{release}"
        );
    }
}

#[test]
fn own_release_from_ctsym() {
    // JDK 25's shape: the own release is in ct.sym like any other.
    let mut ctsym = ZipBuilder::new();
    ctsym
        .deflated(
            "P/java.base/module-info.sig",
            &module_info("java.base", &["java/lang"], &[], None),
        )
        .deflated(
            "P/java.base/java/lang/Object.sig",
            &class("java/lang/Object"),
        )
        .deflated(
            "P/java.base/java/lang/Thread.sig",
            &class("java/lang/Thread"),
        );
    let scratch = Scratch::new("jdk25");
    let home = write_jdk(&scratch, "jdk", "25.0.1", Some(&ctsym));
    let jdk = Jdk::open(&home, &ArchiveCache::new()).unwrap();
    let platform = jdk.platform(25).unwrap();
    assert_eq!(platform.source(), PlatformSource::CtSym);
    assert!(
        platform
            .find(&ClassName::new("java/lang/Thread"))
            .unwrap()
            .is_some()
    );
    // The image's class isn't in ct.sym's release 25, so it isn't served.
    assert!(
        platform
            .find(&ClassName::new("jdk/internal/misc/Unsafe"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn jdk_9_and_10_are_usage_errors() {
    let scratch = Scratch::new("jdk9");
    for version in ["9.0.4", "10.0.2"] {
        let home = write_jdk(&scratch, version, version, None);
        let error = Jdk::open(&home, &ArchiveCache::new()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Usage);
        assert!(
            error
                .message()
                .contains("use JDK 11 or later with --release"),
            "{error}"
        );
    }
}

#[test]
fn a_runtime_without_ctsym_serves_its_own_release() {
    let scratch = Scratch::new("runtime");
    let home = write_jdk(&scratch, "jre", "17.0.1", None);
    let jdk = Jdk::open(&home, &ArchiveCache::new()).unwrap();
    assert_eq!(jdk.releases().unwrap(), [17]);
    let platform = jdk.platform(17).unwrap();
    // Every module is there, unfiltered.
    assert!(
        platform
            .find(&ClassName::new("jdk/swing/interop/X"))
            .unwrap()
            .is_some()
    );
    assert_eq!(jdk.platform(11).unwrap_err().kind(), ErrorKind::Usage);
}

#[test]
fn jdk_8_reads_rt_jar_through_ct_sym() {
    let scratch = Scratch::new("jdk8");
    let mut rt = ZipBuilder::new();
    rt.stored("java/lang/Object.class", &class("java/lang/Object"))
        .stored(
            "com/apple/eawt/Application.class",
            &class("com/apple/eawt/Application"),
        );
    let mut ctsym = ZipBuilder::new();
    ctsym.stored(
        "META-INF/sym/rt.jar/java/lang/Object.class",
        &class("java/lang/Object"),
    );
    let mut jce = ZipBuilder::new();
    jce.stored("javax/crypto/Cipher.class", &class("javax/crypto/Cipher"));
    let mut ext = ZipBuilder::new();
    ext.stored(
        "jdk/nashorn/api/Script.class",
        &class("jdk/nashorn/api/Script"),
    );
    scratch.write("jdk8/release", b"JAVA_VERSION=\"1.8.0_502\"\n");
    scratch.write("jdk8/jre/lib/rt.jar", &rt.finish());
    scratch.write("jdk8/jre/lib/jce.jar", &jce.finish());
    scratch.write("jdk8/jre/lib/ext/nashorn.jar", &ext.finish());
    let home = scratch.0.join("jdk8");
    let cache = ArchiveCache::new();
    {
        // Without lib/ct.sym, rt.jar is read as it is.
        let jdk = Jdk::open(&home, &cache).unwrap();
        let platform = jdk.platform(8).unwrap();
        assert!(
            platform
                .find(&ClassName::new("com/apple/eawt/Application"))
                .unwrap()
                .is_some()
        );
    }
    scratch.write("jdk8/lib/ct.sym", &ctsym.finish());
    // A different JDK now: the cache is keyed by the release file, so drop the old one.
    let jdk = Jdk::open_uncached(&home, &cache).unwrap();
    assert_eq!(jdk.feature_version(), 8);
    assert_eq!(jdk.releases().unwrap(), [8]);
    assert_eq!(jdk.platform(11).unwrap_err().kind(), ErrorKind::Usage);
    let platform = jdk.platform(8).unwrap();
    assert_eq!(platform.source(), PlatformSource::BootClassPath);
    let object = platform
        .find(&ClassName::new("java/lang/Object"))
        .unwrap()
        .unwrap();
    assert!(
        matches!(&object.origin, Origin::Entry { entry, .. } if entry.starts_with("META-INF/sym/rt.jar/"))
    );
    // rt.jar's classes that ct.sym leaves out don't exist for javac 8.
    assert!(
        platform
            .find(&ClassName::new("com/apple/eawt/Application"))
            .unwrap()
            .is_none()
    );
    assert!(
        platform
            .find(&ClassName::new("javax/crypto/Cipher"))
            .unwrap()
            .is_some()
    );
    assert!(
        platform
            .find(&ClassName::new("jdk/nashorn/api/Script"))
            .unwrap()
            .is_some()
    );
    assert!(platform.is_visible(&PackageName::new("sun/misc")));
    assert!(platform.visible_packages().is_none());
    let listing = platform
        .package(&PackageName::new("java/lang"))
        .unwrap()
        .unwrap();
    assert_eq!(listing.names().collect::<Vec<_>>(), ["Object"]);
    assert_eq!(
        platform.class_names().unwrap(),
        [
            "java/lang/Object",
            "javax/crypto/Cipher",
            "jdk/nashorn/api/Script"
        ]
    );
}

#[test]
fn send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Jdk>();
    check::<PlatformApi>();
    check::<ArchiveCache>();
    check::<crate::classpath::ClassPath>();
}
