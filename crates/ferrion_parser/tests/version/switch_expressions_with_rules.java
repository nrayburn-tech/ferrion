class A {
  int m(int i) {
    return switch (i) {
      case 1, 2 -> 1;
      default -> 2;
    };
  }
}
