# Containers

How Ferrion reads zip files, jars, multi-release jars, directories, and the JDK 8 platform API.
Part of the [class file reading design](classfile.md). Module images and `ct.sym` have their own
documents: [jimage](classfile-jimage.md) and [ct.sym](classfile-ctsym.md).

## Zip

One hand-written reader serves jars, `ct.sym`, and JDK 8's jars (see
[dependencies](classfile.md#dependencies) for why it isn't a crate). Section numbers are from
PKWARE's APPNOTE.TXT, version 6.3.10.

### The subset read

- **End of central directory** (4.3.16): found by scanning back from the end of the file, over at
  most 22 + 65,535 bytes (the record plus the longest comment), for the signature 0x06054b50.
  Candidates are tried from the end backwards, those whose comment length reaches exactly to the
  end of the file first, and the first whose central directory reads, with entries, is used. A
  comment can hold bytes that look like an end record; this skips them (a unit test writes such
  a comment).
- **Zip64** (4.3.14, 4.3.15, 4.5.3): when the record's entry count, central directory size, or
  offset is 0xFFFF or 0xFFFFFFFF, the zip64 end of central directory locator just before it gives
  the zip64 record. In each central directory header, a 0xFFFFFFFF size or offset is replaced by
  the next value of the zip64 extended information extra field (id 0x0001), in the order APPNOTE
  gives. A jar needs zip64 with 65,535 or more entries or 4 GiB or more of data.
- **A prefix before the archive** (4.3.6 allows data before the first entry): offsets in the
  central directory are relative to the start of the archive, which isn't always the start of the
  file. The reader computes the prefix as the position where the central directory was found minus
  the offset recorded for it, and adds it to every offset. This covers executable jars with a
  launcher script in front, and JMOD files' four-byte header (offsets in the prototype were off by
  exactly 4 for jmods).
- **Central directory headers** (4.3.12): the name, compression method, flags, sizes, and the local
  header's offset. The central directory is read once, with one positional read, and indexed.
- **Local file headers** (4.3.7): read only for their name and extra field lengths, to find where
  the data starts. Sizes and the method always come from the central directory, so data
  descriptors (4.3.9, flag bit 3), which `ct.sym` and 277,717 of the 870,082 entries in the jar
  survey use, need no handling.
- **Compression methods** (4.4.5): 0 (stored) and 8 (deflate). Any other method is `Unsupported`
  for that entry; the survey found only 0 and 8.
- **Flags** (4.4.4): bit 0 (encrypted) makes the entry `Unsupported`. Bit 11 (UTF-8 names) is
  informational: the JAR specification says names are UTF-8, so names are read as UTF-8 either
  way, and a name that isn't valid UTF-8 is `Malformed` for that entry.
- **Directory entries** (names ending in `/`) are skipped.

