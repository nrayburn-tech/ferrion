package ferrion.oracle;

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Dumps what javac produces for each Java source file, one JSON object per line.
 *
 * <p>Usage: {@code javac-oracle --release <N> <file | @list-file>...}. A list file holds one path per
 * line. Each file is analyzed on its own, so a javac failure on one file is reported for that file
 * and the rest still run.
 */
public final class Main {
  private Main() {}

  public static void main(String[] args) throws IOException {
    String release = null;
    List<Path> files = new ArrayList<>();
    for (int i = 0; i < args.length; i++) {
      String arg = args[i];
      if (arg.equals("--release") && i + 1 < args.length) {
        release = args[++i];
      } else if (arg.startsWith("@")) {
        for (String line : Files.readAllLines(Path.of(arg.substring(1)), StandardCharsets.UTF_8)) {
          if (!line.isBlank()) {
            files.add(Path.of(line.strip()));
          }
        }
      } else if (arg.startsWith("-")) {
        usage("unknown option: " + arg);
      } else {
        files.add(Path.of(arg));
      }
    }
    if (release == null) {
      usage("--release is required");
    }
    if (files.isEmpty()) {
      usage("no input files");
    }

    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    for (Path file : files) {
      String line;
      try {
        line = Oracle.analyze(file, release);
      } catch (IOException e) {
        line = failure(file, "error", e.toString());
      } catch (Throwable t) {
        line = failure(file, "crash", t.toString());
      }
      out.println(line);
    }
    out.flush();
  }

  private static String failure(Path file, String key, String message) {
    Json json = new Json();
    json.beginObject().name("file").value(file.toString()).name(key).value(message).endObject();
    return json.toString();
  }

  private static void usage(String message) {
    System.err.println("javac-oracle: " + message);
    System.err.println("usage: javac-oracle --release <N> <file | @list-file>...");
    System.exit(2);
  }
}
