class A {
  int a, b[], c = 1, d[][] = {};
  String s;

  A() {}

  <T> A(T t) throws E, F {}

  int m()[] { return null; }

  <T extends Comparable<T>> @Nullable T max(T a, T b) { return a; }

  void receiver(@A A this, int x) {}

  void varargs(int first, String @A ... rest) {}

  void cStyle(int x[], final int y) {}

  abstract void noBody();

  static {}

  {}

  ;;

  class Inner {
    Inner(A A.this) {}
  }

  interface InnerInterface {}

  enum InnerEnum { X }

  record InnerRecord() {}

  @interface InnerAnnotation {}
}
