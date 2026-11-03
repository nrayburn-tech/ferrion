package p.q;

import static p.q.A.foo;

import p.q.A.B;
import p.qq.C;

class T {
  A a;
  B b;
  C c;

  void f() {
    foo();
  }
}
