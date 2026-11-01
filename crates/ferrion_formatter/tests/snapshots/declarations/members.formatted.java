class Members {

  int a;
  int b;

  int c;

  void m() {}

  void n() {}

  int d;

  class Inner {}

  int e;

  static {
    init();
  }

  int f;

  {
    init();
  }

  Members() {}

  int g;
  ;
  int h;

  // A comment before a method.
  void o() {}

  /** A doc comment. */
  void q() {}

  int i;

  /** A doc comment. */
  int j;

  int k;
  // A line comment.
  int l;
  /* A block comment. */
  int m;

  enum E {
    X
  }

  void r() {}
  ;

  void s() {}

  int t; // trailing
  int u;
}

interface I {

  void a();

  void b();

  int C = 1;
  int D = 2;

  default void e() {}

  static void f() {}
}

class Anonymous {
  Object o =
      new Object() {
        int a;

        void f() {}

        int b;
      };
}

class Empty {}

class EmptyWithComment {
  // c

}

@interface Q {
  int value() default 1;

  String[] names() default {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cc"
  };

  String[] more() default {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "cccccccccccccccc",
    "ddddddd"
  };

  int[] nums() default {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26
  };

  String text() default
      "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

  Class<?> type() default Object.class;

  Empty nested() default @Empty;

  int E = 2;

  @A
  int x();
}
