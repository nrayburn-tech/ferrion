class A {
  A(int x) {}
  B() {}
  Object o = new Object() {
    Object() {}
  };
}

interface I {
  I() {}
}

enum E {
  X;
  E() {}
  F() {}
}
