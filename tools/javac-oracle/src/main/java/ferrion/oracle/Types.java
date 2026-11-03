package ferrion.oracle;

import com.sun.source.tree.AnnotationTree;
import com.sun.source.tree.AssignmentTree;
import com.sun.source.tree.ClassTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.ExpressionTree;
import com.sun.source.tree.ImportTree;
import com.sun.source.tree.MemberReferenceTree;
import com.sun.source.tree.MethodInvocationTree;
import com.sun.source.tree.ModuleTree;
import com.sun.source.tree.NewClassTree;
import com.sun.source.tree.PackageTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import com.sun.tools.javac.code.Flags;
import com.sun.tools.javac.code.Symbol;
import com.sun.tools.javac.code.Type;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.Modifier;
import javax.lang.model.element.ModuleElement;
import javax.lang.model.element.PackageElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.TypeParameterElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.ArrayType;
import javax.lang.model.type.DeclaredType;
import javax.lang.model.type.ExecutableType;
import javax.lang.model.type.IntersectionType;
import javax.lang.model.type.TypeKind;
import javax.lang.model.type.TypeMirror;
import javax.lang.model.type.TypeVariable;
import javax.lang.model.type.UnionType;
import javax.lang.model.type.WildcardType;
import javax.lang.model.util.Elements;

/**
 * The {@code types} command: compiles one source set as {@code names} does (see {@link Names}),
 * and writes, for each of its files, javac's attributed types, chosen methods, constant values,
 * and variables' finality; then javac's answers to type relation questions about types the source
 * set declares.
 *
 * <p>Usage: {@code javac-oracle types [--relation-types <N>] <names options>...}. {@code
 * --relation-types} is how many of the source set's declared types the relations are asked about
 * (default {@value #RELATION_TYPES}; 0 asks none).
 *
 * <p>Each file's object has, besides {@code file}, {@code generated}, {@code diagnostics}, {@code
 * unanalyzed}, and {@code crash} as {@code names} writes them:
 *
 * <ul>
 *   <li>{@code lombok}: {@code [start, end]} of each Lombok-annotated class, as in {@code names};
 *   <li>{@code analyzeMicros}: the time javac spent analyzing the file's classes;
 *   <li>{@code types}: the file's type strings (see {@link TypeStrings}), each once; the entries
 *       below name a type by its index here;
 *   <li>{@code expressions}: {@code [start, end, kind, type]} for each expression tree, in the
 *       pre-order of {@link TreePathScanner}, but names of packages, types, and methods, type
 *       trees, and trees javac made up;
 *   <li>{@code constants}: {@code [start, end, value]} for each expression with a constant value
 *       (see {@link #constant});
 *   <li>{@code calls}: {@code [start, end, kind, method, type]} for each method invocation (an
 *       explicit or implicit constructor call too), class instance creation, and method
 *       reference: the key of the method javac chose, and its type as invoked, or -1;
 *   <li>{@code variables}: {@code [offset, kind, finality, type]} for each local variable and
 *       parameter, with the offset its key has, and {@code final}, {@code effectively-final}, or
 *       {@code mutable}.
 * </ul>
 *
 * <p>Two values aren't in javac's public API, and come from its internals: a constant's value
 * ({@code Type.constValue()} of the expression's type), and whether a variable is effectively
 * final (the {@code EFFECTIVELY_FINAL} flag of its symbol). What they give was checked against
 * the JLS on small inputs (docs/design/types-oracle-normalization.md).
 */
final class Types implements Names.Command {
  /** The default number of declared types the relations are asked about, per source set. */
  static final int RELATION_TYPES = 48;

  /** How many of a declared type's members it is asked about, with {@code asMemberOf}. */
  private static final int MEMBERS = 3;

  /** How many of a declared type's own methods are paired with methods of its supertypes. */
  private static final int OVERRIDE_METHODS = 4;

  private final int relationTypes;

  /** The declarations of the main compilation's files, for the relations. */
  private final List<Site> sites = new ArrayList<>();

  /** The main compilation's task: the relations are asked of its types. */
  private com.sun.source.util.JavacTask task;

  private Names.Keys keys;

  private Types(int relationTypes) {
    this.relationTypes = relationTypes;
  }

