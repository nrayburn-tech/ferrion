class A {
  boolean f(Object o) {
    return switch (o) {
      case int i -> true;
      default -> false;
    };
  }
}
