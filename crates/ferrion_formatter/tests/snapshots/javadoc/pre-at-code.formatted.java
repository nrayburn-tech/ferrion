class A {
  /**
   * Text a
   *
   * <pre>{@code
   *   @Override
   *   void f();
   * }</pre>
   */
  void v1() {}

  /**
   * Text a
   *
   * <pre>
   *   @Override
   *   void f();
   * </pre>
   */
  void v2() {}

  /**
   * Text a
   *
   * <pre>{@code
   *   x @Override
   *     @Foo y
   * }</pre>
   */
  void v3() {}

  /** Text a {@code @Override void f(); } */
  void v4() {}

  /**
   * Text   a
   * <pre>{@code
   * @param x
   * }</pre>
   * @return r
   */
  void v5() {}

  /**
   * Text   a
   * <pre>{@code
   *   @param x
   * }</pre>
   * Tail   b
   */
  void v6() {}

  /**
   * Text a
   *
   * <pre>{@literal
   *   @Override
   * }</pre>
   */
  void v7() {}
}
