use super::*;
use crate::testing::{ClassBuilder, ImageBuilder, ImageCompression};

fn class(name: &str) -> Vec<u8> {
    let mut b = ClassBuilder::new(name);
    let sig = b.utf8("<T:Ljava/lang/Object;>(Ljava/util/Map<TK;Ljava/lang/String;>;[LTop;I)TT;");
    let signature = b.attribute("Signature", &sig.to_be_bytes());
    b.method(
        1,
        "m",
        "(Ljava/util/Map;[LTop;I)Ljava/lang/Object;",
        &[signature],
    );
    b.utf8("text with NUL \0 and \u{e9}");
    b.long(7);
    b.finish()
}

fn sample(big_endian: bool, compression: ImageCompression) -> ImageBuilder {
    let mut b = ImageBuilder::new();
    b.big_endian = big_endian;
    b.compression = compression;
    b.add(
        "java.base",
        "java/lang/Object.class",
        &class("java/lang/Object"),
    )
    .add("java.base", "module-info.class", &class("module-info"))
    .add(
        "java.base",
        "java/lang/String.class",
        &class("java/lang/String"),
    )
    .add(
        "java.desktop",
        "java/awt/Font.class",
        &class("java/awt/Font"),
    )
    .add("java.desktop", "java/awt/icon.png", &[1, 2, 3])
    .add("java.base", "META-INF/services/x", b"");
    b
}

#[test]
fn every_byte_order_and_compression() {
    for big_endian in [false, true] {
        for compression in [
            ImageCompression::None,
            ImageCompression::Zip,
            ImageCompression::CompactCp,
        ] {
            let builder = sample(big_endian, compression);
            let image = Image::from_bytes(Arc::from(builder.finish())).unwrap();
            assert_eq!(image.is_big_endian(), big_endian);
            assert_eq!(image.version(), (1, 0));
            assert!(image.problems().is_empty(), "{:?}", image.problems());
            assert_eq!(image.modules(), ["java.base".into(), "java.desktop".into()]);
            let names: Vec<_> = image.resources().map(|r| r.name()).collect();
            assert_eq!(
                names,
                [
                    "/java.base/java/lang/Object.class",
                    "/java.base/module-info.class",
                    "/java.base/java/lang/String.class",
                    "/java.desktop/java/awt/Font.class",
                    "/java.desktop/java/awt/icon.png",
                    "/java.base/META-INF/services/x",
                ]
            );
            for (resource, (_, _, content)) in image.resources().zip(&builder.resources) {
                assert_eq!(
                    &resource.read().unwrap(),
                    content,
                    "{resource:?} {compression:?}"
                );
            }
            let object = image.find_class("java.base", "java/lang/Object").unwrap();
            assert!(object.is_class());
            assert_eq!(object.package(), "java/lang");
            assert!(
                image
                    .find_class("java.desktop", "java/lang/Object")
                    .is_none()
            );
            assert!(image.find_class("java.base", "module-info").is_some());
            let mut awt: Vec<_> = image.package("java/awt").map(|r| r.base()).collect();
            awt.sort();
            assert_eq!(awt, ["Font", "icon"]);
        }
    }
}

#[test]
fn version_1_1_reads_and_major_2_does_not() {
    let mut builder = sample(false, ImageCompression::None);
    builder.minor_version = 1;
    let image = Image::from_bytes(Arc::from(builder.finish())).unwrap();
    assert_eq!(image.version(), (1, 1));
    let mut bytes = sample(false, ImageCompression::None).finish();
    bytes[6] = 2;
    assert_eq!(
        Image::from_bytes(Arc::from(bytes)).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn compact_cp_numbers_and_descriptors() {
    let mut c = Cursor::new(
        &[
            0xa3, 0xc0, 0x29, 0xc2, 0x81, 0xe1, 0x02, 0x03, 0x00, 0x01, 0x02, 0x03, 0x80,
        ],
        0,
    );
    assert_eq!(number(&mut c).unwrap(), 3);
    assert_eq!(number(&mut c).unwrap(), 41);
    assert_eq!(number(&mut c).unwrap(), 641);
    assert_eq!(number(&mut c).unwrap(), 0x01_0203);
    assert_eq!(number(&mut c).unwrap(), 0x0001_0203);
    // `100xxxxx` was never seen.
    assert!(number(&mut c).is_err());

    let strings: [&[u8]; 3] = [b"java/lang", b"Class", b""];
    let string = |offset: u32| Ok(strings[offset as usize]);
    assert_eq!(
        descriptor(b"()L;", &[0, 1], string).unwrap(),
        b"()Ljava/lang/Class;"
    );
    assert_eq!(
        descriptor(b"(L;)V", &[2, 1], string).unwrap(),
        b"(LClass;)V"
    );
    assert!(descriptor(b"(L;L;)V", &[0, 1], string).is_err());
}

#[test]
fn malformed_images() {
    let good = sample(false, ImageCompression::Zip).finish();
    for len in (0..good.len()).step_by(7) {
        if let Ok(image) = Image::from_bytes(Arc::from(&good[..len])) {
            for resource in image.resources() {
                let _ = resource.read();
            }
        }
    }
    assert!(Image::from_bytes(Arc::from(&b"\xda\xda\xfe\xca"[..])).is_err());
    let mut bad = good.clone();
    bad[0] = 0;
    assert_eq!(
        Image::from_bytes(Arc::from(bad)).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    // A table length larger than the file.
    let mut bad = good.clone();
    bad[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        Image::from_bytes(Arc::from(bad)).unwrap_err().kind(),
        ErrorKind::Malformed
    );
}

#[test]
fn locations() {
    // The first location of Corretto 21's image (observation O3), with string offsets only.
    let bytes = [
        0x09, 0x03, 0xbe, 0x12, 0x09, 0x69, 0x6f, 0x1a, 0x09, 0x69, 0xe8, 0x20, 0x01, 0x2b, 0x08,
        0x43, 0x96, 0x61, 0x39, 0x01, 0xe8, 0x00,
    ];
    let location = decode_location(&bytes, 0).unwrap();
    assert_eq!(location.module, 0x03be);
    assert_eq!(location.parent, 0x09696f);
    assert_eq!(location.base, 0x0969e8);
    assert_eq!(location.extension, 1);
    assert_eq!(location.offset, 0x0843_9661);
    assert_eq!(location.size, 0x01e8);
    assert!(decode_location(&[0x40, 0], 0).is_err());
    assert!(decode_location(&[0x0f, 1, 2], 0).is_err());
    assert!(decode_location(&[0x00], 5).is_err());
}