  static void main(List<String> args, PrintStream out) throws IOException {
    int relationTypes = RELATION_TYPES;
    List<String> rest = new ArrayList<>();
    for (int i = 0; i < args.size(); i++) {
      if (args.get(i).equals("--relation-types") && i + 1 < args.size()) {
        relationTypes = Integer.parseInt(args.get(++i));
      } else {
        rest.add(args.get(i));
      }
    }
    Names.main(rest, out, new Types(relationTypes));
  }

  @Override
  public boolean modules() {
    return false;
  }

  @Override
  public void write(Json json, Names.FileContext file) {
    if (task != file.task()) {
      task = file.task();
      keys = file.keys();
      sites.clear();
    }
    new Names.Declarations(file.trees(), file.keys(), file.unit(), file.path())
        .scan(file.unit(), null);
    FileTypes types = new FileTypes(file);
    types.scan(file.unit(), null);

    json.name("lombok").beginArray();
    for (long[] range : types.lombok) {
      json.beginArray().value(range[0]).value(range[1]).endArray();
    }
    json.endArray();
    json.name("analyzeMicros").value(file.analyzeMicros());
    json.name("types").beginArray();
    for (String type : types.strings.keySet()) {
      json.value(type);
    }
    json.endArray();
    json.name("expressions").beginArray();
    for (Object[] entry : types.expressions) {
      entry(json, entry);
    }
    json.endArray();
    json.name("constants").beginArray();
    for (Object[] entry : types.constants) {
      entry(json, entry);
    }
    json.endArray();
    json.name("calls").beginArray();
    for (Object[] entry : types.calls) {
      entry(json, entry);
    }
    json.endArray();
    json.name("variables").beginArray();
    for (Object[] entry : types.variables) {
      entry(json, entry);
    }
    json.endArray();
  }

  private static void entry(Json json, Object[] entry) {
    json.beginArray();
    for (Object value : entry) {
      if (value instanceof Long number) {
        json.value(number);
      } else if (value instanceof Integer number) {
        json.value(number);
      } else {
        json.value((String) value);
      }
    }
    json.endArray();
  }

  /** The expressions, calls, constants, and variables of one file. */
  private final class FileTypes extends TreePathScanner<Void, Void> {
    private final Trees trees;
    private final Names.Keys keys;
    private final CompilationUnitTree unit;
    private final SourcePositions positions;
    private final TypeStrings printer;
    /** Each type string, with its index. */
    final Map<String, Integer> strings = new LinkedHashMap<>();
    final List<Object[]> expressions = new ArrayList<>();
    final List<Object[]> constants = new ArrayList<>();
    final List<Object[]> calls = new ArrayList<>();
    final List<Object[]> variables = new ArrayList<>();
    final List<long[]> lombok = new ArrayList<>();
    private final Set<Tree> seen = Collections.newSetFromMap(new IdentityHashMap<>());

    FileTypes(Names.FileContext file) {
      this.trees = file.trees();
      this.keys = file.keys();
      this.unit = file.unit();
      this.positions = trees.getSourcePositions();
      this.printer = new TypeStrings(keys);
    }

    private int index(TypeMirror type) {
      if (type == null) {
        return -1;
      }
      String text = printer.print(type);
      return strings.computeIfAbsent(text, t -> strings.size());
    }

    // Package declarations, imports, and module declarations have names only.

    @Override
    public Void visitPackage(PackageTree node, Void unused) {
      return null;
    }

    @Override
    public Void visitImport(ImportTree node, Void unused) {
      return null;
    }

    @Override
    public Void visitModule(ModuleTree node, Void unused) {
      return null;
    }

    @Override
    public Void visitClass(ClassTree node, Void unused) {
      Element element = trees.getElement(getCurrentPath());
      if (element instanceof TypeElement type) {
        if (Names.isLombok(type)) {
          lombok.add(
              new long[] {
                positions.getStartPosition(unit, node), positions.getEndPosition(unit, node)
              });
        }
        sites(type);
      }
      return super.visitClass(node, null);
    }

