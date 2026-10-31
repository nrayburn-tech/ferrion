class A {
  static {}
  {}
  A() { super(); }
  <T> A(int x) { <String>this(); }
  void f(int[] xs, Object o) throws Exception {
    ;
    {}
    int a = 1, b, c[] = {};
    final var d = 1;
    if (a > 0) a++; else if (a < 0) a--; else {}
    assert a > 0;
    assert a > 0 : "msg";
    while (a < 10) a++;
    do a++; while (a < 20);
    for (;;) break;
    for (a = 0, b = 1; a < 3; a++, b++) continue;
    for (int i = 0, j = 1; i < j; i++) {}
    for (int x : xs) {}
    for (final var y : xs) {}
    l: for (;;) { break l; }
    m: { continue m; }
    synchronized (o) {}
    try {} finally {}
    try {} catch (RuntimeException e) {} catch (final Exception | Error e) {}
    try (java.io.Reader r = null; java.io.Reader s = null) {}
    try (r ; this.r;) {} catch (Exception e) {} finally {}
    class L {}
    record LR() {}
    enum LE { X }
    interface LI {}
    yield(1);
    this.<String>g(null);
    super.toString();
    A.super.toString();
    new A().new B() {};
    Runnable run = () -> { return; };
    throw new Exception();
  }
  java.io.Reader r;
  int k(int i) {
    switch (i) { case 1 -> k(1); case 2 -> { } default -> throw new Error(); }
    switch (i) { }
    switch (i) { case 1: case 2: { } case 3: k(2); default: }
    return switch (i) { case 1: yield 2; default: { yield 3; } };
  }
  class B {}
  <T> void g(T t) {}
}
