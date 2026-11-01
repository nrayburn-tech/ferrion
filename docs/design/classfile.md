# Class file reading

B1 in the [roadmap](../ROADMAP.md) reads compiled classes: class files from jars and directories,
multi-release jars for the target release, and the JDK's platform API for that release. This
document is the overview: what the reader serves, where the platform API comes from, the shape of
`ferrion_classfile`, dependencies, performance, verification, and the decisions made in review.
The inputs side of B1 (`ferrion_project`: flags, the project model, validation messages) is done
with B2, and designed in [project inputs](project-inputs.md).

| Topic | Document | In short |
| --- | --- | --- |
| Class files | [Class file format](classfile-format.md) | A lazy view over the bytes: the constant pool and member tables are indexed once, and attributes, signatures, and annotations are decoded when asked for. Method bodies are skipped. |
| Jars, directories, JDK 8 | [Containers](classfile-containers.md) | A hand-written zip reader (central directory, zip64, stored and deflate) with positional reads. Multi-release jars resolved per release. JDK 8 reads `rt.jar` through its `lib/ct.sym`, as javac 8 does. |
| Module images | [jimage](classfile-jimage.md) | The `lib/modules` format, learned only by observing real images, with the observation log. Ferrion reads the header, location table, strings, and resources, including both compression schemes, and builds its own index. |
| `ct.sym` | [ct.sym](classfile-ctsym.md) | Its layout in each JDK, and how javac uses it with `--release`, learned by observation. Since JDK 25 it holds the JDK's own release too. |

