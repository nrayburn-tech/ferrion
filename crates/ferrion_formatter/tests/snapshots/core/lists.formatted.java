// Comments in delimited lists of every kind: before the first item, after a separator, and
// after the last item.
class Lists {
  void p1(
      // first
      int a, int b) {}

  void p2(int a, /* c */ int b) {}

  void p3(
      int aaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* c */
      int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) {}

  void p4(int a, int b // trailing b
      ) {}

  void p6(
      int aaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
      /* name= */ int bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb) {}

  void p7(
      int a, // after comma
      int b) {}

  void call() {
    foo(
        // first
        a, b);
    foo(
        a, b // trailing b
        );
    foo(
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* c */
        bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
    foo(
        a, // after comma
        b);
    Map<
            // first
            A, B>
        m;
    Map<A, B // trailing B
        >
        n;
    Map<
            Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, /* c */
            Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb>
        o;
    Map<
            A, // after comma
            B>
        q;
  }

  @A(
      // first
      a = 1,
      b = 2)
  @B(
      a = 1,
      b = 2 // trailing b
      )
  @C(
      aaaaaaaaaaaaaaaaaaaaaaaaaaaaa = 1, /* c */
      bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb = 2)
  @D(
      a = 1, // after comma
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
        aaaaaaaaaa, bbbbbbbbbb);
    String[] s = {
      /*
       * a block comment
       */
      "aaaaaaaaaa", "bbbbbbbbbb"
    };
    String[] t = {
      // a line comment
      "aaaaaaaaaa", "bbbbbbbbbb"
    };
  }

  void p(
      /*
       * a block comment
       */
      int a, int b) {}

  // A block comment on one line joins the lines around it unless it is on a line of its own,
  // and one naming a parameter joins them always. The space after a comma and the one before a
  // comment on its line count once.
  void joined() {
    foo(a, /* c */ b);
    foo(/* c */ a, b);
    foo(a, /* c */ b);
    foo(a /* c */, b);
    foo(a, b /* c */);
    foo(a, /* c= */ b);
    int[] y = {
      1,
      /* on its own line */
      2
    };
    x = a /* c */ + b;
    empty.scheduleRebalancing(
        0, /* bufferCeiling= */ 0, /* primaryReservedLimit= */ 0, /* historyBatchSize= */ 1000);
    report(up, /* bufferCap= */ 4095, /* bufferUses= */ 0, /* coreUses= */ 0, /* faults= */ 10000);
    @A(a = 1, /* c */ b = 2)
    int q;
  }

  void n(int a, /* c */ int b) {}

  // A blank line after a comment on a separator's line is removed, unless a comment follows.
  void blankLines() {
    foo(
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, // c
        bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
    foo(
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, // c

        // d
        bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb);
    int[] x = {
      1, // c
      2
    };
  }

  // A comment on a line of its own before an item after the first puts narrow items one per
  // line.
  int[] ownLine = {
    0x0D,
    0x0A,
    0x0D, // header
    // Addresses
    0x21,
    0x22
  };

  void ownLineArguments() {
    foo(
        1,
        2,
        3,
        /* c */
        4,
        5);
  }

  // A comment on a comma's line trails the comma, even with an item after it on the line; one
  // that starts an item's line counts toward the item's width.
  Object trailsComma = {
    d, //
    4, /**/ 9
  };
  Object trailsCommaWide = {
    d, //
    4, /*cccccccccccc*/ 9, 10
  };
  Object startsLineWide = {
    d, //
    4,
    /*cccccccccccc*/ 9,
    10
  };
}
