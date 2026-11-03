class A {
  /**
   * <pre>
   * x
   * </pre>
   *
   * After blank.
   */
  void a() {}

  /**
   * Summary.
   *
   * @param x
   *     <p>desc after blank
   * @param y
   *     <p>starts with p
   * @return
   */
  void b() {}

  /**
   * <ul>
   *   <li>a text after li
   *   <li>b
   *       <p>second para
   *   <li>
   *       <p>p after li
   * </ul>
   */
  void c() {}

  /**
   * <dl>
   *   <dt>t
   *   <dd>d
   * </dl>
   */
  void d() {}

  /**
   * <ul class="x">
   *   <li class="y">a
   * </ul>
   *
   * <h2>Heading </h2>
   *
   * <P class=x>Para. Break<br clear="all">
   * here
   */
  void f() {}

  /**
   * Text
   *
   * <ul>
   *   <li>a
   * </ul>
   *
   * <blockquote>
   *
   * <blockquote>
   * x
   * </blockquote>
   *
   * </blockquote>
   */
  void g() {}

  /**
   * @param x code <code>a
   *   b</code> after
   * @return y
   *     <table><tr><td>x</td></tr></table>
   */
  void h() {}

  /** x */
  void i() {}

  /**
   * <ul>
   *   <li>a
   *       <pre>
   * code
   * </pre>
   *   <li>b
   *       <!-- c -->
   *       d
   * </ul>
   */
  void j() {}
}
