class A {
  AutoCloseable r;
  void m() throws Exception {
    try (this.r) {}
  }
}
