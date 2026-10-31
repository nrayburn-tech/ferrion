class A {
  void m(AutoCloseable r) throws Exception {
    try (r;) {}
  }
}
