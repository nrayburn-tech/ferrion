class A {
  int yield(int x) { return x; }
  void m() {
    int y = yield(1);
  }
}
