# Class file format

Which parts of a class file Ferrion reads, how, and what is lazy. Part of the
[class file reading design](classfile.md). Section numbers are from the JVM Specification, Java SE
27 edition, chapter 4.

## The lazy view

```rust
pub struct ClassFile {
    bytes: Arc<[u8]>,
    constant_pool: Box<[u32]>, // offset of each entry; 0 for the slot after a long or double
    // offsets of this_class, the interface list, the field, method, and attribute tables
}

impl ClassFile {
    /// Checks the structure and indexes it. Never panics.
    pub fn parse(bytes: Arc<[u8]>) -> Result<ClassFile, ClassFileError>;

    pub fn version(&self) -> Version;            // major, minor; preview when minor is 0xFFFF
    pub fn access(&self) -> ClassAccess;         // JVMS table 4.1-B
    pub fn this_class(&self) -> Result<Name<'_>, ClassFileError>;
    pub fn super_class(&self) -> Result<Option<Name<'_>>, ClassFileError>;
    pub fn interfaces(&self) -> impl Iterator<Item = Result<Name<'_>, ClassFileError>>;
    pub fn fields(&self) -> Members<'_>;         // each: access, name, descriptor, attributes
    pub fn methods(&self) -> Members<'_>;
    pub fn attributes(&self) -> Attributes<'_>;  // each: name, and decode() into Attribute
}

/// A name or other text in the constant pool: modified UTF-8 bytes, decoded on request.
pub struct Name<'a>(&'a [u8]);
```

