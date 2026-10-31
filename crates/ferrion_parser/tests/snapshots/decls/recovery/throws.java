// A `throws` clause takes names: javac expects the body at the `<`.
class A {
  void f() throws E<T> {}
  void g() throws F[] {}
}
