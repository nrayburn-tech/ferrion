public abstract final sealed class A permits B {
  private static final int a;
  public static final transient volatile int b;
  protected static synchronized native void c();
  public strictfp void d() {}
  final /* a comment pins the modifiers around it */ public static int e;
  static @Deprecated public final int f;
  @Deprecated final @SuppressWarnings("x") public static int g;
  public static final int h, i;
  public static final int j;
}

public final non-sealed class B extends A {}

interface I {
  public default void m() {}
  public static void n() {}
  public abstract void o();
}
