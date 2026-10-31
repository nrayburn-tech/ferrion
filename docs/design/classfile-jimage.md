# jimage

The module image, `lib/modules` in every JDK 9 and later, holds the JDK's classes and resources.
Part of the [class file reading design](classfile.md). Under its
[recommendation](classfile.md#recommendation), Ferrion reads the image for a JDK's own release
when `ct.sym` doesn't hold that release (observed for JDK 11, 17, and 21).

The format has no public specification. Everything here was learned by observing real images:
hex dumps of installed JDKs' images, small images built with `jlink` with varied options, the
JDK's own `jimage` tool, and the jrt file system through Java's public API. Each fact below cites
the observation that established it, in the [observation log](#observation-log). Where a hypothesis
came from general knowledge rather than from the data, the log says so, along with the test that
confirmed it.

## Summary of the format

An image is an index followed by resource contents:

```text
header             28 bytes
redirect table     4 × table length bytes (s32)
offsets table      4 × table length bytes (u32): each entry's offset in the location table
location table     locations size bytes: one attribute stream per entry
strings            strings size bytes: NUL-terminated modified UTF-8
resources          the rest: each entry's content at (index size + its offset attribute)
```

Index size = 28 + 8 × table length + locations size + strings size, which matches the "Index
Size" `jimage info` prints for every image (O2).

### Byte order

Everything in the header, the two tables, directory contents, and the header of compressed
resources is in the image's byte order. The magic, 0xCAFEDADA, tells which: the first four bytes
are `da da fe ca` in a little-endian image and `ca fe da da` in a big-endian one (O1, O7). The
location attribute streams, the numbers inside `compact-cp` payloads, and the class files
themselves are big-endian in both (O7, O10).

### Header

| Offset | Size | Field | Observed values |
| --- | --- | --- | --- |
| 0 | 4 | magic | 0xCAFEDADA |
| 4 | 2 | minor version | 0 in JDK 11 to 26, 1 in JDK 27 |
| 6 | 2 | major version | 1 |
| 8 | 4 | flags | 0 in every image |
| 12 | 4 | resource count | as `jimage info` prints |
| 16 | 4 | table length | equal to the resource count in every image |
| 20 | 4 | locations size | bytes |
| 24 | 4 | strings size | bytes |

The version is read as one 32-bit value in the image's byte order: 0x00010000 for 1.0, 0x00010001
for 1.1, so major is the high half (O1, O2). Ferrion accepts major version 1 with any minor
version, and reports `Unsupported` for any other major version.

### Locations

Each entry of the offsets table points into the location table, at an attribute stream. Each
attribute starts with a byte `kind << 3 | (length - 1)`, followed by `length` bytes of value,
big-endian. Kind 0 ends the stream (O3).

| Kind | Meaning | Value |
| --- | --- | --- |
| 1 | module | string offset (empty for the two root directories) |
| 2 | parent | string offset: the path inside the module, `java/lang` |
| 3 | base | string offset: the file name without extension |
| 4 | extension | string offset: `class`, `gif`, ... |
| 5 | offset | the content's offset after the index |
| 6 | compressed size | bytes stored; absent or 0 when the content isn't compressed |
| 7 | uncompressed size | bytes of content |

An attribute that is absent is 0, which for strings is offset 0, the empty string (the strings
area starts with a NUL in every observed image). Lengths of 1 to 4 bytes were observed; the
encoding allows 8 (O3, O9).
An entry's name is `/module/parent/base.extension`, leaving out `parent/` and `.extension` when
they're empty. For every resource this matched `jimage list` (O4).

Ferrion doesn't trust attribute order or uniqueness: a kind above 7 or a stream that runs past the
location table is `Malformed`.

### Entry kinds

Every entry is one of three kinds (O4, O5):

- **Resources:** the module attribute names a module, as in `/java.base/java/lang/Object.class`.
- **Module directories:** module `modules`, as in `/modules/java.base/java/lang`. Their content is
  an array of u32 location offsets, one per child, in the image's byte order. Children are files
  (resources) and subdirectories.
- **Package entries:** module `packages`, as in `/packages/java.lang`. Their content is pairs of
  u32: a flag and the string offset of a module name, one pair per module that has a directory
  for the package.
- **Two roots:** `/modules` and `/packages`, with no module attribute and base `modules` or
  `packages`, listing the module directories and package entries.

In JDK 21's image: 28,737 resources, 1,285 module directories, 999 package entries, and the two
roots, which add up to the 31,023 entries in the header.

The flag in package entries changed between versions (O6). In version 1.0 images (JDK 11, 17, 21,
25, 26, and the JetBrains Runtime 25) it is 1 when the module's directory for the package has no
files, only subdirectories (`java.instrument` has `java/lang` only for `java/lang/instrument`),
and 0 otherwise. In version 1.1 images (JDK 27) it is 2 and 6. JDK 27 adds a class
`jdk/internal/jimage/PreviewMode`, which suggests the new bits concern preview content, but no
observed image has any, so nothing more is known. **Ferrion doesn't use the flags, the package
entries, or the module directories:** it builds its own index from the resources' module and
parent attributes, which mean the same in every version.

### The redirect table

The redirect table has the same length as the offsets table. Its values are negative, zero, or
positive (in JDK 27's image: 11,238 negative, all distinct, from -3 to -30,682; 11,296 zero; 8,148
positive, from 0x01000194 to 0x010001CD). For each negative value v, -1 - v is a valid index into
the offsets table, and together they cover 0 to 30,681. This is consistent with a hash table that
maps a name to an entry, either directly (negative) or through a second hash with a new seed
(positive). Ferrion doesn't use it: it doesn't need lookups by full name without an index, since it
builds one in 7.5 ms, and using the table would mean learning the hash function, which nothing
observed so far pins down. Not using it removes one way the format could change under Ferrion.

### Strings

The strings area holds NUL-terminated strings in modified UTF-8 (JVMS 4.4.7): an image built with
`--compress=1` stores constant pool strings there, and they include `c0 80` for NUL, which strict
UTF-8 decoding rejects (O10). An offset into the middle of a string reads as its suffix, so Ferrion
reads a string from the offset to the next NUL and never assumes offsets fall on boundaries.

### Resource content

A resource's content starts at index size + its offset attribute. When the compressed size is 0,
the content is the uncompressed size's bytes (O4). Otherwise it is a compressed resource (O8):

| Offset | Size | Field | Observed values |
| --- | --- | --- | --- |
| 0 | 4 | magic | 0xCAFEFAFA, in the image's byte order |
| 4 | 8 | compressed size | the bytes after this 29-byte header |
| 12 | 8 | uncompressed size | equal to the location's uncompressed size |
| 20 | 4 | decompressor | string offset: `zip` or `compact-cp` |
| 24 | 4 | decompressor configuration | 0xFFFFFFFF in every image |
| 28 | 1 | unknown | 1 in every image |

The payload is the rest: compressed size = stored size - 29 in every resource. One header was
always enough; no payload observed starts with another 0xCAFEFAFA. Ferrion loops anyway, with a
limit of four layers, and checks every size: a payload size, an output size, or a chain that
doesn't match is `Malformed`.

- **`zip`:** the payload is a zlib stream (RFC 1950: `78 9c` and deflate). The JetBrains Runtime
  25 compresses every resource this way, and so do `jlink`'s `--compress=2` and `zip-0` to `zip-9`
  (O8, O9).
- **`compact-cp`:** the payload is the class file with its constant pool's `CONSTANT_Utf8` entries
  replaced, and every other byte unchanged (O10). Built by `jlink --compress=1`, deprecated since
  JDK 21 but still accepted by 27. In the constant pool, two tags that JVMS 4.4 doesn't define
  appear:
  - **tag 23:** a number, the string offset of the entry's text;
  - **tag 25:** a descriptor or signature split into parts: a number for the string offset of a
    *skeleton*, the text with every class name after an `L` removed (`(L;)L;`), then a number
    giving the byte length of what follows, then a pair of numbers per `L` in the skeleton: the
    string offsets of the package (`java/lang`) and the simple name (`String`). The text is the
    skeleton with `package/name` inserted after each `L`. `()Ljava/lang/Class<*>;` has the
    skeleton `()L<*>;`.

  Numbers are big-endian, in one of two forms: when the first byte's high bit is set, bits 6 and 5
  give the length (01 for one byte, 10 for two, 11 for three) and the value is the low five bits
  followed by the remaining bytes; when the high bit is clear, the value is a plain four-byte
  number. A first byte of the form `100xxxxx` never appears, and Ferrion treats it as `Malformed`.
  A package offset pointing at the empty string wasn't observed (every JDK class is in a package);
  Ferrion inserts just the name then.

  The variable-length number was a hypothesis from general knowledge of such encodings, refined
  until it matched every constant pool entry of every class in five images (O10).

## How Ferrion reads an image

1. Read the 28-byte header with a positional read, check the magic and version, and check that
   the index fits in the file.
2. Read the index (1.6 MB for a full JDK) into one buffer.
3. Walk the offsets table once: decode each location, skip the `modules` and `packages` entries and
   the roots, and add each resource to an index keyed by module and package, with the simple name
   and the location offset. Modules come from the resources too: an entry
   `/<module>/module-info.class` is each module's descriptor.
4. Look up a class by package (to find its module, through the platform API's module map) and
   simple name, read its content with a positional read, and decompress it if needed.

The index stores names as string offsets into the index buffer, so it costs a few megabytes. Its
building took 7.5 ms for the JDK 27 image in the prototype.

## Limits

Every size is checked against the file before reading: the index must fit in the file, every
location offset within the location table, every string offset within the strings, and every
content range within the file. An uncompressed size above 64 MiB is `TooLarge` (the largest
resource in any observed image is 378,371 bytes, `jdk.jpackage`'s `JavaApp.icns`, and the largest
class 298,424 bytes, `sun/nio/cs/GB18030.class`). A `zip` payload inflates
with the declared size as a hard limit, and a `compact-cp` payload's output can't exceed the
declared size either.

## Observation log

The JDKs, all installed with mise for macOS on aarch64: Temurin 11.0.32+9, Corretto 17.0.17.10.1,
21.0.11.10.1, 25.0.1.8.1, 26.0.1.8.1, and 27.0.1.0.10, Temurin 27.0.1 (its `release` file says
27.0.1.0.2+1), and the JetBrains Runtime 25.0.4.1-b635.79 (JBRSDK). `$J` below is
`~/.local/share/mise/installs/java`. The throwaway scripts were Python 3 programs outside the
repository, run with `python3 -I`; each is described by what it does.

**O1. Header bytes.** `xxd -l 64 $J/<jdk>/lib/modules` for each JDK. Every image starts with `da da
fe ca`. Bytes 4 to 7 are `00 00 01 00` in JDK 11 to 26 and the JetBrains Runtime, and `01 00 01 00`
in JDK 27 (both vendors). Bytes 8 to 11 are zero. Bytes 12 to 15 and 16 to 19 are equal (34,031 =
0x84ef in Temurin 11). The image sizes: 129 to 149 MB, except the JetBrains Runtime's at 66 MB,
the first sign that it is compressed.

**O2. `jimage info`.** `$J/<jdk>/bin/jimage info $J/<jdk>/lib/modules` prints Major Version 1 and
Minor Version 0 (1 for JDK 27), Flags 0, Resource Count, Table Length, Offsets Size and Redirects
Size (both 4 × table length), Locations Size, Strings Size, and Index Size. The values match the
header fields in the order above, and Index Size = 28 + Offsets + Redirects + Locations + Strings,
for all eight images: for example Temurin 11's 28 + 136,124 + 136,124 + 707,693 + 776,891 =
1,756,860.

**O3. Location encoding.** A script printed the bytes of the location table between consecutive
sorted offsets. Corretto 21's first entry is `09 03 be 12 09 69 6f 1a 09 69 e8 20 01 2b 08 43 96 61
39 01 e8 00`. Read as `kind << 3 | (length - 1)`: kind 1 with 2 bytes (0x03be), kind 2 with 3
bytes, kind 3 with 3, kind 4 with 1 (offset 1, the string `class`), kind 5 with 4, kind 7 with 2,
then 0. Decoding the string offsets gave `jdk.security.jgss`, `com/sun/security/jgss`,
`InquireSecContextPermission`, and `class`. Counting kinds and lengths over all entries of each
image found only kinds 1 to 7 and lengths 1 to 4, in every version including 1.1.

**O4. Names and contents against `jimage`.** `jimage list` printed 28,737 resource lines for
Corretto 21. `jimage extract --dir <scratch> $J/corretto-21.0.11.10.1/lib/modules` wrote the
resources; a script compared each one's bytes with the content read at index size + kind 5, for
the kind 7 size: 28,737 the same, 0 different. The only entries not extracted were the roots
`/modules` and `/packages`. The same check on the JetBrains Runtime (after O8) gave 28,336 the
same, 0 different.

**O5. Directory entries.** A script decoded the content of entries whose module is `modules` or
`packages`. `/modules/java.base/java` (36 bytes) is nine u32 values; read as location offsets they
are `/modules/java.base/java/io`, `.../lang`, `.../math`, and so on. `/modules/java.base` lists
`META-INF`, `apple`, `com`, `java`, `javax`, `jdk`, `sun`, and the resource
`/java.base/module-info.class`. `/packages/java.lang` (24 bytes) is `(0, java.base), (1,
java.instrument), (1, java.management)` as (flag, module name string).

**O6. Package flags against the jrt file system.** A Java program (`Jrt.java`, public API only)
opened each JDK's image with `FileSystems.newFileSystem(URI.create("jrt:/"), Map.of("java.home",
home))` and listed `/packages/java.lang`: three symbolic links, to `/modules/java.base`,
`/modules/java.instrument`, and `/modules/java.management`. jrt doesn't show the flag. A script
then checked, for every pair in every package entry, whether the flag is 1 exactly when the
module's directory for the package has no non-directory children: true for all 1,203 to 1,406
pairs in each version 1.0 image (11, 17, 21, 25, 26, JetBrains Runtime 25), false for all 1,234 in
both JDK 27 images, where the values are 2 (323 pairs, no files) and 6 (911 pairs, files).
`jimage list` of JDK 27 has `jdk/internal/jimage/PreviewMode.class` in `java.base`, absent before.

**O7. Byte order.** `jlink --add-modules java.base --endian big --output <dir>` with JDK 11, 17,
and 21 built images starting `ca fe da da`. JDK 27's `jlink` refused: `specified --endian
BIG_ENDIAN does not match endianness of target platform macos-aarch64`. The host's `jimage` tool
refused the big-endian images (`is not an image file`), so a script compared them with the
little-endian images built by the same JDK: the same names, the same contents for every resource
except three classes that `jlink` generates on each run (`DirectMethodHandle$Holder`,
`DelegatingMethodHandle$Holder`, `Invokers$Holder`, which differ between any two JDK 11 runs, see
O10), and directory contents of the same sizes. Those results came from reading the header,
tables, directory contents, and compressed-resource headers big-endian, and the location streams
the same way as in little-endian images.

**O8. Compressed resources.** In the JetBrains Runtime 25, every resource has a kind 6 attribute.
`Object.class` has stored size 1,340 and uncompressed size 2,412, and its content starts `fa fa fe
ca 1f 05 00 00 00 00 00 00 6c 09 00 00 00 00 00 00 07 00 00 00 ff ff ff ff 01 78 9c`: the magic,
1,311 (= 1,340 - 29), 2,412, string offset 7 (`zip`), 0xFFFFFFFF, 1, then `78 9c`, a zlib header.
Inflating the rest as zlib gave 2,412 bytes. Across the image, every resource had exactly one
`zip` header with those last two values. Checked against `jimage extract` as in O4, and against
jrt as in O11.

**O9. `jlink` compression options.** A script built `java.base`-only images with each JDK's
`jlink`: no compression, `--compress=1`, `--compress=2`, and for JDK 21 and 27 also `zip-0`,
`zip-1`, `zip-6`, and `zip-9`; plus JDK 27 `java.sql` with and without `--strip-debug`, and
Temurin 27's `java.base` plain and with `--compress=1` (Temurin 27 has no jmods, so it links from
its run-time image, JEP 493). Each `--endian big` variant is in O7. JDK 21 and 27 print
`Warning: The 1 argument for --compress is deprecated` (and the same for 2). `--compress=2` and
every `zip-N` produced the `zip` decompressor with identical headers apart from sizes. The zlib
header's second byte follows the level (`78 01` for `zip-0`, `78 9c` for the default, `78 da` for
`zip-9`), and `zip-0`'s image is slightly larger than the uncompressed one.
`--compress=1` produced `compact-cp` on every class and left other resources uncompressed. The
resources of eight of these images matched `jimage extract` from the same JDK byte for byte: JDK
11, 17, 21, 27, and Temurin 27 with `--compress=1`, JDK 21 with `zip-0`, JDK 27 with `zip-9`, and
JDK 27's plain `java.sql` image. Image sizes for `java.base` with JDK 27:
29.5 MB plain, 20.2 MB `compact-cp`, 13.7 MB `zip-6`, 13.7 MB `zip-9`, 14.4 MB `zip-1`, 29.8 MB
`zip-0`.

**O10. `compact-cp`.** In JDK 21's `--compress=1` image, `Object.class`'s payload starts like a
class file (`ca fe ba be 00 00 00 41 00 7c`), then `07 00 02` (as in the plain class), then `17 c2
81` where the plain class has `01 00 17 java/lang/StringBuilder`. String offset 641 in the image
is `java/lang/StringBuilder`. Entries with tag 25 (`19`) lined up with descriptors: `19 c0 29 a3
a7 c0 82` with `()Ljava/lang/Class;`, and the strings at the decoded offsets are `()L;` (41),
`java/lang` (7), `Class` (130); the 3 in between (`a3`) is the length of `a7 c0 82`. The
hypothesis for the numbers' form (above) came from these bytes and general knowledge of
variable-length integers. The first version, which had no four-byte plain form, failed on a
first byte of `00`; the plain form fixed it. A script then compared every constant pool entry of
every class with the same class in the plain image built by the same JDK, and the bytes after the
constant pool: JDK 17, 21, 27, and Temurin 27 (6,390 to 7,533 classes each, about a million
entries each) agreed with no differences; JDK 11 differed only in the three generated `Holder`
classes. A full decoder that rebuilds the class file then matched `jimage extract` of the
`compact-cp` image itself for every resource, JDK 11's included, so those three differences are
between `jlink` runs, not decoding. A big-endian `compact-cp` image from JDK 21 decoded the same
way (numbers big-endian) and matched the plain little-endian image for every resource. The strings
include modified UTF-8 (`c0 80`), which strict UTF-8 decoding rejected. No `CONSTANT_Utf8` entry
was left inline in any of these images.

**O11. Every file through jrt.** `Jrt.java` walked `/modules` of each JDK's image through jrt, in
sorted path order, hashing each path and its bytes with SHA-256 (run with Corretto 27's `java`,
opening the other JDK's image). The throwaway decoder computed the same digest from the image.
They matched for all eight: Temurin 11 (31,377 files), Corretto 17 (27,469), 21 (28,737), 25
(28,135), 26 (28,714), and 27 (28,357), Temurin 27 (28,487), and the JetBrains Runtime 25
(28,336, compressed).

**O12. The redirect table.** A script counted the table's negative, zero, and positive values, as
in [The redirect table](#the-redirect-table), for JDK 27 and Temurin 11 (12,556 negative, 12,536
zero, 8,939 positive from 0x01000194 to 0x010001C3).

To reproduce: install the JDKs with mise, and run the commands above. The scripts are short, and
each description says what it computes; a later round turns them into
[`tools/platform-observe`](classfile.md#an-observation-helper).
