// `if` and `else`: blocks, bodies that aren't blocks, and `else if` chains.
class If {
  void m() {
    if (a) b();
    if (a) b();
    else c();
    if (a) {
      b();
    } else c();
    if (a) b();
    else {
      c();
    }
    if (a) {
      b();
    } else if (c) {
      d();
    } else if (e) f();
    else {
      g();
    }
    if (a) return;
    else if (b) return;
    else return;
    if (a)
      ;
    if (a)
      ;
    else
      ;
    if (a)
      if (b) c();
      else d();
    if (a) for (; ; ) b();
    if (a)
      synchronized (b) {
        c();
      }
    if (a)
      label:
      b();
    if (a) {
      b();
    } else {
      c();
    }
    d();
    if (a) bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb = c;
    if (a) b();
    else ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc = d;
    if (aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) {
      b();
    }
  }
}
