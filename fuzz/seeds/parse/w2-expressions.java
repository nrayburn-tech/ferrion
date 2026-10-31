import java.util.*;
import java.util.function.*;

class Expressions<T> {
  int[] a = {1, 2, 3,};
  Object o = new int[][] {{1}, {2}};
  Function<String, Integer> f = String::length;
  BiFunction<Integer, Integer, Integer> g = (x, y) -> x + y * 2;
  Supplier<List<String>> s = ArrayList::new;

  Object m(Object o, int i, long l) {
    i += i << 2 >> 1 >>> 3;
    i >>>= 1;
    o = i > 0 ? (Object) "a" + 'b' : i < 0 ? null : List.<String>of();
    o = (Runnable & java.io.Serializable) () -> {};
    o = o instanceof String str && !str.isEmpty();
    o = new Expressions<String>().new Inner<>(this, Expressions.this);
    o = int[].class;
    o = switch (i) { case 1 -> 2; default -> { yield 3; } };
    o = (int) -l + (i) - ++i - i--;
    o = this.<T>m(o, a[0], (l));
    return super.toString() + a.length;
  }

  class Inner<U> {
    Inner(Object... args) {}
  }
}
