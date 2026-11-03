class A {
  void f(int a) {
    switch (a) {
      case 1 -> foo();
      default  ->   throw   new X();
    }
  }
}
