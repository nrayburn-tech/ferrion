# ct.sym

`lib/ct.sym` is the file javac reads for `--release` (JEP 247). Part of the
[class file reading design](classfile.md). Under its
[recommendation](classfile.md#recommendation), `ct.sym` is the main source of the platform API for
JDK 9 and later. JDK 8 has a `lib/ct.sym` too, in a different format, covered in
[containers](classfile-containers.md#jdk-8).

`ct.sym` is a zip of class files, both specified, but how its entries are named and what they
contain isn't. Everything below was learned by observing the files of the JDKs listed in the
[observation log](#observation-log) and javac's behavior with `--release`, never from javac's
source.

## Layout

### The container

A zip file. Every entry is deflated, with general purpose flag bits 3 (data descriptor) and 11
(UTF-8 names) set. JDK 27's has 21,495 entries: 17,510 `.sig` files and 3,985 directory entries.
JDK 11's has 7,745: 7,742 `.sig` files, a manifest, and `B/system-modules`.

### Releases as letters

Each top-level directory names a set of releases, one character per release, in base 36: `6` to `9`
for 6 to 9, `A` for 10, `B` for 11, up to `R` for 27. A file that is the same in several releases
is stored once, under a directory naming all of them: `89ABCDEFGHIJKLMNOPQR/java.base/java/io/
BufferedReader.sig` is release 8 to 27's `BufferedReader`. To find a class for release N, look in
the directories whose name contains N's letter: in every observed file, no path appears under two
directories that share a letter, so at most one has it (C8). The letters aren't always in
increasing order: JDK 11 has `876` and `879A`, and JDK 17 `879ABC`, while JDK 21 and later sort
them. Ferrion tests whether a name contains the letter and doesn't rely on order.

Base 36 runs out at `Z`, release 35. What `ct.sym` will do for release 36 isn't knowable yet;
Ferrion reports `Unsupported` for a release above 35 until a JDK shows it.

### Two entry layouts

| JDK | Classes | Module descriptors | Releases |
| --- | --- | --- | --- |
| Temurin 11.0.32 | `<letters>/<package>/<Class>.sig` | `<letters>-modules/<module>/module-info.sig`, for 9 and 10 | 6 to 10, and 11's `system-modules` |
| Corretto 17.0.17, 21.0.11 | `<letters>/<module>/<package>/<Class>.sig` | `<letters>/<module>/module-info.sig` | 7 to 16 (17), 8 to 20 (21), and the own release's `system-modules` |
| Corretto 25.0.1, 26.0.1, 27.0.1, Temurin 27.0.1, JetBrains Runtime 25.0.4.1 | as 17 | as 17 | 8 to the own release, in full |

In the second layout even releases 7 and 8, which have no modules, put each class under a module
directory (`8/java.base/java/lang/Object.sig`, `8/jdk.httpserver/com/sun/net/httpserver/...`).
JDK 12 to 16 weren't observed, so where the layout changed is unknown, and they aren't in the
matrix, which is the LTS releases plus the latest (decision 5 in
[review](classfile.md#decisions-made-in-review)). Ferrion tells the layouts apart from the
entries themselves: directories ending in `-modules` mean the first layout. The same holds for
JDK 22 to 24 and whether `ct.sym` holds their own release.

In the first layout, a class's module for release 9 or 10 comes from the module descriptors: the
module that exports the class's package. A class in a package no descriptor exports belongs to no
known module, which doesn't matter, since such a class is never visible, only reachable as a
supertype. In the second layout the module is the directory.

### `system-modules`

In JDK 11, 17, and 21, the directory for the JDK's own release holds a single file,
`system-modules`, a newline-separated list of module names (60 in JDK 21), and no classes. It lists
the modules javac makes visible when it compiles for its own release from the image. JDK 21's image
has 69 modules; those not listed are `jdk.internal.ed`, `jdk.internal.jvmstat`, `jdk.internal.le`,
`jdk.internal.opt`, `jdk.internal.vm.ci`, `jdk.internal.vm.compiler`,
`jdk.internal.vm.compiler.management`, `jdk.random`, and `jdk.unsupported.desktop`. JDK 25 and
later have no `system-modules` file: the own release is in the file like any other.

### The `.sig` files

Each `.sig` file is a class file (JVMS 4.1, magic 0xCAFEBABE) whose major version is the JDK's own,
whatever release it describes: 55 in JDK 11, 61 in 17, 65 in 21, 71 in 27. What they hold:

- **Members:** public and protected ones, with no `Code` attribute. No private member appears;
  `Object.sig` has `<init>`, `clone`, `equals`, `finalize`, `getClass`, `hashCode`, `notify`,
  `notifyAll`, `toString`, and three `wait` methods, and no `registerNatives`.
- **Classes:** the API's public classes, plus non-public ones that the API needs, such as
  `java/lang/AbstractStringBuilder` (access 0x0420, package-private), and classes from unexported
  packages that exported classes extend (346 in release 27, such as
  `jdk/internal/vm/vector/VectorSupport`). In release 27, `ct.sym` has 5,638 classes in exported
  packages; the image has 9,052 class files there, the difference being non-API classes
  (`java/util/TimSort`, anonymous classes).
- **Attributes,** counted over all of JDK 27's `.sig` files: `InnerClasses`, `Signature`,
  `ConstantValue`, `Exceptions`, `MethodParameters`, `AnnotationDefault`, `Deprecated`, `NestHost`,
  `NestMembers`, `PermittedSubclasses`, `Record`, `RuntimeVisibleAnnotations`,
  `RuntimeInvisibleAnnotations`, `RuntimeVisibleParameterAnnotations`,
  `RuntimeVisibleTypeAnnotations`, and in descriptors `Module`, `ModuleTarget`, `ModuleResolution`,
  and `ModuleMainClass`. No `ModulePackages`, `SourceFile`, or `Code`.
- **Flags the class file version doesn't define:** the `.sig` files for releases 8 to 16 keep
  `ACC_STRICT` on `strictfp` methods (`java/lang/StrictMath`), though their class file version, the
  JDK's own (61 and later), no longer defines it (JVMS 4.6). Ferrion keeps the flags as stored;
  javap 27 reports them as errors (see [in oracle-check](classfile.md#in-oracle-check)).
- **A malformed attribute:** in JDK 21's `ct.sym`, `jdk.httpserver`'s descriptor for releases 18
  to 20 (`IJK/jdk.httpserver/module-info.sig`) has a `ModuleMainClass` whose index points at a
  `CONSTANT_Utf8` rather than a `CONSTANT_Class`. javac accepts it, and `--release 18` sees
  `com.sun.net.httpserver`; Ferrion's module descriptor leaves the attribute out and keeps the
  module. Found by the visibility check, which saw javac's packages that Ferrion's first version
  lost with the whole descriptor.
- **Marker annotations:** annotations whose type names contain `+`, which no Java type can be
  named. Counting only the first annotation of each attribute, so these are lower bounds: invisible
  `jdk/Profile+Annotation` (on 4,412 classes), `jdk/internal/PreviewFeature+Annotation` (598
  classes, methods, and fields), and `sun/Proprietary+Annotation` (32 classes, in
  `jdk.unsupported` and elsewhere); visible `jdk/internal/ValueBased+Annotation` (175 classes) and
  `jdk/internal/javac/Restricted+Annotation` (27 methods). The class model keeps them like any
  annotation; B2 and B3 decide what they mean (javac warns that `sun.misc.Unsafe` is internal
  proprietary API with `--release 21`, observed).

### Vendors and updates

Corretto 27.0.1's and Temurin 27.0.1's `ct.sym` have the same entries with the same CRCs; only
timestamps differ. Corretto 25.0.1's and the JetBrains Runtime 25.0.4.1's differ in 64 entries: in
the JetBrains Runtime, release 25's `java/lang/Throwable` and `java/awt/Font` differ from release
24's, while in Corretto they're shared. So a vendor's `ct.sym` describes its own JDK's own release,
changes included, and the same release can differ between two JDKs.

## How javac uses it

Observed with `javac -verbose --release <N>` compiling a class that uses `java.util.ArrayList`;
`-verbose` prints each class file javac loads:

| JDK | `--release` | `java/lang/Object` loaded from |
| --- | --- | --- |
| Temurin 11 | 11 | `modules/java.base/java/lang/Object.class` (the image) |
| Temurin 11 | 10 | `/9A/java/lang/Object.sig` |
| Temurin 11 | 8, 6 | `/876/java/lang/Object.sig` (6 warns that it is obsolete) |
| Corretto 17 | 17 | the image |
| Corretto 17 | 11 | `/BCDEF/java.base/java/lang/Object.sig` |
| Corretto 21 | 21 | the image |
| Corretto 21 | 8 | `/8/java.base/java/lang/Object.sig` (warns that 8 is obsolete) |
| Corretto 25, JetBrains Runtime 25 | 25 | `/JKLMNOP/java.base/java/lang/Object.sig` |
| Corretto 27 | 27, 26 | `/QR/java.base/java/lang/Object.sig` |
| Corretto 27 | 8 | `/8/java.base/java/lang/Object.sig` |
| Corretto 27 | 7 | `error: release version 7 not supported` |

Visibility, observed by compiling small files:

- **Modules outside `system-modules` aren't visible for the own release:** `jdk.swing.interop`
  (from `jdk.unsupported.desktop`) gives `package jdk.swing.interop does not exist` with javac 21
  `--release 21`, which reads the image. Without `--release` (javac 27, `-source 27 -target 27`)
  it compiles. With `--release 27` and `--release 21` on javac 27, it doesn't exist either: release
  27's module set in JDK 27's `ct.sym` has 58 modules, the image 65, and the missing seven are
  `jdk.crypto.ec`, `jdk.internal.ed`, `jdk.internal.jvmstat`, `jdk.internal.le`, `jdk.internal.md`,
  `jdk.internal.opt`, and `jdk.unsupported.desktop`. Their unqualified exports: 230 in `ct.sym`,
  231 in the image, the difference being `jdk.swing.interop`.
- **Release 8 from a newer JDK isn't JDK 8:** `sun.misc.Unsafe` gives `package sun.misc does not
  exist` with `--release 8` on JDK 11 and 27, while javac 8 accepts it with a warning (see
  [containers](classfile-containers.md#jdk-8)). `com.sun.net.httpserver.HttpServer` resolves:
  `/8/com/sun/net/httpserver/HttpServer.sig` on JDK 11, `/8/jdk.httpserver/...` on 27.
- **Default modules:** `javax.smartcardio` (module `java.smartcardio`) is visible with
  `--release 27`, and `jdk.incubator.vector` isn't visible with `--release 17` or `21` (`package
  jdk.incubator.vector is not visible`) unless `--add-modules jdk.incubator.vector` is given, which
  adds an incubating warning. Its descriptor in those releases has `ModuleResolution` with value
  0x0009; release 27's has no `ModuleResolution`, and `--release 27` sees the package without
  `--add-modules` (see [module visibility](classfile.md#module-visibility)).
- **Releases 9 and 10 keep their own root modules:** with `--release 9` and `--release 10`, javac
  27 (and javac 11 to 25) doesn't see the `java.*` modules that `java.se` doesn't require, such as
  `java.xml.bind`, `java.corba`, and `java.smartcardio`, which `--release 11` and later do see.
  Found by the visibility check on every JDK 11 and later; Ferrion applies the rule by release
  number (see [module visibility](classfile.md#module-visibility)).

## How Ferrion reads it

1. Read the central directory with the [zip reader](classfile-containers.md#zip), once per JDK,
   shared through the `ArchiveCache`.
2. For a release, find the directories containing its letter, and decide the source: if the
   release has `.sig` files, `ct.sym` serves it; if it has only `system-modules`, the image serves
   it, filtered to the listed modules; if it has neither, the JDK can't serve the release, which is
   a usage error (`ErrorKind::Usage`) that `ferrion_project` reports. A JDK without `ct.sym` (a
   `jlink` runtime without `jdk.compiler`) serves only its own release, from every module of its
   image.
3. Build the release's index: for each `.sig` entry of the release, its module (from the directory,
   or from the descriptors' exports in the first layout), package, and simple name.
4. Read module descriptors from the release's `module-info.sig` files, and compute visibility as in
   [module visibility](classfile.md#module-visibility).
5. Look up a class by package and simple name, and inflate its entry.

## Observation log

The JDKs: Temurin 11.0.32+9, Corretto 17.0.17.10.1, 21.0.11.10.1, 25.0.1.8.1, 26.0.1.8.1, and
27.0.1.0.10, Temurin 27.0.1, and the JetBrains Runtime 25.0.4.1-b635.79, from mise, macOS aarch64.
The scripts were throwaway Python programs outside the repository, using `zipfile` and a small
class file reader written from JVMS chapter 4.

**C1. Entries.** A script listed each `ct.sym`'s entries and counted compression methods, flag bits,
top-level directory names, extensions, and directories, giving the counts in
[the container](#the-container) and [two entry layouts](#two-entry-layouts).

**C2. Letters and layouts.** For each release letter, a script printed a sample `.sig` path and any
file that isn't a `.sig`: `B/system-modules` (JDK 11), `H/system-modules` (17), `L/system-modules`
(21), none in 25 to 27. Sample paths: `879A/org/xml/sax/SAXParseException.sig` (11),
`7/java.base/java/awt/peer/ComponentPeer.sig` (17), `89ABCDEFGHIJKLMNOP/java.base/java/util/
UUID.sig` (27). Per-release file counts showed release 11 with 1 file in JDK 11, release 17 with
1 in JDK 17, release 21 with 1 in JDK 21, and release 27 with 5,696 in JDK 27.

**C3. `.sig` contents.** The class file reader dumped `8/java.base/java/lang/Object.sig` and
`I/java.base/java/lang/Thread.sig` from JDK 27: version 71.0, no `Code`, `Signature` and
`Exceptions` on methods. A script parsed all 17,510 `.sig` files of JDK 27 without error and
counted attributes by where they appear, and the types of annotations in annotation attributes.
Another compared `R`'s classes with the JDK 27 image's class files in exported packages (C5).

**C4. javac.** `javac -verbose --release <N> -d <out> Probe.java`, where `Probe` declares
`Object o = new java.util.ArrayList<String>();`, for the JDK and release pairs in
[the table](#how-javac-uses-it), keeping the `[loading ...]` lines for `Object` and `ArrayList`.

**C5. Module sets.** A script read every `module-info.sig` of release `R` in JDK 27's `ct.sym` and
every `/<module>/module-info.class` in its image, and compared module names and unqualified exports
(the `Module` attribute, JVMS 4.7.25). For JDK 21 it compared `L/system-modules` with the image's
modules.

**C6. Visibility.** Small classes compiled with javac 21 and 27: `Interop` (uses
`jdk.swing.interop.SwingInterOpUtils` and `sun.misc.Unsafe`), with `--release 27`, `-source 27
-target 27`, and `--release 21` on javac 27, and `--release 21` on javac 21; `U` (uses
`sun.misc.Unsafe` and `com.sun.net.httpserver.HttpServer`) with `--release 8 -verbose` on javac 11
and 27; `Roots` (uses `javax.smartcardio.Card`, `jdk.incubator.vector.IntVector`, and
`HttpServer`) with `--release 27`, `17`, and `21`, with and without `--add-modules
jdk.incubator.vector`. A script printed the raw `ModuleResolution` attribute of every
`module-info.sig` in JDK 27's `ct.sym` that has one: `0009` for `jdk.incubator.httpclient` (9, 10),
`jdk.incubator.foreign` (14 to 18), `jdk.incubator.jpackage` (14, 15), `jdk.incubator.concurrent`
(19, 20), and `jdk.incubator.vector` (16 to 26).

**C7. Vendors.** A script compared the entry names and CRCs of Corretto 27's and Temurin 27's
`ct.sym`, and of Corretto 25's and the JetBrains Runtime 25's.

**C8. One directory per release and path.** A script recorded, for every `.sig` entry and every
letter in its directory name, the pair (letter, path below the directory), and counted pairs seen
twice: none in any of the eight files. It also listed directory names whose letters aren't in
increasing order: ten in JDK 11, nine in JDK 17, none in 21 and later.
