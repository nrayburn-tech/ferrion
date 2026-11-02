//! Test support, for this crate's tests, the differential check, and the fuzz target: a small
//! stand-in for the platform API, class files described in a line each, and resolution cases
//! written as one text with several files. Not part of the public API.
//!
//! The stand-in platform has the classes the tests name, with the JDK's names, modules, and
//! exports, so that what a case resolves to can be checked by running javac on the same files
//! with a real JDK. Its classes are written here, never copied from a JDK.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use ferrion_classfile::class::{AccessFlags, Exports, ModuleDescriptor, Requires};
use ferrion_classfile::jar::Jar;
use ferrion_classfile::testing::{ClassBuilder, ZipBuilder};
use ferrion_classfile::zip::ZipArchive;
use ferrion_classfile::{
    ClassBytes, ClassName, ClassPathEntry, Origin, PackageListing, PackageName,
};

use crate::decl_names::Symbol;
use crate::input::{ClassPathInput, ClassesInput, Platform};
use crate::symbols::{MemberType, Supertypes, TypeKey};
use crate::{Engine, FileInput, ProjectInput, Snapshot, SourceSetId, SourceSetInput};

/// A class file from a line: a kind (`class`, `interface`, `annotation`, `enum`, `record`,
/// `abstract`, `final`), its binary name, then any of `extends X`, `implements X,Y`,
/// `member Simple Binary` and `private-member Simple Binary` (an `InnerClasses` entry for a
/// member of this class), `outer Binary Simple` (this class is a member of that one), `local`
/// (a local class), `field name descriptor`, `method name descriptor`, and `signature text`.
/// The whole line is split on spaces.
pub fn class_file(spec: &str) -> (String, Vec<u8>) {
    let mut words = spec.split_whitespace();
    let kind = words.next().expect("a kind");
    let name = words.next().expect("a name");
    let mut b = ClassBuilder::new(name);
    b.access = match kind {
        "interface" => 0x0601,
        "annotation" => 0x2601,
        "enum" => 0x4031,
        "record" => 0x0031,
        "abstract" => 0x0421,
        "final" => 0x0031,
        _ => 0x0021,
    };
    if kind == "enum" {
        b.set_super(Some("java/lang/Enum"));
    }
    if kind == "record" {
        b.set_super(Some("java/lang/Record"));
    }
    if name == "java/lang/Object" {
        b.set_super(None);
    }
    let mut inner: Vec<(String, Option<String>, Option<String>, u16)> = Vec::new();
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut signature = None;
    while let Some(word) = words.next() {
        match word {
            "extends" => b.set_super(words.next()),
            "implements" => {
                for interface in words.next().unwrap().split(',') {
                    b.interface(interface);
                }
            }
            "member" | "private-member" => {
                let simple = words.next().unwrap().to_owned();
                let binary = words.next().unwrap().to_owned();
                let flags = if word == "member" { 0x0009 } else { 0x000a };
                inner.push((binary, Some(name.to_owned()), Some(simple), flags));
            }
            "outer" => {
                let outer = words.next().unwrap().to_owned();
                let simple = words.next().unwrap().to_owned();
                inner.push((name.to_owned(), Some(outer), Some(simple), 0x0009));
            }
            "local" => inner.push((name.to_owned(), None, Some("Local".to_owned()), 0)),
            "field" => fields.push((words.next().unwrap(), words.next().unwrap())),
            "method" => methods.push((words.next().unwrap(), words.next().unwrap())),
            "signature" => signature = words.next(),
            other => panic!("unknown word {other:?} in {spec:?}"),
        }
    }
    if !inner.is_empty() {
        let mut contents = (inner.len() as u16).to_be_bytes().to_vec();
        for (binary, outer, simple, flags) in &inner {
            contents.extend_from_slice(&b.class(binary).to_be_bytes());
            let outer = outer.as_ref().map_or(0, |o| b.class(o));
            contents.extend_from_slice(&outer.to_be_bytes());
            let simple = simple.as_ref().map_or(0, |s| b.utf8(s));
            contents.extend_from_slice(&simple.to_be_bytes());
            contents.extend_from_slice(&flags.to_be_bytes());
        }
        let attribute = b.attribute("InnerClasses", &contents);
        b.class_attribute(attribute);
    }
    if kind == "record" {
        let attribute = b.attribute("Record", &0u16.to_be_bytes());
        b.class_attribute(attribute);
    }
    if let Some(signature) = signature {
        let index = b.utf8(signature);
        let attribute = b.attribute("Signature", &index.to_be_bytes());
        b.class_attribute(attribute);
    }
    for (field, descriptor) in fields {
        b.field(0x0001, field, descriptor, &[]);
    }
    for (method, descriptor) in methods {
        b.method(0x0001, method, descriptor, &[]);
    }
    (name.to_owned(), b.finish())
}

