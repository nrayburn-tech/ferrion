class ClassHeaders {
  class A extends Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb implements Ccccccccccccccc, Dddddddd {}
  class B extends Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb {}
  class C implements Cccccccccccccccccccccccccc, Ddddddddddddddddddddddddd, Eeeeeeeeeeeeeeeeeeeeeeeeeeee {}
  class D implements Cccccccccccccccccccccccccc, Ddddddddddddddddddddddddd, Eeeeeeeeeeeeeeeeeeeeeeeeee {
    int x;
  }
  class E<Tttttttttttttttttttt extends Comparable<Tttttttttttttttttttt>, Uuuuuuuuuuuuuuuuuuuuuuu> extends Bbbbbbbbbbbbbbb<Tttttttttttttttttttt> implements Cccccccccccccc {}
  public abstract static class F extends Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb implements Cccccccc {
    int x;
  }
  class G<Tttttttttttttttttttttttttttttttttttt, Uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuu> extends B {}
  interface H extends Cccccccccccccccccccccccccc, Ddddddddddddddddddddddddd, Eeeeeeeeeeeeeeeeeeeeeeeeeee {}
  sealed interface I permits Cccccccccccccccccccccccccc, Ddddddddddddddddddddddddd, Eeeeeeeeeeeeeeeeeee {}
  sealed class J extends Bbbbbbbbbbbbbbbbbbbbb implements Cccccccccccccc permits Ddddddddddddd, Eeeeeeeeeeeee {}
  non-sealed class K extends J {}
  public static final class L extends Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa<Bbbbbbbbbbbbbbbb> {}
  class M extends B /* between clauses */ implements C {}
  class N
  {
    int x;
  }
  class O { /* dangling */ }
  class P {
    // only a comment
  }
  class Q<T> {}
  @interface R {}
}
