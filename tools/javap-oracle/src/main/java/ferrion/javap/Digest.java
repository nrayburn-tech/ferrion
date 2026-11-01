package ferrion.javap;

import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.net.URI;
import java.nio.file.FileSystem;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Enumeration;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/**
 * Digests of the bytes the JDK's own public APIs return for each file of a container, for the
 * container checks: the jrt file system for module images, {@link ZipFile} for zip files, and
 * {@link JarFile} with a release for multi-release jars. One JSON object per line, streamed.
 */
final class Digest {
  private Digest() {}

  private static String sha256(InputStream in) throws IOException {
    MessageDigest digest;
    try {
      digest = MessageDigest.getInstance("SHA-256");
    } catch (NoSuchAlgorithmException e) {
      throw new IllegalStateException(e);
    }
    byte[] buffer = new byte[1 << 16];
    for (int n; (n = in.read(buffer)) > 0; ) {
      digest.update(buffer, 0, n);
    }
    StringBuilder hex = new StringBuilder();
    for (byte b : digest.digest()) {
      hex.append(String.format("%02x", b & 0xff));
    }
    return hex.toString();
  }

  private static void line(PrintStream out, String key, String name, String sha256) {
    out.println(
        new Json().beginObject().name(key).value(name).name("sha256").value(sha256).endObject());
  }

  private static void error(PrintStream out, String name, Exception e) {
    out.println(
        new Json().beginObject().name("name").value(name).name("error").value(e.toString())
            .endObject());
  }

  /** Starts a file's lines: {@code {"file": path}}, or {@code {"file": path, "error": ...}}. */
  private static void file(PrintStream out, String file, Exception e) {
    Json json = new Json().beginObject().name("file").value(file);
    if (e != null) {
      json.name("error").value(e.toString());
    }
    out.println(json.endObject());
  }

  /** Every regular file under {@code /modules} of the image at {@code home}, through jrt. */
  static void jrt(String home, PrintStream out) throws IOException {
    try (FileSystem jrt =
            FileSystems.newFileSystem(URI.create("jrt:/"), Map.of("java.home", home));
        Stream<Path> paths = Files.walk(jrt.getPath("/modules"))) {
      for (Path path : (Iterable<Path>) paths.sorted()::iterator) {
        if (!Files.isRegularFile(path)) {
          continue;
        }
        String name = path.toString().substring("/modules".length());
        try (InputStream in = Files.newInputStream(path)) {
          line(out, "path", name, sha256(in));
        }
      }
    }
  }

  /**
   * Every file name of each zip file, with the bytes {@link ZipFile#getEntry} returns for it. Each
   * file's lines follow a {@code {"file": path}} line.
   */
  static void zip(List<String> files, PrintStream out) {
    for (String file : files) {
      try {
        zip(file, out);
      } catch (IOException | RuntimeException e) {
        file(out, file, e);
      }
    }
  }

  private static void zip(String file, PrintStream out) throws IOException {
    try (ZipFile zip = new ZipFile(file)) {
      file(out, file, null);
      Set<String> names = new LinkedHashSet<>();
      for (Enumeration<? extends ZipEntry> e = zip.entries(); e.hasMoreElements(); ) {
        ZipEntry entry = e.nextElement();
        if (!entry.isDirectory()) {
          names.add(entry.getName());
        }
      }
      for (String name : names) {
        ZipEntry entry = zip.getEntry(name);
        try (InputStream in = zip.getInputStream(entry)) {
          line(out, "name", name, sha256(in));
        } catch (IOException | RuntimeException e) {
          error(out, name, e);
        }
      }
    }
  }

  /**
   * Every name of each jar's versioned view for {@code release} ({@link
   * JarFile#versionedStream}), with the bytes of the entry the view gives for it. Each jar's lines
   * follow a {@code {"file": path}} line and one saying whether the jar is multi-release.
   */
  static void jar(String release, List<String> files, PrintStream out) {
    for (String file : files) {
      try {
        jar(file, release, out);
      } catch (IOException | RuntimeException e) {
        file(out, file, e);
      }
    }
  }

  private static void jar(String file, String release, PrintStream out) throws IOException {
    try (JarFile jar =
        new JarFile(new java.io.File(file), false, ZipFile.OPEN_READ, Runtime.Version.parse(release))) {
      file(out, file, null);
      out.println(
          new Json().beginObject().name("multiRelease").value(jar.isMultiRelease()).endObject());
      List<JarEntry> entries =
          jar.versionedStream().filter(e -> !e.isDirectory()).collect(Collectors.toList());
      for (JarEntry entry : entries) {
        try (InputStream in = jar.getInputStream(entry)) {
          line(out, "name", entry.getName(), sha256(in));
        } catch (IOException | RuntimeException e) {
          error(out, entry.getName(), e);
        }
      }
    }
  }
}
