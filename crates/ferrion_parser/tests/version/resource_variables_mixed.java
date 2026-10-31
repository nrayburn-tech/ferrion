class A {
  void m(AutoCloseable r) throws Exception {
    try (AutoCloseable s = r; r) {}
  }
}
