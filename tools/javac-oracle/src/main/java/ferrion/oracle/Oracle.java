package ferrion.oracle;

import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.Tree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreeScanner;
import com.sun.source.util.Trees;
import com.sun.tools.javac.api.BasicJavacTask;
import com.sun.tools.javac.api.ClientCodeWrapper;
import com.sun.tools.javac.parser.Scanner;
import com.sun.tools.javac.parser.ScannerFactory;
import com.sun.tools.javac.parser.Tokens.Comment;
import com.sun.tools.javac.parser.Tokens.Token;
import com.sun.tools.javac.parser.Tokens.TokenKind;
import com.sun.tools.javac.tree.JCTree;
import com.sun.tools.javac.util.JCDiagnostic;
import com.sun.tools.javac.util.Log;
import java.io.IOException;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.EnumSet;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticListener;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.ToolProvider;

/**
 * Analyzes one source file with javac and returns the results as one line of JSON.
 *
 * <p>Positions are UTF-16 offsets into the file's text, as javac reports them. The output has the
 * {@link Section}s that were asked for:
 *
 * <ul>
 *   <li>{@code tokens}: {@code [kind, start, end]} for each token, ending with {@code EOF};
 *   <li>{@code comments}: {@code [style, start, end]} for each comment, in source order;
 *   <li>{@code scanDiagnostics}: what javac's scanner reports while producing the tokens;
 *   <li>{@code diagnostics}: everything javac reports while parsing, including language-version
 *       errors;
 *   <li>{@code tree}: {@code [kind, start, end, depth, preferred, shared]} for each tree node, in
 *       the pre-order of {@link TreeScanner}. {@code preferred} is the position javac reports
 *       diagnostics about the node at (a declaration's name, a binary operator), and {@code shared}
 *       is 1 when the same node was already written under another parent (the type in {@code int
 *       a, b;}), 0 otherwise. A position javac doesn't have is -1.
 * </ul>
 *
 * <p>Each diagnostic has javac's {@code kind} ({@code ERROR}, {@code WARNING}, ...), its {@code
 * code} ({@code compiler.err.expected}), its {@code start}, {@code position}, and {@code end}, its
 * {@code args}, and its English {@code message}. An argument that is itself a diagnostic, such as
 * the feature in "records are not supported in -source 8", is written as its code ({@code
 * compiler.misc.feature.records}); other arguments are written as text.
 *
 * <p>Tokens, comments, and scan diagnostics come from scanning the file on its own, so they don't
 * depend on how the parser recovers from errors. The file is only parsed, in a javac task of its
 * own, when diagnostics or the tree are asked for.
 */
final class Oracle {
  private Oracle() {}

  /** The parts of the output that can be asked for, by their JSON names. */
  enum Section {
    TOKENS("tokens"),
    COMMENTS("comments"),
    SCAN_DIAGNOSTICS("scanDiagnostics"),
    DIAGNOSTICS("diagnostics"),
    TREE("tree");

    final String json;

    Section(String json) {
      this.json = json;
    }

    static Section named(String name) {
      for (Section section : values()) {
        if (section.json.equals(name)) {
          return section;
        }
      }
      return null;
    }
  }

  static String analyze(Path path, String release, Set<Section> sections) throws IOException {
    String source = Files.readString(path, StandardCharsets.UTF_8);
    JavaFileObject file = new SourceFile(path.toUri(), source);
    Json json = new Json();
    json.beginObject();
    json.name("file").value(path.toString());
    json.name("release").value(release);
    if (sections.contains(Section.TOKENS)
        || sections.contains(Section.COMMENTS)
        || sections.contains(Section.SCAN_DIAGNOSTICS)) {
      writeScan(json, scanTask(release), file, source, sections);
    }
    if (sections.contains(Section.DIAGNOSTICS) || sections.contains(Section.TREE)) {
      Collector collector = new Collector();
      JavacTask task = newTask(List.of(file), release, collector);
      Iterable<? extends CompilationUnitTree> units = task.parse();
      if (sections.contains(Section.DIAGNOSTICS)) {
        json.name(Section.DIAGNOSTICS.json).beginArray();
        for (Diagnostic<? extends JavaFileObject> diagnostic : collector.diagnostics) {
          writeDiagnostic(json, diagnostic);
        }
        json.endArray();
      }
      if (sections.contains(Section.TREE)) {
        json.name(Section.TREE.json).beginArray();
        SourcePositions positions = Trees.instance(task).getSourcePositions();
        for (CompilationUnitTree unit : units) {
          new TreeWriter(json, positions, unit).scan(unit, 0);
        }
        json.endArray();
      }
    }
    json.endObject();
    return json.toString();
  }

