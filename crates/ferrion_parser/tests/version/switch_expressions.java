class A {
  int m(int i) {
    return switch (i) {
      default: yield 1;
    };
  }
}
