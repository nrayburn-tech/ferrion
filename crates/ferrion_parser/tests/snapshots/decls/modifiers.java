public protected private abstract static final transient volatile synchronized native strictfp class A {
  public static final int X = 1;

  @Override public @Nullable String toString() { return null; }

  transient volatile int y;

  synchronized native void n();

  strictfp void s() {}
}

sealed interface S permits T, U {}

final class T implements S {}

non-sealed class U implements S {}

abstract sealed class V permits W {}

sealed @Deprecated class X {}

interface I {
  default void d() {}
}
