class A {
  /** Escape \u0041 and \u00e9t\u00e9 more. */
  void a() {}

  /** {@inheritDoc} More text. */
  void b() {}

  /**
   * Example:
   *
   * {@snippet :
   *   int   x = 1;
   *     if (x) { y(); }
   * }
   *
   * After snippet.
   */
  void c() {}

  /**
   * Inline
   *
   * {@snippet file="a.java" region="r"}
   *
   * text here.
   */
  void d() {}

  /**
   * {@snippet lang=java :
   * // @highlight substring="x"
   * int x;
   * }
   */
  void e() {}

  /** {@code multi line code } */
  void f() {}

  /** {@literal a b} {@value #X} {@index term desc} {@summary Sum mary.} */
  void g() {}
}
