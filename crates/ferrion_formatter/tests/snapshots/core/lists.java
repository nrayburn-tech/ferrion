// Comments in delimited lists of every kind: before the first item, after a separator, and
// after the last item.
class Lists {
  void p1(
      // first
      int a, int b) {}

  void p2(int a, /* c */ int b) {}

  void p3(int aaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* c */ int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) {}

  void p4(int a, int b // trailing b
  ) {}

  void p6(int aaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* name= */ int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) {}

  void p7(int a, // after comma
      int b) {}

  void call() {
    foo(
        // first
        a, b);
    foo(a, b // trailing b
    );
    foo(aaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* c */ bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
    foo(a, // after comma
        b);
    Map<
        // first
        A, B> m;
    Map<A, B // trailing B
    > n;
    Map<Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* c */ Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb> o;
    Map<A, // after comma
        B> q;
  }

  @A(
      // first
      a = 1, b = 2)
  @B(a = 1, b = 2 // trailing b
  )
  @C(aaaaaaaaaaaaaaaaaaaaaaaaaaaaa = 1, /* c */ bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb = 2)
  @D(a = 1, // after comma
      b = 2)
  int f;

  record R(
      // first
      int a, int b) {}

  record S(int a, int b // trailing b
  ) {}

  // Comments before the first item.
  @SuppressWarnings({
    // a line comment
    "unchecked",
    "BooleanLiteral"
  })
  @SuppressWarnings({
    /*
     * a block comment
     */
    "unchecked",
    "BooleanLiteral"
  })
  @SuppressWarnings({
    /* a one-line block comment */
    "unchecked",
    "BooleanLiteral"
  })
  void m() {
    foo(
        /*
         * a block comment
         */
        aaaaaaaaaa,
        bbbbbbbbbb);
    String[] s = {
      /*
       * a block comment
       */
      "aaaaaaaaaa",
      "bbbbbbbbbb"
    };
    String[] t = {
      // a line comment
      "aaaaaaaaaa",
      "bbbbbbbbbb"
    };
  }

  void p(
      /*
       * a block comment
       */
      int a, int b) {}
}