/// A jar of the classes `specs` describe (see [`class_file`]), as a classpath entry.
pub fn jar(specs: &[&str], release: u32) -> ClassPathEntry {
    let mut zip = ZipBuilder::new();
    for spec in specs {
        let (name, bytes) = class_file(spec);
        zip.stored(&format!("{name}.class"), &bytes);
    }
    let archive = ZipArchive::from_bytes(Arc::from(zip.finish())).expect("a valid zip");
    ClassPathEntry::jar(Arc::new(Jar::new(Arc::new(archive), release)))
}

/// A stand-in for a JDK's platform API: classes, modules, and which packages the unnamed module
/// sees.
#[derive(Debug)]
pub struct StubPlatform {
    classes: HashMap<String, Arc<[u8]>>,
    packages: HashMap<String, Vec<Box<str>>>,
    modules: Vec<ModuleDescriptor>,
    module_of: HashMap<String, String>,
    visible: HashSet<String>,
}

/// Each stub module: its name, whether it is resolved by default, what it requires (transitive
/// with a `+`), its exports (`package` or `package>to,to`), and its classes.
type StubModule = (
    &'static str,
    bool,
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
);

const STUB_MODULES: &[StubModule] = &[
    (
        "java.base",
        true,
        &[],
        &[
            "java/lang",
            "java/lang/annotation",
            "java/io",
            "java/util",
            "java/util/function",
            "javax/annotation/processing",
            "jdk/internal/misc>java.desktop",
        ],
        &[
            "class java/lang/Object method toString ()Ljava/lang/String; method hashCode ()I method equals (Ljava/lang/Object;)Z method getClass ()Ljava/lang/Class;",
            "final java/lang/String implements java/lang/CharSequence,java/lang/Comparable,java/io/Serializable method length ()I method isEmpty ()Z",
            "final java/lang/Math method abs (I)I method max (II)I",
            "final java/util/Objects method requireNonNull (Ljava/lang/Object;)Ljava/lang/Object;",
            "class java/io/PrintStream method println ()V method println (Ljava/lang/String;)V",
            "interface java/lang/CharSequence",
            "interface java/lang/Comparable signature <T:Ljava/lang/Object;>Ljava/lang/Object;",
            "interface java/lang/Iterable",
            "interface java/lang/Runnable",
            "interface java/lang/AutoCloseable",
            "interface java/lang/Cloneable",
            "abstract java/lang/Number implements java/io/Serializable",
            "final java/lang/Integer extends java/lang/Number implements java/lang/Comparable field MAX_VALUE I",
            "abstract java/lang/Enum implements java/lang/Comparable,java/io/Serializable",
            "abstract java/lang/Record",
            "class java/lang/Throwable implements java/io/Serializable",
            "class java/lang/Exception extends java/lang/Throwable",
            "class java/lang/RuntimeException extends java/lang/Exception",
            "class java/lang/Error extends java/lang/Throwable",
            "final java/lang/Class",
            "final java/lang/System field out Ljava/io/PrintStream;",
            "class java/lang/Thread implements java/lang/Runnable",
            "annotation java/lang/Deprecated implements java/lang/annotation/Annotation",
            "annotation java/lang/Override implements java/lang/annotation/Annotation",
            "annotation java/lang/FunctionalInterface implements java/lang/annotation/Annotation",
            "annotation java/lang/SuppressWarnings implements java/lang/annotation/Annotation method value ()[Ljava/lang/String;",
            "interface java/lang/annotation/Annotation",
            "annotation java/lang/annotation/Retention implements java/lang/annotation/Annotation",
            "interface java/io/Serializable",
            "class java/io/IOException extends java/lang/Exception",
            "interface java/util/Collection implements java/lang/Iterable",
            "interface java/util/List implements java/util/Collection signature <E:Ljava/lang/Object;>Ljava/lang/Object;Ljava/util/Collection<TE;>;",
            "interface java/util/Set implements java/util/Collection",
            "interface java/util/Map member Entry java/util/Map$Entry",
            "interface java/util/Map$Entry outer java/util/Map Entry",
            "class java/util/ArrayList implements java/util/List",
            "class java/util/HashMap implements java/util/Map",
            "final java/util/Optional",
            "interface java/util/function/Supplier",
            "interface java/util/function/Function",
            "annotation javax/annotation/processing/Generated implements java/lang/annotation/Annotation",
            "final jdk/internal/misc/Unsafe",
        ],
    ),
    (
        "java.desktop",
        true,
        &["java.base"],
        &["java/awt"],
        &[
            "class java/awt/List",
            "class java/awt/Component",
            "class sun/awt/AppContext",
        ],
    ),
    (
        "java.xml",
        true,
        &["java.base"],
        &["javax/xml"],
        &["final javax/xml/XMLConstants"],
    ),
    (
        "java.logging",
        true,
        &["java.base"],
        &["java/util/logging"],
        &["class java/util/logging/Logger"],
    ),
    (
        "java.sql",
        true,
        &["java.base", "+java.xml", "+java.logging"],
        &["java/sql"],
        &["interface java/sql/Connection implements java/lang/AutoCloseable"],
    ),
    (
        "jdk.incubator.vector",
        false,
        &["java.base"],
        &["jdk/incubator/vector"],
        &["abstract jdk/incubator/vector/VectorSpecies"],
    ),
];

