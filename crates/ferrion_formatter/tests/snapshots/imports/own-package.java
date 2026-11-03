package p.q;

import p.q.A;
import p.q.A.B;
import p.qq.C;
import static p.q.A.foo;

class T {
  A a;
  B b;
  C c;

  void f() {
    foo();
  }
}
