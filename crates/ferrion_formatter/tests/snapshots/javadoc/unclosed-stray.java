class A {
  /**
   * Text   a <ul><li>x
   * next   line
   */
  void ul() {}

  /**
   * Text   a <blockquote>x
   * next   line
   */
  void bq() {}

  /**
   * Text   a <!-- never closed
   * next   line
   */
  void cmt() {}

  /**
   * Text   a <a href="x">link
   * next   line
   */
  void a() {}

  /**
   * Text   a <b>bold
   * next   line
   */
  void b() {}

  /**
   * Text   a </pre> b
   * next   line
   */
  void closepre() {}

  /**
   * Text   a </code> b
   * next   line
   */
  void closecode() {}

  /**
   * Text   a </table> b
   * next   line
   */
  void closetable() {}

  /**
   * <pre>{@code a   b</pre> c   d
   * next   line
   */
  void codeInPre() {}

  /**
   * {@code <pre>} c   d
   * next   line
   */
  void preInCode() {}

  /**
   * a < b   and c > d
   * next   line
   */
  void lt() {}

  /**
   * Text   } stray { brace
   * next   line
   */
  void brace() {}

  /**
   * Text   {@foo bar   baz} x
   * next   line
   */
  void unknownInline() {}

  /**
   * <pre>   x   </pre>   y   <pre>z</pre>
   * next   line
   */
  void precode2() {}

  /**
   * <style>p  {  }</style>   x
   * next   line
   */
  void style() {}

  /**
   * <script>a  b</script>   x
   * next   line
   */
  void script() {}

  /**
   * <tt>  a  </tt>   x
   * next   line
   */
  void tt() {}

  /**
   * <code class="x">a   b</code>   x
   * next   line
   */
  void codeattr() {}

  /**
   * <samp>a   b</samp> <kbd>c   d</kbd> <var>e   f</var>
   * next   line
   */
  void samp() {}

}
