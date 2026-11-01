// The first rule's output for statements, before their own rules are written.
class A {
  int m(int a) {
    int x = 1, y;
    if (a > 0) return 1; else if (a < 0) {
      return -1;
    } else {}
    for (int i = 0; i < a; i++) x += i;
    for (String s : list) {}
    while (x > 0) x--;
    do {
      x++;
    } while (x < 10);
    label: for (;;) {
      break label;
    }
    switch (a) {
      case 1: case 2:
      f();
      break;
      default:
      g();
    }
    int z = switch (a) {
      case 1, 2 -> 3;
      case Integer i when i > 5 -> {
        yield 4;
      }
      default -> throw new E();
    };
    try (var r = open(); var s = open()) {
      f();
    } catch (E | F e) {
      g();
    } finally {
      h();
    }
    synchronized (this) {
      assert x > 0 : "positive";
    }
    ;
    return x;
  }
}