    @Override
    public Void visitVariable(VariableTree node, Void unused) {
      Element element = trees.getElement(getCurrentPath());
      if (element != null && Names.Keys.isLocal(element) && seen.add(node)) {
        String key = keys.key(element);
        long offset = Long.parseLong(key.substring(key.lastIndexOf(':') + 1));
        variables.add(
            new Object[] {
              offset, element.getKind().name(), finality(element), index(element.asType())
            });
      }
      return super.visitVariable(node, null);
    }

    @Override
    public Void scan(Tree tree, Void unused) {
      if (tree instanceof ExpressionTree && seen.add(tree)) {
        TreePath path = new TreePath(getCurrentPath(), tree);
        try {
          record(path, tree);
        } catch (RuntimeException e) {
          // A tree javac's API fails on is left out, not the file.
        }
      }
      return super.scan(tree, null);
    }

    private void record(TreePath path, Tree tree) {
      long start = positions.getStartPosition(unit, tree);
      long end = positions.getEndPosition(unit, tree);
      String kind = tree.getKind().name();
      if (tree instanceof MethodInvocationTree invocation) {
        TreePath select = new TreePath(path, invocation.getMethodSelect());
        call(start, end, kind, trees.getElement(select), trees.getTypeMirror(select));
      } else if (tree instanceof NewClassTree || tree instanceof MemberReferenceTree) {
        call(start, end, kind, trees.getElement(path), null);
      }
      if (end < 0 || !isExpression(path, tree)) {
        return;
      }
      TypeMirror type = trees.getTypeMirror(path);
      if (type == null) {
        return;
      }
      expressions.add(new Object[] {start, end, kind, index(type)});
      if (type instanceof Type javac) {
        Object value = javac.constValue();
        if (value != null) {
          String constant = constant(type, value);
          if (constant != null) {
            constants.add(new Object[] {start, end, constant});
          }
        }
      }
    }

    private void call(long start, long end, String kind, Element method, TypeMirror type) {
      if (method instanceof ExecutableElement) {
        calls.add(
            new Object[] {
              start, end, kind, keys.key(method), type instanceof ExecutableType ? index(type) : -1
            });
      }
    }

    /**
     * Whether a tree is an expression whose type is reported: not a type tree, not the name of a
     * package, type, or method, not an annotation, and not an annotation's {@code name = value}.
     */
    private boolean isExpression(TreePath path, Tree tree) {
      if (TYPE_KINDS.contains(tree.getKind().name())) {
        return false;
      }
      Tree parent = path.getParentPath().getLeaf();
      if (tree instanceof AssignmentTree && parent instanceof AnnotationTree) {
        return false;
      }
      return switch (tree.getKind()) {
        case IDENTIFIER, MEMBER_SELECT -> {
          Element element = trees.getElement(path);
          yield !(element instanceof PackageElement
              || element instanceof TypeElement
              || element instanceof TypeParameterElement
              || element instanceof ExecutableElement
              || element instanceof ModuleElement);
        }
        default -> true;
      };
    }

    /** The declarations of {@code type} the relations may ask about. */
    private void sites(TypeElement type) {
      if (relationTypes <= 0 || type.getNestingKind().name().equals("ANONYMOUS")) {
        return;
      }
      String owner = keys.key(type);
      if (type.getSuperclass().getKind() == TypeKind.DECLARED) {
        sites.add(new Site(owner + " superclass", type.getSuperclass()));
      }
      List<? extends TypeMirror> interfaces = type.getInterfaces();
      for (int i = 0; i < interfaces.size(); i++) {
        sites.add(new Site(owner + " interface " + i, interfaces.get(i)));
      }
      for (Element member : type.getEnclosedElements()) {
        if (member.getKind() == ElementKind.FIELD) {
          sites.add(new Site(keys.key(member) + " type", member.asType()));
        } else if (member instanceof ExecutableElement method
            && (member.getKind() == ElementKind.METHOD
                || member.getKind() == ElementKind.CONSTRUCTOR)) {
          String key = keys.key(method);
          List<? extends VariableElement> parameters = method.getParameters();
          for (int i = 0; i < parameters.size(); i++) {
            sites.add(new Site(key + " parameter " + i, parameters.get(i).asType()));
          }
          if (method.getKind() == ElementKind.METHOD
              && method.getReturnType().getKind() != TypeKind.VOID) {
            sites.add(new Site(key + " return", method.getReturnType()));
          }
        }
      }
    }
  }