Not read: the archive comment, extra fields other than zip64's, file attributes, timestamps,
digital signatures, and the CRC (see [dependencies](classfile.md#dependencies)).

### Duplicate names

When the central directory lists the same name twice, the **last** wins. In the survey, one jar of
3,166 has duplicates, and only of directory entries (`archunit-1.4.1.jar`: four directories, each
listed twice).

The design proposed that the first wins, to be checked before relying on it (decision 8 in
[review](classfile.md#decisions-made-in-review)). The check, with a jar written by Python's
`zipfile` holding two `p/A.class` entries, the first with a method `first()` and the second with
`second()`: javac 27 compiling `p.A.first()` against it reports `cannot find symbol`, and
`p.A.second()` compiles. `java.util.zip.ZipFile.getEntry("p/A.class")` returns the second entry's
bytes too (its SHA-256 through the javap oracle's `zip` mode), and `JarFile.versionedStream()`
lists both. So Ferrion takes the last entry in central directory order, matching javac and
`ZipFile`, and the zip container check confirms it on every corpus jar.

### Limits

The central directory's size and every entry's compressed size must fit within the file. An entry
whose declared uncompressed size is above 64 MiB is `TooLarge` for a class or manifest. Inflating
stops at the declared size: more output, or less, is `Malformed`. The entry count is bounded by
the central directory's size (each header is at least 46 bytes), so a lying count can't make the
reader allocate more than the file justifies.

### The survey

To see which features real jars use, a throwaway Python script read every jar in the local Maven
repository (`~/.m2/repository`, 3,166 jars, 870,082 entries, built up by work on unrelated
projects) and counted:

| Feature | Count |
| --- | --- |
| compression methods | 0 (stored): 37,858 entries; 8 (deflate): 832,224 |
| flag bits | bit 3 (data descriptor): 277,717 entries; bit 11 (UTF-8): 835,664 |
| zip64 records, data before the first entry, entries over 4 GiB | none |
| archive comments | 1 jar |
| duplicate names | 1 jar (directories only) |
| `Multi-Release: true` | 224 jars |
| `META-INF/versions/N/` directories | 9: 143 jars, 10: 8, 11: 85, 15: 6, 17: 8, 21: 24, 22: 2, 23: 3, 24: 8 |
| extra field ids | 0x5455 (timestamps), 0xcafe (the JAR marker), 0x7875 (Unix ids), 0x000a (NTFS) |
| class file major versions | 45 to 68 |

No jar in the survey needs zip64 or has a prefix, so those paths are covered by test inputs that
test code writes, and by the fuzz targets.

## Jars

### The manifest

`META-INF/MANIFEST.MF`, read only for its main section, following the JAR File Specification's
manifest format: lines end in CR LF, LF, or CR; a line starting with a single space continues the
previous one; the main section ends at the first blank line; names are compared ignoring ASCII
case. Two attributes are read:

- `Multi-Release`: the jar is multi-release when its value is `true`, ignoring case;
- `Class-Path`: space-separated relative URLs. javac follows them (observed, see
  [lookup](classfile.md#class-path-entries-and-lookup)); `ferrion_project` resolves them against
  the jar's directory and adds them to the classpath. Where javac puts them in the search order,
  and how it treats a missing one, is observed in the round that implements the inputs.

A manifest that doesn't parse is reported and treated as empty, so the jar is still read as a
plain jar.

### Multi-release jars

The JAR File Specification and JEP 238 define them. For a jar whose manifest says `Multi-Release:
true`, read for release R:

- an entry `META-INF/versions/N/<name>`, with N an integer from 9 to R, overrides `<name>`; the
  highest such N wins;
- names that only exist under `META-INF/versions/N/` with N ≤ R exist for release R;
- entries under `META-INF/versions/N/` with N > R, or N not an integer, are ignored;
- a `<name>` under `META-INF/` comes from the base only: `JarFile` doesn't look for versioned
  copies of those names, so `META-INF/versions/N/META-INF/x` neither overrides `META-INF/x` nor
  adds it (the corpus's `coherence-26.07.jar` has `META-INF/versions/22/META-INF/jpms.args` and
  no base one);
- for release 8, a multi-release jar's `META-INF/versions/` entries are ignored entirely: they
  aren't names of their own either, as `JarFile`'s versioned view at release 8 shows;
- in a jar without the attribute, `META-INF/versions/` entries are ordinary entries, as
  `JarFile` lists them, so no class is read from them. The survey found 5,355 versioned entries in
  jars without the attribute, almost all in `-sources` jars.

An entry whose name has an empty segment (`/A.class`, `p//A.class`) is no class: no lookup by
binary name reaches it, so it isn't listed in a package (found by fuzzing).

The jar's index is built per release: two source sets with different releases that use the same
jar share the zip archive and its central directory, and each gets its own resolved view.
`META-INF/versions/N/module-info.class` follows the same rule.

The check against the JDK's implementation uses `java.util.jar.JarFile` opened with
`Runtime.Version.parse("<R>")` (see [container checks](classfile.md#container-checks)).

javac reads a multi-release jar on the classpath for the release it compiles for, observed with a
jar whose `p.A` has a method `first()` in the base entry, `nine()` under `META-INF/versions/9/`, and
`seventeen()` under `META-INF/versions/17/`. javac 27 with `--release 8` compiles a call to
`first()` only; with `--release 11`, `nine()` only; with `--release 17`, `21`, and `27`,
`seventeen()` only; and with `-source 11 -target 11`, `nine()`. So the release to resolve a jar for
is the source set's target release, which is what `ferrion_project` passes.

## Directories

A classpath directory is read on demand: a class is the file `<dir>/<package path>/<Name>.class`,
and a package listing is a `read_dir` of the package's directory. Because macOS and Windows file
systems are usually case-insensitive, the reader compares the name the operating system returns
with the one asked for, and treats a case mismatch as missing, as javac would on a case-sensitive
system. Symbolic links are followed, since build tools create them; a loop is cut by the
operating system's error.

Directories aren't indexed or cached: they are usually build output that changes during a language
server session. The language server's file watching invalidates any listing a caller kept.

## JDK 8

SCOPE.md says the JDK 8 platform API is javac 8's default boot class path in `jre/lib`, then the
jars in `jre/lib/ext`. Observed with Corretto 8.502's `javac -verbose`, which prints its search
path:

```text
jre/lib/resources.jar, jre/lib/rt.jar, jre/lib/sunrsasign.jar, jre/lib/jsse.jar,
jre/lib/jce.jar, jre/lib/charsets.jar, jre/lib/jfr.jar, jre/classes,
jre/lib/ext/sunec.jar, jre/lib/ext/nashorn.jar, jre/lib/ext/cldrdata.jar, jre/lib/ext/dnsns.jar,
jre/lib/ext/localedata.jar, jre/lib/ext/sunjce_provider.jar, jre/lib/ext/sunpkcs11.jar,
jre/lib/ext/jaccess.jar, jre/lib/ext/zipfs.jar
```

`sunrsasign.jar` and `jre/classes` don't exist in that JDK and are skipped. The `ext` jars are in
the order the directory listed them, not sorted.

But javac 8 doesn't read `rt.jar`'s classes from `rt.jar`. `-verbose` shows `[loading
ZipFileIndexFileObject[<jdk>/lib/ct.sym(META-INF/sym/rt.jar/java/lang/Object.class)]]`, and the
same for `sun/misc/Unsafe` and `com/sun/net/httpserver/HttpServer`. JDK 8's `lib/ct.sym` is a zip
of 13,421 stored class files under `META-INF/sym/rt.jar/`, against 20,413 in `rt.jar`: 312 of
`rt.jar`'s packages are absent entirely (such as `com/apple/eawt`, `com/oracle/webservices/
internal/api`), and 249 more are partly absent. The classes present have no `Code` but keep their
members (`Object` and `sun/misc/Unsafe` have as many methods as in `rt.jar`), and carry
`RuntimeInvisibleAnnotations`. The effect: `com.apple.eawt.Application` gives `package
com.apple.eawt does not exist` with javac 8, although `rt.jar` has the class, while
`sun.misc.Unsafe` compiles with the warning `Unsafe is internal proprietary API and may be removed
in a future release`.

So, to match javac 8, Ferrion reads the JDK 8 boot class path in javac 8's order, with `rt.jar`'s
classes taken from `lib/ct.sym` when it exists. The other boot jars and the `ext` jars are read
as they are. SCOPE.md says so (decision 3 in [review](classfile.md#decisions-made-in-review)).
JDK 8 has no modules, so every package is visible. A JDK 8 serves only release 8. For a JRE, which
has `lib/rt.jar` and no `jre/` directory or `ct.sym`, the boot class path is read from `lib`.

`ct.sym` names only the classes; Ferrion lists `rt.jar`'s classes from the entries under
`META-INF/sym/rt.jar/` and reads them there, and a class `rt.jar` has but `ct.sym` doesn't is
missing, as for javac 8.

## JMOD files

Not used under the [recommendation](classfile.md#recommendation), but described for completeness:
a JMOD file starts with the four bytes `4a 4d 01 00` (`JM`, 1, 0; Temurin 11's and Corretto 27's
`java.base.jmod` both) followed by a zip whose offsets are relative to byte 4, with class files
under `classes/`. The zip reader above reads them as zips with a four-byte prefix. In the
prototype, the central directories of Corretto 27's 65 jmods held 28,688 entries, 27,645 of them
classes, all deflated.
