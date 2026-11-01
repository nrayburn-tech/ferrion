// Annotations of every kind, type annotations on every target outside code, generics, nested,
// local, and anonymous classes, records, enums, sealed hierarchies, and constants.

import java.lang.annotation.*;
import java.util.*;

@Retention(RetentionPolicy.CLASS)
@Target({ElementType.TYPE_USE, ElementType.TYPE_PARAMETER})
@interface TA {
  int value() default 0;
}

@Retention(RetentionPolicy.RUNTIME)
@interface RA {
  String s() default "x\0y\n";
  Class<?> c() default List.class;
  RetentionPolicy e() default RetentionPolicy.SOURCE;
  int[] a() default {1, 2};
  RA2 n() default @RA2;
  char ch() default 'q';
  double d() default 1.5;
  float f() default Float.NaN;
  long l() default Long.MIN_VALUE;
  boolean z() default true;
  byte b() default -1;
  short sh() default 3;
  Class<?>[] k() default {int.class, void.class, String[][].class};
  String t() default "tab\there\"q\" back\\ apostrophe\' u\u0001 e\u00e9 del\u007f";
}

@interface RA2 {}

public class Features<@TA(1) T extends Comparable<? super T> & java.io.Serializable>
    extends AbstractList<@TA(2) String> implements @TA(3) RandomAccess {
  public static final String S = "hi\u00e9\ud83d\ude00 \t\"\\";
  public static final String LONE = "\ud800";
  protected static final long L = 5L;
  static final double D = Double.NaN;
  static final double MIN = Double.MIN_VALUE;
  static final double BIG = 1e300;
  static final float F = -0.0f;
  static final float SMALL = 1.0e-5f;
  static final char C = '\u0001';
  static final boolean Z = true;
  static final byte B = -3;
  volatile transient int vt;

  @Deprecated private @TA(4) List<@TA(5) String @TA(6) []>[] f;
  Map.@TA Entry<@TA String, ? extends @TA Object> @TA [][] nested;
  Inner.@TA Deep<String> deep;

  @RA(s = "q", a = {}, n = @RA2)
  public <@TA(7) U extends @TA(8) Number> U m(@RA final @TA(9) int x, U u)
      throws java.io.@TA(10) IOException, RuntimeException {
    return u;
  }

  public String get(int i) { return null; }
  public int size() { return 0; }
  static void varargs(int[]... a) {}
  synchronized static native void nat();
  public final synchronized void fs() {}

  class Inner {
    Inner(@RA int a) {}
    void r(@TA Inner this) {}
    class Deep<X> {}
  }

  enum E { A, B { void f() {} }; E() {} void f() {} }

  record R(@TA @RA int x, List<String> y) {}

  sealed interface Sh permits P1, P2 {}
  final class P1 implements Sh {}
  non-sealed class P2 implements Sh {}

  void local() {
    class Loc {}
    Runnable r = new Runnable() { public void run() {} };
  }

  static { System.out.println(); }
}