  static Set<Section> allSections() {
    return EnumSet.allOf(Section.class);
  }

  private static final Map<String, JavacTask> SCAN_TASKS = new HashMap<>();

  /**
   * The task files are scanned with, one per release. Setting up a task for a release takes longer
   * than scanning most files, and scanning leaves nothing behind in the task: each scan has its own
   * diagnostic handler. It's a task of its own, so the parse in a file's own task still reports the
   * scanner's errors.
   */
  private static JavacTask scanTask(String release) {
    return SCAN_TASKS.computeIfAbsent(release, r -> newTask(List.of(), r, new Collector()));
  }

  private static JavacTask newTask(
      List<JavaFileObject> files, String release, Collector collector) {
    return (JavacTask)
        ToolProvider.getSystemJavaCompiler()
            .getTask(
                null,
                null,
                collector,
                List.of("--release", release, "-proc:none", "-Xlint:-options"),
                null,
                files);
  }

  private static void writeScan(
      Json json, JavacTask task, JavaFileObject file, String source, Set<Section> sections) {
    var context = ((BasicJavacTask) task).getContext();
    Log log = Log.instance(context);
    Log.DeferredDiagnosticHandler deferred = log.new DeferredDiagnosticHandler();
    JavaFileObject previous = log.useSource(file);
    List<Token> tokens = new ArrayList<>();
    List<Comment> comments = new ArrayList<>();
    try {
      Scanner scanner = ScannerFactory.instance(context).newScanner(source, true);
      while (true) {
        scanner.nextToken();
        Token token = scanner.token();
        if (token.comments != null) {
          // javac attaches the comments before a token to it, most recent first.
          comments.addAll(token.comments.reverse());
        }
        tokens.add(token);
        if (token.kind == TokenKind.EOF) {
          break;
        }
      }
    } finally {
      log.useSource(previous);
      log.popDiagnosticHandler(deferred);
    }

    if (sections.contains(Section.TOKENS)) {
      json.name(Section.TOKENS.json).beginArray();
      for (Token token : tokens) {
        json.beginArray().value(token.kind.name()).value(token.pos).value(token.endPos).endArray();
      }
      json.endArray();
    }
    if (sections.contains(Section.COMMENTS)) {
      json.name(Section.COMMENTS.json).beginArray();
      for (Comment comment : comments) {
        json.beginArray()
            .value(comment.getStyle().name())
            .value(comment.getPos().getStartPosition())
            .value(comment.getPos().getEndPosition())
            .endArray();
      }
      json.endArray();
    }
    if (sections.contains(Section.SCAN_DIAGNOSTICS)) {
      json.name(Section.SCAN_DIAGNOSTICS.json).beginArray();
      for (JCDiagnostic diagnostic : deferred.getDiagnostics()) {
        writeDiagnostic(json, diagnostic);
      }
      json.endArray();
    }
  }

  private static void writeDiagnostic(Json json, Diagnostic<?> diagnostic) {
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
        .name("args")
        .beginArray();
    if (diagnostic instanceof JCDiagnostic jc) {
      for (Object arg : jc.getArgs()) {
        json.value(arg instanceof JCDiagnostic nested ? nested.getCode() : String.valueOf(arg));
      }
    }
    json.endArray()
        .name("message")
        .value(diagnostic.getMessage(Locale.ROOT))
        .endObject();
  }

  private static final class TreeWriter extends TreeScanner<Void, Integer> {
    private final Json json;
    private final SourcePositions positions;
    private final CompilationUnitTree unit;
    private final Set<Tree> written = Collections.newSetFromMap(new IdentityHashMap<>());

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
          .value(((JCTree) tree).getPreferredPosition())
          .value(written.add(tree) ? 0 : 1)
          .endArray();
      return tree.accept(this, depth + 1);
    }
  }

  /**
   * Collects what javac reports. It's trusted, so javac gives it its own diagnostics rather than
   * wrappers, whose arguments can't be read.
   */
  @ClientCodeWrapper.Trusted
  private static final class Collector implements DiagnosticListener<JavaFileObject> {
    final List<Diagnostic<? extends JavaFileObject>> diagnostics = new ArrayList<>();

    @Override
    public void report(Diagnostic<? extends JavaFileObject> diagnostic) {
      diagnostics.add(diagnostic);
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
