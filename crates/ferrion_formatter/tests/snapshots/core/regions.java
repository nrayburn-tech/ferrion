class A {
  int   a  =  1;

  // @formatter:off
  int   b  =  2;
      int   c  =  3;
  // @formatter:on
  int   d  =  4;

  void m() {
    int x =   1; /* @formatter:off */ int   y  =  2;
    /* @formatter:on */ int   z  =  3;
  }

  /* @formatter:off */ static   public   int   e; /* @formatter:on */

  static /* @formatter:on */ public int f;

  //@formatter:off
  int   g; //without a space
  //@formatter:on
  //@formatter:on without an off
  //ferrion-disable-next some-rule -- a reason
  int h; //with a space
}

// An `off` without an `on` runs to the end of the file.
// @formatter:off
class   B  {   int   x;   }
