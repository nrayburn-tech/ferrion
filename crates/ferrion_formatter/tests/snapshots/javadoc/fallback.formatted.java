class A {
  /**
   * Unclosed pre.
   * <pre>
   *   int  x;
   * a   b
   * c
   */
  void a() {}

  /**
   * Unclosed code <code>a   b
   * c   d
   * e
   */
  void b() {}

  /**
   * Unclosed table <table><tr><td>a   b
   * c   d
   */
  void c() {}

  /**
   * Unclosed inline {@code a   b
   * c   d
   * e
   */
  void d() {}

  /** Braces {@code a { b c} d e} f g h */
  void e() {}

  /** Unbalanced {@code a } b } c d e */
  void f() {}

  /**
   * <pre>
   * @Override
   * @param x
   * </pre>
   */
  void g() {}

  /** Text with <code>a   b</code> closed code. */
  void h() {}
}
