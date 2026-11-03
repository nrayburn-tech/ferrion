class A {
  /**
   * <pre>x</pre>after   text
   * more
   */
  void afterpre() {}

  /**
   * <ul><li>a<p></ul>
   * more
   */
  void pBeforeEnd() {}

  /**
   * Line<br>
   *
   * Next   para<br><br>double
   */
  void brEnd() {}

  /**
   * Text <!-- <pre> -->   more
   * next
   */
  void cmtPre() {}

  /**
   * Text {@code <!--}   more
   * next -->  x
   */
  void codeCmt() {}

  /**
   * <table><tr><td><ul><li>a</td></tr></table>   x
   * y
   */
  void liTable() {}

  /**
   * <pre>
   * a
   * <p>
   * b
   * </pre>
   */
  void pInPre() {}

  /**
   * <pre><pre>a</pre>   b</pre>   c
   */
  void preNest() {}

  /**
   * <pre>{@code
   * @param x
   * }</pre>
   * @return r
   */
  void codeBlockTagInPre() {}

  /**
   * <ul>
   * <li>
   * <li></li>
   * </ul>
   */
  void emptyLi() {}

  /**
   * <blockquote>
   * <pre>
   * x  y
   * </pre>
   * </blockquote>
   */
  void nestedPInBq() {}

}
