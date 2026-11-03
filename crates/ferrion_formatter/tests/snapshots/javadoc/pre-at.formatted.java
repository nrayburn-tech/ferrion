class A {
  /**
   * Text a
   *
   * <pre>
   * @Override
   * void f();
   * </pre>
   *
   * Tail b
   */
  void atPre() {}

  /**
   * Text a
   *
   * <pre>{@code
   * @Override
   * void f();
   * }</pre>
   *
   * Tail b
   */
  void atPreCode() {}

  /**
   * Text a
   *
   * <pre>{@code
   *   @Override
   * }</pre>
   *
   * Tail b
   */
  void atPreCodeIndented() {}

  /** Text a {@code @Override x} Tail b */
  void atCodeNoPre() {}

  /**
   * Text a
   *
   * @param x
   *     <pre>
   * @Override
   * </pre>
   *
   * @return r
   */
  void atPreAfterTag() {}
}
