class A {
  int m(Object o) {
    return switch (o) {
      case _ -> 1;
    };
  }
}
