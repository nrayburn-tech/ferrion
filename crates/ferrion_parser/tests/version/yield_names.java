class A {
  int yield;
  void yield() {}
  void m() {
    int yield = 1;
    this.yield = yield;
    this.yield();
  }
}
