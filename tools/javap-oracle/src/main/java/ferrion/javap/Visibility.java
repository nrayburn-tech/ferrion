package ferrion.javap;

import java.io.IOException;
import java.io.PrintStream;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ModuleElement;
import javax.lang.model.element.PackageElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.util.Elements;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;

import com.sun.source.util.JavacTask;

/**
 * What javac sees with {@code --release <N>}, through public APIs only ({@code javax.tools},
 * {@code javax.lang.model}, and {@code com.sun.source.util.JavacTask}): the modules, the classes
 * in each, and the packages code in the unnamed module can name. Runs on the JDK under test, so
 * its own {@code ct.sym} and image answer.
 *
 * <p>Output, one JSON object per line: {@code {"module": name, "classes": [...]}} for each module
 * javac resolves, then {@code {"visible": [...], "all": false}}, the packages visible from the
 * unnamed module in internal form. A release without modules (8) has no list of packages to walk,
 * so the caller names the packages to look up, and the output is one module named {@code ""}
 * with their classes, then {@code {"visible": [], "all": true}}.
 */
final class Visibility {
  private Visibility() {}

  static void run(String release, List<String> packages, PrintStream out) throws IOException {
    JavaCompiler javac = ToolProvider.getSystemJavaCompiler();
    if (javac == null) {
      throw new IllegalStateException("this Java runtime has no compiler");
    }
    JavaFileObject probe =
        new SimpleJavaFileObject(URI.create("string:///Probe.java"), JavaFileObject.Kind.SOURCE) {
          @Override
          public CharSequence getCharContent(boolean ignoreEncodingErrors) {
            return "class Probe {}";
          }
        };
    try (StandardJavaFileManager files =
        javac.getStandardFileManager(null, null, StandardCharsets.UTF_8)) {
      JavacTask task =
          (JavacTask)
              javac.getTask(
                  null,
                  files,
                  diagnostic -> {},
                  List.of("--release", release, "-proc:none", "-Xlint:-options"),
                  null,
                  List.of(probe));
      task.analyze();
      Elements elements = task.getElements();
      List<ModuleElement> modules = new ArrayList<>(elements.getAllModuleElements());
      modules.removeIf(ModuleElement::isUnnamed);
      modules.sort(Comparator.comparing(m -> m.getQualifiedName().toString()));
      ModuleElement unnamed = elements.getModuleElement("");
      if (modules.isEmpty()) {
        // Without modules there is no list of packages to walk, so the caller names them.
        List<Element> found = new ArrayList<>();
        for (String name : packages) {
          PackageElement element = elements.getPackageElement(name.replace('/', '.'));
          if (element != null) {
            found.add(element);
          }
        }
        out.println(module("", classes(elements, found)));
        out.println(new Json().beginObject().name("visible").beginArray().endArray().name("all")
            .value(true).endObject());
        return;
      }
      List<String> visible = new ArrayList<>();
      for (ModuleElement module : modules) {
        String name = module.getQualifiedName().toString();
        out.println(module(name, classes(elements, module.getEnclosedElements())));
        for (Element element : module.getEnclosedElements()) {
          if (element.getKind() != ElementKind.PACKAGE) {
            continue;
          }
          String packageName = ((PackageElement) element).getQualifiedName().toString();
          if (elements.getPackageElement(unnamed, packageName) != null) {
            visible.add(packageName.replace('.', '/'));
          }
        }
      }
      Collections.sort(visible);
      Json json = new Json().beginObject().name("visible").beginArray();
      for (String p : visible) {
        json.value(p);
      }
      out.println(json.endArray().name("all").value(false).endObject());
    }
  }

  private static String module(String name, List<String> classes) {
    Json json = new Json().beginObject().name("module").value(name).name("classes").beginArray();
    for (String c : classes) {
      json.value(c);
    }
    return json.endArray().endObject().toString();
  }

  /**
   * The binary names, in internal form, of the classes javac's model has in {@code packages}: each
   * package's top-level types and, recursively, their member types. Local and anonymous classes
   * aren't members, so they aren't listed.
   */
  private static List<String> classes(Elements elements, List<? extends Element> packages) {
    List<String> classes = new ArrayList<>();
    List<Element> work = new ArrayList<>();
    for (Element element : packages) {
      if (element.getKind() == ElementKind.PACKAGE) {
        work.addAll(element.getEnclosedElements());
      }
    }
    while (!work.isEmpty()) {
      Element element = work.remove(work.size() - 1);
      if (!(element instanceof TypeElement)) {
        continue;
      }
      classes.add(elements.getBinaryName((TypeElement) element).toString().replace('.', '/'));
      for (Element member : element.getEnclosedElements()) {
        if (member instanceof TypeElement) {
          work.add(member);
        }
      }
    }
    Collections.sort(classes);
    return classes;
  }
}
