class A {
  void m(AutoCloseable c) throws Exception {
    try (var r = c) {}
  }
}