`parse` walks the structure once: the magic and version, every constant pool entry (checking its
tag and that it fits), the interface list, and every field, method, and attribute header, checking
that each length stays within its parent. It doesn't decode text or attribute contents. Over every
class of the JDK 27 image this walk took 23 ms for 27,661 classes, 135 MB (see
[performance](classfile.md#memory-and-performance)).

After `parse`, the structure is known to be sound, so iterating members and attributes can't fail;
decoding a constant pool reference, a piece of text, or an attribute's contents can, and returns
`Result`. A class whose `Signature` is malformed still has readable members.

Bytes after the last attribute are reported as `Malformed` (JVMS 4.8 allows none) after the rest of
the class has been indexed, so the caller can still use it.

## What is read

### Versions (4.1)

Major versions 45 to 71 (Java 1.0.2 to 27), and preview class files (minor version 0xFFFF, for
major 56 and later). A newer major version is `Unsupported`, as javac rejects class files from a
newer release. Observed majors: 45 to 68 across the jar survey, 52 and 71 in the JDK 27 image, and
only the JDK's own major in `ct.sym`.

### Constant pool (4.4)

Every tag in table 4.4-B: `Utf8`, `Integer`, `Float`, `Long`, `Double`, `Class`, `String`,
`Fieldref`, `Methodref`, `InterfaceMethodref`, `NameAndType`, `MethodHandle`, `MethodType`,
`Dynamic`, `InvokeDynamic`, `Module`, and `Package`, with `Long` and `Double` taking two slots
(4.4.5). An unknown tag is `Malformed`, since the pool can't be walked past it. References are
checked when followed: an index outside the pool, at the unusable slot, or at the wrong tag is
`Malformed`.

### Text: modified UTF-8 (4.4.7)

`CONSTANT_Utf8` holds modified UTF-8: NUL is `c0 80`, and characters outside the Basic
Multilingual Plane are two three-byte surrogates. Observed in practice: JDK class constants
include `c0 80` (see the [jimage log](classfile-jimage.md#observation-log), O10).

- **Names** (class, member, package, and module names, descriptors, signatures) decode to Rust
  strings. When the bytes are valid UTF-8, which they are for nearly every name, they are borrowed
  as they are; otherwise they are converted (`c0 80` to NUL, surrogate pairs to one character). A
  lone surrogate can't be in a Rust string, so a name containing one is `Unsupported`.
- **String constants** (`ConstantValue` of a `String` field, string annotation values) can hold
  any UTF-16 sequence, lone surrogates included, and B3's constant folding needs them exactly. They
  are exposed as a `JavaStr`, the modified UTF-8 bytes with conversion to UTF-16 and to a lossy
  Rust string.

### Names (4.2) and descriptors (4.3)

Class names are in the internal form (4.2.1, `java/util/Map$Entry`). Field and method descriptors
(4.3.2, 4.3.3) are parsed when asked for. An array type has at most 255 dimensions (4.4.1); more
is `Malformed`. Names aren't checked against 4.2.2's rules beyond being non-empty: javac reads
names it couldn't write, and B2 doesn't need them to be valid Java identifiers.

### The class (4.1), fields (4.5), and methods (4.6)

Access flags (tables 4.1-B, 4.5-A, 4.6-A), `this_class`, `super_class` (0 only for
`java/lang/Object` and `module-info`), the interfaces, and each member's flags, name, descriptor,
and attributes.

### Attributes (4.7)

| Attribute | Section | Where | Decoded into, and why |
| --- | --- | --- | --- |
| `ConstantValue` | 4.7.2 | field | the constant (int, long, float, double, or `JavaStr`), for constant folding |
| `Code` | 4.7.3 | method | skipped, with everything inside it |
| `Exceptions` | 4.7.5 | method | thrown types, when no `Signature` gives them |
| `InnerClasses` | 4.7.6 | class | nested classes: outer, simple name, and the flags of table 4.7.6-A, which a nested class's own access flags don't carry |
| `EnclosingMethod` | 4.7.7 | class | the enclosing class and method of local and anonymous classes |
| `Synthetic` | 4.7.8 | all | a flag |
| `Signature` | 4.7.9 | class, field, method, record component | text, parsed on demand (below) |
| `SourceFile` | 4.7.10 | class | the file name, for diagnostics |
| `Deprecated` | 4.7.15 | all | a flag |
| `RuntimeVisibleAnnotations`, `RuntimeInvisibleAnnotations` | 4.7.16, 4.7.17 | all | annotations (below) |
| `RuntimeVisibleParameterAnnotations`, `RuntimeInvisibleParameterAnnotations` | 4.7.18, 4.7.19 | method | annotations per parameter |
| `RuntimeVisibleTypeAnnotations`, `RuntimeInvisibleTypeAnnotations` | 4.7.20, 4.7.21 | class, field, method, record component | type annotations (below) |
| `AnnotationDefault` | 4.7.22 | method | an element value |
| `BootstrapMethods` | 4.7.23 | class | not decoded: only code and dynamic constants use it |
| `MethodParameters` | 4.7.24 | method | names and flags, including mandated and synthetic parameters |
| `Module`, `ModulePackages`, `ModuleMainClass` | 4.7.25 to 4.7.27 | `module-info` | the module descriptor |
| `NestHost`, `NestMembers` | 4.7.28, 4.7.29 | class | nestmates, for private access |
| `Record` | 4.7.30 | class | components with their attributes |
| `PermittedSubclasses` | 4.7.31 | class | sealed hierarchies |

Not in the JVMS, seen in the JDK's module descriptors: `ModuleTarget`, `ModuleResolution`, and
`ModuleHashes`. They are kept with their name and bytes; `ModuleResolution`'s raw u2 value is
exposed for [module visibility](classfile.md#module-visibility). `SourceDebugExtension` (4.7.11)
isn't decoded. Any other attribute (4.7.1 lets class files carry their own) is kept with its name
and bytes, so the [javap comparison](classfile.md#what-without-loss-means) can tell "not decoded"
from "lost".

The attributes counted in the JDK 27 image's 27,661 classes, outside `Code`, were all in this
table: `BootstrapMethods`, `Deprecated`, `EnclosingMethod`, `InnerClasses`, `Module`,
`ModuleHashes`, `ModuleMainClass`, `ModulePackages`, `ModuleResolution`, `ModuleTarget`,
`NestHost`, `NestMembers`, `PermittedSubclasses`, `Record`, both kinds of annotations, visible
type annotations, `Signature`, `SourceFile`, `ConstantValue`, `AnnotationDefault`, `Exceptions`,
`MethodParameters`, `RuntimeVisibleParameterAnnotations`, and `Code`.

### Annotations (4.7.16)

Each annotation is its type (a field descriptor) and element-value pairs. Element values (4.7.16.1)
are constants (`B C D F I J S Z s`), enum constants (`e`), classes (`c`), nested annotations (`@`),
and arrays (`[`). Nested annotations and arrays nest, so decoding keeps an explicit depth and
reports `Malformed` beyond 64 levels, far more than any real annotation, rather than risking the
stack. Decoding is lazy: an annotation attribute yields annotations one at a time, and a value is
decoded when asked for.

The parameter annotation attributes number parameters their own way (4.7.18 notes that the count
may differ from the descriptor's). How javac's class files align them with the descriptor and with
`MethodParameters` for inner class constructors and enum constructors is compared with `javap` on
test classes in the implementation round, and recorded there.

### Type annotations (4.7.20)

Each type annotation has a target (4.7.20.1), a path (4.7.20.2), and the annotation itself.

- **Targets read**, outside `Code`: class type parameter (0x00), method type parameter (0x01),
  supertype (0x10, where 65,535 means the superclass), type parameter bounds (0x11, 0x12), field
  or record component type (0x13), return type or constructed type (0x14), receiver (0x15), formal
  parameter (0x16), and `throws` (0x17). Targets 0x40 to 0x4B appear only inside `Code`, which is
  skipped; one outside it is `Malformed`.
- **Paths** are kept as a list of steps: deeper in an array type (0), deeper in a nested type (1),
  a wildcard's bound (2), and a type argument with its index (3). Applying a path to a type is B3's
  job, once it has parsed the signature or descriptor the path walks.

### Signatures (4.7.9.1)

Signatures are parsed only when a member is used, into a small tree:

- class signatures: type parameters with their bounds, the superclass, and the interfaces;
- method signatures: type parameters, parameter types, the result, and `throws` types;
- field signatures: a reference type.

Types are base types, type variables, arrays, and class types with their type arguments (`*`,
`+`, `-`, or a type) and inner class suffixes (`.Inner<...>`). Parsing uses an explicit stack, so
deeply nested type arguments can't overflow the call stack, and reports `Malformed` beyond 255
array dimensions or 1,000 levels of nesting. A malformed signature is an error for that member
only, and B3 decides how to treat the member (for example as erroneous, which is how incomplete
analysis is reported).

## Limits

| Limit | Value | Why |
| --- | --- | --- |
| class file size | 64 MiB | the container's limit; the largest class seen is 298,424 bytes |
| array dimensions | 255 | JVMS 4.4.1 |
| annotation nesting | 64 | stack safety; real annotations nest a few levels |
| signature nesting | 1,000 | stack safety in callers that recurse over the tree |

Constant pool, member, and attribute counts are u2 values, so they can't exceed 65,535; attribute
lengths are checked against their parent, so a lying length can't make the reader allocate more
than the input's size.
