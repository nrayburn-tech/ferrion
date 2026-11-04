// A `;` after an enum's or annotation type's `}` joins its line, and goes on a line of its own
// after any other body, or after a comment.
class T {
  enum A {
    X
  };

  enum B {
    X;
    int y;
  };

  @interface C {
    int x();
  };

  enum D {
    X
  } // c
  ;

  class E {
    int x;
  }
  ;

  interface F {
    int x = 1;
  }
  ;

  record G(int a) {
    int f() {
      return 1;
    }
  }
  ;

  void m() {
    x();
  }
  ;
}

enum H {
  X
};

class I {
  int x;
}
;
