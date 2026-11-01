class A {
  int m(int x) {
    return switch (x) {
      default -> {
        yield(1,);
      }
    };
  }
}
