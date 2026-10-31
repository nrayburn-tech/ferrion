// releases: 20 21
class A {
  int f(Object o, int i) {
    switch (i) {
      case 1: case 2: break;
      default:
    }
    int y = switch (o) {
      case Integer n when n > 0 -> n;
      case R(int x, var z) -> x;
      case String s -> { yield s.length(); }
      case null, default -> 0;
    };
    return y;
  }
  record R(int x, Object z) {}
}
