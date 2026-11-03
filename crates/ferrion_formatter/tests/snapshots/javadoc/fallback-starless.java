class A {

  /** Short <pre> unclosed */
  void b() {}

        /**
           * Unclosed code <code>a   b
   */
  void c() {}

  /**
   * Closed code spanning <code>a   b
   *    c   d</code> word word word word word word word word word word word word word word word word
   */
  void d() {}

  /**
   * <code>x</code> and {@code <code>} and {@code </pre>} text  text
   */
  void e() {}

  /**
   * a <pre>x</pre> b <PRE>
   *  y   z
   * </PRE>
   */
  void f() {}

  /**
   * Upper <UL><LI>one<LI>two</UL> <TABLE><TR><TD>x</TD></TR></TABLE>  y
   */
  void g() {}
}
