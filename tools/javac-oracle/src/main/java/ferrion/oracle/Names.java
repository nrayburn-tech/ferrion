package ferrion.oracle;

import com.sun.source.tree.AnnotationTree;
import com.sun.source.tree.AssignmentTree;
import com.sun.source.tree.ClassTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.MemberReferenceTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodInvocationTree;
import com.sun.source.tree.NewClassTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TaskEvent;
import com.sun.source.util.TaskListener;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import com.sun.tools.javac.api.ClientCodeWrapper;
import com.sun.tools.javac.tree.JCTree;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import javax.lang.model.element.AnnotationMirror;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.ModuleElement;
import javax.lang.model.element.NestingKind;
import javax.lang.model.element.PackageElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.TypeParameterElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.ArrayType;
import javax.lang.model.type.DeclaredType;
import javax.lang.model.type.TypeKind;
import javax.lang.model.type.TypeMirror;
import javax.lang.model.util.Elements;
import javax.lang.model.util.Types;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticListener;
import javax.tools.FileObject;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;

/**
 * The {@code names} command: compiles one source set with javac and writes, for each of its files,
 * javac's errors and what every name resolves to.
 *
 * <p>Usage: {@code javac-oracle names --release <N> [--roots <paths>] [--classpath <paths>]
 * [--sourcepath <paths>] [--processor-path <paths>] [--generated <dir>] [--base <dir>] <file |
 * @list-file>...}. Path lists are separated by the platform's path separator.
 *
 * <ul>
 *   <li>Every file is on javac's command line, so the source set's own types always come from
 *       source. {@code --sourcepath} has the source roots of the source sets it depends on, read
 *       with {@code -implicit:none -Xprefer:source}; {@code --classpath} has its jars and
 *       directories.
 *   <li>With {@code --generated}, annotation processors run ({@code -proc:full}), found on {@code
 *       --processor-path} or else on the classpath, as javac finds them, and write their sources
 *       there. Class files and resources go to a temporary directory that is deleted afterwards, so
 *       nothing else is written. Without it, no processor runs.
 *   <li>{@code module-info.java} is left out, and hidden on the source path, so the source set is
 *       compiled in the unnamed module with its dependencies on the classpath.
 *   <li>{@code analyze()} attributes every file. If javac throws, each file is compiled again on
 *       its own, with the source set's roots and generated sources on the source path and no
 *       processors, so a crash loses only the files it happens in.
 * </ul>
 *
 * <p>The output is one JSON object per line: one per {@code module-info.java}, with {@code
 * skipped}, then one per other file in command-line order, then one per generated file, then a
 * last one for the source set. Paths under {@code --base} are written
 * relative to it, with {@code /}. Offsets are UTF-16, as javac's are, and -1 where javac has none.
 * Each file's object has:
 *
 * <ul>
 *   <li>{@code file}, and {@code generated: 1} for a file a processor wrote;
 *   <li>{@code diagnostics}: javac's errors in the file, as the other commands write them;
 *   <li>{@code lombok}: {@code [start, end]} of each class that has, or has a member with, an
 *       annotation from a {@code lombok} package;
 *   <li>{@code names}: {@code [start, end, context, qualifier, key]} for each identifier and member
 *       select tree, in the pre-order of {@link TreePathScanner}, with a sixth element for method
 *       names, the type whose members are searched for the method (or {@code ""}). See {@link
 *       #context}, {@link #qualifier}, and {@link Keys}.
 *   <li>{@code crash}, instead of the rest, when javac threw on the file.
 * </ul>
 *
 * <p>The last object is {@code {"sourceSet": {...}}} with {@code diagnostics} (errors with no file
 * or in a file not compiled, such as one on the source path), {@code processors} (the processor
 * classes registered in the processor path), {@code crash} when the whole run threw and files were
 * compiled one at a time, and {@code millis}.
 */
final class Names {
  private Names() {}

  private static final String USAGE =
      "usage: javac-oracle names --release <N> [--roots <paths>] [--classpath <paths>]"
          + " [--sourcepath <paths>] [--processor-path <paths>] [--generated <dir>]"
          + " [--base <dir>] <file | @list-file>...";

  /** What a run is given. */
  static final class Request {
    String release;
    List<Path> roots = new ArrayList<>();
    List<Path> classpath = new ArrayList<>();
    List<Path> sourcepath = new ArrayList<>();
    List<Path> processorPath;
    Path generated;
    Path base;
    List<Path> files = new ArrayList<>();
  }

  static void main(List<String> args, PrintStream out) throws IOException {
    Request request = parse(args);
    long start = System.nanoTime();
    Json trailer = new Json();
    trailer.beginObject().name("sourceSet").beginObject();
    try {
      new Run(request, out, trailer).all();
    } finally {
      trailer
          .name("millis")
          .value((System.nanoTime() - start) / 1_000_000)
          .endObject()
          .endObject();
      out.println(trailer);
      out.flush();
    }
  }