impl StubPlatform {
    fn new() -> StubPlatform {
        let mut platform = StubPlatform {
            classes: HashMap::new(),
            packages: HashMap::new(),
            modules: Vec::new(),
            module_of: HashMap::new(),
            visible: HashSet::new(),
        };
        for &(module, default, requires, exports, classes) in STUB_MODULES {
            let mut packages = Vec::new();
            for spec in classes {
                let (name, bytes) = class_file(spec);
                let (package, simple) = name.rsplit_once('/').unwrap_or(("", &name));
                platform
                    .packages
                    .entry(package.to_owned())
                    .or_default()
                    .push(simple.into());
                if !packages.contains(&package.to_owned()) {
                    packages.push(package.to_owned());
                }
                platform.module_of.insert(name.clone(), module.to_owned());
                platform.classes.insert(name, Arc::from(bytes));
            }
            let exports: Vec<Exports> = exports
                .iter()
                .map(|e| {
                    let (package, to) = e.split_once('>').unwrap_or((e, ""));
                    Exports {
                        package: package.to_owned(),
                        flags: 0,
                        to: to
                            .split(',')
                            .filter(|t| !t.is_empty())
                            .map(Into::into)
                            .collect(),
                    }
                })
                .collect();
            if default {
                for export in exports.iter().filter(|e| e.to.is_empty()) {
                    platform.visible.insert(export.package.clone());
                }
            }
            platform.modules.push(ModuleDescriptor {
                name: module.to_owned(),
                flags: 0,
                version: None,
                requires: requires
                    .iter()
                    .map(|r| Requires {
                        module: r.trim_start_matches('+').to_owned(),
                        flags: if r.starts_with('+') {
                            AccessFlags::TRANSITIVE
                        } else {
                            0
                        },
                        version: None,
                    })
                    .collect(),
                exports,
                opens: Vec::new(),
                uses: Vec::new(),
                provides: Vec::new(),
                packages: Some(packages),
                main_class: None,
                resolution: (!default).then_some(9),
                problems: Vec::new(),
            });
        }
        platform
    }
}

impl Platform for StubPlatform {
    fn find(&self, name: &ClassName) -> ferrion_classfile::Result<Option<ClassBytes>> {
        Ok(self.classes.get(name.as_str()).map(|bytes| ClassBytes {
            bytes: bytes.clone(),
            origin: Origin::File(PathBuf::from("stub").into()),
        }))
    }

    fn package(&self, name: &PackageName) -> ferrion_classfile::Result<Option<PackageListing>> {
        Ok(self
            .packages
            .get(name.as_str())
            .map(|names| PackageListing::new(names.clone())))
    }

    fn modules(&self) -> &[ModuleDescriptor] {
        &self.modules
    }

    fn is_visible(&self, package: &PackageName) -> bool {
        self.visible.contains(package.as_str())
    }

