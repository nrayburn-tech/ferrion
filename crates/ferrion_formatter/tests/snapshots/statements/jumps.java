// Labels, `break`, `continue`, `return`, `throw`, `yield`, `assert`, and `synchronized`.
class Jumps {
  int m() {
    label: for (;;) { break label; }
    label2:
    for (;;) { continue label2; }
    outer: { break outer; }
    inner: x++;
    while (a) { break; }
    while (a) { continue; }
    synchronized (this) { x++; }
    synchronized (aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) { x++; }
    assert a;
    assert a : "message";
    assert aaaaaaaaaaaaaaaaaaaaa : "message message message message message message message message message";
    assert aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;
    throw new IllegalStateException();
  }

  int n() {
    return switch (a) { default: yield 1; };
  }

  void o() {
    return;
  }
}