  private static Request parse(List<String> args) throws IOException {
    Request request = new Request();
    for (int i = 0; i < args.size(); i++) {
      String arg = args.get(i);
      if (arg.startsWith("--")) {
        if (i + 1 >= args.size()) {
          usage(arg + " needs a value");
        }
        String value = args.get(++i);
        switch (arg) {
          case "--release" -> request.release = value;
          case "--roots" -> request.roots = paths(value);
          case "--classpath" -> request.classpath = paths(value);
          case "--sourcepath" -> request.sourcepath = paths(value);
          case "--processor-path" -> request.processorPath = paths(value);
          case "--generated" -> request.generated = Path.of(value);
          case "--base" -> request.base = Path.of(value);
          default -> usage("unknown option: " + arg);
        }
      } else if (arg.startsWith("@")) {
        for (String line : Files.readAllLines(Path.of(arg.substring(1)), StandardCharsets.UTF_8)) {
          if (!line.isBlank()) {
            request.files.add(Path.of(line.strip()));
          }
        }
      } else if (arg.startsWith("-")) {
        usage("unknown option: " + arg);
      } else {
        request.files.add(Path.of(arg));
      }
    }
    if (request.release == null) {
      usage("--release is required");
    }
    if (request.files.isEmpty()) {
      usage("no input files");
    }
    return request;
  }

  private static List<Path> paths(String list) {
    List<Path> paths = new ArrayList<>();
    for (String path : list.split(File.pathSeparator)) {
      if (!path.isEmpty()) {
        paths.add(Path.of(path));
      }
    }
    return paths;
  }

  private static void usage(String message) {
    System.err.println("javac-oracle: " + message);
    System.err.println(USAGE);
    System.exit(2);
  }

  /** One run over a source set. */
  private static final class Run {
    private final Request request;
    private final PrintStream out;
    private final Json trailer;
    private final JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();

    Run(Request request, PrintStream out, Json trailer) {
      this.request = request;
      this.out = out;
      this.trailer = trailer;
    }

    void all() throws IOException {
      List<Path> files = new ArrayList<>();
      for (Path file : request.files) {
        if (file.getFileName().toString().equals("module-info.java")) {
          Json json = new Json();
          json.beginObject()
              .name("file")
              .value(relative(file))
              .name("skipped")
              .value("module declaration")
              .endObject();
          out.println(json);
        } else {
          files.add(file);
        }
      }
      trailer.name("processors").beginArray();
      for (String processor : processors()) {
        trailer.value(processor);
      }
      trailer.endArray();
      if (files.isEmpty()) {
        // Only a module declaration: javac would refuse to run with no files.
        trailer.name("diagnostics").beginArray().endArray();
        return;
      }
      Path classes = Files.createTempDirectory("javac-oracle-names");
      try {
        Compiled compiled;
        try {
          compiled = compile(files, request.sourcepath, request.generated, classes);
        } catch (Throwable t) {
          trailer.name("crash").value(t.toString());
          compiled = null;
        }
        if (compiled != null) {
          compiled.write(files);
          compiled.writeOther(trailer);
        } else {
          oneAtATime(files, classes);
        }
      } finally {
        deleteTree(classes);
      }
    }

    /** After a crash: each file on its own, with the rest on the source path. */
    private void oneAtATime(List<Path> files, Path classes) {
      List<Path> sourcepath = new ArrayList<>(request.roots);
      if (request.generated != null && Files.isDirectory(request.generated)) {
        sourcepath.add(request.generated);
      }
      sourcepath.addAll(request.sourcepath);
      trailer.name("diagnostics").beginArray();
      for (Path file : files) {
        try {
          Compiled compiled = compile(List.of(file), sourcepath, null, classes);
          compiled.write(List.of(file));
          compiled.writeOtherDiagnostics(trailer);
        } catch (Throwable t) {
          Json json = new Json();
          json.beginObject()
              .name("file")
              .value(relative(file))
              .name("crash")
              .value(t.toString())
              .endObject();
          out.println(json);
        }
      }
      trailer.endArray();
    }

