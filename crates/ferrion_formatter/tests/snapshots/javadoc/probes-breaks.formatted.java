class A {
  /**
   * Summary.
   *
   * @return desc after blank
   */
  int a() {}

  /**
   * Text.
   * <!-- c -->
   * More. <br>
   * end
   */
  void b() {}

  /**
   * <li>a text
   *
   *     <ul>
   *       <li>b
   *     </ul>
   *
   *     after
   */
  void d() {}

  /**
   * Text
   * </ul>
   *
   * stray close
   */
  void e() {}

  /** two leading */
  void f() {}

  /**
   * Text.
   *
   * @deprecated use
   *     <ul>
   *       <li>other
   *     </ul>
   */
  void g() {}

  /**
   * Text
   *
   * <ul>
   *   <li>one
   * </ul>
   *
   * <pre>
   * x
   * </pre>
   *
   * <ul>
   *   <li>two
   * </ul>
   */
  void h() {}
}
