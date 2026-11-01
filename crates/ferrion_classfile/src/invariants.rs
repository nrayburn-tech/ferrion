//! Checks that the readers keep the promises their APIs document, for the fuzz targets in
//! `fuzz/` and the tests. Not part of the public API.
//!
//! Each check reads its input with every accessor, so a panic anywhere is found, and fails with a
//! message when a documented promise is broken: an entry the index lists that then can't be found,
//! a read whose size differs from the declared one without an error, or a signature that doesn't
//! print back as its text.

use std::sync::Arc;

use crate::class::{Annotation, Attribute, Attributes, ClassFile, ElementValue};
use crate::ctsym::{CtSym, ReleaseContent};
use crate::jar::{Jar, Manifest};
use crate::jimage::Image;
use crate::testing::{ClassBuilder, ImageBuilder, ImageCompression};
use crate::text::{Name, decode_utf16, encode_utf16};
use crate::zip::{MAX_ENTRY_SIZE, ZipArchive};
use crate::{descriptor, signature};

type Check = Result<(), String>;

/// The releases jars and `ct.sym` are resolved at.
const RELEASES: [u32; 8] = [8, 9, 10, 11, 17, 21, 25, 27];

/// Reads `bytes` as a class file and calls every accessor.
pub fn check_class(bytes: &[u8]) -> Check {
    let Ok(class) = ClassFile::parse(Arc::from(bytes)) else {
        return Ok(());
    };
    let _ = class.version().is_preview();
    let _ = class.access();
    let _ = class.trailing_bytes();
    for index in 0..class.constant_pool_count() {
        if let Ok(crate::class::Constant::Utf8(name)) = class.constant(index) {
            check_name(name)?;
        }
    }
    if let Ok(name) = class.this_class() {
        check_name(name)?;
    }
    let _ = class.super_class();
    for interface in class.interfaces() {
        let _ = interface;
    }
    let signature = class.signature();
    if let Ok(Some(text)) = signature
        && let Ok(text) = text.to_str()
    {
        check_signature_kind(&text, Kind::Class)?;
    }
    check_attributes(class.attributes())?;
    for (members, kind) in [
        (class.fields(), Kind::Field),
        (class.methods(), Kind::Method),
    ] {
        let expected = members.len();
        let mut count = 0;
        for member in members {
            count += 1;
            let _ = member.access();
            let _ = member.name().map(check_name);
            if let Ok(text) = member
                .descriptor()
                .and_then(|d| d.to_str().map(|s| s.into_owned()))
            {
                match kind {
                    Kind::Field => {
                        round_trip(&text, descriptor::parse_field(&text).map(|d| d.to_string()))?
                    }
                    _ => round_trip(
                        &text,
                        descriptor::parse_method(&text).map(|d| d.to_string()),
                    )?,
                }
            }
            if let Ok(Some(text)) = member.signature()
                && let Ok(text) = text.to_str()
            {
                check_signature_kind(&text, kind)?;
            }
            check_attributes(member.attributes())?;
        }
        if count != expected {
            return Err(format!(
                "members iterated {count} times, but the length said {expected}"
            ));
        }
    }
    if class.access().contains(crate::class::AccessFlags::MODULE) {
        let _ = class.module_descriptor();
    }
    Ok(())
}

fn round_trip(text: &str, printed: crate::Result<String>) -> Check {
    match printed {
        Ok(printed) if printed != text => {
            Err(format!("`{text}` parsed and printed as `{printed}`"))
        }
        _ => Ok(()),
    }
}