    private Compiled compile(
        List<Path> files, List<Path> sourcepath, Path generated, Path classes) throws IOException {
      Collector collector = new Collector();
      StandardJavaFileManager standard =
          compiler.getStandardFileManager(collector, null, StandardCharsets.UTF_8);
      try (HidingFileManager manager = new HidingFileManager(standard)) {
        List<String> options = new ArrayList<>();
        Collections.addAll(
            options,
            "--release",
            request.release,
            "-encoding",
            "UTF-8",
            "-implicit:none",
            "-Xprefer:source",
            "-Xmaxerrs",
            String.valueOf(Integer.MAX_VALUE),
            "-nowarn",
            "-Xlint:none",
            "-d",
            classes.toString());
        options.add("-classpath");
        options.add(joined(request.classpath));
        // Given even when empty: without it, javac looks for sources on the classpath.
        options.add("-sourcepath");
        options.add(joined(sourcepath));
        if (generated != null) {
          Files.createDirectories(generated);
          options.add("-proc:full");
          options.add("-s");
          options.add(generated.toString());
          if (request.processorPath != null) {
            options.add("-processorpath");
            options.add(joined(request.processorPath));
          }
        } else {
          options.add("-proc:none");
        }
        Iterable<? extends JavaFileObject> units =
            standard.getJavaFileObjectsFromPaths(files);
        JavacTask task =
            (JavacTask) compiler.getTask(null, manager, collector, options, null, units);
        // Every compilation unit javac parses, generated ones included, in order.
        Map<URI, CompilationUnitTree> parsed = new LinkedHashMap<>();
        // The units javac started analyzing a class of: attribution comes first, and flow
        // analysis, which ends the event, is skipped once there are errors.
        Set<URI> analyzed = new HashSet<>();
        task.addTaskListener(
            new TaskListener() {
              @Override
              public void started(TaskEvent event) {
                if (event.getKind() == TaskEvent.Kind.ANALYZE) {
                  analyzed.add(event.getSourceFile().toUri());
                }
              }

              @Override
              public void finished(TaskEvent event) {
                if (event.getKind() == TaskEvent.Kind.PARSE) {
                  parsed.put(event.getSourceFile().toUri(), event.getCompilationUnit());
                }
              }
            });
        task.analyze();
        return new Compiled(task, parsed, analyzed, collector, generated);
      }
    }

    /** What one compilation produced. */
    private final class Compiled {
      private final Trees trees;
      private final Keys keys;
      private final Map<URI, CompilationUnitTree> parsed;
      private final Set<URI> analyzed;
      private final Map<URI, List<Diagnostic<? extends JavaFileObject>>> errors =
          new LinkedHashMap<>();
      private final List<Diagnostic<? extends JavaFileObject>> otherErrors = new ArrayList<>();
      private final Path generated;
      private final Set<URI> written = new HashSet<>();

      Compiled(
          JavacTask task,
          Map<URI, CompilationUnitTree> parsed,
          Set<URI> analyzed,
          Collector collector,
          Path generated) {
        this.trees = Trees.instance(task);
        this.keys = new Keys(task.getElements(), task.getTypes(), this::relative);
        this.parsed = parsed;
        this.analyzed = analyzed;
        this.generated = generated;
        for (Diagnostic<? extends JavaFileObject> diagnostic : collector.diagnostics) {
          if (diagnostic.getKind() != Diagnostic.Kind.ERROR) {
            continue;
          }
          JavaFileObject source = diagnostic.getSource();
          if (source == null) {
            otherErrors.add(diagnostic);
          } else {
            errors.computeIfAbsent(source.toUri(), u -> new ArrayList<>()).add(diagnostic);
          }
        }
      }

      private String relative(FileObject file) {
        return Run.this.relative(Path.of(file.toUri()));
      }

      /** Writes `files`, then the generated files. */
      void write(List<Path> files) {
        for (Path file : files) {
          writeFile(file.toUri(), Run.this.relative(file), false);
        }
        if (generated == null) {
          return;
        }
        Path root = generated.toAbsolutePath().normalize();
        for (Map.Entry<URI, CompilationUnitTree> unit : parsed.entrySet()) {
          URI uri = unit.getKey();
          if (!written.contains(uri)
              && uri.getScheme().equals("file")
              && Path.of(uri).normalize().startsWith(root)) {
            writeFile(uri, Run.this.relative(Path.of(uri)), true);
          }
        }
      }

      private void writeFile(URI uri, String path, boolean isGenerated) {
        written.add(uri);
        Json json = new Json();
        json.beginObject().name("file").value(path);
        if (isGenerated) {
          json.name("generated").value(1);
        }
        CompilationUnitTree unit = findUnit(uri);
        // A file with no classes, such as one commented out whole, has nothing to analyze.
        if (unit != null && !unit.getTypeDecls().isEmpty() && !analyzed.contains(uri)) {
          json.name("unanalyzed").value(1);
        }
        json.name("diagnostics").beginArray();
        for (Diagnostic<? extends JavaFileObject> diagnostic :
            errors.getOrDefault(uri, List.of())) {
          Oracle.writeDiagnostic(json, diagnostic);
        }
        json.endArray();
        if (unit == null) {
          json.name("crash").value("javac didn't parse the file");
        } else {
          FileNames names = new FileNames(trees, keys, unit, path);
          names.collect();
          json.name("lombok").beginArray();
          for (long[] range : names.lombok) {
            json.beginArray().value(range[0]).value(range[1]).endArray();
          }
          json.endArray();
          json.name("names").beginArray();
          for (String[] name : names.names) {
            json.beginArray();
            json.value(Long.parseLong(name[0])).value(Long.parseLong(name[1]));
            for (int i = 2; i < name.length; i++) {
              json.value(name[i]);
            }
            json.endArray();
          }
          json.endArray();
        }
        json.endObject();
        out.println(json);
      }

      private CompilationUnitTree findUnit(URI uri) {
        CompilationUnitTree unit = parsed.get(uri);
        if (unit != null) {
          return unit;
        }
        // The same file can be named by a URI that differs in form.
        Path path = Path.of(uri).normalize();
        for (Map.Entry<URI, CompilationUnitTree> entry : parsed.entrySet()) {
          if (entry.getKey().getScheme().equals("file")
              && Path.of(entry.getKey()).normalize().equals(path)) {
            return entry.getValue();
          }
        }
        return null;
      }

