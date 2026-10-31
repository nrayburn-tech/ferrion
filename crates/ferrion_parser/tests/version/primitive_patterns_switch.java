class A {
  boolean m(Object o) {
    return switch (o) {
      case int i -> true;
      default -> false;
    };
  }
}
