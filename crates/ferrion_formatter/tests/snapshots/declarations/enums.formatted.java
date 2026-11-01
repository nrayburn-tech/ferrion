class Enums {
  enum A {
    X,
    Y,
    Z
  }

  enum B {
    X,
    Y,
    Z,
  }

  enum C {
    X,
    Y,
    Z;
  }

  enum D {
    X,
    Y,
    Z,
    ;
  }

  enum E {
    X(1),
    Y(2),
    Z(3);
    final int x;

    E(int x) {
      this.x = x;
    }
  }

  enum F {
    X {
      void f() {}
    },
    Y
  }

  enum G {
    @Deprecated
    X,
    @A
    @B
    Y,
    @C(1)
    Z
  }

  enum H {
    X("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    Y
  }

  enum I {
    // leading
    X, // after x
    Y
  }

  enum J {}

  enum K {
    ;
  }

  enum L {
    X,
    Y;

    void f() {}
  }

  enum M
      implements
          Cccccccccccccccccccccccccc, Ddddddddddddddddddddddddd, Eeeeeeeeeeeeeeeeeeeeeeeeeee {
    X
  }

  enum N
      implements Cccccccccccccccccccccccccc, Ddddddddddddddddddddddddd, Eeeeeeeeeeeeeeeeeeeeeeeee {
    X
  }

  enum Ooooooooooooooooooooooooooooooooooooooooooooooooooooooo
      implements Cccccccccccccc, Dddddddddd {
    X
  }

  enum P {
    X,

    Y,
    Z
  }

  enum Q {
    X {},
    Y {
      int x;
    }
  }

  enum R {
    X /* c */,
    Y
  }

  enum S {
    X, /* c */
    Y
  }

  enum T {
    X,
    Y;

    /** doc */
    int x;
  }

  enum U {
    X,
    Y;
    private final int x = 1;
    private final int y = 2;
  }

  enum V {
    ;

    void f() {}
  }

  enum W {
    /** doc X */
    X,
    /** doc Y */
    Y
  }
}
