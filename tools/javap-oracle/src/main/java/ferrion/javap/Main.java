package ferrion.javap;

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * The oracle for Ferrion's class file reading (docs/design/classfile.md, "Verification"). Each
 * mode writes one JSON object per line to standard output:
 *
 * <ul>
 *   <li>{@code javap <file | @list-file>...}: {@code javap -v -p}'s output for each class file,
 *       normalized (see {@link Javap}): {@code {"file", "lines"}} or {@code {"file", "error"}};
 *   <li>{@code jrt <java.home>}: a SHA-256 of every file the jrt file system shows for the image
 *       of the JDK at {@code java.home};
 *   <li>{@code zip <file | @list-file>...}: a SHA-256 of each file name's entry, through {@code
 *       ZipFile};
 *   <li>{@code jar <release> <file | @list-file>...}: the same through {@code JarFile}'s
 *       versioned view at the release;
 *   <li>{@code visibility <release> [<package> | @list-file]...}: the modules, classes, and
 *       visible packages javac sees with {@code --release}, on the JDK running the harness (see
 *       {@link Visibility}); the packages are looked up for releases without modules.
 * </ul>
 */
public final class Main {
  private Main() {}

  /** How many class files one javap run reads. */
  private static final int BATCH = 200;

  private static final String USAGE =
      "usage: javap-oracle javap <file | @list-file>...\n"
          + "       javap-oracle jrt <java.home>\n"
          + "       javap-oracle zip <file | @list-file>...\n"
          + "       javap-oracle jar <release> <file | @list-file>...\n"
          + "       javap-oracle visibility <release> [<package> | @list-file]...";

  public static void main(String[] args) throws IOException {
    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    if (args.length == 0) {
      usage();
    }
    switch (args[0]) {
      case "javap":
        javap(files(args), out);
        break;
      case "jrt":
        require(args, 2);
        Digest.jrt(args[1], out);
        break;
      case "zip":
        Digest.zip(files(args), out);
        break;
      case "jar":
        if (args.length < 2) {
          usage();
        }
        Digest.jar(args[1], files(java.util.Arrays.copyOfRange(args, 1, args.length)), out);
        break;
      case "visibility":
        if (args.length < 2) {
          usage();
        }
        Visibility.run(args[1], files(java.util.Arrays.copyOfRange(args, 1, args.length)), out);
        break;
      default:
        usage();
    }
    out.flush();
  }

  private static void require(String[] args, int count) {
    if (args.length != count) {
      usage();
    }
  }

  private static void usage() {
    System.err.println(USAGE);
    System.exit(2);
  }

  private static List<String> files(String[] args) throws IOException {
    List<String> files = new ArrayList<>();
    for (int i = 1; i < args.length; i++) {
      if (args[i].startsWith("@")) {
        for (String line :
            Files.readAllLines(Path.of(args[i].substring(1)), StandardCharsets.UTF_8)) {
          if (!line.isBlank()) {
            files.add(line.strip());
          }
        }
      } else {
        files.add(args[i]);
      }
    }
    return files;
  }

  private static void javap(List<String> files, PrintStream out) {
    for (int start = 0; start < files.size(); start += BATCH) {
      List<String> batch = files.subList(start, Math.min(files.size(), start + BATCH));
      for (Map.Entry<String, Javap.Result> result : Javap.run(batch).entrySet()) {
        Json json = new Json().beginObject().name("file").value(result.getKey());
        if (result.getValue().error != null) {
          json.name("error").value(result.getValue().error);
        }
        if (result.getValue().lines != null) {
          json.name("lines").beginArray();
          for (String line : result.getValue().lines) {
            json.value(line);
          }
          json.endArray();
        }
        out.println(json.endObject());
      }
      out.flush();
    }
  }
}
