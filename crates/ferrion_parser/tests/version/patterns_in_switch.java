class A {
  int m(Object o) {
    switch (o) {
      case String s:
        return 1;
      default:
        return 0;
    }
  }
}