/// A name that decodes compares equal to its text, and its UTF-16 encodes back to the same
/// units.
fn check_name(name: Name<'_>) -> Check {
    if let Ok(text) = name.to_str()
        && !name.eq_str(&text)
    {
        return Err(format!("{name:?} isn't eq_str to its own text"));
    }
    if let Ok(units) = decode_utf16(name.as_bytes()) {
        let again = decode_utf16(&encode_utf16(&units)).map_err(|e| e.to_string())?;
        if again != units {
            return Err(format!("{name:?} doesn't survive encoding its UTF-16"));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Kind {
    Class,
    Field,
    Method,
}

fn check_signature_kind(text: &str, kind: Kind) -> Check {
    match kind {
        Kind::Class => round_trip(text, signature::parse_class(text).map(|s| s.to_string())),
        Kind::Method => round_trip(text, signature::parse_method(text).map(|s| s.to_string())),
        Kind::Field => round_trip(text, signature::parse_field(text).map(|s| s.to_string())),
    }
}

/// Parses `text` as each kind of signature and descriptor; whatever parses must print back as
/// `text`.
pub fn check_signature(text: &str) -> Check {
    for kind in [Kind::Class, Kind::Field, Kind::Method] {
        check_signature_kind(text, kind)?;
    }
    round_trip(text, descriptor::parse_field(text).map(|d| d.to_string()))?;
    round_trip(text, descriptor::parse_method(text).map(|d| d.to_string()))
}

fn check_attributes(attributes: Attributes<'_>) -> Check {
    let expected = attributes.len();
    let mut count = 0;
    for info in attributes {
        count += 1;
        let _ = info.name();
        let Ok(attribute) = info.decode() else {
            continue;
        };
        check_attribute(attribute)?;
    }
    if count != expected {
        return Err(format!(
            "attributes iterated {count} times, but the length said {expected}"
        ));
    }
    Ok(())
}

fn check_attribute(attribute: Attribute<'_>) -> Check {
    match attribute {
        Attribute::ConstantValue(crate::class::ConstantValue::String(text)) => {
            let _ = text.to_utf16();
            let _ = text.to_string_lossy();
        }
        Attribute::Exceptions(refs)
        | Attribute::NestMembers(refs)
        | Attribute::PermittedSubclasses(refs)
        | Attribute::ModulePackages(refs) => {
            let expected = refs.len();
            if refs.count() != expected {
                return Err("a reference list's length is wrong".to_owned());
            }
        }
        Attribute::InnerClasses(entries) => entries.for_each(drop),
        Attribute::Annotations { annotations, .. } => {
            for annotation in annotations.flatten() {
                check_annotation(&annotation)?;
            }
        }
        Attribute::ParameterAnnotations { parameters, .. } => {
            for annotations in parameters.flatten() {
                for annotation in annotations.flatten() {
                    check_annotation(&annotation)?;
                }
            }
        }
        Attribute::TypeAnnotations { annotations, .. } => {
            for annotation in annotations.flatten() {
                let _ = annotation.target.target_type();
                check_annotation(&annotation.annotation)?;
            }
        }
        Attribute::AnnotationDefault(value) => check_value(value)?,
        Attribute::Record(components) => {
            for component in components {
                let _ = component.name();
                if let Ok(text) = component
                    .descriptor()
                    .and_then(|d| d.to_str().map(|s| s.into_owned()))
                {
                    round_trip(&text, descriptor::parse_field(&text).map(|d| d.to_string()))?;
                }
                if let Ok(Some(text)) = component.signature()
                    && let Ok(text) = text.to_str()
                {
                    check_signature_kind(&text, Kind::Field)?;
                }
                check_attributes(component.attributes())?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Walks an annotation's values with an explicit stack, as deep as they go.
fn check_annotation(annotation: &Annotation<'_>) -> Check {
    let _ = annotation.type_descriptor();
    for (name, value) in annotation.elements().flatten() {
        let _ = name.to_str();
        check_value(value)?;
    }
    Ok(())
}

fn check_value(value: ElementValue<'_>) -> Check {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            ElementValue::String(text) => {
                let _ = text.to_string_lossy();
            }
            ElementValue::Annotation(annotation) => {
                let _ = annotation.type_descriptor();
                stack.extend(annotation.elements().flatten().map(|(_, v)| v));
            }
            ElementValue::Array(values) => {
                let expected = values.len();
                let items: Vec<_> = values.collect();
                if items.len() > expected {
                    return Err("an array yielded more values than its length".to_owned());
                }
                stack.extend(items.into_iter().flatten());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Reads `bytes` as a zip file: every entry is found by its name, and reads to its declared
/// size or fails.
pub fn check_zip(bytes: &[u8]) -> Check {
    let Ok(archive) = ZipArchive::from_bytes(Arc::from(bytes)) else {
        return Ok(());
    };
    check_archive(&archive)
}

fn check_archive(archive: &ZipArchive) -> Check {
    let _ = archive.problems();
    for entry in archive.entries() {
        let Some(found) = archive.find(entry.name()) else {
            return Err(format!("{:?} is listed but not found", entry.name()));
        };
        if found.name() != entry.name() || found.index() < entry.index() {
            return Err(format!(
                "{:?} found as {:?}, not the last with the name",
                entry, found
            ));
        }
        if let Ok(data) = archive.read(entry, MAX_ENTRY_SIZE)
            && data.len() as u64 != entry.size()
        {
            return Err(format!(
                "{entry:?} read {} bytes, not {}",
                data.len(),
                entry.size()
            ));
        }
    }
    let unique: Vec<&str> = archive.unique_entries().map(|e| e.name()).collect();
    if unique.windows(2).any(|w| w[0] >= w[1]) {
        return Err("unique entries aren't sorted and unique".to_owned());
    }
    Ok(())
}

/// Reads `bytes` as a manifest, and as a zip resolved as a jar at several releases: every name
/// the jar lists is found, and every class its packages list is read.
pub fn check_jar(bytes: &[u8]) -> Check {
    let _ = Manifest::parse(bytes);
    let Ok(archive) = ZipArchive::from_bytes(Arc::from(bytes)) else {
        return Ok(());
    };
    let archive = Arc::new(archive);
    for release in RELEASES {
        let jar = Jar::new(archive.clone(), release);
        let _ = jar.manifest_problem();
        for (name, entry) in jar.entries() {
            match jar.find(name) {
                Some(found) if found.index() == entry.index() => {}
                other => {
                    return Err(format!(
                        "{name:?} at release {release} found as {other:?}, not {entry:?}"
                    ));
                }
            }
        }
        let packages: Vec<String> = jar.packages().map(str::to_owned).collect();
        for package in packages {
            for simple in jar.package(&package).unwrap_or_default() {
                let name = if package.is_empty() {
                    simple.to_owned()
                } else {
                    format!("{package}/{simple}")
                };
                if jar.find(&format!("{name}.class")).is_none() {
                    return Err(format!("{name} is listed in its package but not found"));
                }
                let _ = jar.read_class(&name);
            }
        }
    }
    Ok(())
}

/// Reads `bytes` as a jimage: every resource is found by its module and name, and reads to its
/// declared size or fails.
pub fn check_jimage(bytes: &[u8]) -> Check {
    let Ok(image) = Image::from_bytes(Arc::from(bytes)) else {
        return Ok(());
    };
    let _ = image.problems();
    let _ = (image.version(), image.is_big_endian());
    for resource in image.resources() {
        let name = resource.name();
        if !image.modules().iter().any(|m| **m == *resource.module()) {
            return Err(format!("{name}'s module isn't listed"));
        }
        if !image.package(&resource.package()).any(|r| r.name() == name) {
            return Err(format!("{name} isn't in its package's resources"));
        }
        if resource.is_class() {
            let package = resource.package();
            let class = if package.is_empty() {
                resource.base()
            } else {
                format!("{package}/{}", resource.base())
            };
            if image.find_class(resource.module(), &class).is_none() {
                return Err(format!("{name} isn't found as class {class}"));
            }
        }
        if let Ok(data) = resource.read()
            && data.len() as u64 != resource.size()
        {
            return Err(format!(
                "{name} read {} bytes, not {}",
                data.len(),
                resource.size()
            ));
        }
    }
    Ok(())
}

/// Builds a valid image from `data`, then changes a few bytes of its index and resources, so the
/// fuzzer reaches the location, string, and decompression code that random bytes rarely do.
pub fn structured_image(data: &[u8]) -> Vec<u8> {
    let mut data = data.iter().copied();
    let mut next = || data.next().unwrap_or(0);
    let mut builder = ImageBuilder::new();
    let flags = next();
    builder.big_endian = flags & 1 != 0;
    builder.compression = match (flags >> 1) % 3 {
        0 => ImageCompression::None,
        1 => ImageCompression::Zip,
        _ => ImageCompression::CompactCp,
    };
    builder.minor_version = u16::from(flags >> 7);
    const MODULES: [&str; 3] = ["java.base", "java.desktop", "m"];
    const PACKAGES: [&str; 4] = ["java/lang", "java/util", "", "a/b/c"];
    let count = next() % 6;
    for i in 0..count {
        let module = MODULES[usize::from(next()) % MODULES.len()];
        let package = PACKAGES[usize::from(next()) % PACKAGES.len()];
        let simple = format!("C{i}");
        let name = if package.is_empty() {
            simple.clone()
        } else {
            format!("{package}/{simple}")
        };
        let content = if next() % 4 == 0 {
            (0..next()).map(|_| next()).collect()
        } else {
            let mut class = ClassBuilder::new(&name);
            let descriptor = [
                "(Ljava/lang/String;I)V",
                "Ljava/util/List;",
                "[[LTop;",
                "TT;",
            ];
            for _ in 0..next() % 4 {
                class.utf8(descriptor[usize::from(next()) % descriptor.len()]);
            }
            class.long(i64::from(next()));
            class.finish()
        };
        let extension = if next() % 5 == 0 { "png" } else { "class" };
        let path = format!("{}.{extension}", name);
        builder.add(module, &path, &content);
    }
    let mut image = builder.finish();
    // Then (offset, value) pairs change bytes.
    while let (Some(offset), Some(value)) = (data.next(), data.next()) {
        let high = data.next().unwrap_or(0);
        if image.is_empty() {
            break;
        }
        let at = (usize::from(high) << 8 | usize::from(offset)) % image.len();
        image[at] = value;
    }
    image
}

/// Reads `bytes` as a zip and then as `ct.sym`, resolving every release: every class a release
/// lists is found.
pub fn check_ctsym(bytes: &[u8]) -> Check {
    let Ok(archive) = ZipArchive::from_bytes(Arc::from(bytes)) else {
        return Ok(());
    };
    let ctsym = CtSym::new(Arc::new(archive));
    let mut releases = ctsym.releases();
    releases.extend(RELEASES);
    releases.push(36);
    for release in releases {
        if let Ok(ReleaseContent::SystemModules(modules)) = ctsym.content(release) {
            let _ = modules;
        }
        let Ok(index) = ctsym.release(release) else {
            continue;
        };
        let _ = (index.modules(), index.problems());
        let packages: Vec<String> = index.packages().map(str::to_owned).collect();
        for package in packages {
            let names: Vec<String> = index
                .package(&package)
                .into_iter()
                .flatten()
                .map(str::to_owned)
                .collect();
            for simple in names {
                let name = if package.is_empty() {
                    simple.clone()
                } else {
                    format!("{package}/{simple}")
                };
                let Some((entry, _)) = index.find(&name) else {
                    return Err(format!(
                        "{name} is listed for release {release} but not found"
                    ));
                };
                if let Some(entry) = ctsym.archive().entry(entry) {
                    let _ = entry.read();
                }
            }
            let _ = index.module_of(&package);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ZipBuilder;

    #[test]
    fn checks_pass_on_valid_input() {
        let mut b = ClassBuilder::new("p/A");
        let sig = b.utf8("<T:Ljava/lang/Object;>Ljava/lang/Object;");
        let attribute = b.attribute("Signature", &sig.to_be_bytes());
        b.class_attribute(attribute);
        b.field(0, "f", "[[Ljava/lang/String;", &[]);
        b.method(0, "m", "(IJ)V", &[]);
        let class = b.finish();
        check_class(&class).unwrap();
        let mut zip = ZipBuilder::new();
        zip.stored("META-INF/MANIFEST.MF", b"Multi-Release: true\n")
            .deflated("p/A.class", &class)
            .deflated("META-INF/versions/11/p/A.class", &class)
            .deflated("8/java.base/p/A.sig", &class);
        let zip = zip.finish();
        check_zip(&zip).unwrap();
        check_jar(&zip).unwrap();
        check_ctsym(&zip).unwrap();
        check_signature("Ljava/util/Map<TK;+[I>.Entry;").unwrap();
        for seed in 0..64u8 {
            let data: Vec<u8> = (0..200u8)
                .map(|i| i.wrapping_mul(seed).wrapping_add(seed))
                .collect();
            let image = structured_image(&data[..2 + usize::from(seed % 30)]);
            check_jimage(&image).unwrap();
        }
    }
}
