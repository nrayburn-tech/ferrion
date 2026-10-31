class A {
  int m(String o) {
    return switch (o) {
      case String s -> 1;
    };
  }
}
