// javac parses any type after `extends` and `implements`, and rejects these only after parsing.
class A extends int[] implements B[], C<D>.E {}

class F {
  void f() throws @G H, a.b.I {}
}