  /** Kinds of trees that are types, by name: older JDKs lack some. */
  private static final Set<String> TYPE_KINDS =
      Set.of(
          "PRIMITIVE_TYPE",
          "ARRAY_TYPE",
          "PARAMETERIZED_TYPE",
          "UNION_TYPE",
          "INTERSECTION_TYPE",
          "ANNOTATED_TYPE",
          "EXTENDS_WILDCARD",
          "SUPER_WILDCARD",
          "UNBOUNDED_WILDCARD",
          "VAR_TYPE",
          "ANNOTATION",
          "TYPE_ANNOTATION");

  /** {@code final}, {@code effectively-final}, or {@code mutable}. */
  private static String finality(Element variable) {
    if (variable.getModifiers().contains(Modifier.FINAL)) {
      return "final";
    }
    if (variable instanceof Symbol symbol) {
      for (Object flag : Flags.asFlagSet(symbol.flags())) {
        // By name: the flag's bit, and the enum's class, aren't the same on every JDK.
        if (flag instanceof Enum<?> e && e.name().equals("EFFECTIVELY_FINAL")) {
          return "effectively-final";
        }
      }
    }
    return "mutable";
  }

  /**
   * A constant in the oracle's form: {@code <type> <value>}, with integers in decimal, floating
   * point in Java's hexadecimal form, and characters and strings quoted and escaped (see {@link
   * #quoted}). javac holds a {@code boolean}'s, {@code char}'s, {@code byte}'s, or {@code short}'s
   * value as an {@code Integer}.
   */
  static String constant(TypeMirror type, Object value) {
    return switch (type.getKind()) {
      case BOOLEAN ->
          "boolean "
              + (value instanceof Boolean b ? b : ((Number) value).intValue() != 0 ? "true" : "false");
      case CHAR ->
          "char "
              + quoted(
                  Character.toString(
                      value instanceof Character c ? c : (char) ((Number) value).intValue()));
      case BYTE -> "byte " + ((Number) value).byteValue();
      case SHORT -> "short " + ((Number) value).shortValue();
      case INT -> "int " + ((Number) value).intValue();
      case LONG -> "long " + ((Number) value).longValue();
      case FLOAT -> "float " + Float.toHexString(((Number) value).floatValue());
      case DOUBLE -> "double " + Double.toHexString(((Number) value).doubleValue());
      case DECLARED -> value instanceof String s ? "String " + quoted(s) : null;
      default -> null;
    };
  }

  /**
   * A string in double quotes, with printable ASCII as it is but {@code "} and {@code \}
   * escaped with {@code \}, and every other UTF-16 unit as {@code \}{@code uXXXX} in lowercase.
   */
  static String quoted(String text) {
    StringBuilder out = new StringBuilder(text.length() + 2).append('"');
    for (int i = 0; i < text.length(); i++) {
      char c = text.charAt(i);
      if (c == '"' || c == '\\') {
        out.append('\\').append(c);
      } else if (c >= 0x20 && c <= 0x7e) {
        out.append(c);
      } else {
        out.append(String.format("\\u%04x", (int) c));
      }
    }
    return out.append('"').toString();
  }

  /**
   * Types as the oracle writes them (docs/design/types-oracle-normalization.md, "Type strings"):
   * binary names with {@code /}, type arguments in {@code <>}, a type variable as {@code
   * <name>:<owner>}, captured type variables numbered in each string with their bounds at their
   * first appearance.
   */
  static final class TypeStrings {
    private final Names.Keys keys;
    /** The captures of the string being written, by their element, numbered from 1. */
    private final Map<Element, Integer> captures = new IdentityHashMap<>();

    TypeStrings(Names.Keys keys) {
      this.keys = keys;
    }

    String print(TypeMirror type) {
      captures.clear();
      StringBuilder out = new StringBuilder();
      write(out, type);
      return out.toString();
    }

