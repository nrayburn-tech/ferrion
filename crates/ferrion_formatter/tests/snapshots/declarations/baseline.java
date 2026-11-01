// The first rule's output for declarations, before their own rules are written.
package p.q;

import static java.util.Map.entry;
import java.util.*;

@SuppressWarnings({"unchecked", "rawtypes"})
public class A<T extends Comparable<T>> extends B implements C, D {
  private static final int X = 1, Y[] = {2};

  public A(int a, String... rest) throws E { super(a); }

  <U> U m(@Deprecated final U u) { return u; }

  static { }

  enum E { ONE, TWO(2) { void f() {} }; E() {} E(int x) {} }

  record R(int a, String b) implements C { R { } }

  @interface Q { int value() default 1; String[] names() default {}; }
}
