final public abstract sealed class A permits B {
  static final private int a;
  final static public transient volatile int b;
  synchronized native static protected void c();
  strictfp public void d() {}
  final /* a comment pins the modifiers around it */ static public int e;
  static @Deprecated public final int f;
  @Deprecated final @SuppressWarnings("x") public static int g;
  final static public int h, i;
  static
  public
  final int j;
}

final non-sealed public class B extends A {}

interface I {
  default public void m() {}
  static public void n() {}
  abstract public void o();
}
