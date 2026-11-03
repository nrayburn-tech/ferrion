class A {
  /**
   * @param x desc on next line
   * @return returned
   * @throws
   *     IllegalArgumentExceptionWithAVeryLongNameThatGoesOnAndOnAndOnAndOnAndOnAndOnAndOnForeverAndEver
   *     if bad
   * @see <a href="http://example.com/a/b/c">Example site</a>
   * @see "Quoted string"
   */
  int a(int x) {}

  /**
   * Summary.
   *
   * <ul>
   *   <li>x
   * </ul>
   *
   * @return r
   */
  int d() {}

  @Deprecated
  /** Between annotation */
  void e() {}

  int f;

  /** Trailing doc */

  /** A */
  /** B */
  void g() {}
}
