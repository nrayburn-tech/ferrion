use super::*;
use crate::testing::ZipBuilder;

fn archive(builder: &ZipBuilder) -> ZipArchive {
    ZipArchive::from_bytes(Arc::from(builder.finish())).unwrap()
}

fn sample() -> ZipBuilder {
    let mut b = ZipBuilder::new();
    b.stored("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\n\r\n")
        .deflated("p/A.class", &[7; 1000])
        .stored("p/", b"")
        .deflated("p/q/B.class", b"")
        .stored("r.txt", b"hello");
    b
}

fn names(archive: &ZipArchive) -> Vec<&str> {
    archive.entries().map(|e| e.name()).collect()
}

#[test]
fn stored_and_deflated() {
    for zip64 in [false, true] {
        for data_descriptors in [false, true] {
            let mut b = sample();
            b.zip64 = zip64;
            b.data_descriptors = data_descriptors;
            let archive = archive(&b);
            // Directories aren't indexed.
            assert_eq!(
                names(&archive),
                ["META-INF/MANIFEST.MF", "p/A.class", "p/q/B.class", "r.txt"]
            );
            let entry = archive.find("p/A.class").unwrap();
            assert_eq!((entry.method(), entry.size()), (8, 1000));
            assert_eq!(entry.read().unwrap(), [7; 1000]);
            assert_eq!(archive.find("p/q/B.class").unwrap().read().unwrap(), b"");
            assert_eq!(archive.find("r.txt").unwrap().read().unwrap(), b"hello");
            assert!(archive.find("p/").is_none());
            assert!(archive.find("missing").is_none());
            let prefixed: Vec<_> = archive.with_prefix("p/").map(|e| e.name()).collect();
            assert_eq!(prefixed, ["p/A.class", "p/q/B.class"]);
            assert!(archive.problems().is_empty());
        }
    }
}

#[test]
fn prefix_and_comment() {
    let mut b = sample();
    b.prefix = b"#!/bin/sh\nexec java -jar \"$0\"\n".to_vec();
    b.comment = b"a comment".to_vec();
    let archive = archive(&b);
    assert_eq!(archive.prefix(), b.prefix.len() as u64);
    assert_eq!(archive.find("r.txt").unwrap().read().unwrap(), b"hello");
    // A JMOD file's four-byte header, with zip64.
    let mut b = sample();
    b.prefix = b"JM\x01\x00".to_vec();
    b.zip64 = true;
    let archive = self::archive(&b);
    assert_eq!(archive.prefix(), 4);
    assert_eq!(
        archive.find("p/A.class").unwrap().read().unwrap(),
        [7; 1000]
    );
}

#[test]
fn a_comment_holding_an_end_record() {
    // A comment that contains the end record's signature: the real record is the one whose
    // comment reaches the end of the file.
    let mut b = sample();
    let mut comment = b"x".to_vec();
    comment.extend_from_slice(&EOCD.to_le_bytes());
    comment.extend_from_slice(&[0; 18]);
    b.comment = comment;
    let archive = archive(&b);
    assert_eq!(archive.len(), 4);
}

#[test]
fn duplicates_last_wins() {
    let mut b = ZipBuilder::new();
    b.stored("p/A.class", b"first")
        .stored("p/B.class", b"b")
        .stored("p/A.class", b"second");
    let archive = archive(&b);
    assert_eq!(archive.len(), 3);
    assert_eq!(
        archive.find("p/A.class").unwrap().read().unwrap(),
        b"second"
    );
    let unique: Vec<_> = archive.unique_entries().map(|e| e.name()).collect();
    assert_eq!(unique, ["p/A.class", "p/B.class"]);
}

#[test]
fn entry_errors() {
    let mut b = ZipBuilder::new();
    b.stored("ok", b"ok")
        .add("odd", b"data", 12)
        .stored("secret", b"x");
    b.entries[2].flags |= 1;
    b.entries.push(crate::testing::ZipEntrySpec {
        name: vec![b'a', 0xff],
        data: Vec::new(),
        method: 0,
        flags: 0,
    });
    let archive = archive(&b);
    assert_eq!(
        archive.find("odd").unwrap().read().unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        archive.find("secret").unwrap().read().unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(archive.problems().len(), 1);
    assert_eq!(archive.problems()[0].kind(), ErrorKind::Malformed);
    let entry = archive.find("ok").unwrap();
    assert_eq!(
        archive.read(entry, 1).unwrap_err().kind(),
        ErrorKind::TooLarge
    );
}

#[test]
fn inflating_checks_sizes() {
    let data = miniz_oxide::deflate::compress_to_vec(b"hello world", 6);
    assert_eq!(inflate(&data, 11, false).unwrap(), b"hello world");
    assert_eq!(
        inflate(&data, 10, false).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    assert_eq!(
        inflate(&data, 12, false).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    assert_eq!(
        inflate(&data[..data.len() - 2], 11, false)
            .unwrap_err()
            .kind(),
        ErrorKind::Malformed
    );
    let zlib = miniz_oxide::deflate::compress_to_vec_zlib(b"hello world", 6);
    assert_eq!(inflate(&zlib, 11, true).unwrap(), b"hello world");
    assert!(inflate(&zlib, 11, false).is_err());
}

#[test]
fn malformed_archives() {
    let good = sample().finish();
    // No truncation of the end of the file panics; most are errors.
    for len in 0..good.len() {
        if let Ok(archive) = ZipArchive::from_bytes(Arc::from(&good[..len])) {
            for entry in archive.entries() {
                let _ = entry.read();
            }
        }
    }
    assert!(ZipArchive::from_bytes(Arc::from(&b"PK"[..])).is_err());
    // A count too large for the directory.
    let mut bad = good.clone();
    let end = bad.len() - 22;
    bad[end + 10..end + 12].copy_from_slice(&5000u16.to_le_bytes());
    assert_eq!(
        ZipArchive::from_bytes(Arc::from(bad)).unwrap_err().kind(),
        ErrorKind::Malformed
    );
    // A stored entry whose sizes differ.
    let mut b = ZipBuilder::new();
    b.stored("a", b"abc");
    let mut bytes = b.finish();
    let central = bytes.len() - 22 - (46 + 1);
    bytes[central + 24..central + 28].copy_from_slice(&2u32.to_le_bytes());
    let archive = ZipArchive::from_bytes(Arc::from(bytes)).unwrap();
    assert_eq!(
        archive.find("a").unwrap().read().unwrap_err().kind(),
        ErrorKind::Malformed
    );
}

#[test]
fn reads_from_disk() {
    let scratch = crate::source::tests::Scratch::new("zip");
    let path = scratch.write("a.jar", &sample().finish());
    let archive = ZipArchive::open(&path).unwrap();
    assert_eq!(archive.path(), Some(path.as_path()));
    let error = archive
        .read(archive.find("p/A.class").unwrap(), 10)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "{}(p/A.class): the entry's size, 1000 bytes, is above the limit of 10",
            path.display()
        )
    );
    assert_eq!(
        archive.find("p/A.class").unwrap().read().unwrap(),
        [7; 1000]
    );
    let missing = ZipArchive::open(&scratch.0.join("missing.jar")).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::Io);
}
