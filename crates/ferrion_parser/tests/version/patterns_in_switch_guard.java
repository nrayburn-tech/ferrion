class A {
  int m(Object o) {
    return switch (o) {
      case String s when s.isEmpty() -> 1;
      default -> 0;
    };
  }
}