    private void write(StringBuilder out, TypeMirror type) {
      switch (type.getKind()) {
        case BOOLEAN, BYTE, SHORT, CHAR, INT, LONG, FLOAT, DOUBLE, VOID ->
            out.append(type.getKind().name().toLowerCase(java.util.Locale.ROOT));
        case NULL -> out.append("null");
        case NONE -> out.append("none");
        case ERROR -> out.append("error ").append(type);
        case PACKAGE -> out.append("package ").append(type);
        case ARRAY -> {
          write(out, ((ArrayType) type).getComponentType());
          out.append("[]");
        }
        case DECLARED -> declared(out, (DeclaredType) type);
        case TYPEVAR -> typeVariable(out, (TypeVariable) type);
        case WILDCARD -> {
          WildcardType wildcard = (WildcardType) type;
          out.append('?');
          if (wildcard.getExtendsBound() != null) {
            out.append(" extends ");
            write(out, wildcard.getExtendsBound());
          } else if (wildcard.getSuperBound() != null) {
            out.append(" super ");
            write(out, wildcard.getSuperBound());
          }
        }
        case INTERSECTION -> joined(out, ((IntersectionType) type).getBounds(), "&");
        case UNION -> joined(out, ((UnionType) type).getAlternatives(), "|");
        case EXECUTABLE -> executable(out, (ExecutableType) type);
        default -> out.append("other ").append(type);
      }
    }

    private void joined(StringBuilder out, List<? extends TypeMirror> types, String separator) {
      for (int i = 0; i < types.size(); i++) {
        if (i > 0) {
          out.append(separator);
        }
        write(out, types.get(i));
      }
    }

    private void declared(StringBuilder out, DeclaredType type) {
      TypeElement element = (TypeElement) type.asElement();
      TypeMirror enclosing = type.getEnclosingType();
      if (!Names.Keys.isLocalType(element)
          && enclosing.getKind() == TypeKind.DECLARED
          && hasArguments((DeclaredType) enclosing)) {
        write(out, enclosing);
        out.append('$').append(element.getSimpleName());
      } else {
        out.append(keys.owner(element));
      }
      List<? extends TypeMirror> arguments = type.getTypeArguments();
      if (!arguments.isEmpty()) {
        out.append('<');
        joined(out, arguments, ",");
        out.append('>');
      }
    }

    /** Whether a type, or a type enclosing it, has type arguments. */
    private static boolean hasArguments(DeclaredType type) {
      for (TypeMirror t = type; t.getKind() == TypeKind.DECLARED; ) {
        DeclaredType declared = (DeclaredType) t;
        if (!declared.getTypeArguments().isEmpty()) {
          return true;
        }
        t = declared.getEnclosingType();
      }
      return false;
    }

    private void typeVariable(StringBuilder out, TypeVariable type) {
      Element element = type.asElement();
      Element owner =
          element instanceof TypeParameterElement parameter ? parameter.getGenericElement() : null;
      if (owner instanceof TypeElement typeOwner) {
        out.append(element.getSimpleName()).append(':').append(keys.owner(typeOwner));
      } else if (owner instanceof ExecutableElement method) {
        out.append(element.getSimpleName())
            .append(':')
            .append(keys.key(method).substring("method ".length()));
      } else {
        // A captured type variable: javac's has no declaration.
        Integer number = captures.get(element);
        if (number != null) {
          out.append("capture#").append(number);
          return;
        }
        number = captures.size() + 1;
        captures.put(element, number);
        out.append("capture#").append(number).append(" extends ");
        write(out, type.getUpperBound());
        TypeMirror lower = type.getLowerBound();
        if (lower != null && lower.getKind() != TypeKind.NULL && lower.getKind() != TypeKind.NONE) {
          out.append(" super ");
          write(out, lower);
        }
      }
    }

    private void executable(StringBuilder out, ExecutableType type) {
      List<? extends TypeVariable> variables = type.getTypeVariables();
      if (!variables.isEmpty()) {
        out.append('<');
        joined(out, variables, ",");
        out.append('>');
      }
      out.append('(');
      joined(out, type.getParameterTypes(), ",");
      out.append(')');
      write(out, type.getReturnType());
      if (!type.getThrownTypes().isEmpty()) {
        out.append(" throws ");
        joined(out, type.getThrownTypes(), ",");
      }
    }
  }

  /** A declaration's type the relations may ask about: where it is, and the type. */
  private record Site(String where, TypeMirror type) {}