      void writeOther(Json trailer) {
        trailer.name("diagnostics").beginArray();
        writeOtherDiagnostics(trailer);
        trailer.endArray();
      }

      /** Errors with no file, or in a file that wasn't written: on the source path. */
      void writeOtherDiagnostics(Json trailer) {
        for (Diagnostic<? extends JavaFileObject> diagnostic : otherErrors) {
          Oracle.writeDiagnostic(trailer, diagnostic);
        }
        for (Map.Entry<URI, List<Diagnostic<? extends JavaFileObject>>> entry :
            errors.entrySet()) {
          if (written.contains(entry.getKey())) {
            continue;
          }
          for (Diagnostic<? extends JavaFileObject> diagnostic : entry.getValue()) {
            trailer.beginObject().name("file").value(relativeUri(entry.getKey())).name("error");
            Oracle.writeDiagnostic(trailer, diagnostic);
            trailer.endObject();
          }
        }
      }

      private String relativeUri(URI uri) {
        return uri.getScheme().equals("file") ? Run.this.relative(Path.of(uri)) : uri.toString();
      }
    }

    String relative(Path path) {
      Path absolute = path.toAbsolutePath().normalize();
      if (request.base != null) {
        Path base = request.base.toAbsolutePath().normalize();
        if (absolute.startsWith(base)) {
          absolute = base.relativize(absolute);
        }
      }
      return absolute.toString().replace(File.separatorChar, '/');
    }

    /** The processors registered in the processor path, or the classpath without one. */
    private Set<String> processors() {
      Set<String> found = new TreeSet<>();
      String service = "META-INF/services/javax.annotation.processing.Processor";
      List<Path> path = request.processorPath != null ? request.processorPath : request.classpath;
      if (request.generated == null) {
        return found;
      }
      for (Path entry : path) {
        try {
          String text = null;
          if (Files.isDirectory(entry)) {
            Path file = entry.resolve(service);
            if (Files.exists(file)) {
              text = Files.readString(file, StandardCharsets.UTF_8);
            }
          } else if (Files.isRegularFile(entry)) {
            try (ZipFile zip = new ZipFile(entry.toFile())) {
              ZipEntry zipEntry = zip.getEntry(service);
              if (zipEntry != null) {
                try (InputStream in = zip.getInputStream(zipEntry)) {
                  text = new String(in.readAllBytes(), StandardCharsets.UTF_8);
                }
              }
            }
          }
          if (text != null) {
            for (String line : text.split("\n")) {
              String name = line.replaceAll("#.*", "").strip();
              if (!name.isEmpty()) {
                found.add(name);
              }
            }
          }
        } catch (IOException e) {
          // javac reports a classpath entry it can't read.
        }
      }
      return found;
    }
  }

  private static String joined(List<Path> paths) {
    StringBuilder text = new StringBuilder();
    for (Path path : paths) {
      if (!text.isEmpty()) {
        text.append(File.pathSeparatorChar);
      }
      text.append(path);
    }
    return text.toString();
  }