Everything about the jimage format, `ct.sym`, javac's `--release` behavior, and the JDK's tools in
these documents was learned by running them on real JDKs and on images built with `jlink`, never
from OpenJDK's source or any other implementation's. Each observation is recorded with the
commands that show it. See [What was consulted](#what-was-consulted).

## What it serves

- **Name resolution (B2):** a lookup by binary name (`java/util/Map$Entry`) and by package, lazy
  so that a run that touches a hundred JDK classes reads a hundred, and a cheap listing of a
  package's class names without reading any class. B2 also needs to know whether a package exists
  at all, to tell a package from a type in a qualified name.
- **Type attribution (B3):** supertypes, members with their descriptors and generic signatures,
  inner class relationships, constant values, annotations (including invisible ones and type
  annotations), records, sealed hierarchies, and module descriptors. Signatures are parsed only
  when a member is used.
- **The platform API by release:** the classes javac sees with `--release <N>`, from the JDK the
  user names. Only packages exported to everyone are visible, as with javac (SCOPE.md).
- **The language server:** one long-lived process. Archives and their indexes are shared across
  source sets and projects that use the same jar or JDK, memory stays proportional to what is
  used, and a jar rebuilt on disk is noticed rather than read half-old, half-new.
- **Multiple source sets:** each source set has its own classpath, module path, Java release, and
  JDK. Two source sets with the same JDK and release share one platform API.
- **Robustness:** malformed jars, images, and class files produce errors, never panics, and an
  error in one class doesn't stop the rest. B5's incomplete-analysis reporting is built on these
  errors.

## The platform API for JDK 9 and later

For JDK 8 the platform API is the boot class path either way (see
[containers](classfile-containers.md#jdk-8)). For JDK 9 and later there are four ways to get it.
This section compares them and recommends a combination.

### What javac does

The comparison rests on what javac reads with `--release`, observed with `javac -verbose`, which
prints the file each class is loaded from (see [ct.sym](classfile-ctsym.md#how-javac-uses-it)):

| JDK | `--release` older than the JDK | `--release` equal to the JDK |
| --- | --- | --- |
| Temurin 11.0.32 | `ct.sym` (6 to 10) | the image (`modules/java.base/...`) |
| Corretto 17.0.17 | `ct.sym` (7 to 16) | the image |
| Corretto 21.0.11 | `ct.sym` (8 to 20) | the image |
| Corretto 25.0.1, JetBrains Runtime 25.0.4.1 | `ct.sym` (8 to 24) | `ct.sym` |
| Corretto 26.0.1, Corretto 27.0.1, Temurin 27.0.1 | `ct.sym` (8 to N-1) | `ct.sym` |

When javac reads its own release from the image, `ct.sym` still decides which modules exist: its
directory for that release holds only a `system-modules` file listing them, and javac hides the
rest. For example, `jdk.unsupported.desktop` (which exports `jdk.swing.interop`) is in the image
but not in that list, and javac 21 with `--release 21` reports `package jdk.swing.interop does not
exist`, while javac 27 with `-source 27 -target 27` compiles the same file. Between JDK 21 and 25
(22 to 24 weren't observed), `ct.sym` started holding the current release in full.

### The options

**1. Reading `lib/modules` directly (jimage).** The [jimage document](classfile-jimage.md)
describes the format as observed, and a throwaway reader built from it matched the jrt file system
on every resource of eight JDK images, and `jimage extract` (or, for big-endian images, the
little-endian image from the same JDK) on every resource of the `jlink` images checked, covering
each compression scheme and byte order.

- **Covers:** the JDK's own release only, every JDK 9 and later, including compressed images.
  Older releases still need `ct.sym`.
- **Risk:** no specification; everything is learned by observation. The format already changed
  once in the observed range: JDK 27 images are version 1.1, and the flags in `/packages` entries
  changed meaning (1 and 0 became 2 and 6). Images also have two compression schemes and both byte
  orders. Ferrion's reader uses only the parts that stayed the same across 11 to 27, and doesn't
  need the hash function or the `/packages` flags.
- **Cost:** opening reads the 1.6 MB index (0.3 ms) and builds Ferrion's own package index
  (7.5 ms, 977 packages). Reading every class of the JDK 27 image with positional reads takes
  28 to 35 ms; inflating every class of the compressed JetBrains Runtime image takes 590 ms, which
  lazy reading spreads over the classes actually used. Peak memory 8 to 9 MB.
- **Needs:** only the JDK's files.

**2. `ct.sym` only.** A zip of class files (`.sig`), one directory per set of releases (see
[ct.sym](classfile-ctsym.md)).

- **Covers:** every `--release` the JDK supports below its own version, and its own version too
  from JDK 25. For JDK 11 to 21 (and up to some version between 22 and 24), the own release isn't
  there, so `ct.sym` alone can't serve the most common target, the JDK's own release.
- **Risk:** the container is a zip and the entries are class files, both specified, so only the
  directory layout is learned by observation. The layout changed between JDK 11 (no module
  directories, module descriptors under `<releases>-modules/`) and 17 (a module directory under
  each release directory), and the `system-modules` file disappeared by 25. The release letters
  (`8`, `9`, `A` for 10, up to `R` for 27) run out at `Z`, release 35.
- **Fidelity:** it is exactly javac's `--release` view: public and protected members, constant
  values, signatures, and annotations, without method bodies or private members, plus the classes
  in unexported packages that API classes need as supertypes.
- **Cost:** reading the central directory of the JDK 27 `ct.sym` (21,495 entries) takes 1.1 ms;
  inflating all 5,696 entries of release 27 takes 40 ms. Peak memory 7 MB.
- **Needs:** only the JDK's files.

**3. `jmods/*.jmod`.** Four bytes (`JM`, 1, 0) followed by a zip with `classes/`, `lib/`, `conf/`,
and `legal/` entries; JEP 261 describes JMOD files as a ZIP-based format.

- **Covers:** the JDK's own release, where the files exist. Temurin 11, Corretto 17, 21, 25, 26,
  and 27, and the JetBrains Runtime 25 ship them (65 to 70 files, 75 to 90 MB). Temurin 27 doesn't:
  it is built for JEP 493 (linking from the run-time image) and has no `jmods` directory. JDK 8
  has none. JREs and `jlink` images never have them.
- **Risk:** low: a zip after a four-byte header. But the files are optional, so they can only be a
  fallback.
- **Cost:** the central directories of all 65 JDK 27 jmods take 3 to 25 ms; inflating all 27,645
  classes takes 0.55 s, since every entry is deflated.
- **Needs:** a JDK that ships them.

**4. A one-time export through a JVM.** Run the user's JDK once with a small Ferrion helper that
walks the jrt file system (`FileSystems.newFileSystem(URI.create("jrt:/"), Map.of("java.home",
home))`, a public API) and writes the classes, or a Ferrion summary of them, to a cache.

- **Covers:** the JDK's own release, any JDK 9 and later, whatever its image format. Older releases
  still need `ct.sym`: jrt only shows the image.
- **Risk:** no format risk for the image. The risk moves to process management: the JDK has to run
  on the machine Ferrion runs on, which fails for a JDK for another platform, a JDK in a sandbox
  that forbids starting processes, or a broken JDK. The cache needs a location, a key, and
  invalidation: the JDK's path, the contents of its `release` file, and the size and modification
  time of `lib/modules`.
- **Cost:** `java -version` takes 0.03 s on Corretto 27 here; walking and digesting every file
  through jrt takes 0.36 s with a 370 MB peak (both runs warm). Writing a cache adds to that once
  per JDK; later runs read the cache at the speed of option 1.
- **Per environment:** the CLI and CI would start a JVM on the first run with each JDK, so a cold
  CI runner pays it on every build unless the cache is kept. The language server pays it once per
  JDK. The Maven plugin and the IntelliJ extension already run in a JVM, but the Ferrion binary
  does the analysis, so they'd only help by running the export themselves. With no JVM that can
  start, there is no fallback but another option.

### Recommendation

Use `ct.sym` and the image together, with no jmod or JVM fallback in B1:

1. **`ct.sym` for every release it holds in full,** including the JDK's own release from JDK 25
   on. This is exactly what javac reads, so name resolution matches javac by construction, and it
   is the cheapest option.
2. **The image for the JDK's own release when `ct.sym` doesn't hold it** (observed for JDK 11, 17,
   and 21), filtered to the modules in `ct.sym`'s `system-modules` list and to their unqualified
   exports, which is what javac does.
3. **No version tables:** which source serves a release is decided from what `ct.sym` contains,
   not from the JDK's version number, so the unobserved JDK 22 to 24 work either way, and so do 12
   to 16, whose `ct.sym` layout is told apart by its entries.

The reasons:

- The jimage risk is bounded. Only JDKs whose `ct.sym` lacks their own release need the image,
  and those are JDKs already released (JDK 24 and older, observed up to 21). Their formats are
  version 1.0, observed and verified on 11, 17, 21, 25, and 26. JDK 27's version 1.1 change doesn't
  matter for the platform API, because JDK 27 serves release 27 from `ct.sym`. If a future JDK
  went back to reading its own release from the image, the image reader still handles version 1.x
  and reports a clear error for a major version it doesn't know.
- jmods would be a second reader for the same classes that only some JDKs ship, so they add test
  burden without covering a case the image doesn't.
- The JVM export trades a bounded format risk for an unbounded environment one (a JDK that can't
  run here), and adds a cache to invalidate. It stays a possible fallback if a future JDK's image
  can't be read and `ct.sym` doesn't cover its release; the costs above are the starting point.

### What the exit criterion means

B1's exit criterion says every class in the JDK 8, 11, 17, 21, and latest GA platform APIs, from
more than one vendor and including a compressed image, is read without loss compared with `javap`.
Under this recommendation:

- **"Every class in the platform API"** is every class Ferrion serves for each release the JDK
  supports: the `.sig` entries of `ct.sym` for that release, plus the image's classes in the
  visible modules for an own release served from the image, plus JDK 8's boot class path. Each is
  compared with `javap` on the same bytes (see [verification](#verification)).
- **The containers are checked separately:** the bytes Ferrion reads from an image must equal what
  the jrt file system returns, and from `ct.sym`, jars, and jmods what `java.util.zip` returns.
- **"Including a compressed image":** the JetBrains Runtime 25 has a compressed image, but under
  this recommendation its platform API comes from `ct.sym`, so the image path would never read it.
  Two things cover it: the image reader is checked against jrt on every image (JetBrains Runtime 25
  included) whether or not the platform API uses it, and the JetBrains Runtime 21
  (`jetbrains-21.0.11-b1163.125` from mise), whose own release is served from its compressed image,
  is in the matrix. The criterion's wording stays (decision 2 in
  [review](#decisions-made-in-review)).

## The crate

`ferrion_classfile` is internal (`publish = false`), so its API serves the workspace. Its shape, as
implemented:

```rust
// Names, in the internal form of JVMS 4.2.1 ("java/util/Map$Entry", "java/util").
pub struct ClassName(Arc<str>);
pub struct PackageName(Arc<str>);
pub struct ModuleName(Arc<str>);

/// One classpath entry: a directory, or a jar resolved for a release. Cheap to clone.
pub struct ClassPathEntry;
impl ClassPathEntry {
    pub fn directory(path: &Path) -> ClassPathEntry;
    pub fn jar(jar: Arc<Jar>) -> ClassPathEntry; // Jar: a ZipArchive resolved for a release
}

/// An ordered list of entries. The first entry that has a class wins, as with javac.
pub struct ClassPath;
impl ClassPath {
    pub fn new(entries: Vec<ClassPathEntry>) -> ClassPath;
    pub fn find(&self, name: &ClassName) -> Result<Option<ClassBytes>>;
    pub fn package(&self, name: &PackageName) -> Result<Option<PackageListing>>;
}

/// The bytes of one class, with where they came from (for diagnostics).
pub struct ClassBytes { pub bytes: Arc<[u8]>, pub origin: Origin }

/// The class names in a package, without reading any class: simple names, nested classes'
/// "Outer$Inner" included, sorted.
pub struct PackageListing;

/// A JDK on disk: its layout (JDK 8 or 9+), version from the `release` file, and archives.
pub struct Jdk;
impl Jdk {
    /// Shared through the cache; JDK 9 and 10, and JDK 7 and older, are a usage error.
    pub fn open(home: &Path, cache: &ArchiveCache) -> Result<Arc<Jdk>>;
    pub fn feature_version(&self) -> u32;
    /// The releases it serves, decided from what `ct.sym` holds.
    pub fn releases(&self) -> Result<Vec<u32>>;
    /// The platform API for `release`, or a usage error saying why this JDK can't serve it.
    pub fn platform(&self, release: u32) -> Result<Arc<PlatformApi>>;
}

/// The classes javac sees with `--release <release>` on this JDK.
pub struct PlatformApi;
impl PlatformApi {
    pub fn release(&self) -> u32;
    pub fn source(&self) -> PlatformSource; // CtSym, Image, or BootClassPath
    /// Any class the platform has, exported or not, for supertypes and signatures.
    pub fn find(&self, name: &ClassName) -> Result<Option<ClassBytes>>;
    /// Whether code in the unnamed module can name classes in `package`.
    pub fn is_visible(&self, package: &PackageName) -> bool;
    pub fn package(&self, name: &PackageName) -> Result<Option<PackageListing>>;
    pub fn modules(&self) -> &[ModuleDescriptor]; // for B2's readability, from module-info
}

/// Archives, jars at each release, and JDKs, shared by everything in the process.
pub struct ArchiveCache;

pub mod class;      // ClassFile: the lazy view (classfile-format.md)
pub mod descriptor; // field and method descriptors (JVMS 4.3)
pub mod signature;  // generic signatures (JVMS 4.7.9.1)
pub mod zip;        // ZipArchive (classfile-containers.md)
pub mod jar;        // Manifest, Jar: multi-release resolution
pub mod jimage;     // Image (classfile-jimage.md)
pub mod ctsym;      // CtSym (classfile-ctsym.md)

#[doc(hidden)]
pub mod invariants; // checks for tests and fuzz targets, like the lexer's
#[doc(hidden)]
pub mod testing;    // writers of class files, zips, and images for tests and fuzz seeds
```

### Class path entries and lookup

Every entry answers the same two questions: the bytes of a class by internal name, and the class
names in a package. Each kind answers them from an index built when the entry is first used:

- **Directory:** a path join, then a check that the file name matches exactly, since macOS and
  Windows file systems are case-insensitive and javac must not find `foo/Bar.class` for `foo/BAR`.
  Package listings come from `read_dir`. Directories (often build output) aren't cached beyond
  one query; the language server tells the reader when files change.
- **Jar:** the central directory, indexed by package. A multi-release jar's index resolves each
  name for the entry's release once, when the entry is created (see
  [containers](classfile-containers.md#multi-release-jars)).
- **Platform sources** (`ct.sym` for a release, the image's visible modules, JDK 8's boot class
  path) sit behind `PlatformApi`, which has the same two lookups plus visibility.

javac takes the first classpath entry that has a class (observed: with `-cp c.jar:b.jar`, where both
jars have `lib.OnlyInB` but only `b.jar`'s has the method used, javac reports `cannot find symbol`;
swapping the order compiles). javac also follows the `Class-Path` attribute of a classpath jar's
manifest (observed: `-cp a.jar` where `a.jar`'s manifest says `Class-Path: b.jar` finds classes in
`b.jar`). Expanding `Class-Path` belongs to `ferrion_project`, which builds the `ClassPath`; the
jar reader exposes the attribute.

Source classes shadowing classpath classes is B2's job: B1 only answers for compiled classes.

### Module visibility

Each module's descriptor comes from its `module-info` (the `Module` attribute, JVMS 4.7.25, and
`ModulePackages`, 4.7.26, where present). For code in the unnamed module, which is all code until
the module path is handled strictly, the visible packages are the unqualified exports of the
modules javac resolves by default. Observed with `--release`:

- every module in the release's module set that exports a package is visible: `java.smartcardio`
  isn't required by `java.se` but is visible with `--release 27`;
- except for releases 9 and 10, where of the `java.*` modules only `java.se` is a root, as JEP 261
  first described it: javac 27 with `--release 9` and `--release 10` doesn't see `java.xml.bind`,
  `java.corba`, `javax.smartcardio`, and the other `java.*` modules `java.se` doesn't require
  (found by the visibility check on every JDK 11 and later). This is a rule of those releases,
  not a choice of source, so it depends on the release number;
- the modules a root requires are resolved too, and their unqualified exports are visible;
- a module whose descriptor has a `ModuleResolution` attribute with value 0x0009 isn't visible
  without `--add-modules` (`jdk.incubator.vector` with `--release 17` and `--release 21`: `package
  jdk.incubator.vector is not visible`; with `--add-modules` it compiles with an incubating
  warning). In JDK 27's `ct.sym`, release 27's descriptor of that module has no such attribute,
  and `--release 27` sees it without `--add-modules`. `ModuleResolution` isn't in the JVMS; Ferrion
  treats the low bit of its value as "not resolved by default", which is what these observations
  show, and reads the attribute's raw value for B2;
- unexported packages are never visible, but their classes can still be read: `ct.sym` has classes
  of unexported packages that API classes extend (for example `jdk/internal/vm/vector`), and B2
  needs them for supertypes. So `find` returns any class the platform has, and `is_visible` says
  whether a name can be resolved to it.

Named modules in sources, qualified exports to them, and `--add-modules` are B2's: B1 provides the
descriptors and the default view.

### The class model

`class::ClassFile::parse(bytes)` checks the magic, the version, and the constant pool's structure,
and records the offset of every constant pool entry and member table: the measured cost is
23 ms for all 27,661 classes of the JDK 27 image, under 1 µs per class. Everything else is decoded
on demand from the bytes, which the `ClassFile` holds as an `Arc<[u8]>`: names, flags, members,
attributes, and signatures. Method bodies (`Code`) are skipped entirely. The
[format document](classfile-format.md) lists every part read, by JVMS section.

The model doesn't build symbols: B2 decides what to keep, as summaries or symbols, and what to drop.
B1 holds no parsed classes beyond the `ClassFile` a caller has.

### Errors

Every read returns `Result`. An error records the origin (archive path and entry, or file path),
the byte offset, and a kind:

- `Io`: the file can't be read;
- `Malformed`: a structure is truncated, an index points outside its table or at the wrong kind of
  constant, a length overruns its attribute, or text isn't valid modified UTF-8;
- `Unsupported`: well-formed but not handled, such as a zip compression method other than stored
  and deflate, an encrypted entry, a class file version newer than Ferrion knows, or an image
  major version other than 1;
- `TooLarge`: a declared size above a limit (see [limits](classfile-format.md#limits));
- `Changed`: the file's size, modification time, or identity changed since it was opened;
- `Usage`: the request can't be served, such as a release the JDK doesn't have, or a JDK 9 or 10
  (decision 5 in [review](#decisions-made-in-review)). The message says what to do instead:
  `... is JDK 9, which isn't supported; use JDK 11 or later with --release 9`.

A malformed class is reported for that class only. Readers never index without a bounds check and
never trust a length: all reads go through a cursor that returns `Malformed` at the end of its
input. Fuzz targets check that nothing panics.

### Threads and sharing

Everything is immutable after it is built, so `ClassPath`, `PlatformApi`, and the archives are
`Send` and `Sync`, shared through `Arc`. Reads use positional I/O (`read_exact_at` on Unix,
`seek_read` on Windows), which doesn't share a file cursor, so concurrent reads of one archive need
no lock. An archive's index is built when it is opened, and archives are opened on first use
through the cache.

`ArchiveCache` is process-wide: archives are keyed by canonical path, size, modification time, and
file identity, and held weakly, so two source sets or two projects in the language server that use
the same jar or JDK share one archive and one index, and an archive nobody uses is dropped. A jar
rebuilt on disk gets a new key, so new lookups see the new file; an old archive that notices the
change on a read reports `Changed`, and the caller rebuilds its `ClassPath`. Open file handles are
capped at 128 (an LRU of handles reopened on demand, checking identity), since a large build can
have hundreds of jars per source set.

## Dependencies

- **Deflate: `miniz_oxide` 0.9.1,** license MIT OR Zlib OR Apache-2.0, all allowed by `deny.toml`.
  It is written in Rust with `#![forbid(unsafe_code)]`, and depends only on `adler2` (0BSD OR MIT
  OR Apache-2.0, also `forbid(unsafe_code)`). Both are already in the local registry cache. It
  inflated every class of the JetBrains Runtime image (133 MB) in 0.59 s, about 225 MB/s, which is
  enough for lazy reading. Faster inflaters (`zlib-rs`, `libdeflater`) use `unsafe` code or C, and
  aren't needed.
- **Zip: hand-written,** about 400 lines. The `zip` crate reads through `Read + Seek`, which needs a
  lock per archive to share it across threads, and brings features Ferrion doesn't use (writing,
  encryption, other compression methods). Ferrion needs the central directory, zip64, a prefix
  before the archive, and stored and deflate entries (see
  [containers](classfile-containers.md#zip)), with positional reads and its own error type. The
  same reader serves jars, `ct.sym`, and JDK 8's jars.
- **No memory mapping.** `memmap2` (MIT OR Apache-2.0) maps files only through `unsafe` functions,
  which the workspace denies, and mapping isn't needed (see below).
- **No CRC check at run time.** javac's behavior on a bad CRC wasn't observed, the class parser
  catches most corruption, and checking adds a dependency (`crc32fast`) and time. Tests and fuzz
  invariants check CRCs with a small table-driven implementation in test code.
- **jimage and `ct.sym`:** no crate; both are Ferrion's own readers.

## Memory and performance

Measured with a throwaway prototype outside the repository (Rust, release build, Apple M1 Pro,
macOS 26.6, warm file cache, second of two runs):

| Operation | Input | Time | Peak memory |
| --- | --- | --- | --- |
| Open the image index | JDK 27 `lib/modules`, 146 MB, 30,682 entries | 0.3 ms | |
| Build Ferrion's package index | same | 7.5 ms | |
| Read every class, positional reads | same, 27,661 classes, 135 MB | 28 to 35 ms | 8 MB |
| Read the whole file, then slice | same | 19 ms to read | 154 MB |
| Read every class, inflating | JetBrains Runtime 25 image, 27,535 classes | 590 ms | 9 MB |
| Parse every class's structure | JDK 27 image's classes | 23 ms | |
| Read `ct.sym`'s central directory | JDK 27 `ct.sym`, 21,495 entries | 1.1 ms | |
| Read release 27's entries | same, 5,696 entries, 5 MB | 40 ms | 7 MB |
| Read every class from jmods | JDK 27, 65 jmods, 27,645 classes | 0.55 s | 7 MB |
| Walk every file through jrt | JDK 27, JVM | 0.36 s | 370 MB |

The decisions:

- **Positional reads, not mapping or whole-file reads.** Mapping isn't available without `unsafe`,
  and reading a whole image costs its size in memory (154 MB per JDK) for no gain: positional reads
  of every class took about as long. Positional reads also mean a file truncated or replaced while
  open gives an I/O error or `Changed`, not a crash, which a mapping wouldn't.
- **Indexes are built once and shared.** An image index costs under 10 ms and a few megabytes; a
  `ct.sym` index about 1 ms. Indexes store names as offsets into a shared string buffer rather
  than as separate strings.
- **Class bytes aren't cached by B1.** Reading a class costs a positional read (and an inflate for
  compressed sources), a few microseconds; B2 keeps what it needs as symbols. An in-memory cache of
  inflated bytes can be added if profiling shows repeated reads.

Targets, checked by the benchmarks in `tools/bench` (docs/design/benchmarks.md) on the JDK 27
image and `ct.sym`: opening the platform API for a release in under 15 ms, finding and parsing a
class in under 10 µs on average (uncompressed) and under 50 µs (compressed), and under 20 MB
resident for a platform API and its indexes. The language server's memory grows with the
number of distinct JDKs and jars in use, not with the number of projects.

Measured with the implementation (release build, same machine, warm file cache, Ferrion alone,
without the oracle's JVMs). These came from a throwaway program that called the crate's API, before
`tools/bench` existed; `cargo benchmark run classfile` measures operations like them now.

| Operation | Input | Time | Peak memory |
| --- | --- | --- | --- |
| Open a JDK (image index and `ct.sym` directory) | Corretto 27 | 12 ms | |
| Open the platform API for a release from `ct.sym` | Corretto 27, releases 8 to 27 | 1.4 to 3.6 ms | |
| Open the platform API for the own release from the image | Corretto 21, JetBrains Runtime 21 | 0.4, 0.9 ms | |
| Find and parse a class, average over every class | release 27 from `ct.sym` | 9.8 µs | |
| Find and parse a class, average over every class | release 21 from Corretto 21's image | 5.8 µs | |
| Find and parse a class, average over every class | release 21 from the JetBrains Runtime 21's compressed image | 26 µs | |
| Read every image resource | Corretto 27, 28,357 resources | 36 ms | |
| Read every image resource, inflating | JetBrains Runtime 21, 28,893 resources | 686 ms | |
| Read every resource, and find, parse, and check every class of every release | each JDK of the matrix | 0.5 to 4.1 s | 10.6 to 17.1 MB |

## Verification

### What "without loss" means

The class model is compared with `javap -v -p` on the same bytes, for every part Ferrion reads:
the version, flags, names, supertypes, every field, method, and record component with its flags,
descriptor, and attributes, and every class attribute, with constant values, signatures, annotations
and their element values, parameter annotations, type annotations with their targets and paths,
inner classes, nest and permitted subclasses, module descriptors, and method parameters. What isn't
read isn't compared: `Code` and the attributes inside it, `SourceDebugExtension`, and the constant
pool listing itself (it is compared through the entries it resolves to). An attribute Ferrion
doesn't know is kept with its name and bytes, and the comparison fails if `javap` shows an
attribute that Ferrion's model has neither decoded nor kept, so nothing is lost silently.

### The javap oracle

`tools/javap-oracle` is a small Java harness in the style of `tools/javac-oracle` (Maven, Ferrion's
license, test-only, public APIs only), compiled for Java 11 so it can run on every JDK it checks
(decision 7 in [review](#decisions-made-in-review)). Its `javap` mode runs `javap -v -p` in
process through `java.util.spi.ToolProvider` on batches of 200 class files, and writes, per class,
javap's lines after a normalization that keeps everything javap prints about the parts Ferrion
reads and drops the rest:

- the header lines about the file (`Classfile`, `Last modified`, checksums, `Compiled from`),
  since `SourceFile` is compared instead;
- the constant pool listing, compared through the entries it resolves to;
- `Code` and every line inside it, and the entries of `BootstrapMethods` and
  `SourceDebugExtension`, whose headers stay;
- constant pool indexes: a reference followed by a `//` comment becomes the comment, and a raw
  annotation (`#49(#50=I#51)`) is dropped, since javap prints the readable form after it;
- blank lines and trailing spaces.

Ferrion's side, `tools/oracle-check/src/classfile/render.rs`, prints the class from Ferrion's model
in javap's format: declarations from the parsed signatures, flags, constants in Java's
`Float.toString` and `Double.toString` forms, javap's string escapes, annotations in its readable
form, type annotation targets and paths, module descriptors, and the attributes Ferrion keeps as
bytes (`ModuleTarget`, `ModuleHashes`, and unknown attributes as javap's hex dump). The comparison
is line for line, and a mismatch shows the lines around the first difference as each side printed
them. javap 27 reads every class file version, so one JDK runs it for every input.

`ct.sym` entries end in `.sig`, so the check writes each class's bytes, as Ferrion read them, to a
scratch directory as `.class` files and passes those: the oracle compares the parser, and the
container checks below compare the readers. Each distinct class (by SHA-256) is compared once per
run, and javap's output is cached by content hash.

`tools/oracle-check/classfile-cases/` holds Java sources written for the purpose, the class files
javac compiles from them, and javap's output for each, so `cargo test` compares the renderer and
the reader with javap without a JDK, for every attribute, every type annotation target outside
code, and javap's constant and string formats (`cargo oracle classfile-cases` regenerates them).

### Container checks

The harness's other modes write, per file, a SHA-256 of the bytes the JDK's own public APIs return:

- `jrt <java.home>`: every regular file under `/modules` in the jrt file system opened on the JDK
  under test (`FileSystems.newFileSystem(URI.create("jrt:/"), Map.of("java.home", home))`),
  against every resource Ferrion's image reader indexes, on every image whether or not the
  platform API reads it;
- `zip <file>...`: every name of a zip file with `java.util.zip.ZipFile.getEntry`'s bytes, for
  `ct.sym`, JDK 8's jars and `lib/ct.sym`, and the corpus jars;
- `jar <release> <file>...`: `java.util.jar.JarFile` opened with `Runtime.Version.parse("<N>")`,
  every name of its `versionedStream()` with its bytes, and whether it is multi-release, for each
  corpus jar at each release that uses it;
- `visibility <release>`: on the JDK under test, through the javac API (`javax.tools`,
  `javax.lang.model`, and `com.sun.source.util.JavacTask`) with `--release <N>`: the modules javac
  resolves, the member classes of each module's packages, and the packages
  `Elements.getPackageElement(unnamedModule, name)` finds. Release 8 has no modules to walk, so the
  check names the packages to look up.

### In oracle-check

`cargo oracle classfile` runs them all:

- `--jdk <home>` (repeatable): every release the JDK serves, with javap; the image against jrt;
  `ct.sym`, or JDK 8's jars, against `ZipFile`; and, for JDK 11 and later, every release against
  javac. `--release` limits the releases.
- corpus projects (the default without `--jdk`, or named): every jar in their lock files
  (`tools/corpus/lock/*.json`), fetched by `cargo corpus fetch`, against `ZipFile`, against
  `JarFile` at the releases of the source sets that use it, and each class with javap.

A failure is any difference the check can't explain. The explained ones are reported as notes,
each with its reason:

- **javap's errors on flags:** javap 27 reports a flag the class file's version doesn't define as
  an error line among its output, and prints the rest. `ct.sym`'s `.sig` files for releases 8 to
  16 carry `ACC_STRICT` on methods with the JDK's own class file version (61 and later, where JVMS
  4.6 no longer defines it), and JDK 11's image has four generated `Holder` classes with
  `ACC_PRIVATE`. Ferrion keeps the flags as stored; the comparison drops javap's error lines and
  prints the flags as javap does.
- **Malformed parts both sides report:** JDK 21's `ct.sym` has a `ModuleMainClass` pointing at a
  `CONSTANT_Utf8` (in `jdk.httpserver`'s descriptor for releases 18 to 20). javap prints `Error: Not
  a ClassEntry`; Ferrion's module descriptor leaves the attribute out and keeps the module, as
  javac does (before this was found, the module's exports were lost).
- **What javac's model lists:** javac's model lists the member classes of the modules it resolves.
  Ferrion also serves classes in modules not resolved by default, in packages a module doesn't
  export (supertypes of exported classes), local, anonymous, and synthetic classes and their
  members, and `module-info` and `package-info`. And in JDK 11's flat `ct.sym`, javac's model puts
  a package's classes in a module that has a subpackage of it (`java.awt`'s in
  `java.datatransfer`), so classes are compared across modules.

- **javap's indentation of unknown attributes:** javap prints an attribute it doesn't know two
  columns deeper than the others, so one that follows `Code`, `BootstrapMethods`, or
  `SourceDebugExtension` reads as part of it, and the normalization drops it with that
  attribute's lines; the renderer leaves out the same ones (AspectJ's attributes in the corpus).

The oracle's output is cached in `target/oracle`, or under `FERRION_ORACLE_CACHE`, keyed by the
harness jar and the JDK running it, one file per distinct class: about 2.3 GB for the whole
matrix and corpus.

### JDKs and vendors

The matrix is the LTS releases from 8, plus the latest release, from more than one vendor, with
compressed images:

| JDK | Why |
| --- | --- |
| Corretto 8.502 | JDK 8, boot class path through `lib/ct.sym` |
| Temurin 11.0.32 | JDK 11, the flat `ct.sym` layout, own release from the image |
| Corretto 17.0.17 | the module `ct.sym` layout, own release from the image |
| Corretto 21.0.11, JetBrains Runtime 21.0.11 | own release from the image; a compressed one |
| Corretto 25.0.1, JetBrains Runtime 25.0.4.1 | own release from `ct.sym`; a compressed image |
| Corretto 27.0.1, Temurin 27.0.1 | latest GA from two vendors, image version 1.1, Temurin without jmods |

All are available from mise, and from `actions/setup-java` (distributions `corretto`, `temurin`,
and `jetbrains`). Corretto 26 isn't in the matrix; the observations of it above stay as they were
made. JDK 22 to 24 and 12 to 16 aren't observed: the reader decides from what `ct.sym` contains,
so it handles them by that rule (decision 5). The weekly oracle workflow covers the whole matrix
and every corpus project's jars; pull-request CI checks Temurin 27's own release and one
project's jars. The `jlink` variants in the [observation log](classfile-jimage.md#observation-log)
(compression, byte order) are covered by unit tests on small images written by test code, and the
implementation was also checked on `jlink` images of a module of its own (see
[jimage](classfile-jimage.md#how-ferrion-reads-an-image)).

### Tests

- Unit tests on small inputs written by test code (`ferrion_classfile::testing`): zip files with
  each feature (stored, deflate, zip64, data descriptors, a prefix, a comment, a comment holding an
  end record, duplicate names), jimage files in both byte orders with each compression, `ct.sym`
  layouts of both kinds, fake JDK homes of each kind, and manifests.
- Class files compiled by javac from Ferrion's own test sources, with javap's output, in
  `tools/oracle-check/classfile-cases/`.
- Malformed inputs, each expecting a specific error kind, and every truncation of valid inputs.

### Fuzz targets

Each target fails when the code panics or when `ferrion_classfile::invariants` finds a broken
promise (an entry the index lists that then can't be found, a read whose size differs from the
declared one without an error, a signature or descriptor that doesn't print back as its text):

- `classfile`: arbitrary bytes parsed as a class, then every accessor called, every signature,
  descriptor, and annotation decoded;
- `signature`: arbitrary text parsed as class, method, and field signatures and as descriptors;
- `zip`: arbitrary bytes as an archive, every entry listed and read with the size limits;
- `jar`: arbitrary bytes as a manifest, and as a zip resolved as a multi-release jar at several
  releases;
- `jimage`: arbitrary bytes as an image, indexed and every resource read;
- `jimage_structured`: a valid image built from the fuzz input, in either byte order and with each
  compression, then a few of its bytes changed, so the fuzzer reaches the location, string, and
  decompression code that random bytes rarely do;
- `ctsym`: zip inputs read as `ct.sym`, every release resolved.

Seeds are small files written by `testing::fuzz_seeds`, never copied from a JDK, and a test checks
that the committed ones are what it writes. They use the dictionary `fuzz/classfile.dict`.

### Results

Run on the matrix with `cargo oracle classfile`, javap from Corretto 27, on the machine of the
measurements above. Every check passed: no failure on any JDK, with the explained differences of
[in oracle-check](#in-oracle-check) as notes.

| JDK | Releases | javap: distinct classes | jrt: files | zip: entries | visibility: items |
| --- | --- | --- | --- | --- | --- |
| Corretto 8.502 | 8 | 17,629 | | 38,779 | |
| Temurin 11.0.32 | 8 to 11 | 33,658 | 31,377 | 7,744 | 26,589 |
| Corretto 17.0.17 | 8 to 17 | 37,377 | 27,469 | 13,653 | 55,196 |
| Corretto 21.0.11 | 8 to 21 | 39,186 | 28,737 | 17,770 | 75,377 |
| JetBrains Runtime 21.0.11 | 8 to 21 | 39,341 | 28,893, compressed | 17,770 | 75,427 |
| Corretto 25.0.1 | 8 to 25 | 13,911 | 28,135 | 16,974 | 90,542 |
| JetBrains Runtime 25.0.4.1 | 8 to 25 | 13,932 | 28,336, compressed | 16,995 | 90,552 |
| Corretto 27.0.1 | 8 to 27 | 14,282 | 28,357 | 17,510 | 101,448 |
| Temurin 27.0.1 | 8 to 27 | 14,282 | 28,487 | 17,510 | 101,448 |

The javap column counts distinct classes; identical classes in several releases are compared once
(87,011 more for JDK 27). JDK 8 has no visibility check: the harness runs on JDK 11 and later, and
JDK 8 has no modules. A JDK's first run took 17 to 57 seconds with two javap JVMs, and 3.5 to 18
seconds with javap's output cached; the run's peak resident memory, 0.5 to 1.6 GB, is the javap
JVMs' (each limited to a 2 GB heap), while Ferrion alone stays under 18 MB (see
[memory and performance](#memory-and-performance)). The two `jlink` images of
[O13](classfile-jimage.md#observation-log) passed the same checks.

The checks found these, each fixed before the results above:

- a signature parser bug: type arguments that are all unbounded wildcards (`Class<*>`) were
  rejected;
- the root modules of releases 9 and 10 (see [module visibility](#module-visibility));
- JDK 21's malformed `ModuleMainClass`, which lost `jdk.httpserver`'s exports for releases 18 to
  20 (see [ct.sym](classfile-ctsym.md#the-sig-files));
- duplicate zip entries, where javac takes the last (decision 8);
- a multi-release jar's versioned entries, which aren't names of their own at release 8 either.

On the corpus projects' dependency jars (845 jars and one `.pom` a build lists as a dependency,
each at its source sets' release), every check passed:

| Project | Release | zip: entries | jar: entries | javap: distinct classes |
| --- | --- | --- | --- | --- |
| jedis | 8 | 21,552 | 20,334 | 19,739 |
| guava | 8 | 51,441 | 51,429 | 35,261 |
| spring-data-jpa | 17 | 68,768 | 62,686 | 54,972 |
| lucene | 21 | 15,257 | 15,257 | 8,269 |
| hertzbeat | 25 | 195,245 | 190,783 | 120,596 |

The first run took 6 minutes (370 s, peak 1.2 GB with the javap JVMs); with javap's output cached,
about 90 seconds. The notes: five classes with flags their version doesn't define, and the `.pom`,
which neither `ZipFile` nor Ferrion reads as a zip. The corpus found, besides the release 8 jar
view above, only differences in how the renderer printed javap's formats (the `MethodParameters`
column, varargs on a parameter before the last, quoted names, AspectJ's unknown attributes), each
fixed in the renderer.

Each fuzz target ran for 5 minutes, one process at a time (`-rss_limit_mb=2048`), from the seeds:

| Target | Runs | Result |
| --- | --- | --- |
| `classfile` | 24,990,945 | no failure |
| `signature` | 23,861,437 | no failure |
| `zip` | 12,535,759 | no failure |
| `jar` | 4,622,219 | no failure after one fix |
| `jimage` | 11,928,242 | no failure after three fixes |
| `jimage_structured` | 3,367,636 | no failure after one fix |
| `ctsym` | 5,085,134 | no failure after one fix |

No target found a panic. They found indexes listing classes no lookup could find: a jar entry
with an empty path segment (`/Spample.class`), and image resources whose names couldn't be
decoded, weren't in modified UTF-8's one form, held a NUL (`c0 80`, compared with the query's
plain UTF-8), or had a `/` in the file name. Each was fixed, and its minimized input is a
regression test in `crates/ferrion_classfile/tests/fuzz-regressions/`.

### An observation helper

To keep the observations reproducible as JDKs change, a later round adds `tools/platform-observe`:
a Java program (public APIs only: jrt, `ZipFile`, `javax.tools`) and a script that, for a JDK
home, prints the facts these documents rely on: the image header, the decompressors used, the
jrt digest, `ct.sym`'s release letters and layout, and where javac loads `java.lang.Object` from
with `--release` at each supported release. It isn't written now; the commands in the observation
logs are the reference until it is.

## What was consulted

- **Specifications:** the JVM Specification, Java SE 27 edition, chapter 4; the JAR File
  Specification (manifests and multi-release jars); PKWARE's APPNOTE for zip; JEP 261 (modules,
  JMOD files), JEP 238 (multi-release jars), JEP 247 (`--release`), JEP 493 (linking without
  jmods).
- **Observation:** hex dumps of `lib/modules` and `ct.sym` from nine JDK installs, 31 images built
  with `jlink` with varied options, the output of `jimage info`, `list`, and `extract`, the jrt
  file system and `java.util.zip` through their public APIs, `javac -verbose` with `--release`, and
  javac's errors on small inputs written for the purpose. Throwaway Python and Rust programs
  outside the repository tested each hypothesis about the formats against those outputs.
- **Not consulted:** the source of OpenJDK (including its jimage, jrt, `ct.sym`, and class reading
  code), `javap`, ASM, or any other class file or image reader. Where the hypotheses came from
  general knowledge, such as the concept of a variable-length integer, the documents say so and
  record the observation that confirmed them.
- **Crates:** `miniz_oxide`'s and `memmap2`'s published metadata (license, `unsafe` policy), and
  `miniz_oxide` through its public API in the prototype.

## Decisions made in review

The user reviewed this design and accepted every recommendation:

1. **Platform API source:** `ct.sym` for every release it holds in full. The JDK's own image is used
   for its own release when `ct.sym` lacks it, filtered by `system-modules` and unqualified
   exports. Which source serves a release is decided by what `ct.sym` contains, never by a version
   table. There is no jmod and no JVM-export fallback.
2. **Exit criterion:** its wording stays. The image reader is checked against jrt on every image,
   compressed ones included, and the JetBrains Runtime 21, whose own release comes from its
   compressed image, is in the matrix.
3. **JDK 8:** `rt.jar`'s classes are read through `lib/ct.sym` (`META-INF/sym/rt.jar/`) when it
   exists, matching javac 8; the other boot jars and the `ext` jars are read directly. SCOPE.md
   says so.
4. **Always the `--release` view** of the target release, even for projects built with
   `-source`/`-target`. The differences (`jdk.swing.interop` resolving, newer APIs in older
   targets) are a known category of disagreement.
5. **Unobserved JDKs:** JDK 9 and 10 installations are a usage error, since no build exists for
   this platform (mise offers none); releases 9 and 10 stay targetable through a newer JDK's
   `ct.sym`, and the error says to use JDK 11 or later with `--release 9` or `10`. The check is in
   `Jdk::open`, the platform API's entry point, since `ferrion_project` comes later. JDK 22 to 24
   aren't installed, since the project now focuses on the LTS releases and the latest; like JDK 12
   to 16, they are unobserved but handled by the content-driven rule, and the `ct.sym` layout
   change is detected from the entries.
6. **`compact-cp` and big-endian images** are supported.
7. **The javap oracle** runs `javap` through `ToolProvider` and parses its text, in
   `tools/javap-oracle`.
8. **Duplicate zip entries:** the design proposed that the first entry in central directory order
   wins, after checking javac's choice with a test jar. The check showed that javac and `ZipFile`
   take the **last**, so Ferrion does (see [containers](classfile-containers.md#duplicate-names)).
