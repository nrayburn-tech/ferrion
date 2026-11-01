class A {
  void a(int) {}
  void b(int x,) {}
  void c(int x y) {}
  void d(int... x, int y) {}
  void e(int... x[]) {}
  void f(int x, A this) {}
  void g(A this[]) {}
  void h(public int x) {}
  void i(int x {}
  void j() throws {}
}
class B {
  void a(A a.b) {}
  void b(A this.x) {}
  void c(A... this) {}
  void d(A... this[]) {}
}
