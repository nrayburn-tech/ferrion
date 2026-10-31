public abstract class A<T extends Number & Comparable<T>, U> extends B<T> implements C, D<U> {
  ;
  private static final long serialVersionUID = 1L, other = 2L, third;
  abstract void m() throws E1, E2;
  protected native int n(int a, final String... b) throws Exception;
  @Override public abstract String toString();
  public <V> @A String q();
  abstract void r(A this, int x);
  abstract int[] s() @A [] [];
  abstract void t(@A A A.this);
  interface I { int C = 1; void m(); <T> void g(); }
  public @interface Ann { int v() default 1; String[] w() default {"a", "b"}; Class<?> c(); }
  @interface Bare {}
  enum En implements I { X, Y(1), @Deprecated Z { }, W() { }, V(); }
  enum Empty { ; }
  record Rec<R>(@Deprecated int a, String... b) implements I {}
  sealed interface S permits F, G {}
  final class F implements S {}
  non-sealed class G implements S {}
  static class Inner {}
  @A(1) @B(x = 1, y = "s") @Cc({1, 2}) @Dd(@E) @Ff(z = {@E, @E}) @G() @p.H class Annotated {}
}
