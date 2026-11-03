class A {
  /**
   * Prose one prose two spaced.
   *
   * <p>Closed code spanning <code>a   b
   *    c   d</code> word word word word word word word word word word word word word word word word
   * next line
   */
  void d() {}

  /**
   * Prose one
   * prose two   spaced.
   * <pre>
   *   unclosed
   */
  void a() {}

  /**
   * Prose one
   * prose two   spaced. {@code unclosed
   * more
   */
  void b() {}

  /**
   * Prose one
   * prose two   spaced. <table>
   * more
   */
  void c() {}

  /**
   * word word word word word word word word word word word word word word word word word <code>
   * a b c d</code>
   */
  void e() {}
}
