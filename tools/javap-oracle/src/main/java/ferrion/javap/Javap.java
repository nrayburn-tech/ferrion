package ferrion.javap;

import java.io.PrintWriter;
import java.io.StringWriter;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.spi.ToolProvider;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Runs {@code javap -v -p} in process, through {@link ToolProvider}, and normalizes its output for
 * comparison with Ferrion's model.
 *
 * <p>The normalization keeps everything {@code javap} prints about the parts Ferrion reads, and
 * drops what Ferrion doesn't compare:
 *
 * <ul>
 *   <li>the header lines about the file ({@code Classfile}, {@code Last modified}, checksums,
 *       {@code Compiled from}), since {@code SourceFile} is compared instead;
 *   <li>the constant pool listing, which is compared through the entries it resolves to;
 *   <li>{@code Code} and every line inside it, and the entries of {@code BootstrapMethods} and
 *       {@code SourceDebugExtension}, whose headers stay;
 *   <li>constant pool indexes: a reference followed by a {@code //} comment becomes the comment,
 *       and a raw annotation ({@code #49(#50=I#51)}) is dropped, since the readable form follows;
 *   <li>blank lines and trailing spaces.
 * </ul>
 */
final class Javap {
  private Javap() {}

  private static final ToolProvider JAVAP =
      ToolProvider.findFirst("javap").orElseThrow(() -> new IllegalStateException("no javap"));

  /** The result for one class file: its normalized lines, an error, or both. */
  static final class Result {
    final List<String> lines;
    final String error;

    Result(List<String> lines, String error) {
      this.lines = lines;
      this.error = error;
    }
  }

  /** Runs {@code javap} on {@code files} at once, falling back to one at a time on errors. */
  static Map<String, Result> run(List<String> files) {
    Map<String, Result> results = new LinkedHashMap<>();
    Output output = invoke(files);
    if (output.status == 0 && output.err.isEmpty()) {
      Map<String, List<String>> blocks = split(output.out);
      if (blocks.size() == files.size() && blocks.keySet().containsAll(files)) {
        for (String file : files) {
          results.put(file, new Result(normalize(blocks.get(file)), null));
        }
        return results;
      }
    }
    for (String file : files) {
      Output single = invoke(List.of(file));
      Map<String, List<String>> blocks = split(single.out);
      String error = single.err.strip();
      String status = "javap exited with status " + single.status + (error.isEmpty() ? "" : ": " + error);
      if (blocks.size() == 1) {
        // javap can print a class and still fail, after reporting errors among its lines.
        results.put(
            file,
            new Result(
                normalize(blocks.values().iterator().next()), single.status == 0 ? null : status));
      } else {
        String message = error.isEmpty() ? single.out.strip() : error;
        results.put(file, new Result(null, message.isEmpty() ? status : message));
      }
    }
    return results;
  }

  private static final class Output {
    final int status;
    final String out;
    final String err;

    Output(int status, String out, String err) {
      this.status = status;
      this.out = out;
      this.err = err;
    }
  }

  private static Output invoke(List<String> files) {
    StringWriter out = new StringWriter();
    StringWriter err = new StringWriter();
    List<String> args = new ArrayList<>(List.of("-v", "-p"));
    args.addAll(files);
    int status;
    try (PrintWriter o = new PrintWriter(out);
        PrintWriter e = new PrintWriter(err)) {
      status = JAVAP.run(o, e, args.toArray(new String[0]));
    } catch (RuntimeException ex) {
      return new Output(1, out.toString(), err + ex.toString());
    }
    return new Output(status, out.toString(), err.toString());
  }

  /** Splits javap's output into one block of lines per class, by its "Classfile" line. */
  private static Map<String, List<String>> split(String out) {
    Map<String, List<String>> blocks = new LinkedHashMap<>();
    List<String> current = null;
    for (String line : out.split("\r?\n", -1)) {
      if (line.startsWith("Classfile ")) {
        current = new ArrayList<>();
        blocks.put(line.substring("Classfile ".length()), current);
      } else if (current != null) {
        current.add(line);
      }
    }
    return blocks;
  }

  private static final Pattern RAW_ANNOTATION = Pattern.compile("^(\\s*\\d+: )#\\d+\\(");
  private static final Pattern DEFAULT_VALUE = Pattern.compile("^(\\s*default_value:) .*$");
  private static final Pattern REFERENCES =
      Pattern.compile("^(#\\d+|[0-9a-f]+|[,.;:=]|of|\\s)+$");

  static List<String> normalize(List<String> block) {
    List<String> lines = new ArrayList<>();
    boolean constantPool = false;
    int skipDeeperThan = -1;
    for (String raw : block) {
      String line = stripTrailing(raw);
      if (line.isEmpty()) {
        continue;
      }
      int indent = indent(line);
      String text = line.substring(indent);
      if (constantPool) {
        if (indent > 0) {
          continue;
        }
        constantPool = false;
      }
      if (skipDeeperThan >= 0) {
        if (indent > skipDeeperThan) {
          continue;
        }
        skipDeeperThan = -1;
      }
      if (indent == 2
          && (text.startsWith("Last modified ")
              || text.startsWith("SHA-256 checksum ")
              || text.startsWith("MD5 checksum ")
              || text.startsWith("Compiled from "))) {
        continue;
      }
      if (indent == 0 && text.equals("Constant pool:")) {
        constantPool = true;
        continue;
      }
      if (text.equals("Code:")) {
        skipDeeperThan = indent;
        continue;
      }
      if (text.equals("BootstrapMethods:") || text.equals("SourceDebugExtension:")) {
        skipDeeperThan = indent;
        lines.add(line);
        continue;
      }
      lines.add(stripReferences(line, indent, text));
    }
    return lines;
  }

  /** Replaces constant pool references with what they resolve to. */
  static String stripReferences(String line, int indent, String text) {
    String prefix = line.substring(0, indent);
    Matcher raw = RAW_ANNOTATION.matcher(line);
    if (raw.find()) {
      int end = matchingParen(line, raw.end() - 1);
      String rest = end < 0 ? "" : line.substring(end + 1);
      if (rest.startsWith(": ")) {
        rest = rest.substring(2);
      }
      return stripTrailing(raw.group(1) + rest);
    }
    Matcher defaultValue = DEFAULT_VALUE.matcher(line);
    if (defaultValue.matches()) {
      return defaultValue.group(1);
    }
    if (text.startsWith("ConstantValue: ")) {
      return line;
    }
    int hash = text.indexOf('#');
    if (hash < 0 || hash + 1 >= text.length() || !Character.isDigit(text.charAt(hash + 1))) {
      return line;
    }
    int comment = text.indexOf("//", hash);
    String references = comment < 0 ? text.substring(hash) : text.substring(hash, comment);
    if (!REFERENCES.matcher(references).matches()) {
      return line;
    }
    if (comment < 0) {
      return line;
    }
    String resolved = text.substring(comment + 2);
    if (resolved.startsWith(" ")) {
      resolved = resolved.substring(1);
    }
    return stripTrailing(prefix + text.substring(0, hash) + resolved);
  }

  private static int matchingParen(String line, int open) {
    int depth = 0;
    for (int i = open; i < line.length(); i++) {
      char c = line.charAt(i);
      if (c == '(') {
        depth++;
      } else if (c == ')') {
        depth--;
        if (depth == 0) {
          return i;
        }
      }
    }
    return -1;
  }

  private static int indent(String line) {
    int i = 0;
    while (i < line.length() && line.charAt(i) == ' ') {
      i++;
    }
    return i;
  }

  private static String stripTrailing(String line) {
    int end = line.length();
    while (end > 0 && line.charAt(end - 1) == ' ') {
      end--;
    }
    return line.substring(0, end);
  }
}
