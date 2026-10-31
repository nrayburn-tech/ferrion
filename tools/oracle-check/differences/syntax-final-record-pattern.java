class A {
  boolean f(Object o) {
    return switch (o) {
      case final R(var x) -> true;
      default -> false;
    };
  }
  record R(Object x) {}
}