    fn module_of(&self, name: &ClassName) -> Option<String> {
        self.module_of.get(name.as_str()).cloned()
    }
}

/// The stand-in platform, made once per process.
pub fn stub_platform() -> Arc<dyn Platform> {
    static PLATFORM: LazyLock<Arc<StubPlatform>> = LazyLock::new(|| Arc::new(StubPlatform::new()));
    PLATFORM.clone()
}

/// A resolution case: source sets, jars, and files in one text.
///
/// ```text
/// //- jar lib.jar
/// //- class class p/J
/// //- set main 21 lib.jar
/// //- set test 21 main
/// //- file main p/A.java
/// package p; class A extends J {}
/// ```
///
/// A `set` line gives the set's name, release, and classpath in order (other sets, and jars by
/// name); every set has the stand-in platform unless the line ends with `no-platform`. A `class`
/// line adds a class to the last jar (see [`class_file`]). A `file` line starts a file of a
/// set, whose text is the lines up to the next `//-` line.
pub fn case_input(text: &str) -> ProjectInput {
    let mut jars: Vec<(String, Vec<String>)> = Vec::new();
    let mut sets: Vec<(String, u32, Vec<String>, bool)> = Vec::new();
    let mut files: Vec<(String, String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(directive) = line.strip_prefix("//- ") {
            let mut words = directive.split_whitespace();
            match words.next() {
                Some("jar") => jars.push((words.next().unwrap().to_owned(), Vec::new())),
                Some("class") => {
                    let spec = directive["class ".len()..].to_owned();
                    jars.last_mut().expect("a jar line first").1.push(spec);
                }
                Some("set") => {
                    let name = words.next().unwrap().to_owned();
                    let release = words.next().unwrap().parse().unwrap();
                    let mut classpath: Vec<String> = words.map(str::to_owned).collect();
                    let platform = classpath.last().is_none_or(|w| w != "no-platform");
                    if !platform {
                        classpath.pop();
                    }
                    sets.push((name, release, classpath, platform));
                }
                Some("file") => {
                    let set = words.next().unwrap().to_owned();
                    let path = words.next().unwrap().to_owned();
                    files.push((set, path, String::new()));
                }
                other => panic!("unknown directive {other:?}"),
            }
        } else if let Some((_, _, text)) = files.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    let set_index: HashMap<&str, usize> = sets
        .iter()
        .enumerate()
        .map(|(i, s)| (s.0.as_str(), i))
        .collect();
    let mut input = ProjectInput::default();
    for (name, release, classpath, platform) in &sets {
        let mut set = SourceSetInput::new(name, *release);
        if *platform {
            set.platform = Some(stub_platform());
        }
        for entry in classpath {
            if let Some(&index) = set_index.get(entry.as_str()) {
                set.classpath
                    .push(ClassPathInput::SourceSet(SourceSetId::from_index(index)));
            } else {
                let (_, specs) = jars
                    .iter()
                    .find(|(jar, _)| jar == entry)
                    .unwrap_or_else(|| panic!("no set or jar named {entry}"));
                let specs: Vec<&str> = specs.iter().map(String::as_str).collect();
                set.classpath.push(ClassPathInput::Classes(ClassesInput {
                    entry: jar(&specs, *release),
                    path: PathBuf::from(entry),
                    module: None,
                }));
            }
        }
        set.files = files
            .iter()
            .filter(|(s, _, _)| s == name)
            .map(|(_, path, text)| FileInput::new(path, text.as_str()))
            .collect();
        input.source_sets.push(set);
    }
    input
}

/// What the engine resolves in every file, as text: for each file, each name in its declarations
/// with its text and symbol key, then each of its types' entries. For snapshot tests and the
/// differential check.
pub fn dump(engine: &Engine) -> String {
    dump_snapshot(&engine.snapshot())
}

/// [`dump`] at a snapshot.
pub fn dump_snapshot(snapshot: &Snapshot) -> String {
    let mut out = String::new();
    for set in snapshot.source_sets().unwrap() {
        let info = snapshot.source_set(set).unwrap();
        for &file in snapshot.files(set).unwrap().iter() {
            let path = snapshot.file(file).unwrap().path;
            writeln!(out, "== {} {}", info.name, path.display()).unwrap();
            out.push_str(&dump_file(snapshot, file));
        }
    }
    out
}

/// One file's part of [`dump`]: its names, in declarations and bodies, then its types, then the
/// local and anonymous classes its bodies declare.
pub fn dump_file(snapshot: &Snapshot, file: crate::FileId) -> String {
    let mut out = String::new();
    let set = snapshot.file(file).unwrap().set;
    let text = snapshot.text(file).unwrap();
    let names = snapshot.file_names(file).unwrap();
    for name in names.names.iter() {
        let written = &text[name.start as usize..name.end as usize];
        let key = snapshot.symbol_key(&name.symbol).unwrap();
        writeln!(out, "{written} -> {key}").unwrap();
    }
    let summary = snapshot.summary(file).unwrap();
    for index in 0..summary.types.len() {
        let key = snapshot.type_key(file, index).unwrap();
        out.push_str(&describe_type(snapshot, set, key));
    }
    let locals = snapshot.body_names(file).unwrap().local_types.len();
    for index in 0..locals as u32 {
        let key = TypeKey::Local { file, index };
        out.push_str(&describe_type(snapshot, set, key));
    }
    out
}

/// A type's entry, supertypes, and member types, a few lines of text.
pub fn describe_type(snapshot: &Snapshot, view: SourceSetId, key: TypeKey) -> String {
    let names = snapshot.names().clone();
    let mut out = String::new();
    let type_text = |key: TypeKey| -> String {
        snapshot
            .symbol_key(&Symbol::Type(key))
            .map(|k| k.0)
            .unwrap_or_default()
    };
    let error_text = |e: &crate::symbols::ErrorSymbol| {
        snapshot
            .symbol_key(&Symbol::Error(e.clone()))
            .map(|k| k.0)
            .unwrap_or_default()
    };
    writeln!(out, "# {}", type_text(key)).unwrap();
    if let Some(supertypes) = snapshot.supertypes(key).unwrap() {
        let Supertypes {
            superclass,
            interfaces,
            cycle,
        } = &*supertypes;
        let one = |r: &Result<TypeKey, crate::symbols::ErrorSymbol>| match r {
            Ok(k) => type_text(*k),
            Err(e) => error_text(e),
        };
        if let Some(superclass) = superclass {
            writeln!(out, "  superclass {}", one(superclass)).unwrap();
        }
        for interface in interfaces.iter() {
            writeln!(out, "  interface {}", one(interface)).unwrap();
        }
        if !cycle.is_empty() {
            let mut members: Vec<String> = cycle.iter().map(|&k| type_text(k)).collect();
            members.sort();
            writeln!(out, "  cyclic with {}", members.join(", ")).unwrap();
        }
    }
    let members = snapshot.member_types(view, key).unwrap();
    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    for (name, member) in members.iter() {
        let text = match member {
            MemberType::One(k) => type_text(*k),
            MemberType::Ambiguous(keys) => format!("ambiguous among {}", keys.len()),
        };
        listed.insert(names.text(name).to_string(), text);
    }
    for (name, text) in listed {
        writeln!(out, "  member {name}: {text}").unwrap();
    }
    if let Some(entry) = snapshot.type_entry(key).unwrap() {
        let mut flags = Vec::new();
        if entry.incomplete {
            flags.push("incomplete");
        }
        if entry.lombok {
            flags.push("lombok");
        }
        if entry.generated {
            flags.push("generated");
        }
        if !flags.is_empty() {
            writeln!(out, "  {}", flags.join(" ")).unwrap();
        }
        for field in entry.fields.iter() {
            let ty = snapshot.erased_descriptor(&field.ty).unwrap();
            writeln!(out, "  field {} {ty}", names.text(field.name)).unwrap();
        }
        for method in entry.methods.iter() {
            let mut descriptor = String::from("(");
            for parameter in method.parameters.iter() {
                descriptor.push_str(&snapshot.erased_descriptor(parameter).unwrap());
            }
            descriptor.push(')');
            match &method.result {
                Some(result) => descriptor.push_str(&snapshot.erased_descriptor(result).unwrap()),
                None => descriptor.push('V'),
            }
            let implicit = if method.implicit { " implicit" } else { "" };
            writeln!(
                out,
                "  method {}{descriptor}{implicit}",
                names.text(method.name)
            )
            .unwrap();
        }
        for error in entry.errors.iter() {
            writeln!(out, "  {}", error_text(error)).unwrap();
        }
    }
    out
}