  private static void deleteTree(Path dir) throws IOException {
    try (Stream<Path> paths = Files.walk(dir)) {
      for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }

  /**
   * The names of one compilation unit.
   *
   * <p>A name's {@code context} is where the name chain it's part of appears ({@code a.b.c} is a
   * chain: the select {@code a.b.c}, the select {@code a.b}, and the identifier {@code a}):
   *
   * <ul>
   *   <li>{@code package}, {@code import}, {@code module}: in a package declaration, an import, or
   *       a module declaration's directives;
   *   <li>{@code annotation}: an annotation's name; {@code annotation-element}: the element name
   *       of an annotation's {@code name = value} argument;
   *   <li>{@code type}: a type (a declaration's type, {@code extends}, a cast, a type argument, the
   *       class instantiated by {@code new}, and so on);
   *   <li>{@code method}: the name of a method call, {@code m} in {@code m()} or {@code a.m} in
   *       {@code a.m()}; its qualifiers are {@code expression};
   *   <li>{@code reference}: the qualifier of a method reference ({@code A} in {@code A::m}), which
   *       is a type or an expression;
   *   <li>{@code expression}: anywhere else;
   *   <li>{@code keyword}: {@code this}, {@code super}, and {@code class}, which javac's trees have
   *       as names ({@code this}, {@code A.this}, {@code A.class}).
   * </ul>
   *
   * <p>A member select's {@code qualifier} is what its expression is: {@code p} a package, {@code
   * t} a type, {@code this} for {@code this} and {@code super}, {@code v} a field or variable of a
   * declared type without type arguments, declared with its type written (not {@code var}, not a
   * lambda parameter without a type), when the expression is a name chain, and {@code e} anything
   * else. An identifier's is {@code -}.
   */
  private static final class FileNames extends TreePathScanner<Void, Void> {
    private final Trees trees;
    private final Keys keys;
    private final CompilationUnitTree unit;
    private final SourcePositions positions;
    private final String path;
    final List<String[]> names = new ArrayList<>();
    final List<long[]> lombok = new ArrayList<>();
    private final Set<Tree> seen = Collections.newSetFromMap(new IdentityHashMap<>());

    FileNames(Trees trees, Keys keys, CompilationUnitTree unit, String path) {
      this.trees = trees;
      this.keys = keys;
      this.unit = unit;
      this.positions = trees.getSourcePositions();
      this.path = path;
    }

    void collect() {
      new Declarations().scan(unit, null);
      scan(unit, null);
    }

    /** Records where locals and local classes are declared, which their keys name. */
    private final class Declarations extends TreePathScanner<Void, Void> {
      @Override
      public Void visitVariable(VariableTree node, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        if (element != null && Keys.isLocal(element)) {
          keys.declare(element, path, preferred(node));
          if (implicitlyTyped(node)) {
            keys.implicitlyTyped.add(element);
          }
        }
        return super.visitVariable(node, null);
      }

      @Override
      public Void visitClass(ClassTree node, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        if (element instanceof TypeElement type && Keys.isLocalType(type)) {
          keys.declare(element, path, preferred(node));
        }
        return super.visitClass(node, null);
      }
    }

    /** `var` (javac's `VAR_TYPE`), or a lambda parameter without a type (no type tree). */
    private boolean implicitlyTyped(VariableTree node) {
      Tree type = node.getType();
      return type == null
          || type.getKind() == Tree.Kind.VAR_TYPE
          || positions.getEndPosition(unit, type) < 0;
    }

    private long preferred(Tree tree) {
      return ((JCTree) tree).getPreferredPosition();
    }

    @Override
    public Void visitIdentifier(IdentifierTree node, Void unused) {
      record(node, "-");
      return super.visitIdentifier(node, null);
    }

    @Override
    public Void visitMemberSelect(MemberSelectTree node, Void unused) {
      record(node, qualifier(node));
      return super.visitMemberSelect(node, null);
    }

    @Override
    public Void visitClass(ClassTree node, Void unused) {
      if (trees.getElement(getCurrentPath()) instanceof TypeElement type && isLombok(type)) {
        lombok.add(
            new long[] {
              positions.getStartPosition(unit, node), positions.getEndPosition(unit, node)
            });
      }
      return super.visitClass(node, null);
    }

    private boolean isLombok(TypeElement type) {
      if (hasLombok(type)) {
        return true;
      }
      for (Element member : type.getEnclosedElements()) {
        if (hasLombok(member)) {
          return true;
        }
      }
      return false;
    }

    private static boolean hasLombok(Element element) {
      for (AnnotationMirror annotation : element.getAnnotationMirrors()) {
        Element type = annotation.getAnnotationType().asElement();
        if (type instanceof TypeElement t && t.getQualifiedName().toString().startsWith("lombok.")) {
          return true;
        }
      }
      return false;
    }

    private void record(Tree node, String qualifier) {
      if (!seen.add(node)) {
        return;
      }
      TreePath current = getCurrentPath();
      String context = context(current);
      Element element = trees.getElement(current);
      String key = keys.key(element);
      List<String> name = new ArrayList<>(6);
      name.add(String.valueOf(positions.getStartPosition(unit, node)));
      name.add(String.valueOf(positions.getEndPosition(unit, node)));
      name.add(context);
      name.add(qualifier);
      name.add(key);
      if (context.equals("method")) {
        name.add(searched(current, element));
      }
      names.add(name.toArray(new String[0]));
    }

    private String qualifier(MemberSelectTree node) {
      Tree expression = node.getExpression();
      TreePath path = new TreePath(getCurrentPath(), expression);
      Element element =
          expression instanceof IdentifierTree || expression instanceof MemberSelectTree
              ? trees.getElement(path)
              : null;
      if (element == null) {
        return "e";
      }
      if (element.getKind() == ElementKind.PACKAGE) {
        return "p";
      }
      if (element instanceof TypeElement || element instanceof TypeParameterElement) {
        return "t";
      }
      if (element instanceof VariableElement variable) {
        String name = variable.getSimpleName().toString();
        if (name.equals("this") || name.equals("super")) {
          return "this";
        }
        if (isNameChain(expression)
            && !keys.implicitlyTyped.contains(variable)
            && isPlainDeclared(variable.asType())) {
          return "v";
        }
      }
      return "e";
    }

    private static boolean isNameChain(Tree tree) {
      while (tree instanceof MemberSelectTree select) {
        tree = select.getExpression();
      }
      return tree instanceof IdentifierTree;
    }

    private static boolean isPlainDeclared(TypeMirror type) {
      return type.getKind() == TypeKind.DECLARED
          && ((DeclaredType) type).getTypeArguments().isEmpty()
          && ((TypeElement) ((DeclaredType) type).asElement()).getTypeParameters().isEmpty();
    }

    /** The type searched for a method name, as JLS 15.12.1 finds it, or "" when it's B3's. */
    private String searched(TreePath path, Element element) {
      Tree leaf = path.getLeaf();
      if (leaf instanceof IdentifierTree identifier) {
        String name = identifier.getName().toString();
        if (name.equals("this") || name.equals("super")) {
          return "";
        }
        for (TreePath p = path; p != null; p = p.getParentPath()) {
          if (p.getLeaf() instanceof ClassTree
              && trees.getElement(p) instanceof TypeElement type
              && keys.hasMethod(type, name)) {
            return keys.key(type);
          }
        }
        // Not a member of an enclosing class: a static import.
        return element == null ? "" : keys.key(element.getEnclosingElement());
      }
      MemberSelectTree select = (MemberSelectTree) leaf;
      Tree expression = select.getExpression();
      if (!(expression instanceof IdentifierTree || expression instanceof MemberSelectTree)) {
        return "";
      }
      Element qualifier = trees.getElement(new TreePath(path, expression));
      if (qualifier instanceof TypeElement) {
        return keys.key(qualifier);
      }
      if (qualifier instanceof VariableElement variable) {
        String name = variable.getSimpleName().toString();
        if ((name.equals("this") || name.equals("super"))
            && keys.types.asElement(variable.asType()) instanceof TypeElement type) {
          return keys.key(type);
        }
      }
      return "";
    }

    /** See the class comment. */
    private static String context(TreePath path) {
      Tree leaf = path.getLeaf();
      if (isKeyword(leaf)) {
        return "keyword";
      }
      TreePath top = path;
      while (top.getParentPath() != null
          && top.getParentPath().getLeaf() instanceof MemberSelectTree select
          && select.getExpression() == top.getLeaf()) {
        top = top.getParentPath();
      }
      String context = topContext(top);
      if (top != path && (context.equals("method") || context.equals("annotation-element"))) {
        return "expression";
      }
      return context;
    }

    private static boolean isKeyword(Tree tree) {
      String name =
          tree instanceof IdentifierTree identifier
              ? identifier.getName().toString()
              : ((MemberSelectTree) tree).getIdentifier().toString();
      return name.equals("this") || name.equals("super") || name.equals("class");
    }

    private static String topContext(TreePath top) {
      Tree tree = top.getLeaf();
      TreePath parentPath = top.getParentPath();
      Tree parent = parentPath.getLeaf();
      return switch (parent.getKind()) {
        case PACKAGE -> "package";
        case IMPORT -> "import";
        case MODULE, EXPORTS, OPENS, REQUIRES, USES, PROVIDES -> "module";
        case ANNOTATION, TYPE_ANNOTATION ->
            ((AnnotationTree) parent).getAnnotationType() == tree ? "annotation" : "expression";
        case ASSIGNMENT ->
            ((AssignmentTree) parent).getVariable() == tree
                    && parentPath.getParentPath() != null
                    && parentPath.getParentPath().getLeaf() instanceof AnnotationTree
                ? "annotation-element"
                : "expression";
        case METHOD_INVOCATION ->
            ((MethodInvocationTree) parent).getMethodSelect() == tree
                ? "method"
                : ((MethodInvocationTree) parent).getTypeArguments().contains(tree)
                    ? "type"
                    : "expression";
        case MEMBER_REFERENCE ->
            ((MemberReferenceTree) parent).getQualifierExpression() == tree
                ? "reference"
                : "type";
        case NEW_CLASS ->
            ((NewClassTree) parent).getIdentifier() == tree
                    || ((NewClassTree) parent).getTypeArguments().contains(tree)
                ? "type"
                : "expression";
        case VARIABLE -> ((VariableTree) parent).getType() == tree ? "type" : "expression";
        case METHOD -> {
          var method = (com.sun.source.tree.MethodTree) parent;
          yield method.getDefaultValue() == tree ? "expression" : "type";
        }
        case CLASS,
            TYPE_PARAMETER,
            PARAMETERIZED_TYPE,
            ARRAY_TYPE,
            ANNOTATED_TYPE,
            UNION_TYPE,
            INTERSECTION_TYPE,
            EXTENDS_WILDCARD,
            SUPER_WILDCARD,
            DECONSTRUCTION_PATTERN ->
            "type";
        case NEW_ARRAY ->
            ((com.sun.source.tree.NewArrayTree) parent).getType() == tree ? "type" : "expression";
        case TYPE_CAST ->
            ((com.sun.source.tree.TypeCastTree) parent).getType() == tree ? "type" : "expression";
        case INSTANCE_OF ->
            ((com.sun.source.tree.InstanceOfTree) parent).getType() == tree
                ? "type"
                : "expression";
        default -> "expression";
      };
    }
  }

  /**
   * Keys for elements, as docs/design/semantic.md's table gives them:
   *
   * <ul>
   *   <li>{@code package java.util}, {@code module java.base};
   *   <li>{@code type java/util/Map$Entry <origin>}, where the origin is {@code platform}, {@code
   *       source <path>}, or {@code class <classpath entry>};
   *   <li>{@code type local <file>:<offset>} for a local or anonymous class, or a member of one,
   *       at the preferred position of its declaration;
   *   <li>{@code typevar <owner key> T};
   *   <li>{@code field <owner>.<name>} and {@code method <owner>.<name><erased descriptor>}, where
   *       the owner is a binary name with {@code /}, or {@code local <file>:<offset>};
   *   <li>{@code local <file>:<offset>} for a local variable, parameter, exception parameter,
   *       resource, or binding, at the preferred position of its declaration;
   *   <li>{@code none} when javac gives no element, and {@code other <kind> <name>} for any other.
   * </ul>
   */
  static final class Keys {
    private final Elements elements;
    final Types types;
    private final java.util.function.Function<FileObject, String> relative;
    private final Map<Element, String> declared = new IdentityHashMap<>();
    final Set<Element> implicitlyTyped = Collections.newSetFromMap(new IdentityHashMap<>());
    private final Map<Element, Set<String>> methodNames = new IdentityHashMap<>();
    private final Map<Element, String> typeKeys = new IdentityHashMap<>();
    private final Path javaHome = Path.of(System.getProperty("java.home")).normalize();

    Keys(
        Elements elements,
        Types types,
        java.util.function.Function<FileObject, String> relative) {
      this.elements = elements;
      this.types = types;
      this.relative = relative;
    }

    static boolean isLocal(Element element) {
      return switch (element.getKind()) {
        case LOCAL_VARIABLE, PARAMETER, EXCEPTION_PARAMETER, RESOURCE_VARIABLE, BINDING_VARIABLE ->
            true;
        default -> false;
      };
    }

    /** A local or anonymous class, or a type declared inside one. */
    static boolean isLocalType(TypeElement type) {
      for (Element e = type; e != null; e = e.getEnclosingElement()) {
        if (e instanceof TypeElement t
            && (t.getNestingKind() == NestingKind.LOCAL
                || t.getNestingKind() == NestingKind.ANONYMOUS)) {
          return true;
        }
        if (e instanceof PackageElement) {
          return false;
        }
      }
      return false;
    }

    void declare(Element element, String file, long offset) {
      declared.put(element, file + ":" + offset);
    }

    boolean hasMethod(TypeElement type, String name) {
      return methodNames
          .computeIfAbsent(
              type,
              t -> {
                Set<String> names = new HashSet<>();
                for (Element member : elements.getAllMembers((TypeElement) t)) {
                  if (member.getKind() == ElementKind.METHOD) {
                    names.add(member.getSimpleName().toString());
                  }
                }
                return names;
              })
          .contains(name);
    }

    String key(Element element) {
      if (element == null) {
        return "none";
      }
      return switch (element.getKind()) {
        case PACKAGE -> "package " + ((PackageElement) element).getQualifiedName();
        case MODULE -> "module " + ((ModuleElement) element).getQualifiedName();
        case CLASS, INTERFACE, ENUM, RECORD, ANNOTATION_TYPE -> typeKey((TypeElement) element);
        case TYPE_PARAMETER ->
            "typevar "
                + key(((TypeParameterElement) element).getGenericElement())
                + " "
                + element.getSimpleName();
        case FIELD, ENUM_CONSTANT -> fieldKey(element);
        case METHOD, CONSTRUCTOR -> methodKey((ExecutableElement) element);
        case LOCAL_VARIABLE,
            PARAMETER,
            EXCEPTION_PARAMETER,
            RESOURCE_VARIABLE,
            BINDING_VARIABLE -> {
          String at = declared.get(element);
          yield at != null ? "local " + at : "local ? " + element.getSimpleName();
        }
        default -> "other " + element.getKind() + " " + element.getSimpleName();
      };
    }

    private String typeKey(TypeElement type) {
      return typeKeys.computeIfAbsent(
          type,
          t -> {
            TypeElement typeElement = (TypeElement) t;
            if (typeElement.asType().getKind() == TypeKind.ERROR) {
              return "error " + typeElement.getQualifiedName();
            }
            if (isLocalType(typeElement)) {
              String at = declared.get(typeElement);
              return "type local " + (at != null ? at : "? " + typeElement.getSimpleName());
            }
            return "type " + binaryName(typeElement) + " " + origin(typeElement);
          });
    }

    private String fieldKey(Element field) {
      Element owner = field.getEnclosingElement();
      if (owner instanceof TypeElement type) {
        return "field " + owner(type) + "." + field.getSimpleName();
      }
      // An array's `length`.
      return "field " + owner + "." + field.getSimpleName();
    }

    private String methodKey(ExecutableElement method) {
      StringBuilder key = new StringBuilder("method ");
      Element owner = method.getEnclosingElement();
      key.append(owner instanceof TypeElement type ? owner(type) : String.valueOf(owner));
      key.append('.').append(method.getSimpleName()).append('(');
      for (VariableElement parameter : method.getParameters()) {
        descriptor(key, parameter.asType());
      }
      key.append(')');
      if (method.getKind() == ElementKind.CONSTRUCTOR) {
        key.append('V');
      } else {
        descriptor(key, method.getReturnType());
      }
      return key.toString();
    }

    /** A type's binary name with `/`, or `local <file>:<offset>` for a local class. */
    private String owner(TypeElement type) {
      if (isLocalType(type)) {
        String at = declared.get(type);
        return "local " + (at != null ? at : "? " + type.getSimpleName());
      }
      return binaryName(type);
    }

    private String binaryName(TypeElement type) {
      String binary = elements.getBinaryName(type).toString();
      String packageName = elements.getPackageOf(type).getQualifiedName().toString();
      if (packageName.isEmpty()) {
        return binary;
      }
      return packageName.replace('.', '/') + "/" + binary.substring(packageName.length() + 1);
    }

    private void descriptor(StringBuilder out, TypeMirror type) {
      TypeMirror erased = types.erasure(type);
      switch (erased.getKind()) {
        case BOOLEAN -> out.append('Z');
        case BYTE -> out.append('B');
        case CHAR -> out.append('C');
        case SHORT -> out.append('S');
        case INT -> out.append('I');
        case LONG -> out.append('J');
        case FLOAT -> out.append('F');
        case DOUBLE -> out.append('D');
        case VOID -> out.append('V');
        case ARRAY -> {
          out.append('[');
          descriptor(out, ((ArrayType) erased).getComponentType());
        }
        case DECLARED ->
            out.append('L')
                .append(owner((TypeElement) ((DeclaredType) erased).asElement()))
                .append(';');
        default -> out.append('L').append(erased).append(';');
      }
    }

    /** Where a type came from: `platform`, `source <path>`, or `class <classpath entry>`. */
    private String origin(TypeElement type) {
      JavaFileObject file;
      try {
        file = elements.getFileObjectOf(type) instanceof JavaFileObject f ? f : null;
      } catch (RuntimeException e) {
        file = null;
      }
      if (file == null) {
        return "unknown";
      }
      if (file.getKind() == JavaFileObject.Kind.SOURCE) {
        return "source " + relative.apply(file);
      }
      URI uri = file.toUri();
      String scheme = uri.getScheme();
      if (scheme.equals("jrt")) {
        return "platform";
      }
      Path container;
      if (scheme.equals("jar")) {
        String text = uri.getSchemeSpecificPart();
        int bang = text.indexOf("!/");
        container = Path.of(URI.create(bang >= 0 ? text.substring(0, bang) : text));
      } else if (scheme.equals("file")) {
        container = classDirectory(Path.of(uri), type);
      } else {
        return "unknown " + uri;
      }
      if (container.normalize().startsWith(javaHome)) {
        return "platform";
      }
      return "class " + relative.apply(new PathObject(container));
    }

    /** The directory a class file is in, as a classpath entry: its package's directories up. */
    private Path classDirectory(Path classFile, TypeElement type) {
      Path dir = classFile.getParent();
      String packageName = elements.getPackageOf(type).getQualifiedName().toString();
      if (!packageName.isEmpty()) {
        for (int i = packageName.split("\\.").length; i > 0 && dir != null; i--) {
          dir = dir.getParent();
        }
      }
      return dir;
    }
  }

  /** A path as a file object, so {@link Keys} relativizes it as it does files. */
  private static final class PathObject extends javax.tools.SimpleJavaFileObject {
    PathObject(Path path) {
      super(path.toUri(), Kind.OTHER);
    }
  }

  /**
   * javac's file manager, with {@code module-info.java} hidden on the source path, so a source
   * set that depends on a modular one is still compiled in the unnamed module.
   */
  @ClientCodeWrapper.Trusted
  private static final class HidingFileManager
      extends javax.tools.ForwardingJavaFileManager<StandardJavaFileManager> {
    HidingFileManager(StandardJavaFileManager manager) {
      super(manager);
    }

    @Override
    public Iterable<JavaFileObject> list(
        Location location, String packageName, Set<JavaFileObject.Kind> kinds, boolean recurse)
        throws IOException {
      Iterable<JavaFileObject> files = super.list(location, packageName, kinds, recurse);
      if (location != javax.tools.StandardLocation.SOURCE_PATH || !packageName.isEmpty()) {
        return files;
      }
      List<JavaFileObject> kept = new ArrayList<>();
      for (JavaFileObject file : files) {
        if (!file.isNameCompatible("module-info", JavaFileObject.Kind.SOURCE)) {
          kept.add(file);
        }
      }
      return kept;
    }

    @Override
    public JavaFileObject getJavaFileForInput(
        Location location, String className, JavaFileObject.Kind kind) throws IOException {
      if (location == javax.tools.StandardLocation.SOURCE_PATH
          && className.equals("module-info")) {
        return null;
      }
      return super.getJavaFileForInput(location, className, kind);
    }
  }

  /** Collects what javac reports; trusted, so javac gives it its own diagnostics. */
  @ClientCodeWrapper.Trusted
  private static final class Collector implements DiagnosticListener<JavaFileObject> {
    final List<Diagnostic<? extends JavaFileObject>> diagnostics = new ArrayList<>();

    @Override
    public void report(Diagnostic<? extends JavaFileObject> diagnostic) {
      diagnostics.add(diagnostic);
    }
  }
}