  /**
   * Writes the relations line: javac's answers through {@code javax.lang.model.util.Types} about
   * a sample of the types the source set declares. The sample is the first {@link
   * #relationTypes} distinct type strings, ordered by a hash of the string, so it is the same on
   * every run; each type's first site in the sample's order says where it is declared.
   */
  @Override
  public void finish(PrintStream out) {
    if (task == null || relationTypes <= 0) {
      return;
    }
    javax.lang.model.util.Types types = task.getTypes();
    Elements elements = task.getElements();
    TypeStrings printer = new TypeStrings(keys);
    Map<String, Integer> strings = new LinkedHashMap<>();
    java.util.function.Function<TypeMirror, Integer> index =
        type -> strings.computeIfAbsent(printer.print(type), t -> strings.size());

    // The sample.
    List<Site> ordered = new ArrayList<>();
    Map<String, String> printed = new HashMap<>();
    for (Site site : sites) {
      if (!hasError(site.type())) {
        printed.put(site.where(), printer.print(site.type()));
        ordered.add(site);
      }
    }
    ordered.sort(
        Comparator.comparingLong((Site s) -> hash(printed.get(s.where())))
            .thenComparing(s -> printed.get(s.where()))
            .thenComparing(Site::where));
    List<Site> sample = new ArrayList<>();
    Set<String> taken = new HashSet<>();
    for (Site site : ordered) {
      if (sample.size() >= relationTypes) {
        break;
      }
      if (taken.add(printed.get(site.where()))) {
        sample.add(site);
      }
    }

    Json json = new Json();
    json.beginObject().name("relations").beginObject();
    json.name("sites").beginArray();
    for (Site site : sample) {
      json.beginArray().value(site.where()).value(index.apply(site.type())).endArray();
    }
    json.endArray();
    json.name("queries").beginArray();
    for (Site site : sample) {
      TypeMirror type = site.type();
      int t = index.apply(type);
      query(json, "erasure", t, () -> index.apply(types.erasure(type)));
      query(json, "capture", t, () -> index.apply(types.capture(type)));
      List<? extends TypeMirror> supertypes = types.directSupertypes(type);
      json.beginArray().value("supertypes").value(t).beginArray();
      for (TypeMirror supertype : supertypes) {
        json.value(index.apply(supertype));
      }
      json.endArray().endArray();
      for (TypeMirror supertype : supertypes) {
        int s = index.apply(supertype);
        relation(json, "subtype", t, s, () -> types.isSubtype(type, supertype));
        relation(json, "subtype", s, t, () -> types.isSubtype(supertype, type));
        relation(json, "assignable", t, s, () -> types.isAssignable(type, supertype));
        relation(json, "assignable", s, t, () -> types.isAssignable(supertype, type));
      }
    }
    for (Site a : sample) {
      for (Site b : sample) {
        int s = index.apply(a.type());
        int t = index.apply(b.type());
        relation(json, "subtype", s, t, () -> types.isSubtype(a.type(), b.type()));
        relation(json, "assignable", s, t, () -> types.isAssignable(a.type(), b.type()));
        relation(json, "same", s, t, () -> types.isSameType(a.type(), b.type()));
      }
    }
    for (Site site : sample) {
      if (site.type() instanceof DeclaredType declared
          && declared.asElement() instanceof TypeElement owner) {
        members(json, types, elements, declared, owner, index);
      }
    }
    json.endArray();
    json.name("types").beginArray();
    for (String type : strings.keySet()) {
      json.value(type);
    }
    json.endArray();
    json.endObject().endObject();
    out.println(json);
  }

