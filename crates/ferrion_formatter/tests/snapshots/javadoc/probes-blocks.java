class A {
  /**
   * Text <!-- c -->
   *
   * After comment.
   * <h3>H</h3>
   *
   * After heading.
   * <blockquote>q</blockquote>
   *
   * After quote.
   * <table><tr><td>x</td></tr></table>
   *
   * After table.
   * {@snippet : x }
   *
   * After snippet.
   */
  void a() {}

  /**
   * <ul><li>x
   * @return y
   *     more
   */
  int b() {}

  /**
   * <pre>
   * x
   * </pre>
   * <p>Text after p.
   */
  void d() {}

  /**
   * <ul>
   * <li>a
   * <table><tr><td>x</td></tr></table>
   * <h3>H</h3>
   * {@snippet : y }
   * <blockquote>q</blockquote>
   * </ul>
   */
  void e() {}

  /**
   * Text <b>bold
   *
   * </b> after.
   */
  void f() {}

  /**
   * Line one<br>
   * <ul><li>x</ul>
   * <br>
   * end
   */
  void g() {}

  /**
   * @param a <ul><li>x</ul> after list
   * @param b <pre>x</pre> after pre
   */
  void h() {}

  /**
   * <ol>
   *   <li>one
   *   <li>two
   * </ol>
   * <ol>
   * <li>three
   * </ol>
   */
  void i() {}

  /**
   *
   * <p>
   *
   * Lead blanks then p.
   */
  void j() {}

  /** <p>One line p. */
  void k() {}

  /**
   * <!-- c --> Text.
   */
  void l() {}

  /**
   * Text<p>glued p.
   */
  void m() {}

  /**
   * <li>x
   * @return y
   */
  int n() {}
}
