class A {
  /** x {@code a b} y z */
  void codesp() {}

  /** x {@code a b} y */
  void codesplit() {}

  /**
   * word word word word word word word word word word word word word word word word word
   * aaaaaaaa @foo bar
   */
  void atwrap() {}

  /**
   * word word word word word word word word word word word word word word word word word
   * aaaaaaaa @param bar
   */
  void atwrap2() {}

  /**
   * text
   *
   * @notatag hmm
   */
  void atmid() {}

  /**
   * word word word word word word word word word word word word word word word word word aaaaa
   * {@literal @param} b
   */
  void atinline() {}

  /**
   * word word word word word word word word word word word word word word word word word aaaa
   * user@example.com bbbbbbbb
   */
  void emailwrap() {}

  /**
   * word word word word word word word word word word word word word word word word word aaaaaaaaa
   *
   * <p>bbbbbbbbbb
   */
  void ltwrap() {}

  /** a&nbsp;b &lt;c&gt; &amp; &#64; */
  void nbsp() {}
}
