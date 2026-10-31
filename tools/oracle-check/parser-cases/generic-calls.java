class A<T> {
  <U> void g() {}
  void f() {
    this.<String>g();
    java.util.List<String> l = java.util.Collections.<String>emptyList();
    Object o = new <String>A<String>() {};
    java.util.List<String> d = new java.util.ArrayList<>();
  }
}
