class A {
  int m(Object o) {
    switch (o) {
      case R(int x):
        return 1;
      default:
        return 0;
    }
  }
}
