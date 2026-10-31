package p;
@A(x = 1, y = {2, @B}) public final class A<T> extends B implements C, D {
  private int x = 1, y[];
  A(int a, String... b) throws E { }
  <T> @C void f(final A this, int z[]) {}
  static { }
  { }
  ;
  enum E { X, Y(1) { }, ; int z; }
  record R<T>(int x, T... y) implements I { R { } static int q; }
  @interface N { int v() default 3; }
  sealed interface I extends J, K permits L { default void g() {} }
  non-sealed class L {}
}
