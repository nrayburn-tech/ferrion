// Object creation: `new` and its arguments as a call's, and an anonymous class body after a
// space.
class New {
  void m() {
    x = new A<>();
    x = new <T>Foo<U>();
    x = outer.new Inner();
    x =
        new SomeVeryLongClassName(
            argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBB, argumentThree);
    x =
        new java.util.ArrayList<String>(
            argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBBBBBBBBBB);
    x =
        new Object() {
          int x;
        };
    x = new Object() {};
    foo(
        new Runnable() {
          public void run() {}
        });
    foo(
        a,
        new Runnable() {
          public void run() {}
        });
    new Foo(
        argumentNumberOneAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBBBBBBBB, argumentNumberThreeC) {
      void run() {}
    };
    throw new IllegalArgumentException(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" + variable);
  }
}
