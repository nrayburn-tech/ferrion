class A {
  int a = 1;

  // @formatter:off
  int   b  =  2;
      int   c  =  3;
  // @formatter:on
  int d = 4;

  void m() {
    int x = 1;
    /* @formatter:off */ int   y  =  2;
    /* @formatter:on */ int z = 3;
  }

  /* @formatter:off */ static   public   int   e; /* @formatter:on */

  static /* @formatter:on */ public int f;
}

// An `off` without an `on` runs to the end of the file.
// @formatter:off
class   B  {   int   x;   }
