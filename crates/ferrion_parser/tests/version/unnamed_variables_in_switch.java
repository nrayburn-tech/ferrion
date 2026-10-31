class A {
  int m(Object o) {
    return switch (o) {
      case String _ -> 1;
      default -> 0;
    };
  }
}
