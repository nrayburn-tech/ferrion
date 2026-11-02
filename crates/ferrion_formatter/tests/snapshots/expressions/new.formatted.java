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

  // A chain on an anonymous class keeps its first link on the `}`, and the body is indented
  // from the line the `new` starts on.
  void chains() {
    new Sorter(x) {
      void get() {
        run();
      }
    }.sort(0, len);
    x =
        new Sorter(x) {
          void get() {
            run();
          }
        }.sort(0, len).then(y);
    new Sorter(x) {
      void get() {
        run();
      }
    }.sort(
        argumentNumberOneAAAAAAAAAAAAAAAAAAA, argumentNumberTwoBBBBBBBBBBBBBBBBBB, cccccccccccccc);
    new Sorter(x) {
      void get() {
        run();
      }
    }.sort(argumentNumberOneAAAAAAAAAAAAAAAAAAA)
        .then(argumentNumberTwoBBBBBBBBBBBBBBBBBB)
        .then(ccccccccccccc);
    new Sorter(x) {
      void get() {
        run();
      }
    }.aaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.ccccccccccccccccccccccccc
        .ddddddddddd();
    new Sorter(x) {}.sort(argumentNumberOneAAAAAAAAAAAAAAAAAAA)
        .then(argumentNumberTwoBBBBBBBBBBBBBBBBBB)
        .then(ccccccccccccc);
    foo(
        new Sorter(x) {
          void get() {
            run();
          }
        }.sort(0, len));
  }
}
