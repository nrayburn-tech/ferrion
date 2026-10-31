class A {
  class B {
    static int f;
    static final int CONSTANT = 1;
    static final Object g = null;
    static void m() {}
    static {}
    static class C {}
    interface I {}
    enum E { X }
  }
  Object o = new Object() {
    static int h;
  };
  void m() {
    class L {
      static int i;
    }
  }
  static class S {
    static int j;
  }
}
interface J {
  class K {
    static int k;
  }
}
enum F {
  X {
    static int l;
  };
  class G {
    static int n;
  }
}
