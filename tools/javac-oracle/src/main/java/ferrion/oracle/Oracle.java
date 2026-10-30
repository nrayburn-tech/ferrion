package ferrion.oracle;

import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.Tree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreeScanner;
import com.sun.source.util.Trees;
import com.sun.tools.javac.api.BasicJavacTask;
import com.sun.tools.javac.parser.Scanner;
import com.sun.tools.javac.parser.ScannerFactory;
import com.sun.tools.javac.parser.Tokens.Comment;
import com.sun.tools.javac.parser.Tokens.Token;
import com.sun.tools.javac.parser.Tokens.TokenKind;
import com.sun.tools.javac.util.Log;
import java.io.IOException;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.ToolProvider;

/**
 * Analyzes one source file with javac and returns the results as one line of JSON.
 *
 * <p>Positions are UTF-16 offsets into the file's text, as javac reports them. The output has:
 *
 * <ul>
 *   <li>{@code tokens}: {@code [kind, start, end]} for each token, ending with {@code EOF};
 *   <li>{@code comments}: {@code [style, start, end]} for each comment, in source order;
 *   <li>{@code diagnostics}: everything javac reports while parsing, including language-version
 *       errors;
 *   <li>{@code tree}: {@code [kind, start, end, depth]} for each tree node, in pre-order.
 * </ul>
 */
final class Oracle {
  private Oracle() {}

  static String analyze(Path path, String release) throws IOException {
    String source = Files.readString(path, StandardCharsets.UTF_8);
    JavaFileObject file = new SourceFile(path.toUri(), source);
    DiagnosticCollector<JavaFileObject> collector = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            ToolProvider.getSystemJavaCompiler()
                .getTask(
                    null,
                    null,
                    collector,
                    List.of("--release", release, "-proc:none", "-Xlint:-options"),
                    null,
                    List.of(file));
    Iterable<? extends CompilationUnitTree> units = task.parse();
    List<Diagnostic<? extends JavaFileObject>> diagnostics = List.copyOf(collector.getDiagnostics());

    Json json = new Json();
    json.beginObject();
    json.name("file").value(path.toString());
    json.name("release").value(release);
    writeTokens(json, task, file, source);
    writeDiagnostics(json, diagnostics);
    json.name("tree").beginArray();
    SourcePositions positions = Trees.instance(task).getSourcePositions();
    for (CompilationUnitTree unit : units) {
      new TreeWriter(json, positions, unit).scan(unit, 0);
    }
    json.endArray();
    json.endObject();
    return json.toString();
  }

  private static void writeTokens(Json json, JavacTask task, JavaFileObject file, String source) {
    var context = ((BasicJavacTask) task).getContext();
    Log log = Log.instance(context);
    // The parse above already reported the scanner's errors.
    Log.DiscardDiagnosticHandler discard = log.new DiscardDiagnosticHandler();
    JavaFileObject previous = log.useSource(file);
    List<Comment> comments = new ArrayList<>();
    json.name("tokens").beginArray();
    try {
      Scanner scanner = ScannerFactory.instance(context).newScanner(source, true);
      while (true) {
        scanner.nextToken();
        Token token = scanner.token();
        if (token.comments != null) {
          // javac attaches the comments before a token to it, most recent first.
          comments.addAll(token.comments.reverse());
        }
        json.beginArray().value(token.kind.name()).value(token.pos).value(token.endPos).endArray();
        if (token.kind == TokenKind.EOF) {
          break;
        }
      }
    } finally {
      log.useSource(previous);
      log.popDiagnosticHandler(discard);
    }
    json.endArray();

    json.name("comments").beginArray();
    for (Comment comment : comments) {
      int start = comment.getPos().getStartPosition();
      json.beginArray()
          .value(comment.getStyle().name())
          .value(start)
          .value(commentEnd(source, start))
          .endArray();
    }
    json.endArray();
  }

  /** javac doesn't record where a comment ends, so it's found from the comment's start. */
  private static int commentEnd(String source, int start) {
    if (source.startsWith("/*", start)) {
      int close = source.indexOf("*/", start + 2);
      return close < 0 ? source.length() : close + 2;
    }
    int end = start;
    while (end < source.length() && source.charAt(end) != '\n' && source.charAt(end) != '\r') {
      end++;
    }
    return end;
  }

  private static void writeDiagnostics(
      Json json, List<Diagnostic<? extends JavaFileObject>> diagnostics) {
    json.name("diagnostics").beginArray();
    for (Diagnostic<? extends JavaFileObject> diagnostic : diagnostics) {
      json.beginObject()
          .name("kind")
          .value(diagnostic.getKind().name())
          .name("code")
          .value(diagnostic.getCode())
          .name("start")
          .value(diagnostic.getStartPosition())
          .name("position")
          .value(diagnostic.getPosition())
          .name("end")
          .value(diagnostic.getEndPosition())
          .name("message")
          .value(diagnostic.getMessage(Locale.ROOT))
          .endObject();
    }
    json.endArray();
  }

  private static final class TreeWriter extends TreeScanner<Void, Integer> {
    private final Json json;
    private final SourcePositions positions;
    private final CompilationUnitTree unit;

    TreeWriter(Json json, SourcePositions positions, CompilationUnitTree unit) {
      this.json = json;
      this.positions = positions;
      this.unit = unit;
    }

    @Override
    public Void scan(Tree tree, Integer depth) {
      if (tree == null) {
        return null;
      }
      json.beginArray()
          .value(tree.getKind().name())
          .value(positions.getStartPosition(unit, tree))
          .value(positions.getEndPosition(unit, tree))
          .value(depth)
          .endArray();
      return tree.accept(this, depth + 1);
    }
  }

  private static final class SourceFile extends SimpleJavaFileObject {
    private final String source;

    SourceFile(URI uri, String source) {
      super(uri, Kind.SOURCE);
      this.source = source;
    }

    @Override
    public CharSequence getCharContent(boolean ignoreEncodingErrors) {
      return source;
    }
  }
}
