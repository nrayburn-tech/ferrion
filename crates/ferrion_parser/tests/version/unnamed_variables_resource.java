class A {
  void m(AutoCloseable r) throws Exception {
    try (AutoCloseable _ = r) {}
  }
}
