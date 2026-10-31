enum E {
  A, B(1), C { void f() {} }, @Deprecated D;
  E() {}
  E(int x) {}
}
