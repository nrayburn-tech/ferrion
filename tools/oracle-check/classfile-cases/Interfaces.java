// Interfaces, default and private methods, wildcards and bounds, and abstract classes.

import java.util.*;

public interface Interfaces<T> extends Comparable<T>, java.io.Serializable {
  String A = "x\ny";
  char CH = '\'';
  double PI = 3.141592653589793;
  long MAXL = Long.MAX_VALUE;
  int MINI = Integer.MIN_VALUE;
  double NEGINF = Double.NEGATIVE_INFINITY;

  default void d(String... args) {}

  static <X extends Number & Comparable<? super X>> void s(
      List<? extends X> l, Map<String, ? super Integer>[] m) throws Exception {}

  abstract int x();

  private void p() {}
}

abstract class Abstract implements Runnable, Cloneable {
  protected abstract void am();
  @Deprecated(since = "9", forRemoval = true) public static final Object O = null;
  public static class Nested<A> { public class Inner<B> {} }
  Nested<String>.Inner<Integer> ni;
  <E extends Exception> void thrower() throws E {}
}