  /** {@code asMemberOf}, {@code isSubsignature}, and {@code overrides} for a declared type. */
  private void members(
      Json json,
      javax.lang.model.util.Types types,
      Elements elements,
      DeclaredType declared,
      TypeElement owner,
      java.util.function.Function<TypeMirror, Integer> index) {
    int t = index.apply(declared);
    List<Element> members = new ArrayList<>();
    for (Element member : elements.getAllMembers(owner)) {
      if (member.getKind() == ElementKind.METHOD || member.getKind() == ElementKind.FIELD) {
        members.add(member);
      }
    }
    members.sort(Comparator.comparingLong((Element m) -> hash(keys.key(m))));
    for (Element member : members.subList(0, Math.min(MEMBERS, members.size()))) {
      String key = keys.key(member);
      json.beginArray().value("member").value(t).value(key);
      try {
        json.value(index.apply(types.asMemberOf(declared, member)));
      } catch (RuntimeException e) {
        json.value(-1);
      }
      json.endArray();
    }
    // The type's own methods, each with its supertypes' methods of the same name.
    List<ExecutableElement> own = new ArrayList<>();
    for (Element member : owner.getEnclosedElements()) {
      if (member.getKind() == ElementKind.METHOD) {
        own.add((ExecutableElement) member);
      }
    }
    own.sort(Comparator.comparingLong((ExecutableElement m) -> hash(keys.key(m))));
    List<TypeElement> supertypes = new ArrayList<>();
    supertypes(types, declared, supertypes, Collections.newSetFromMap(new IdentityHashMap<>()));
    for (ExecutableElement method : own.subList(0, Math.min(OVERRIDE_METHODS, own.size()))) {
      for (TypeElement supertype : supertypes) {
        for (Element other : supertype.getEnclosedElements()) {
          if (other.getKind() == ElementKind.METHOD
              && other.getSimpleName().contentEquals(method.getSimpleName())) {
            ExecutableElement overridden = (ExecutableElement) other;
            json.beginArray()
                .value("subsignature")
                .value(t)
                .value(keys.key(method))
                .value(keys.key(overridden));
            try {
              json.value(
                  types.isSubsignature(
                      (ExecutableType) types.asMemberOf(declared, method),
                      (ExecutableType) types.asMemberOf(declared, overridden)));
            } catch (RuntimeException e) {
              json.value("error");
            }
            json.endArray();
            json.beginArray()
                .value("overrides")
                .value(keys.key(method))
                .value(keys.key(overridden))
                .value(keys.key(owner))
                .value(elements.overrides(method, overridden, owner))
                .endArray();
          }
        }
      }
    }
  }

  /** Every class and interface a type's direct supertypes reach, each once, in order. */
  private static void supertypes(
      javax.lang.model.util.Types types,
      TypeMirror type,
      List<TypeElement> found,
      Set<Element> seen) {
    for (TypeMirror supertype : types.directSupertypes(type)) {
      if (supertype instanceof DeclaredType declared
          && declared.asElement() instanceof TypeElement element
          && seen.add(element)) {
        found.add(element);
        supertypes(types, supertype, found, seen);
      }
    }
  }

  private interface Answer<T> {
    T get();
  }

  private static void query(Json json, String name, int type, Answer<Integer> answer) {
    json.beginArray().value(name).value(type);
    try {
      json.value(answer.get());
    } catch (RuntimeException e) {
      json.value(-1);
    }
    json.endArray();
  }

  private static void relation(Json json, String name, int s, int t, Answer<Boolean> answer) {
    json.beginArray().value(name).value(s).value(t);
    try {
      json.value(answer.get());
    } catch (RuntimeException e) {
      json.value("error");
    }
    json.endArray();
  }

  /** Whether a type has an error type in it, so it isn't asked about. */
  private static boolean hasError(TypeMirror type) {
    return switch (type.getKind()) {
      case ERROR -> true;
      case ARRAY -> hasError(((ArrayType) type).getComponentType());
      case DECLARED -> {
        for (TypeMirror argument : ((DeclaredType) type).getTypeArguments()) {
          if (hasError(argument)) {
            yield true;
          }
        }
        yield false;
      }
      case WILDCARD -> {
        WildcardType wildcard = (WildcardType) type;
        yield (wildcard.getExtendsBound() != null && hasError(wildcard.getExtendsBound()))
            || (wildcard.getSuperBound() != null && hasError(wildcard.getSuperBound()));
      }
      default -> false;
    };
  }

  /** FNV-1a over the string's UTF-8 bytes: the sample's order, the same on every JVM. */
  static long hash(String text) {
    long hash = 0xcbf29ce484222325L;
    for (byte b : text.getBytes(StandardCharsets.UTF_8)) {
      hash ^= b & 0xff;
      hash *= 0x100000001b3L;
    }
    return hash;
  }
}
