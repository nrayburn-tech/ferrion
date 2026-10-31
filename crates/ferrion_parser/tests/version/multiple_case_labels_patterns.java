class A {
  int m(Object o) {
    return switch (o) {
      case Integer _, String _ -> 1;
      default -> 0;
    };
  }
}
